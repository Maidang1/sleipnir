//! The Ledger: every Run this app has seen, plus the rules for what to show.

use crate::run::{LaunchId, PaneKey, Run, RunEvent, RunState};

/// Wall-clock source injected by the caller so tests stay deterministic.
pub type UnixMillisFn = fn() -> u64;

/// Badge kind for a pane set. Aggregated in a later task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    Running,
    Succeeded,
    Failed,
}

/// Aggregated chrome badge for a set of panes. Aggregated in a later task.
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
}

impl Ledger {
    pub fn new(launch_id: LaunchId) -> Self {
        Self { launch_id, runs: Vec::new(), now_unix_ms: default_unix_ms }
    }

    /// Test seam: fixed wall clock.
    pub fn with_clock(launch_id: LaunchId, now_unix_ms: UnixMillisFn) -> Self {
        Self { launch_id, runs: Vec::new(), now_unix_ms }
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.runs.iter()
    }

    pub fn apply(&mut self, event: RunEvent) {
        match event {
            RunEvent::Started { pane, command, cwd, at_ms, inferred } => {
                // One Run per Pane at a time: an unfinished predecessor is Abandoned.
                self.abandon_running_in(pane, at_ms);
                let unix_ms = (self.now_unix_ms)();
                self.runs.push(Run::start(
                    self.launch_id, pane, command, cwd, at_ms, unix_ms, inferred,
                ));
            }
            RunEvent::Finished { pane, exit_code, at_ms } => {
                // An orphan Finished (no Started) is dropped: never invent half a Run.
                if let Some(run) = self.running_in_mut(pane) {
                    run.finish(exit_code, at_ms);
                }
            }
            RunEvent::PaneClosed { pane, at_ms } => self.abandon_running_in(pane, at_ms),
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
    use crate::run::{LaunchId, PaneKey, RunEvent, RunState};
    use std::time::Duration;

    fn pane() -> PaneKey {
        PaneKey::new_v4()
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
}
