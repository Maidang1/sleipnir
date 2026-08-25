//! The Ledger: every Run this app has seen, plus the rules for what to show.

use crate::redact::redact_command;
use crate::run::{LaunchId, PaneKey, Run, RunEvent, RunId, RunState};
use std::time::Duration;

/// Wall-clock source injected by the caller so tests stay deterministic.
pub type UnixMillisFn = fn() -> u64;

const DEFAULT_SUCCESS_THRESHOLD_SECS: u64 = 5;
const MS_PER_DAY: u64 = 24 * 60 * 60 * 1000;

/// Retention policy, from settings.
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub days: u64,
    pub max_runs: usize,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            days: 7,
            max_runs: 500,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub kind: BadgeKind,
    /// How many runs of `kind` this badge stands for (≥ 1).
    pub count: usize,
    /// For `Running`: millis elapsed on the oldest running Run. Else 0.
    pub elapsed_ms: u64,
}

pub struct Ledger {
    launch_id: LaunchId,
    /// Oldest first.
    runs: Vec<Run>,
    now_unix_ms: UnixMillisFn,
    redact: bool,
    success_threshold_secs: u64,
    retention: Retention,
    focused_pane: Option<PaneKey>,
    window_active: bool,
}

impl Ledger {
    pub fn new(launch_id: LaunchId) -> Self {
        Self {
            launch_id,
            runs: Vec::new(),
            now_unix_ms: default_unix_ms,
            redact: true,
            success_threshold_secs: DEFAULT_SUCCESS_THRESHOLD_SECS,
            retention: Retention::default(),
            focused_pane: None,
            window_active: false,
        }
    }

    /// Test seam: fixed wall clock.
    pub fn with_clock(launch_id: LaunchId, now_unix_ms: UnixMillisFn) -> Self {
        Self {
            launch_id,
            runs: Vec::new(),
            now_unix_ms,
            redact: true,
            success_threshold_secs: DEFAULT_SUCCESS_THRESHOLD_SECS,
            retention: Retention::default(),
            focused_pane: None,
            window_active: false,
        }
    }

    pub fn set_redact(&mut self, on: bool) {
        self.redact = on;
    }

    /// Attention 的成功阈值（复用 `notify_on_command_finish_secs`）。
    pub fn set_success_threshold_secs(&mut self, secs: u64) {
        self.success_threshold_secs = secs;
    }

    pub fn set_retention(&mut self, retention: Retention) {
        self.retention = retention;
    }

    /// 结束事件发生时，若该 pane 正被 focus 且窗口活跃，则直接标记为已看过。
    pub fn set_focus(&mut self, pane: Option<PaneKey>, window_active: bool) {
        self.focused_pane = pane;
        self.window_active = window_active;
    }

    /// focus 一个 pane：清空该 pane 的**全部**待看 Run。
    pub fn mark_pane_seen(&mut self, pane: PaneKey) {
        for run in &mut self.runs {
            if run.pane == pane {
                run.seen = true;
            }
        }
    }

    /// 面板点击某条 Run 时用。
    pub fn mark_run_seen(&mut self, id: RunId) {
        if let Some(run) = self.runs.iter_mut().find(|r| r.id == id) {
            run.seen = true;
        }
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.runs.iter()
    }

    pub fn launch_id(&self) -> LaunchId {
        self.launch_id
    }

    pub fn snapshot(&self) -> Vec<Run> {
        self.runs.clone()
    }

    pub fn failed_attention_count(&self) -> usize {
        self.attention()
            .filter(|r| r.state == RunState::Failed)
            .count()
    }

    /// 已结束且未看过（Failed 无阈值；Succeeded 需 ≥ 阈值；Unknown 同 Succeeded；
    /// Abandoned 不进 Attention —— 它是进程没了，不是「跑完了等你看」）。
    pub fn attention(&self) -> impl Iterator<Item = &Run> {
        let threshold = Duration::from_secs(self.success_threshold_secs);
        self.runs
            .iter()
            .filter(move |run| self.in_attention(run, threshold))
    }

    /// 给一组 PaneKey（一个 tab 的全部 pane）算徽标。
    /// 输入集合是 Attention ∪ Running；优先级 Failed > Running > Succeeded，count 只数同类。
    pub fn badge_for(&self, panes: &[PaneKey], now_mono_ms: u64) -> Option<Badge> {
        let in_panes = |run: &Run| panes.contains(&run.pane);
        let threshold = Duration::from_secs(self.success_threshold_secs);

        let failed = self
            .runs
            .iter()
            .filter(|r| {
                in_panes(r) && self.in_attention(r, threshold) && r.state == RunState::Failed
            })
            .count();
        if failed > 0 {
            return Some(Badge {
                kind: BadgeKind::Failed,
                count: failed,
                elapsed_ms: 0,
            });
        }

        let mut running_count = 0;
        let mut oldest_started: Option<u64> = None;
        for run in self
            .runs
            .iter()
            .filter(|r| in_panes(r) && r.state == RunState::Running)
        {
            running_count += 1;
            oldest_started = Some(match oldest_started {
                Some(t) => t.min(run.started_at_mono_ms()),
                None => run.started_at_mono_ms(),
            });
        }
        if running_count > 0 {
            let started = oldest_started.unwrap_or(0);
            return Some(Badge {
                kind: BadgeKind::Running,
                count: running_count,
                elapsed_ms: now_mono_ms.saturating_sub(started),
            });
        }

        let succeeded = self
            .runs
            .iter()
            .filter(|r| {
                in_panes(r)
                    && self.in_attention(r, threshold)
                    && matches!(r.state, RunState::Succeeded | RunState::Unknown)
            })
            .count();
        if succeeded > 0 {
            return Some(Badge {
                kind: BadgeKind::Succeeded,
                count: succeeded,
                elapsed_ms: 0,
            });
        }

        None
    }

    /// 时间窗 + 条数双约束，先到先裁。
    pub fn prune(&mut self) {
        let now = (self.now_unix_ms)();
        let window_ms = self.retention.days.saturating_mul(MS_PER_DAY);
        let cutoff = now.saturating_sub(window_ms);
        self.runs.retain(|r| r.started_at_unix_ms >= cutoff);
        if self.runs.len() > self.retention.max_runs {
            let drop = self.runs.len() - self.retention.max_runs;
            self.runs.drain(..drop);
        }
    }

    /// 启动时载入历史：全部标记 `seen = true`（Attention 不跨重启）；
    /// 状态为 `Running` 的 Run 强制转为 `Abandoned`（spec §3.1：徽标不会在启动时凭历史数据出现）。
    pub fn load_history(&mut self, runs: Vec<Run>) {
        for mut run in runs {
            run.seen = true;
            if run.state == RunState::Running {
                run.state = RunState::Abandoned;
            }
            self.runs.push(run);
        }
        self.runs.sort_by_key(|r| r.started_at_unix_ms);
    }

    pub fn apply(&mut self, event: RunEvent) {
        match event {
            RunEvent::Started {
                pane,
                command,
                cwd,
                at_ms,
                inferred,
                anchor,
            } => {
                // One Run per Pane at a time: an unfinished predecessor is Abandoned.
                self.abandon_running_in(pane, at_ms);
                let unix_ms = (self.now_unix_ms)();
                let command = if self.redact {
                    redact_command(&command)
                } else {
                    command
                };
                let mut run =
                    Run::start(self.launch_id, pane, command, cwd, at_ms, unix_ms, inferred);
                run.anchor = anchor;
                self.runs.push(run);
            }
            RunEvent::Finished {
                pane,
                exit_code,
                at_ms,
            } => {
                // An orphan Finished (no Started) is dropped: never invent half a Run.
                let seen_now = self.window_active && self.focused_pane == Some(pane);
                if let Some(run) = self.running_in_mut(pane) {
                    run.finish(exit_code, at_ms);
                    if seen_now {
                        run.seen = true;
                    }
                }
            }
            RunEvent::PaneClosed { pane, at_ms } => self.abandon_running_in(pane, at_ms),
        }
    }

    fn in_attention(&self, run: &Run, threshold: Duration) -> bool {
        if run.seen {
            return false;
        }
        match run.state {
            RunState::Failed => true,
            RunState::Succeeded | RunState::Unknown => run.duration >= threshold,
            RunState::Running | RunState::Abandoned => false,
        }
    }

    fn running_in_mut(&mut self, pane: PaneKey) -> Option<&mut Run> {
        self.runs
            .iter_mut()
            .rev()
            .find(|r| r.pane == pane && r.state == RunState::Running)
    }

    fn abandon_running_in(&mut self, pane: PaneKey, at_ms: u64) {
        if let Some(run) = self.running_in_mut(pane) {
            run.abandon(at_ms);
        }
    }
}

fn default_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{Anchor, LaunchId, PaneKey, RunEvent, RunState};

    fn pane() -> PaneKey {
        PaneKey::new_v4()
    }

    fn fixed_clock() -> u64 {
        1_700_000_000_000
    }

    /// Build a Run the way `store` would: JSON round-trip, so `started_at_mono_ms` is 0.
    fn disk_run(pane: PaneKey, command: &str, started_at_unix_ms: u64, state: RunState) -> Run {
        let exit_code = match state {
            RunState::Succeeded => Some(0),
            RunState::Failed => Some(1),
            _ => None,
        };
        let value = serde_json::json!({
            "id": RunId::new_v4(),
            "launch_id": LaunchId::new_v4(),
            "pane": pane,
            "command": command,
            "started_at_unix_ms": started_at_unix_ms,
            "duration": { "secs": 0, "nanos": 0 },
            "exit_code": exit_code,
            "state": state,
        });
        serde_json::from_value(value).expect("disk-shaped Run")
    }

    #[test]
    fn exit_zero_succeeds() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "cargo build", None, 1_000));
        ledger.apply(RunEvent::finished(p, Some(0), 42_000));
        let run = ledger.runs().next().unwrap();
        assert_eq!(run.state, RunState::Succeeded);
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.duration, Duration::from_millis(41_000));
        assert_eq!(run.command, "cargo build");
    }

    #[test]
    fn nonzero_exit_fails() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "npm run deploy", None, 0));
        ledger.apply(RunEvent::finished(p, Some(1), 500));
        assert_eq!(ledger.runs().next().unwrap().state, RunState::Failed);
    }

    #[test]
    fn missing_status_is_unknown() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "ssh prod-01", None, 0));
        ledger.apply(RunEvent::finished(p, None, 100));
        assert_eq!(ledger.runs().next().unwrap().state, RunState::Unknown);
    }

    #[test]
    fn inferred_runs_are_marked() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started_inferred(p, "vim", None, 0));
        let run = ledger.runs().next().unwrap();
        assert!(run.inferred, "busy-probe runs must be marked inferred");
    }

    /// 一个 pane 同时只能有一个 Run：新的 Started 让上一条变 Abandoned。
    #[test]
    fn second_start_abandons_the_first() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "first", None, 0));
        ledger.apply(RunEvent::started(p, "second", None, 10));
        let states: Vec<_> = ledger.runs().map(|r| r.state).collect();
        assert!(states.contains(&RunState::Abandoned));
        assert!(states.contains(&RunState::Running));
    }

    /// 没有对应 Started 的 Finished 必须被丢弃，不能造出半条 Run。
    #[test]
    fn orphan_finish_is_ignored() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.apply(RunEvent::finished(pane(), Some(0), 10));
        assert_eq!(ledger.runs().count(), 0);
    }

    #[test]
    fn closing_a_pane_abandons_its_running_run() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "long thing", None, 0));
        ledger.apply(RunEvent::PaneClosed { pane: p, at_ms: 90 });
        assert_eq!(ledger.runs().next().unwrap().state, RunState::Abandoned);
    }

    #[test]
    fn redact_off_preserves_original_command() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.set_redact(false);
        let p = pane();
        ledger.apply(RunEvent::started(
            p,
            "AWS_SECRET_ACCESS_KEY=abc123 aws s3 ls",
            None,
            0,
        ));
        assert_eq!(
            ledger.runs().next().unwrap().command,
            "AWS_SECRET_ACCESS_KEY=abc123 aws s3 ls"
        );
    }

    #[test]
    fn prune_drops_runs_older_than_the_window() {
        let mut ledger = Ledger::with_clock(LaunchId::new_v4(), fixed_clock);
        let today = fixed_clock();
        let eight_days_ago = today - 8 * MS_PER_DAY;
        ledger.load_history(vec![
            disk_run(pane(), "old", eight_days_ago, RunState::Succeeded),
            disk_run(pane(), "today", today, RunState::Succeeded),
        ]);
        ledger.prune();
        let cmds: Vec<_> = ledger.runs().map(|r| r.command.as_str()).collect();
        assert_eq!(cmds, ["today"]);
    }

    #[test]
    fn prune_caps_total_runs() {
        let mut ledger = Ledger::with_clock(LaunchId::new_v4(), fixed_clock);
        ledger.set_retention(Retention {
            days: 7,
            max_runs: 500,
        });
        let today = fixed_clock();
        let runs: Vec<_> = (0..600)
            .map(|i| {
                disk_run(
                    pane(),
                    &format!("c{i}"),
                    today + i as u64,
                    RunState::Succeeded,
                )
            })
            .collect();
        ledger.load_history(runs);
        ledger.prune();
        let cmds: Vec<_> = ledger.runs().map(|r| r.command.as_str()).collect();
        assert_eq!(cmds.len(), 500);
        assert_eq!(cmds.first().copied(), Some("c100"));
        assert_eq!(cmds.last().copied(), Some("c599"));
    }

    #[test]
    fn failed_runs_always_enter_attention() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.set_success_threshold_secs(5);
        let p = pane();
        ledger.apply(RunEvent::started(p, "boom", None, 0));
        ledger.apply(RunEvent::finished(p, Some(1), 100));
        assert_eq!(ledger.attention().count(), 1);
    }

    #[test]
    fn short_success_does_not_enter_attention() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "ls", None, 0));
        ledger.apply(RunEvent::finished(p, Some(0), 1_000));
        assert_eq!(ledger.attention().count(), 0);
    }

    #[test]
    fn long_success_enters_attention() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "build", None, 0));
        ledger.apply(RunEvent::finished(p, Some(0), 6_000));
        assert_eq!(ledger.attention().count(), 1);
    }

    #[test]
    fn finishing_in_a_focused_pane_is_seen_immediately() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.set_focus(Some(p), true);
        ledger.apply(RunEvent::started(p, "fail", None, 0));
        ledger.apply(RunEvent::finished(p, Some(1), 100));
        assert_eq!(ledger.attention().count(), 0);
        assert!(ledger.badge_for(&[p], 100).is_none());
    }

    #[test]
    fn focus_clears_all_pending_attention_for_that_pane() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started(p, "first", None, 0));
        ledger.apply(RunEvent::finished(p, Some(1), 10));
        ledger.apply(RunEvent::started(p, "second", None, 20));
        ledger.apply(RunEvent::finished(p, Some(1), 30));
        assert_eq!(ledger.attention().count(), 2);
        ledger.mark_pane_seen(p);
        assert_eq!(ledger.attention().count(), 0);
        assert_eq!(
            ledger.runs().count(),
            2,
            "mark_pane_seen must clear Attention without deleting Runs"
        );
    }

    #[test]
    fn badge_prefers_failure_over_running_over_success() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p_fail = pane();
        let p_run = pane();
        let p_ok = pane();
        ledger.apply(RunEvent::started(p_fail, "fail", None, 0));
        ledger.apply(RunEvent::finished(p_fail, Some(1), 100));
        ledger.apply(RunEvent::started(p_ok, "ok", None, 0));
        ledger.apply(RunEvent::finished(p_ok, Some(0), 6_000));
        ledger.apply(RunEvent::started(p_run, "run", None, 0));
        let badge = ledger.badge_for(&[p_fail, p_run, p_ok], 10_000).unwrap();
        assert_eq!(badge.kind, BadgeKind::Failed);
        assert_eq!(badge.count, 1);
    }

    #[test]
    fn badge_counts_only_its_own_kind() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p1 = pane();
        let p2 = pane();
        let p3 = pane();
        ledger.apply(RunEvent::started(p1, "f1", None, 0));
        ledger.apply(RunEvent::finished(p1, Some(1), 10));
        ledger.apply(RunEvent::started(p2, "f2", None, 0));
        ledger.apply(RunEvent::finished(p2, Some(2), 10));
        ledger.apply(RunEvent::started(p3, "run", None, 0));
        let badge = ledger.badge_for(&[p1, p2, p3], 100).unwrap();
        assert_eq!(badge.kind, BadgeKind::Failed);
        assert_eq!(badge.count, 2);
    }

    #[test]
    fn running_badge_reports_elapsed_of_the_oldest_running_run() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p1 = pane();
        let p2 = pane();
        ledger.apply(RunEvent::started(p1, "old", None, 1_000));
        ledger.apply(RunEvent::started(p2, "new", None, 5_000));
        let badge = ledger.badge_for(&[p1, p2], 8_000).unwrap();
        assert_eq!(badge.kind, BadgeKind::Running);
        assert_eq!(badge.count, 2);
        assert_eq!(badge.elapsed_ms, 7_000);
    }

    #[test]
    fn no_badge_when_nothing_pending() {
        let ledger = Ledger::new(LaunchId::new_v4());
        assert!(ledger.badge_for(&[pane()], 0).is_none());
    }

    #[test]
    fn loaded_history_is_seen_so_badges_never_resurrect_after_restart() {
        let p = pane();
        let run = disk_run(p, "npm test", fixed_clock(), RunState::Failed);
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.load_history(vec![run]);
        assert_eq!(ledger.attention().count(), 0);
        assert!(ledger.badge_for(&[p], 0).is_none());
    }

    #[test]
    fn started_run_keeps_in_memory_anchor() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let p = pane();
        ledger.apply(RunEvent::started_at(
            p,
            "cargo test",
            None,
            0,
            false,
            Some(Anchor {
                line: 42,
                column: 3,
            }),
        ));
        let run = ledger.runs().next().unwrap();
        assert_eq!(
            run.anchor,
            Some(Anchor {
                line: 42,
                column: 3
            })
        );
    }

    #[test]
    fn loaded_running_becomes_abandoned() {
        let p = pane();
        let run = disk_run(p, "long thing", fixed_clock(), RunState::Running);
        assert_eq!(run.state, RunState::Running);
        assert_eq!(
            run.started_at_mono_ms(),
            0,
            "serde skip leaves mono clock at 0"
        );
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.load_history(vec![run]);
        let loaded = ledger.runs().next().unwrap();
        assert_eq!(loaded.state, RunState::Abandoned);
        assert!(loaded.seen);
        assert_eq!(ledger.attention().count(), 0);
        assert!(
            ledger.badge_for(&[p], 60_000).is_none(),
            "abandoned history must not produce a running badge"
        );
    }
}
