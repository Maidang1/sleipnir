//! GPUI global that owns the process-wide Ledger and its debounced writer.

use gpui::{App, BorrowAppContext, Global, Task};
use run_ledger::{
    default_runs_path, load_runs, save_runs, Badge, LaunchId, Ledger, PaneKey, Retention, Run,
    RunEvent,
};
use sleipnir_settings::{RunLedgerMode, TerminalSettings};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FLUSH_DELAY: Duration = Duration::from_secs(2);

const FIRST_PERSIST_NOTICE: &str = "Sleipnir 开始在 runs.json 里记录你跑过的命令（脱敏后的命令行 + 耗时 + 退出码，不含输出）。设置 run_ledger 可关闭。";

pub struct RunLedgerGlobal {
    core: LedgerCore,
    started_at: Instant,
    _flush: Task<()>,
}

impl Global for RunLedgerGlobal {}

/// Mode + disk logic, testable without GPUI.
struct LedgerCore {
    ledger: Ledger,
    mode: RunLedgerMode,
    path: PathBuf,
    dirty: bool,
    announced: bool,
    retention: Retention,
    redact: bool,
    success_threshold_secs: u64,
}

#[derive(Debug)]
enum FlushOutcome {
    Skipped,
    Wrote { first_announce: bool },
    Failed,
}

impl LedgerCore {
    fn new(path: PathBuf, mode: RunLedgerMode, retention: Retention, redact: bool, threshold: u64) -> Self {
        let mut core = Self {
            ledger: Ledger::new(LaunchId::new_v4()),
            mode,
            path,
            dirty: false,
            announced: false,
            retention,
            redact,
            success_threshold_secs: threshold,
        };
        core.sync_ledger_settings();
        if mode == RunLedgerMode::Persist {
            core.load_from_disk();
        }
        core
    }

    fn sync_ledger_settings(&mut self) {
        self.ledger.set_redact(self.redact);
        self.ledger.set_retention(self.retention);
        self.ledger.set_success_threshold_secs(self.success_threshold_secs);
    }

    fn reset_ledger(&mut self) {
        self.ledger = Ledger::new(LaunchId::new_v4());
        self.sync_ledger_settings();
    }

    fn load_from_disk(&mut self) {
        let (runs, announced) = load_runs(&self.path);
        self.announced = announced;
        self.ledger.load_history(runs);
    }

    fn apply(&mut self, event: RunEvent) {
        if self.mode == RunLedgerMode::Off {
            return;
        }
        self.ledger.apply(event);
        self.dirty = true;
    }

    fn set_mode(&mut self, mode: RunLedgerMode) {
        if mode == self.mode {
            return;
        }
        let prev = self.mode;
        self.mode = mode;
        match (prev, mode) {
            (_, RunLedgerMode::Off) => {
                self.reset_ledger();
                self.dirty = false;
            }
            (RunLedgerMode::Off, RunLedgerMode::Persist) => {
                self.load_from_disk();
            }
            (RunLedgerMode::Memory, RunLedgerMode::Persist) => {
                self.load_from_disk();
                self.dirty = true;
            }
            (RunLedgerMode::Persist, RunLedgerMode::Memory)
            | (RunLedgerMode::Off, RunLedgerMode::Memory) => {
                // Keep memory (or stay empty); stop writing.
            }
            _ => {}
        }
    }

    fn configure(&mut self, retention: Retention, redact: bool, threshold: u64) {
        self.retention = retention;
        self.redact = redact;
        self.success_threshold_secs = threshold;
        self.sync_ledger_settings();
    }

    fn flush(&mut self) -> FlushOutcome {
        if self.mode != RunLedgerMode::Persist || !self.dirty {
            return FlushOutcome::Skipped;
        }
        let first_announce = !self.announced;
        let announced = true;
        let runs: Vec<Run> = self.ledger.runs().cloned().collect();
        match save_runs(&self.path, &runs, self.retention, announced) {
            Ok(()) => {
                self.announced = announced;
                self.dirty = false;
                FlushOutcome::Wrote { first_announce }
            }
            Err(err) => {
                log::warn!("run ledger write failed ({}); falling back to memory: {err}", self.path.display());
                self.mode = RunLedgerMode::Memory;
                FlushOutcome::Failed
            }
        }
    }

    fn clear(&mut self) {
        self.reset_ledger();
        self.dirty = false;
        if self.mode == RunLedgerMode::Persist {
            if let Err(err) = std::fs::remove_file(&self.path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("failed to delete {}: {err}", self.path.display());
                }
            }
        }
    }
}

impl RunLedgerGlobal {
    pub fn init(cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let settings = TerminalSettings::get_global(cx);
        let path = default_runs_path(&sleipnir_settings::config_dir());
        let core = LedgerCore::new(
            path,
            settings.run_ledger,
            Retention {
                days: settings.run_ledger_retention_days,
                max_runs: settings.run_ledger_max_runs,
            },
            settings.run_ledger_redact,
            settings.notify_on_command_finish_secs,
        );
        cx.set_global(Self {
            core,
            started_at: Instant::now(),
            _flush: Task::ready(()),
        });
    }

    pub fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn apply(&mut self, event: RunEvent, cx: &mut App) {
        self.core.apply(event);
        if self.core.mode == RunLedgerMode::Persist && self.core.dirty {
            self.schedule_flush(cx);
        }
    }

    pub fn set_mode(&mut self, mode: RunLedgerMode, cx: &mut App) {
        self.core.set_mode(mode);
        if self.core.mode == RunLedgerMode::Persist && self.core.dirty {
            self.schedule_flush(cx);
        }
    }

    pub fn reload_settings(&mut self, cx: &mut App) {
        let settings = TerminalSettings::get_global(cx);
        self.core.configure(
            Retention {
                days: settings.run_ledger_retention_days,
                max_runs: settings.run_ledger_max_runs,
            },
            settings.run_ledger_redact,
            settings.notify_on_command_finish_secs,
        );
        self.set_mode(settings.run_ledger, cx);
    }

    fn schedule_flush(&mut self, cx: &mut App) {
        self._flush = cx.spawn(async move |cx| {
            cx.background_executor().timer(FLUSH_DELAY).await;
            cx.update(|cx| {
                if cx.has_global::<RunLedgerGlobal>() {
                    cx.global_mut::<RunLedgerGlobal>().flush_now();
                }
            });
        });
    }

    pub fn flush_now(&mut self) {
        if let FlushOutcome::Wrote { first_announce: true } = self.core.flush() {
            log::info!("{FIRST_PERSIST_NOTICE}");
        }
    }

    pub fn clear(&mut self, _cx: &mut App) {
        self.core.clear();
    }

    pub fn clear_in(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, cx| this.clear(cx));
    }

    pub fn badge_for(&self, panes: &[PaneKey], now_ms: u64) -> Option<Badge> {
        if self.core.mode == RunLedgerMode::Off {
            return None;
        }
        self.core.ledger.badge_for(panes, now_ms)
    }

    pub fn mode(&self) -> RunLedgerMode {
        self.core.mode
    }

    pub fn launch_id(&self) -> LaunchId {
        self.core.ledger.launch_id()
    }

    pub fn snapshot(&self) -> Vec<Run> {
        if self.core.mode == RunLedgerMode::Off {
            return Vec::new();
        }
        self.core.ledger.snapshot()
    }

    pub fn failed_attention_count(&self) -> usize {
        if self.core.mode == RunLedgerMode::Off {
            return 0;
        }
        self.core.ledger.failed_attention_count()
    }

    pub fn attention_count(&self) -> usize {
        if self.core.mode == RunLedgerMode::Off {
            return 0;
        }
        self.core.ledger.attention().count()
    }

    pub fn pane_has_attention(&self, pane: PaneKey) -> bool {
        if self.core.mode == RunLedgerMode::Off {
            return false;
        }
        self.core.ledger.attention().any(|run| run.pane == pane)
    }

    pub fn pane_has_failed_attention(&self, pane: PaneKey) -> bool {
        if self.core.mode == RunLedgerMode::Off {
            return false;
        }
        self.core
            .ledger
            .attention()
            .any(|run| run.pane == pane && run.state == run_ledger::RunState::Failed)
    }

    pub fn mark_run_seen(&mut self, id: run_ledger::RunId) {
        self.core.ledger.mark_run_seen(id);
    }

    pub fn set_focus(&mut self, pane: Option<PaneKey>, window_active: bool) {
        self.core.ledger.set_focus(pane, window_active);
    }

    pub fn mark_pane_seen(&mut self, pane: PaneKey) {
        self.core.ledger.mark_pane_seen(pane);
    }

    pub fn apply_in(cx: &mut App, event: RunEvent) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, cx| this.apply(event, cx));
    }

    pub fn flush_now_in(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, _cx| this.flush_now());
    }

    pub fn reload_settings_in(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, cx| this.reload_settings(cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_ledger::RunState;
    use std::fs;
    use uuid::Uuid;

    fn pane() -> PaneKey {
        Uuid::new_v4()
    }

    fn now_ms() -> u64 {
        1_700_000_000_000
    }

    fn write_failed_run(path: &std::path::Path, pane: PaneKey, command: &str) {
        let value = serde_json::json!({
            "version": 1,
            "announced": false,
            "runs": [{
                "id": Uuid::new_v4(),
                "launch_id": Uuid::new_v4(),
                "pane": pane,
                "command": command,
                "started_at_unix_ms": now_ms(),
                "duration": { "secs": 1, "nanos": 0 },
                "exit_code": 1,
                "state": "failed",
            }]
        });
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn mode_off_drops_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let mut core = LedgerCore::new(path, RunLedgerMode::Off, Retention::default(), true, 5);
        core.apply(RunEvent::started(pane(), "ls", None, 0));
        assert_eq!(core.ledger.runs().count(), 0);
        assert!(!core.dirty);
    }

    #[test]
    fn mode_memory_never_touches_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let p = pane();
        write_failed_run(&path, p, "old");
        let original = fs::read(&path).unwrap();
        let mut core = LedgerCore::new(
            path.clone(),
            RunLedgerMode::Memory,
            Retention::default(),
            true,
            5,
        );
        assert_eq!(core.ledger.runs().count(), 0, "Memory must not read disk");
        core.apply(RunEvent::started(pane(), "cargo test", None, 0));
        assert_eq!(core.ledger.runs().count(), 1);
        assert!(matches!(core.flush(), FlushOutcome::Skipped));
        assert_eq!(fs::read(&path).unwrap(), original, "Memory must not write disk");
    }

    #[test]
    fn switching_to_off_clears_memory_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let mut core = LedgerCore::new(
            path.clone(),
            RunLedgerMode::Persist,
            Retention::default(),
            true,
            5,
        );
        core.apply(RunEvent::started(pane(), "sleep 1", None, 0));
        assert!(matches!(core.flush(), FlushOutcome::Wrote { .. }));
        let on_disk = fs::read(&path).unwrap();
        assert!(!on_disk.is_empty());
        core.set_mode(RunLedgerMode::Off);
        assert_eq!(core.ledger.runs().count(), 0);
        assert_eq!(fs::read(&path).unwrap(), on_disk);
    }

    #[test]
    fn switching_to_persist_loads_history_as_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let p = pane();
        write_failed_run(&path, p, "false");
        let mut core = LedgerCore::new(
            path,
            RunLedgerMode::Off,
            Retention::default(),
            true,
            5,
        );
        assert_eq!(core.ledger.runs().count(), 0);
        core.set_mode(RunLedgerMode::Persist);
        assert_eq!(core.ledger.runs().count(), 1);
        assert_eq!(core.ledger.runs().next().unwrap().state, RunState::Failed);
        assert_eq!(core.ledger.attention().count(), 0, "loaded history is seen");
    }

    #[test]
    fn first_persist_write_notifies_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let mut core = LedgerCore::new(
            path.clone(),
            RunLedgerMode::Persist,
            Retention::default(),
            true,
            5,
        );
        core.apply(RunEvent::started(pane(), "echo once", None, 0));
        match core.flush() {
            FlushOutcome::Wrote { first_announce } => assert!(first_announce),
            other => panic!("expected first write, got {other:?}"),
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"announced\": true"), "{text}");
        core.apply(RunEvent::started(pane(), "echo twice", None, 10));
        match core.flush() {
            FlushOutcome::Wrote { first_announce } => assert!(!first_announce),
            other => panic!("expected second write, got {other:?}"),
        }
    }
}
