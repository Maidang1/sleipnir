//! Restore-banner copy for a pane that has ledger history. Pure so tests do not need GPUI.

use run_ledger::{LaunchId, PaneKey, Run, RunState};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};
use terminal::UNRECOGNIZED_COMMAND;

/// One-line chrome banner: how many commands ran here, and what the last one did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub summary: String,
}

/// Session-local dismissals so a restore banner vanishes on first keystroke.
#[derive(Clone, Debug, Default)]
pub struct TombstoneGate {
    dismissed: HashSet<PaneKey>,
}

impl TombstoneGate {
    pub fn dismiss(&mut self, pane: PaneKey) {
        self.dismissed.insert(pane);
    }

    pub fn banner(
        &self,
        runs: &[Run],
        pane: PaneKey,
        current_launch: LaunchId,
    ) -> Option<Tombstone> {
        if self.dismissed.contains(&pane) {
            return None;
        }
        tombstone_from_runs(runs, pane, current_launch)
    }
}

/// `None` when this pane has no prior-launch speakable run. Otherwise the
/// historical count plus what the last named command did.
pub fn tombstone_from_runs(
    runs: &[Run],
    pane: PaneKey,
    current_launch: LaunchId,
) -> Option<Tombstone> {
    let mut mine: Vec<&Run> = runs
        .iter()
        .filter(|run| run.pane == pane && run.launch_id != current_launch)
        .collect();
    if mine.is_empty() {
        return None;
    }
    mine.sort_by_key(|run| (ended_at_unix_ms(run), run.started_at_unix_ms));
    let last = mine.iter().rev().copied().find(|run| speakable(run))?;
    let count = mine.len();
    let state_word = match last.state {
        RunState::Failed => "failed",
        RunState::Succeeded => "succeeded",
        RunState::Running => "running",
        RunState::Unknown => "unknown",
        RunState::Abandoned => "interrupted",
    };
    let when = relative_wall(ended_at_unix_ms(last));
    let detail = match last.exit_code {
        Some(code) => format!("exit {code} · {when}"),
        None => when,
    };
    Some(Tombstone {
        summary: format!(
            "Ran {count} commands here last session · last `{}` {state_word} ({detail})",
            last.command
        ),
    })
}

fn speakable(run: &Run) -> bool {
    run.state != RunState::Running && run.command != UNRECOGNIZED_COMMAND
}

fn ended_at_unix_ms(run: &Run) -> u64 {
    run.started_at_unix_ms
        .saturating_add(u64::try_from(run.duration.as_millis()).unwrap_or(u64::MAX))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn relative_wall(end_unix_ms: u64) -> String {
    const MIN: u64 = 60_000;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    let now = now_unix_ms();
    let ago = now.saturating_sub(end_unix_ms);
    if ago < MIN {
        "just now".into()
    } else if ago < HOUR {
        format!("{}m ago", ago / MIN)
    } else if ago < DAY {
        format!("{}h ago", ago / HOUR)
    } else {
        format!("{}d ago", ago / DAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prior() -> LaunchId {
        LaunchId::new_v4()
    }

    fn current() -> LaunchId {
        LaunchId::new_v4()
    }

    fn display(
        launch: LaunchId,
        pane: PaneKey,
        command: &str,
        state: RunState,
        exit: Option<i32>,
        started: u64,
    ) -> Run {
        Run::for_display(launch, pane, command, state, exit, started, true)
    }

    #[test]
    fn empty_is_none() {
        let pane = PaneKey::new_v4();
        let now = current();
        assert!(tombstone_from_runs(&[], pane, now).is_none());
        let other = display(
            prior(),
            PaneKey::new_v4(),
            "npm test",
            RunState::Failed,
            Some(1),
            1_000,
        );
        assert!(
            tombstone_from_runs(&[other], pane, now).is_none(),
            "a different pane must not produce a tombstone"
        );
    }

    #[test]
    fn current_launch_runs_are_not_a_tombstone() {
        let pane = PaneKey::new_v4();
        let now = current();
        let live = display(now, pane, "cargo test", RunState::Failed, Some(1), 1_000);
        assert!(
            tombstone_from_runs(&[live], pane, now).is_none(),
            "this-launch runs are live work, not a restore tombstone"
        );
    }

    #[test]
    fn previous_launch_finished_run_is_a_tombstone() {
        let pane = PaneKey::new_v4();
        let now = current();
        let run = display(prior(), pane, "npm test", RunState::Failed, Some(1), 1_000);
        let tomb = tombstone_from_runs(&[run], pane, now).expect("tombstone");
        assert!(
            tomb.summary.contains("npm test"),
            "expected command in {}",
            tomb.summary
        );
        assert!(
            tomb.summary.contains("failed"),
            "expected failed in {}",
            tomb.summary
        );
        assert!(
            tomb.summary.contains("Ran 1 commands here last session"),
            "expected count in {}",
            tomb.summary
        );
    }

    #[test]
    fn running_is_not_narrated_as_last() {
        let pane = PaneKey::new_v4();
        let now = current();
        let then = prior();
        let finished = display(then, pane, "npm test", RunState::Failed, Some(1), 1_000);
        let running = display(then, pane, "sleep 999", RunState::Running, None, 9_000);
        let tomb = tombstone_from_runs(&[finished, running], pane, now).expect("tombstone");
        assert!(
            tomb.summary.contains("`npm test` failed"),
            "expected last speakable command, got {}",
            tomb.summary
        );
        assert!(
            !tomb.summary.contains("running"),
            "must not narrate Running as last: {}",
            tomb.summary
        );
        assert!(
            tomb.summary.contains("Ran 2 commands here last session"),
            "count still includes the unspeakable run: {}",
            tomb.summary
        );
    }

    #[test]
    fn unrecognized_is_not_narrated_as_last() {
        let pane = PaneKey::new_v4();
        let now = current();
        let then = prior();
        let finished = display(then, pane, "npm test", RunState::Succeeded, Some(0), 1_000);
        let dummy = display(
            then,
            pane,
            UNRECOGNIZED_COMMAND,
            RunState::Failed,
            Some(1),
            9_000,
        );
        let tomb = tombstone_from_runs(&[finished, dummy], pane, now).expect("tombstone");
        assert!(
            tomb.summary.contains("`npm test` succeeded"),
            "expected last speakable command, got {}",
            tomb.summary
        );
        assert!(
            !tomb.summary.contains(UNRECOGNIZED_COMMAND),
            "must not narrate the placeholder as last: {}",
            tomb.summary
        );
    }

    #[test]
    fn no_banner_when_every_prior_run_is_unspeakable() {
        let pane = PaneKey::new_v4();
        let now = current();
        let then = prior();
        let dummy = display(
            then,
            pane,
            UNRECOGNIZED_COMMAND,
            RunState::Unknown,
            None,
            1_000,
        );
        let running = display(then, pane, "whatever", RunState::Running, None, 2_000);
        assert!(
            tombstone_from_runs(&[dummy, running], pane, now).is_none(),
            "count without a speakable last command is not a fact worth a banner"
        );
    }

    #[test]
    fn dismissed_pane_hides_banner() {
        let pane = PaneKey::new_v4();
        let now = current();
        let run = display(prior(), pane, "npm test", RunState::Failed, Some(1), 1_000);
        let mut gate = TombstoneGate::default();
        assert!(gate.banner(std::slice::from_ref(&run), pane, now).is_some());
        gate.dismiss(pane);
        assert!(gate.banner(std::slice::from_ref(&run), pane, now).is_none());
    }
}
