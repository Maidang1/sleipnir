//! GPUI global that owns the process-wide Run Ledger as a pure in-memory
//! fact registry.
//!
//! The product surface (panel, grouping, persistence) moved out of core to
//! the Run Ledger plugin. What stays here is what the rest of core still
//! reads: the tab Failed red wash, the Dock badge counts, run_id → pane
//! routing for plugin dispatch, `RenderTarget::Block` anchor lookup, and the
//! run_id allocation behind `ScrollToRun` and the RunStarted/RunFinished
//! host events. `RunLedgerMode::Persist` is still accepted from settings for
//! compatibility, but in core it behaves exactly like `Memory` — the plugin
//! owns runs.json now.

use gpui::{App, BorrowAppContext, Global};
use run_ledger::{Badge, LaunchId, Ledger, PaneKey, Retention, Run, RunEvent};
use sleipnir_settings::{RunLedgerMode, TerminalSettings};
use std::time::Instant;

/// Prune the in-memory ledger every N applied events so a long session does
/// not grow without bound. The plugin owns real disk retention.
const PRUNE_EVERY: u64 = 64;

pub struct RunLedgerGlobal {
    core: LedgerCore,
    started_at: Instant,
}

impl Global for RunLedgerGlobal {}

/// Mode + ledger, testable without GPUI.
struct LedgerCore {
    ledger: Ledger,
    mode: RunLedgerMode,
    redact: bool,
    success_threshold_secs: u64,
    since_prune: u64,
}

impl LedgerCore {
    fn new(mode: RunLedgerMode, redact: bool, threshold: u64) -> Self {
        let mut core = Self {
            ledger: Ledger::new(LaunchId::new_v4()),
            mode,
            redact,
            success_threshold_secs: threshold,
            since_prune: 0,
        };
        core.sync_ledger_settings();
        core
    }

    fn sync_ledger_settings(&mut self) {
        self.ledger.set_redact(self.redact);
        self.ledger
            .set_success_threshold_secs(self.success_threshold_secs);
        // In-memory bound only; disk retention is the plugin's business.
        self.ledger.set_retention(Retention::default());
    }

    fn reset_ledger(&mut self) {
        self.ledger = Ledger::new(LaunchId::new_v4());
        self.sync_ledger_settings();
    }

    fn apply(&mut self, event: RunEvent) {
        if self.mode == RunLedgerMode::Off {
            return;
        }
        self.ledger.apply(event);
        self.since_prune += 1;
        if self.since_prune >= PRUNE_EVERY {
            self.since_prune = 0;
            self.ledger.prune();
        }
    }

    fn set_mode(&mut self, mode: RunLedgerMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        if mode == RunLedgerMode::Off {
            self.reset_ledger();
        }
    }

    fn configure(&mut self, redact: bool, threshold: u64) {
        self.redact = redact;
        self.success_threshold_secs = threshold;
        self.sync_ledger_settings();
    }
}

impl RunLedgerGlobal {
    pub fn init(cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let settings = TerminalSettings::get_global(cx);
        let core = LedgerCore::new(
            settings.run_ledger,
            settings.run_ledger_redact,
            settings.notify_on_command_finish_secs,
        );
        cx.set_global(Self {
            core,
            started_at: Instant::now(),
        });
    }

    pub fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn apply(&mut self, event: RunEvent) {
        self.core.apply(event);
    }

    pub fn set_mode(&mut self, mode: RunLedgerMode) {
        self.core.set_mode(mode);
    }

    pub fn reload_settings(&mut self, cx: &mut App) {
        let settings = TerminalSettings::get_global(cx);
        self.core.configure(
            settings.run_ledger_redact,
            settings.notify_on_command_finish_secs,
        );
        self.set_mode(settings.run_ledger);
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

    pub fn rebase_anchors(&mut self, pane: PaneKey, removed: i32) {
        if self.core.mode == RunLedgerMode::Off {
            return;
        }
        self.core.ledger.rebase_anchors(pane, removed);
    }

    pub fn clear_anchors(&mut self, pane: PaneKey) {
        if self.core.mode == RunLedgerMode::Off {
            return;
        }
        self.core.ledger.clear_anchors(pane);
    }

    pub fn apply_block_anchor_change(
        &mut self,
        pane: PaneKey,
        change: &terminal::BlockAnchorChange,
    ) {
        match change {
            terminal::BlockAnchorChange::Rebase(removed) => self.rebase_anchors(pane, *removed),
            terminal::BlockAnchorChange::Invalidate => self.clear_anchors(pane),
        }
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
        cx.update_global(|this: &mut RunLedgerGlobal, _cx| this.apply(event));
    }

    pub fn reload_settings_in(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, cx| this.reload_settings(cx));
    }

    pub fn rebase_anchors_in(cx: &mut App, pane: PaneKey, removed: i32) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, _cx| this.rebase_anchors(pane, removed));
    }

    pub fn clear_anchors_in(cx: &mut App, pane: PaneKey) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, _cx| this.clear_anchors(pane));
    }

    pub fn apply_block_anchor_change_in(
        cx: &mut App,
        pane: PaneKey,
        change: &terminal::BlockAnchorChange,
    ) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.update_global(|this: &mut RunLedgerGlobal, _cx| {
            this.apply_block_anchor_change(pane, change)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_ledger::RunState;
    use uuid::Uuid;

    fn pane() -> PaneKey {
        Uuid::new_v4()
    }

    fn failed_run(core: &mut LedgerCore, pane: PaneKey, command: &str) {
        core.ledger.apply(RunEvent::started(pane, command, None, 0));
        core.ledger.apply(RunEvent::finished(pane, Some(1), 10));
    }

    #[test]
    fn mode_off_drops_events() {
        let mut core = LedgerCore::new(RunLedgerMode::Off, true, 5);
        core.apply(RunEvent::started(pane(), "ls", None, 0));
        assert_eq!(core.ledger.runs().count(), 0);
    }

    #[test]
    fn memory_records_runs_and_exposes_failed_attention() {
        let mut core = LedgerCore::new(RunLedgerMode::Memory, true, 5);
        let p = pane();
        failed_run(&mut core, p, "false");
        assert_eq!(core.ledger.runs().count(), 1);
        assert_eq!(core.ledger.failed_attention_count(), 1);
        let badge = core.ledger.badge_for(&[p], 10).expect("failed badge");
        assert_eq!(badge.kind, run_ledger::BadgeKind::Failed);
    }

    #[test]
    fn persist_is_memory_in_core() {
        // The plugin owns disk; Persist must behave exactly like Memory here.
        let mut core = LedgerCore::new(RunLedgerMode::Persist, true, 5);
        let p = pane();
        failed_run(&mut core, p, "false");
        assert_eq!(core.ledger.runs().count(), 1);
        assert_eq!(core.ledger.failed_attention_count(), 1);
        core.set_mode(RunLedgerMode::Memory);
        assert_eq!(core.ledger.runs().count(), 1, "no disk round-trip to lose");
    }

    #[test]
    fn switching_to_off_clears_memory() {
        let mut core = LedgerCore::new(RunLedgerMode::Memory, true, 5);
        failed_run(&mut core, pane(), "sleep 1");
        assert_eq!(core.ledger.runs().count(), 1);
        core.set_mode(RunLedgerMode::Off);
        assert_eq!(core.ledger.runs().count(), 0);
    }

    #[test]
    fn apply_prunes_finished_history_to_the_in_memory_cap() {
        let mut core = LedgerCore::new(RunLedgerMode::Memory, true, 5);
        for i in 0..(PRUNE_EVERY * 10) {
            let pane = pane();
            core.apply(RunEvent::started(pane, &format!("c{i}"), None, i * 2));
            core.apply(RunEvent::finished(pane, Some(0), i * 2 + 1));
        }
        assert_eq!(core.ledger.runs().count(), Retention::default().max_runs);
        assert!(
            core.ledger
                .runs()
                .all(|run| run.state == RunState::Succeeded)
        );
    }

    #[test]
    fn apply_keeps_active_runs_beyond_the_history_cap() {
        let mut core = LedgerCore::new(RunLedgerMode::Memory, true, 5);
        let count = PRUNE_EVERY * 10;
        for i in 0..count {
            core.apply(RunEvent::started(pane(), &format!("c{i}"), None, i));
        }
        assert_eq!(core.ledger.runs().count(), count as usize);
        assert!(core.ledger.runs().all(|run| run.state == RunState::Running));
    }

    #[test]
    fn rebase_anchors_preserves_other_panes() {
        let pane = pane();
        let other = PaneKey::new_v4();
        let mut global = RunLedgerGlobal {
            core: LedgerCore::new(RunLedgerMode::Memory, true, 5),
            started_at: Instant::now(),
        };
        global.apply(RunEvent::Started {
            pane,
            command: "first".into(),
            cwd: None,
            at_ms: 0,
            inferred: false,
            anchor: Some(run_ledger::Anchor { line: 2, column: 0 }),
        });
        global.apply(RunEvent::Started {
            pane,
            command: "second".into(),
            cwd: None,
            at_ms: 1,
            inferred: false,
            anchor: Some(run_ledger::Anchor { line: 8, column: 4 }),
        });
        global.apply(RunEvent::Started {
            pane: other,
            command: "other".into(),
            cwd: None,
            at_ms: 2,
            inferred: false,
            anchor: Some(run_ledger::Anchor {
                line: 11,
                column: 7,
            }),
        });

        global.rebase_anchors(pane, 5);

        let snapshot = global.snapshot();
        assert_eq!(snapshot[0].anchor, None);
        assert_eq!(
            snapshot[1].anchor,
            Some(run_ledger::Anchor { line: 3, column: 4 })
        );
        assert_eq!(
            snapshot[2].anchor,
            Some(run_ledger::Anchor {
                line: 11,
                column: 7
            })
        );
    }

    #[test]
    fn clear_anchors_clears_one_pane_only() {
        let pane = pane();
        let other = PaneKey::new_v4();
        let mut global = RunLedgerGlobal {
            core: LedgerCore::new(RunLedgerMode::Memory, true, 5),
            started_at: Instant::now(),
        };
        global.apply(RunEvent::Started {
            pane,
            command: "clear".into(),
            cwd: None,
            at_ms: 0,
            inferred: false,
            anchor: Some(run_ledger::Anchor { line: 4, column: 2 }),
        });
        global.apply(RunEvent::Started {
            pane: other,
            command: "keep".into(),
            cwd: None,
            at_ms: 1,
            inferred: false,
            anchor: Some(run_ledger::Anchor { line: 6, column: 3 }),
        });

        global.clear_anchors(pane);

        let snapshot = global.snapshot();
        assert_eq!(snapshot[0].anchor, None);
        assert_eq!(
            snapshot[1].anchor,
            Some(run_ledger::Anchor { line: 6, column: 3 })
        );
    }

    #[test]
    fn block_anchor_change_wrapper_matches_rebase_and_clear() {
        let pane = pane();
        let other = PaneKey::new_v4();
        let mut global = RunLedgerGlobal {
            core: LedgerCore::new(RunLedgerMode::Memory, true, 5),
            started_at: Instant::now(),
        };
        global.apply(RunEvent::Started {
            pane,
            command: "pane".into(),
            cwd: None,
            at_ms: 0,
            inferred: false,
            anchor: Some(run_ledger::Anchor { line: 7, column: 1 }),
        });
        global.apply(RunEvent::Started {
            pane: other,
            command: "other".into(),
            cwd: None,
            at_ms: 1,
            inferred: false,
            anchor: Some(run_ledger::Anchor { line: 9, column: 2 }),
        });

        global.apply_block_anchor_change(pane, &terminal::BlockAnchorChange::Rebase(5));
        let snapshot = global.snapshot();
        assert_eq!(
            snapshot[0].anchor,
            Some(run_ledger::Anchor { line: 2, column: 1 })
        );
        assert_eq!(
            snapshot[1].anchor,
            Some(run_ledger::Anchor { line: 9, column: 2 })
        );

        global.apply_block_anchor_change(pane, &terminal::BlockAnchorChange::Invalidate);
        let snapshot = global.snapshot();
        assert_eq!(snapshot[0].anchor, None);
        assert_eq!(
            snapshot[1].anchor,
            Some(run_ledger::Anchor { line: 9, column: 2 })
        );
    }
}
