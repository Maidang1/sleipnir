//! Pure helpers for the Run Ledger overlay. GPUI is optional; these stay testable
//! without a window.

use run_ledger::{LaunchId, PaneKey, Run, RunId, RunState};
use std::cmp::Reverse;
use std::time::Duration;

const MS_PER_DAY: u64 = 24 * 60 * 60 * 1000;

/// One overlay row, copied off a [`Run`] snapshot so the panel does not hold the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    pub id: RunId,
    pub pane: PaneKey,
    pub command: String,
    pub state: RunState,
    pub duration: Duration,
    pub exit_code: Option<i32>,
    pub inferred: bool,
    pub launch_id: LaunchId,
    pub started_at_unix_ms: u64,
    pub cwd: Option<String>,
}

/// Newest start time first. Equal timestamps keep input order.
pub fn rows_from_runs(runs: &[Run]) -> Vec<LedgerRow> {
    let mut rows: Vec<LedgerRow> = runs.iter().map(row_from_run).collect();
    rows.sort_by_key(|row| Reverse(row.started_at_unix_ms));
    rows
}

/// Overlay section for `row`. Current-launch finished runs (not Abandoned) are
/// "待看"; Attention does not survive a restart, so other launches fall through
/// to the calendar buckets.
pub fn group_label(row: &LedgerRow, now_unix_ms: u64, current_launch: LaunchId) -> &'static str {
    if row.state == RunState::Running {
        return "进行中";
    }
    if row.launch_id == current_launch
        && matches!(
            row.state,
            RunState::Failed | RunState::Succeeded | RunState::Unknown
        )
    {
        return "待看";
    }
    if row.started_at_unix_ms / MS_PER_DAY == now_unix_ms / MS_PER_DAY {
        "今天"
    } else {
        "更早"
    }
}

/// Jump is only valid while the pane's scrollback still exists: same launch, not Abandoned.
pub fn can_jump(row: &LedgerRow, current_launch: LaunchId) -> bool {
    row.launch_id == current_launch && row.state != RunState::Abandoned
}

/// `"✗ cargo test  1.2s"` — icon, command, duration.
pub fn row_summary(row: &LedgerRow) -> String {
    format!(
        "{} {}  {}",
        state_icon(row.state),
        row.command,
        format_secs(row.duration)
    )
}

fn row_from_run(run: &Run) -> LedgerRow {
    LedgerRow {
        id: run.id,
        pane: run.pane,
        command: run.command.clone(),
        state: run.state,
        duration: run.duration,
        exit_code: run.exit_code,
        inferred: run.inferred,
        launch_id: run.launch_id,
        started_at_unix_ms: run.started_at_unix_ms,
        cwd: run.cwd.clone(),
    }
}

fn state_icon(state: RunState) -> &'static str {
    match state {
        RunState::Failed => "✗",
        RunState::Succeeded => "✓",
        RunState::Running => "●",
        RunState::Unknown => "?",
        RunState::Abandoned => "–",
    }
}

fn format_secs(duration: Duration) -> String {
    format!("{:.1}s", duration.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_ledger::{Ledger, RunEvent};

    fn launch() -> LaunchId {
        LaunchId::new_v4()
    }

    fn row(state: RunState, launch_id: LaunchId, started_at_unix_ms: u64) -> LedgerRow {
        LedgerRow {
            id: RunId::new_v4(),
            pane: PaneKey::new_v4(),
            command: "cargo test".into(),
            state,
            duration: Duration::from_millis(1_200),
            exit_code: None,
            inferred: false,
            launch_id,
            started_at_unix_ms,
            cwd: None,
        }
    }

    fn run_at(command: &str, unix_ms: u64, exit: Option<i32>) -> Run {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.set_redact(false);
        let pane = PaneKey::new_v4();
        ledger.apply(RunEvent::started(pane, command, Some("/tmp".into()), 0));
        if let Some(code) = exit {
            ledger.apply(RunEvent::finished(pane, Some(code), 1_200));
        }
        let mut run = ledger.snapshot().pop().expect("started run");
        run.started_at_unix_ms = unix_ms;
        run
    }

    #[test]
    fn groups_running_unseen_today_and_earlier() {
        let current = launch();
        let other = launch();
        let today = 1_700_000_000_000;
        let yesterday = today - MS_PER_DAY;

        assert_eq!(
            group_label(&row(RunState::Running, other, yesterday), today, current),
            "进行中"
        );
        assert_eq!(
            group_label(&row(RunState::Failed, current, yesterday), today, current),
            "待看"
        );
        assert_eq!(
            group_label(&row(RunState::Succeeded, current, today), today, current),
            "待看"
        );
        assert_eq!(
            group_label(&row(RunState::Unknown, current, today), today, current),
            "待看"
        );
        assert_eq!(
            group_label(&row(RunState::Abandoned, current, today), today, current),
            "今天"
        );
        assert_eq!(
            group_label(&row(RunState::Failed, other, today), today, current),
            "今天"
        );
        assert_eq!(
            group_label(&row(RunState::Failed, other, yesterday), today, current),
            "更早"
        );
    }

    #[test]
    fn can_jump_only_within_the_current_launch() {
        let current = launch();
        let other = launch();
        assert!(can_jump(&row(RunState::Running, current, 0), current));
        assert!(can_jump(&row(RunState::Failed, current, 0), current));
        assert!(can_jump(&row(RunState::Succeeded, current, 0), current));
        assert!(!can_jump(&row(RunState::Failed, other, 0), current));
    }

    #[test]
    fn abandoned_cannot_jump() {
        let current = launch();
        assert!(!can_jump(&row(RunState::Abandoned, current, 0), current));
    }

    #[test]
    fn rows_are_newest_first() {
        let older = run_at("old", 1_000, Some(0));
        let newer = run_at("new", 2_000, Some(1));
        let rows = rows_from_runs(&[older, newer.clone()]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].command, "new");
        assert_eq!(rows[0].id, newer.id);
        assert_eq!(rows[0].started_at_unix_ms, 2_000);
        assert_eq!(rows[1].command, "old");
        assert_eq!(rows[1].cwd.as_deref(), Some("/tmp"));
        assert_eq!(rows[1].exit_code, Some(0));
        assert_eq!(rows[1].state, RunState::Succeeded);
    }

    #[test]
    fn row_summary_uses_state_icon_and_duration() {
        let mut failed = row(RunState::Failed, launch(), 0);
        assert_eq!(row_summary(&failed), "✗ cargo test  1.2s");
        failed.state = RunState::Succeeded;
        assert_eq!(row_summary(&failed), "✓ cargo test  1.2s");
        failed.state = RunState::Running;
        assert_eq!(row_summary(&failed), "● cargo test  1.2s");
        failed.state = RunState::Unknown;
        assert_eq!(row_summary(&failed), "? cargo test  1.2s");
        failed.state = RunState::Abandoned;
        assert_eq!(row_summary(&failed), "– cargo test  1.2s");
    }
}
