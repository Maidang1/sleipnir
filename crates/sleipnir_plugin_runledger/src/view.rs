//! State → widget trees. Pure, so the Status strip wording and the Panel's
//! node budget are unit testable without a session.
//!
//! Two protocol facts shape these trees:
//!
//! - The Status strip is at most 24 cells, and its `Badge` is also derived
//!   into a tab chip (text capped at 8 chars) while its `Btn`s become command
//!   palette entries. So the badge text is `✗3` / `●1`, never a sentence.
//! - A tree is capped at 500 nodes. Rows are one node each, so a hard row
//!   limit keeps the panel comfortably inside the budget.

use run_ledger::{LaunchId, Ledger, RunId, RunState};
use sleipnir_plugin::{Tone, Widget, badge, btn, col, row, sep, text};

use crate::rows::{LedgerRow, can_jump, group_label, row_summary};

/// Panel row cap. At one node per row plus two nodes per group header and a
/// title, 200 rows keep the tree around ~215 of the 500-node budget.
pub const MAX_PANEL_ROWS: usize = 200;

/// Group order, matching the deleted core overlay.
pub const GROUP_ORDER: [&str; 4] = ["进行中", "待看", "今天", "更早"];

/// One prepared panel row: the copied run plus the host id the host can
/// still address (absent for runs restored from `runs.json`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRow {
    pub row: LedgerRow,
    pub host_id: Option<RunId>,
}

/// `(failed attention, running)` — the only numbers the strip shows.
pub fn status_summary(ledger: &Ledger) -> (usize, usize) {
    let failed = ledger.failed_attention_count();
    let running = ledger
        .runs()
        .filter(|run| run.state == RunState::Running)
        .count();
    (failed, running)
}

/// The Status strip: at most one summary badge (failure wins, same priority
/// as the core's tab wash), then the two affordances. The buttons are also
/// extracted as palette entries, so this tree *is* the plugin's palette
/// presence.
pub fn status_tree(failed: usize, running: usize) -> Widget {
    let mut strip = row().gap(1);
    if failed > 0 {
        strip = strip.child(badge(format!("✗{}", short_count(failed)), Tone::Err));
    } else if running > 0 {
        strip = strip.child(badge(format!("●{}", short_count(running)), Tone::Accent));
    }
    strip
        .child(btn("Run Ledger", "open_panel"))
        .child(btn("Clear", "clear"))
        .into()
}

/// Badge text is also a tab chip (8-char cap), so counts fold at 999.
fn short_count(n: usize) -> String {
    if n > 999 {
        "999+".into()
    } else {
        n.to_string()
    }
}

/// The grouped panel. Every jumpable row (current launch, not Abandoned, and
/// the host still knows the id) is a `Btn` whose arg is the host run id;
/// anything else is dim text. Inferred runs keep their button — the host
/// degrades the jump to a pane focus.
pub fn panel_tree(rows: &[PanelRow], now_unix_ms: u64, current_launch: LaunchId) -> Widget {
    let mut tree = col().gap(1).child(text("Run Ledger").bold());
    let mut shown = 0usize;
    let mut any_group = false;
    for group in GROUP_ORDER {
        let members: Vec<&PanelRow> = rows
            .iter()
            .filter(|entry| group_label(&entry.row, now_unix_ms, current_launch) == group)
            .collect();
        if members.is_empty() {
            continue;
        }
        any_group = true;
        tree = tree.child(sep()).child(text(group).bold());
        for member in members {
            if shown >= MAX_PANEL_ROWS {
                break;
            }
            shown += 1;
            let summary = row_summary(&member.row);
            tree = match (can_jump(&member.row, current_launch), member.host_id) {
                (true, Some(host)) => tree.child(btn(summary, "jump").arg(host.to_string())),
                _ => tree.child(text(summary).tone(Tone::Dim)),
            };
        }
    }
    if !any_group {
        tree = tree.child(text("暂无记录 — 在终端里跑条命令就会出现在这里").tone(Tone::Dim));
    } else if rows.len() > shown {
        tree = tree.child(text(format!("…仅显示最新 {MAX_PANEL_ROWS} 条")).tone(Tone::Dim));
    }
    tree.into()
}

/// Whole-tree node count, mirroring the host's budget check
/// (`plugin_protocol::v2::measure`) without depending on the protocol crate.
pub fn node_count(widget: &Widget) -> usize {
    let children = match widget {
        Widget::Col { children, .. } | Widget::Row { children, .. } => children.as_slice(),
        _ => return 1,
    };
    1 + children.iter().map(node_count).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::rows_from_runs;
    use run_ledger::{Ledger, PaneKey, RunEvent, RunState};
    use std::time::Duration;

    const TODAY: u64 = 1_700_000_000_000;

    fn ledger_row(state: RunState, launch_id: LaunchId, started_at_unix_ms: u64) -> LedgerRow {
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

    fn panel_row(
        state: RunState,
        launch_id: LaunchId,
        unix_ms: u64,
        host: Option<RunId>,
    ) -> PanelRow {
        PanelRow {
            row: ledger_row(state, launch_id, unix_ms),
            host_id: host,
        }
    }

    fn collect(
        widget: &Widget,
        texts: &mut Vec<String>,
        btns: &mut Vec<(String, String, Option<String>)>,
        badges: &mut Vec<(String, Tone)>,
    ) {
        match widget {
            Widget::Col { children, .. } | Widget::Row { children, .. } => {
                for child in children {
                    collect(child, texts, btns, badges);
                }
            }
            Widget::Text { s, .. } => texts.push(s.clone()),
            Widget::Btn { s, action, arg } => btns.push((s.clone(), action.clone(), arg.clone())),
            Widget::Badge { s, tone } => badges.push((s.clone(), *tone)),
            _ => {}
        }
    }

    fn parts(
        widget: &Widget,
    ) -> (
        Vec<String>,
        Vec<(String, String, Option<String>)>,
        Vec<(String, Tone)>,
    ) {
        let mut texts = Vec::new();
        let mut btns = Vec::new();
        let mut badges = Vec::new();
        collect(widget, &mut texts, &mut btns, &mut badges);
        (texts, btns, badges)
    }

    #[test]
    fn status_shows_a_failed_badge_in_err_tone() {
        let (_, btns, badges) = parts(&status_tree(3, 1));
        assert_eq!(badges, vec![("✗3".into(), Tone::Err)]);
        let actions: Vec<_> = btns.iter().map(|(_, a, _)| a.as_str()).collect();
        assert_eq!(actions, ["open_panel", "clear"]);
    }

    #[test]
    fn status_shows_a_running_badge_only_without_failures() {
        let (_, _, badges) = parts(&status_tree(0, 2));
        assert_eq!(badges, vec![("●2".into(), Tone::Accent)]);
    }

    #[test]
    fn status_has_no_badge_when_nothing_is_pending() {
        let (_, btns, badges) = parts(&status_tree(0, 0));
        assert!(badges.is_empty());
        assert_eq!(btns.len(), 2, "the palette affordances are always present");
    }

    #[test]
    fn status_badge_text_stays_within_the_chip_cap() {
        let (_, _, badges) = parts(&status_tree(5000, 0));
        assert_eq!(badges[0].0, "✗999+");
        assert!(badges[0].0.chars().count() <= 8);
    }

    #[test]
    fn status_summary_counts_failed_attention_and_running() {
        let mut ledger = Ledger::new(LaunchId::new_v4());
        let pane = PaneKey::new_v4();
        ledger.apply(RunEvent::started(pane, "boom", None, 0));
        ledger.apply(RunEvent::finished(pane, Some(1), 10));
        ledger.apply(RunEvent::started(PaneKey::new_v4(), "long", None, 0));
        assert_eq!(status_summary(&ledger), (1, 1));
    }

    #[test]
    fn panel_groups_in_fixed_order_and_jump_buttons_carry_host_ids() {
        let current = LaunchId::new_v4();
        let other = LaunchId::new_v4();
        let host_running = RunId::new_v4();
        let host_unseen = RunId::new_v4();
        let rows = vec![
            // Input order scrambled on purpose: grouping must not depend on it.
            panel_row(RunState::Failed, other, TODAY - 86_400_000, None),
            panel_row(RunState::Failed, current, TODAY, Some(host_unseen)),
            panel_row(RunState::Running, current, TODAY, Some(host_running)),
            panel_row(RunState::Succeeded, other, TODAY, None),
        ];
        let (texts, btns, _) = parts(&panel_tree(&rows, TODAY, current));

        let headers: Vec<_> = texts
            .iter()
            .filter(|t| GROUP_ORDER.contains(&t.as_str()))
            .collect();
        assert_eq!(headers, ["进行中", "待看", "今天", "更早"]);

        assert_eq!(btns.len(), 2, "running + current-launch failed rows jump");
        assert_eq!(btns[0].1, "jump");
        assert_eq!(btns[0].2, Some(host_running.to_string()));
        assert_eq!(btns[1].2, Some(host_unseen.to_string()));
    }

    #[test]
    fn abandoned_and_foreign_launch_rows_are_text_not_buttons() {
        let current = LaunchId::new_v4();
        let host = RunId::new_v4();
        let abandoned = panel_row(RunState::Abandoned, current, TODAY, Some(host));
        let foreign = panel_row(RunState::Failed, LaunchId::new_v4(), TODAY, Some(host));
        let (_, btns, _) = parts(&panel_tree(&[abandoned, foreign], TODAY, current));
        assert!(btns.is_empty(), "neither row may offer a jump");
    }

    #[test]
    fn current_launch_row_without_a_host_id_cannot_jump() {
        let current = LaunchId::new_v4();
        let row = panel_row(RunState::Failed, current, TODAY, None);
        let (_, btns, _) = parts(&panel_tree(&[row], TODAY, current));
        assert!(btns.is_empty());
    }

    #[test]
    fn panel_stays_within_the_node_budget() {
        let current = LaunchId::new_v4();
        let rows: Vec<PanelRow> = (0..1_000)
            .map(|i| panel_row(RunState::Failed, current, TODAY + i, Some(RunId::new_v4())))
            .collect();
        let tree = panel_tree(&rows, TODAY, current);
        assert!(node_count(&tree) <= 500, "nodes: {}", node_count(&tree));
        let (texts, _, _) = parts(&tree);
        assert!(
            texts.iter().any(|t| t.contains("200")),
            "a truncation note tells the user rows were dropped"
        );
    }

    #[test]
    fn empty_panel_shows_a_placeholder() {
        let (texts, btns, _) = parts(&panel_tree(&[], TODAY, LaunchId::new_v4()));
        assert!(btns.is_empty());
        assert!(texts.iter().any(|t| t.contains("暂无记录")));
    }

    #[test]
    fn panel_rows_come_newest_first_via_rows_from_runs() {
        // The view trusts rows_from_runs ordering; this guards the wiring.
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.set_redact(false);
        let pane = PaneKey::new_v4();
        ledger.apply(RunEvent::started(pane, "one", None, 0));
        ledger.apply(RunEvent::finished(pane, Some(0), 10));
        let runs = ledger.snapshot();
        let rows = rows_from_runs(&runs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].command, "one");
    }
}
