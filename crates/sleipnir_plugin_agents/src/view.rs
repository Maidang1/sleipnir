//! State → widget trees. Pure, so the Status strip wording and the Panel's
//! node budget are unit testable without a session.
//!
//! Same protocol facts as the Run Ledger view:
//!
//! - The Status strip is at most 24 cells; its `Badge` is also derived into
//!   a tab chip (8-char cap) and its `Btn`s become command palette entries.
//! - A tree is capped at 500 nodes, so rows are hard-capped.
//!
//! Wording stays conservative on purpose: every status is process/session
//! status, never turn or task progress. A row says "running" (the shell Run
//! containing the agent process is still open — including while the agent
//! waits at its prompt), "exited — unseen/seen" (the latest agent session
//! ended; nothing about success), or "unknown". Never a percentage, never
//! "done", never "ready", never "waiting for approval" — the current
//! protocol carries no such facts. The exited-unseen badge is `!N` in Warn
//! tone: an exit is not a success, so no checkmark and no Ok tone. Managed
//! sessions follow the same rule: `TaskStatus` labels describe assignment
//! delivery, never task success.
//!
//! Only *exited* rows are clickable. `scroll_to_run` jumps to the run's
//! start anchor; on a still-open run that would yank the user away from the
//! live output tail they are watching, while on an exited session the anchor
//! is exactly the completed output the user asked to review. Running and
//! unknown rows are dim text by choice — a focus-by-pane host call exists
//! (the coordination adapter uses it), but an observer row that silently
//! yanks scroll position would be the wrong default here.

use std::collections::BTreeMap;

use sleipnir_plugin::{PaneKey, Tone, Widget, badge, btn, col, row, sep, text};

use crate::adapter::ManagedRow;
use crate::state::{AgentStatus, PaneAgent, RunFinishOutcome};

/// One managed coordination session as a panel row. Built by the adapter
/// from `agent_coordination` snapshots; display-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedSessionRow {
    pub session: agent_coordination::AgentSessionId,
    pub kind: agent_coordination::AgentKind,
    pub name: Option<String>,
    /// `true` when the session writer is the human.
    pub human_owned: bool,
    /// `true` while the coordination session is open.
    pub open: bool,
    /// `true` when the session has a bound pane.
    pub bound: bool,
    /// Status of the session's latest task, if any. Display-only; never a
    /// success claim.
    pub latest_task: Option<agent_coordination::TaskStatus>,
    /// The worker's `report_awaiting_human` note, when the latest task is
    /// awaiting a human. Not an approval prompt. Already flattened/clipped
    /// by the adapter.
    pub awaiting_detail: Option<String>,
    /// A short, flattened excerpt of the latest task's recorded result, if
    /// one exists. Never the full result text.
    pub result_excerpt: Option<String>,
}

/// The one notification this plugin sends: a tracked, live agent session's
/// containing run finished while the pane was unfocused
/// (`Running → ExitedUnseen`). Returns the `(title, body)` for
/// `HostCall::Notify`, or `None` for every other outcome — focused exits,
/// orphan finishes, ignored finishes — none of which may notify.
///
/// Wording says only that the session exited and output is ready to review:
/// never success, never task completion, never approval state.
pub fn exit_notice(outcome: &RunFinishOutcome) -> Option<(String, String)> {
    let RunFinishOutcome::Exited {
        agent,
        cwd,
        seen: false,
    } = outcome
    else {
        return None;
    };
    let title = format!("{agent} session exited");
    let body = match cwd.as_deref().and_then(cwd_basename) {
        Some(base) => format!("Output in {base} is ready to review."),
        None => "Output is ready to review.".into(),
    };
    Some((title, body))
}

/// Shared panel row budget, in worst-case nodes: observer rows cost at most
/// 3 (a `Row` of text plus a Release button), managed-session rows at most 6
/// (a `Row` of text plus four buttons). One budget across both sections —
/// headers, title, caption, and truncation notes cost ≈ 20 more, keeping the
/// whole tree under the host's 500-node cap.
pub const PANEL_ROW_NODE_BUDGET: usize = 440;

/// Worst-case node cost of one observer row.
const OBSERVER_ROW_NODES: usize = 3;
/// Worst-case node cost of one managed-session row.
const MANAGED_ROW_NODES: usize = 6;

/// Group order, matching `AgentStatus`'s declaration order. "Running" /
/// "Unknown" describe the current agent process; "Exited" groups describe
/// the latest session.
pub const GROUP_ORDER: [(AgentStatus, &str); 4] = [
    (AgentStatus::Running, "Running"),
    (AgentStatus::ExitedUnseen, "Exited — unseen"),
    (AgentStatus::ExitedSeen, "Exited — seen"),
    (AgentStatus::Unknown, "Unknown"),
];

/// The Status strip: at most one summary badge, then the palette-contributing
/// button. Unseen exits outrank a managed session waiting on a human, which
/// outranks running agents — the earlier ones are the rows asking for eyes.
/// `!N` (Warn) means "an agent session ended", not that it succeeded;
/// `◐N` (Warn) means "a managed session's native agent waits on a human";
/// `●N` (Accent) only means a containing Run is open.
pub fn status_tree(running: usize, exited_unseen: usize, awaiting_managed: usize) -> Widget {
    let mut strip = row().gap(1);
    if exited_unseen > 0 {
        strip = strip.child(badge(
            format!("!{}", short_count(exited_unseen)),
            Tone::Warn,
        ));
    } else if awaiting_managed > 0 {
        strip = strip.child(badge(
            format!("◐{}", short_count(awaiting_managed)),
            Tone::Warn,
        ));
    } else if running > 0 {
        strip = strip.child(badge(format!("●{}", short_count(running)), Tone::Accent));
    }
    strip.child(btn("Agents", "open_panel")).into()
}

/// Badge text is also a tab chip (8-char cap), so counts fold at 999.
fn short_count(n: usize) -> String {
    if n > 999 {
        "999+".into()
    } else {
        n.to_string()
    }
}

/// The grouped panel. Only exited rows (seen or unseen) with a run id from
/// this session are `Btn`s that jump via the host's `scroll_to_run`; every
/// other row is dim text. Rows for panes the adapter manages are marked
/// `·managed`; a human-owned managed row additionally carries a **Release**
/// button — the human's explicit, host-side path to hand the session back
/// to the coordinator (never a coordinator-wire op).
///
/// Below the observer groups, a "Managed sessions" section lists every
/// coordination session from the shared registry — open and closed — with
/// kind, name, short session id, writer, binding, and latest task status
/// (delivery wording only, never success), plus the actions the ownership
/// state allows. Interrupt exists only on coordinator-owned rows whose
/// latest task is in flight; Close only on coordinator-owned rows;
/// human-owned rows get Release; closed rows get nothing.
///
/// Both sections share [`PANEL_ROW_NODE_BUDGET`]: rows stop rendering when
/// the next row would exceed it, and a truncation note says so.
pub fn panel_tree(
    rows: &[PaneAgent],
    managed: &BTreeMap<PaneKey, ManagedRow>,
    sessions: &[ManagedSessionRow],
) -> Widget {
    let mut tree = col()
        .gap(1)
        .child(text("Agents").bold())
        .child(text("Process/session status only — not task progress").tone(Tone::Dim));
    let mut spent = 0usize;
    let mut truncated = false;
    let mut any_group = false;
    'groups: for (status, label) in GROUP_ORDER {
        let members: Vec<&PaneAgent> = rows.iter().filter(|r| r.status == status).collect();
        if members.is_empty() {
            continue;
        }
        // Past the budget, stop whole-group: a header with zero rows under
        // it is worse than no header.
        if spent + OBSERVER_ROW_NODES > PANEL_ROW_NODE_BUDGET {
            truncated = true;
            break;
        }
        any_group = true;
        tree = tree.child(sep()).child(text(label).bold());
        for member in members {
            if spent + OBSERVER_ROW_NODES > PANEL_ROW_NODE_BUDGET {
                truncated = true;
                break 'groups;
            }
            spent += OBSERVER_ROW_NODES;
            let mut summary = row_summary(member);
            let managed_row = managed.get(&member.pane);
            if managed_row.is_some() {
                summary.push_str(" ·managed");
            }
            tree = match managed_row {
                Some(m) if m.human_owned => tree.child(
                    row()
                        .gap(1)
                        .child(text(summary).tone(Tone::Dim))
                        .child(btn("Release", "release").arg(m.session.as_uuid().to_string())),
                ),
                _ => match (can_jump(member), member.last_run) {
                    (true, Some(run)) => tree.child(btn(summary, "jump").arg(run.to_string())),
                    _ => tree.child(text(summary).tone(Tone::Dim)),
                },
            };
        }
    }
    if !any_group {
        tree = tree.child(
            text("No known agents — start one (claude, codex, gemini, opencode, …) in a pane")
                .tone(Tone::Dim),
        );
    }
    if !sessions.is_empty() {
        // A header with no row under it is worse than no header.
        if spent + MANAGED_ROW_NODES <= PANEL_ROW_NODE_BUDGET {
            tree = tree.child(sep()).child(text("Managed sessions").bold());
            for session in sessions {
                if spent + MANAGED_ROW_NODES > PANEL_ROW_NODE_BUDGET {
                    truncated = true;
                    break;
                }
                spent += MANAGED_ROW_NODES;
                tree = tree.child(managed_session_row(session));
            }
        } else {
            truncated = true;
        }
    }
    if truncated {
        tree = tree.child(text("…truncated to fit the panel budget").tone(Tone::Dim));
    }
    tree.into()
}

/// One managed session: a summary text plus the buttons its state allows.
/// Six nodes at most (row, text, four buttons). Closed rows are dim text
/// only — nothing can act on a closed session.
fn managed_session_row(session: &ManagedSessionRow) -> Widget {
    let mut summary = format!(
        "{} {} · {} · {}",
        kind_label(session.kind),
        session
            .name
            .clone()
            .unwrap_or_else(|| short_session_id(session.session)),
        if session.human_owned {
            "Human"
        } else {
            "Coordinator"
        },
        if !session.open {
            "closed"
        } else if session.bound {
            "bound"
        } else {
            "unbound"
        },
    );
    if let Some(status) = session.latest_task {
        summary.push_str(" · ");
        summary.push_str(task_label(status));
    }
    if let Some(detail) = &session.awaiting_detail {
        summary.push_str(" · ");
        summary.push_str(detail);
    }
    if let Some(excerpt) = &session.result_excerpt {
        summary.push_str(" · result: ");
        summary.push_str(excerpt);
    }
    if !session.open {
        return text(summary).tone(Tone::Dim).into();
    }
    let mut line = row().gap(1).child(text(summary));
    let arg = || session.session.as_uuid().to_string();
    if session.bound {
        line = line.child(btn("Focus", "managed_focus").arg(arg()));
    }
    if session.human_owned {
        line = line.child(btn("Release", "release").arg(arg()));
    } else {
        line = line.child(btn("Take over", "managed_takeover").arg(arg()));
        if session.latest_task.is_some_and(|s| s.is_in_flight()) {
            line = line.child(btn("Interrupt", "managed_interrupt").arg(arg()));
        }
        line = line.child(btn("Close", "managed_close").arg(arg()));
    }
    line.into()
}

/// The serde/snake_case spelling of a kind; `Debug` would give `"Codex"`.
fn kind_label(kind: agent_coordination::AgentKind) -> &'static str {
    use agent_coordination::AgentKind;
    match kind {
        AgentKind::Codex => "codex",
        AgentKind::Claude => "claude",
        AgentKind::Gemini => "gemini",
        AgentKind::Opencode => "opencode",
    }
}

/// Delivery wording for a task status. Never a success claim: "settled"
/// means no longer in flight, not that the work succeeded.
fn task_label(status: agent_coordination::TaskStatus) -> &'static str {
    use agent_coordination::TaskStatus;
    match status {
        TaskStatus::Accepted => "accepted",
        TaskStatus::Dispatching => "dispatching",
        TaskStatus::Running => "running",
        TaskStatus::AwaitingHuman => "awaiting human",
        TaskStatus::Interrupting => "interrupting",
        TaskStatus::Settled => "settled",
        TaskStatus::Unknown => "unknown",
        TaskStatus::FailedDelivery => "failed delivery",
    }
}

/// First 8 chars of the session UUID — enough to tell sessions apart when
/// the coordinator gave no name.
fn short_session_id(session: agent_coordination::AgentSessionId) -> String {
    session.as_uuid().to_string().chars().take(8).collect()
}

/// Jumping is only honest for an exited session: the anchor is its
/// completed output. A still-open run's anchor is its start, away from the
/// live tail.
fn can_jump(record: &PaneAgent) -> bool {
    matches!(
        record.status,
        AgentStatus::ExitedUnseen | AgentStatus::ExitedSeen
    )
}

/// `"● claude — repo"` — status icon, agent id, cwd basename.
pub fn row_summary(record: &PaneAgent) -> String {
    let mut summary = format!("{} {}", status_icon(record.status), record.agent);
    if let Some(base) = record.cwd.as_deref().and_then(cwd_basename) {
        summary.push_str(" — ");
        summary.push_str(base);
    }
    summary
}

/// The last non-empty path component ("/work/repo" → "repo"; "/" → "/").
fn cwd_basename(cwd: &str) -> Option<&str> {
    if cwd.is_empty() {
        return None;
    }
    Some(cwd.rsplit('/').find(|part| !part.is_empty()).unwrap_or(cwd))
}

fn status_icon(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "●",
        AgentStatus::ExitedUnseen => "!",
        AgentStatus::ExitedSeen => "○",
        AgentStatus::Unknown => "?",
    }
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
    use agent_coordination::{AgentKind, TaskStatus};
    use sleipnir_plugin::{PaneKey, RunId};

    fn record(status: AgentStatus, run: Option<RunId>) -> PaneAgent {
        PaneAgent {
            pane: PaneKey::new_v4(),
            agent: "claude".into(),
            cwd: Some("/work/sleipnir".into()),
            status,
            last_run: run,
        }
    }

    fn managed_session(
        name: Option<&str>,
        human_owned: bool,
        bound: bool,
        latest_task: Option<TaskStatus>,
    ) -> ManagedSessionRow {
        ManagedSessionRow {
            session: agent_coordination::AgentSessionId::new(),
            kind: AgentKind::Claude,
            name: name.map(str::to_string),
            human_owned,
            open: true,
            bound,
            latest_task,
            awaiting_detail: None,
            result_excerpt: None,
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
    fn status_shows_unseen_exits_before_awaiting_and_running() {
        let (_, btns, badges) = parts(&status_tree(2, 3, 1));
        assert_eq!(badges, vec![("!3".into(), Tone::Warn)]);
        let actions: Vec<_> = btns.iter().map(|(_, a, _)| a.as_str()).collect();
        assert_eq!(actions, ["open_panel"]);
    }

    #[test]
    fn status_shows_awaiting_managed_before_running() {
        let (_, _, badges) = parts(&status_tree(2, 0, 1));
        assert_eq!(badges, vec![("◐1".into(), Tone::Warn)]);
    }

    #[test]
    fn status_shows_running_only_without_unseen_exits_or_awaiting() {
        let (_, _, badges) = parts(&status_tree(2, 0, 0));
        assert_eq!(badges, vec![("●2".into(), Tone::Accent)]);
    }

    #[test]
    fn status_has_no_badge_when_nothing_is_tracked() {
        let (_, btns, badges) = parts(&status_tree(0, 0, 0));
        assert!(badges.is_empty());
        assert_eq!(btns.len(), 1, "the palette affordance is always present");
    }

    #[test]
    fn status_badge_text_stays_within_the_chip_cap() {
        let (_, _, badges) = parts(&status_tree(0, 5000, 0));
        assert_eq!(badges[0].0, "!999+");
        assert!(badges[0].0.chars().count() <= 8);
        let (_, _, badges) = parts(&status_tree(0, 0, 5000));
        assert_eq!(badges[0].0, "◐999+");
        assert!(badges[0].0.chars().count() <= 8);
        let (_, _, badges) = parts(&status_tree(5000, 0, 0));
        assert_eq!(badges[0].0, "●999+");
        assert!(badges[0].0.chars().count() <= 8);
    }

    #[test]
    fn panel_groups_in_fixed_order() {
        let rows = vec![
            // Input order scrambled on purpose: grouping must not depend on it.
            record(AgentStatus::Unknown, None),
            record(AgentStatus::ExitedSeen, Some(RunId::new_v4())),
            record(AgentStatus::Running, Some(RunId::new_v4())),
            record(AgentStatus::ExitedUnseen, Some(RunId::new_v4())),
        ];
        let (texts, _, _) = parts(&panel_tree(&rows, &BTreeMap::new(), &[]));
        let headers: Vec<_> = texts
            .iter()
            .filter(|t| GROUP_ORDER.iter().any(|(_, label)| label == t))
            .collect();
        assert_eq!(
            headers,
            ["Running", "Exited — unseen", "Exited — seen", "Unknown"]
        );
    }

    #[test]
    fn panel_states_that_status_is_not_task_progress() {
        let (texts, _, _) = parts(&panel_tree(
            &[record(AgentStatus::Running, None)],
            &BTreeMap::new(),
            &[],
        ));
        assert!(
            texts.iter().any(|t| t.contains("not task progress")),
            "the panel must frame every row as process/session status"
        );
    }

    #[test]
    fn exited_rows_jump_and_carry_the_run_id() {
        let run = RunId::new_v4();
        let rows = vec![
            record(AgentStatus::ExitedUnseen, Some(run)),
            record(AgentStatus::ExitedSeen, Some(RunId::new_v4())),
        ];
        let (_, btns, _) = parts(&panel_tree(&rows, &BTreeMap::new(), &[]));
        assert_eq!(btns.len(), 2);
        assert_eq!(btns[0].1, "jump");
        assert_eq!(btns[0].2, Some(run.to_string()));
        assert!(btns[0].0.contains("claude"));
        assert!(
            btns[0].0.contains("sleipnir"),
            "cwd basename: {}",
            btns[0].0
        );
    }

    #[test]
    fn running_rows_are_noninteractive_even_with_a_run_id() {
        // scroll_to_run jumps to the run's *start* anchor; on a live run that
        // pulls the user away from the output tail they are watching.
        let rows = vec![record(AgentStatus::Running, Some(RunId::new_v4()))];
        let (texts, btns, _) = parts(&panel_tree(&rows, &BTreeMap::new(), &[]));
        assert!(btns.is_empty());
        assert!(texts.iter().any(|t| t.contains("● claude")));
    }

    #[test]
    fn unknown_and_runless_rows_are_noninteractive_text() {
        // Unknown panes have no observed run to jump to, and observer rows
        // deliberately stay noninteractive — the row must not pretend to be
        // clickable.
        let rows = vec![
            record(AgentStatus::Unknown, None),
            record(AgentStatus::ExitedUnseen, None),
        ];
        let (texts, btns, _) = parts(&panel_tree(&rows, &BTreeMap::new(), &[]));
        assert!(btns.is_empty());
        assert!(texts.iter().any(|t| t.contains("? claude")));
    }

    #[test]
    fn panel_stays_within_the_node_budget() {
        let rows: Vec<PaneAgent> = (0..1_000)
            .map(|_| record(AgentStatus::ExitedUnseen, Some(RunId::new_v4())))
            .collect();
        let tree = panel_tree(&rows, &BTreeMap::new(), &[]);
        assert!(node_count(&tree) <= 500, "nodes: {}", node_count(&tree));
        let (texts, _, _) = parts(&tree);
        assert!(
            texts.iter().any(|t| t.contains("truncated")),
            "a truncation note tells the user rows were dropped"
        );
    }

    #[test]
    fn a_full_earlier_group_leaves_no_empty_headers() {
        // A Running group that exhausts the shared budget; the Exited groups
        // must not render bare headers with zero rows under them.
        let mut rows: Vec<PaneAgent> = (0..200)
            .map(|_| record(AgentStatus::Running, Some(RunId::new_v4())))
            .collect();
        rows.push(record(AgentStatus::ExitedUnseen, Some(RunId::new_v4())));
        rows.push(record(AgentStatus::ExitedSeen, Some(RunId::new_v4())));
        rows.push(record(AgentStatus::Unknown, None));
        let (texts, btns, _) = parts(&panel_tree(&rows, &BTreeMap::new(), &[]));
        let headers: Vec<_> = texts
            .iter()
            .filter(|t| GROUP_ORDER.iter().any(|(_, label)| label == t))
            .collect();
        assert_eq!(headers, ["Running"], "no empty group headers: {headers:?}");
        assert_eq!(btns.len(), 0, "running rows are text");
        assert!(
            texts.iter().any(|t| t.contains("truncated")),
            "the truncation note still appears"
        );
    }

    #[test]
    fn empty_panel_shows_a_placeholder() {
        let (texts, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &[]));
        assert!(btns.is_empty());
        assert!(texts.iter().any(|t| t.contains("No known agents")));
    }

    #[test]
    fn managed_rows_are_marked_and_stay_noninteractive_when_coordinator_owned() {
        let agent_session = agent_coordination::AgentSessionId::new();
        let rec = record(AgentStatus::Running, None);
        let pane = rec.pane;
        let mut managed = BTreeMap::new();
        managed.insert(
            pane,
            ManagedRow {
                session: agent_session,
                human_owned: false,
            },
        );
        let (texts, btns, _) = parts(&panel_tree(&[rec], &managed, &[]));
        assert!(btns.is_empty(), "running rows never become buttons");
        assert!(texts.iter().any(|t| t.contains("·managed")));
    }

    #[test]
    fn human_owned_managed_row_offers_a_release_button() {
        let agent_session = agent_coordination::AgentSessionId::new();
        let rec = record(AgentStatus::Running, None);
        let mut managed = BTreeMap::new();
        managed.insert(
            rec.pane,
            ManagedRow {
                session: agent_session,
                human_owned: true,
            },
        );
        let (_, btns, _) = parts(&panel_tree(&[rec], &managed, &[]));
        assert_eq!(btns.len(), 1);
        assert_eq!(btns[0].0, "Release");
        assert_eq!(btns[0].1, "release");
        assert_eq!(btns[0].2, Some(agent_session.as_uuid().to_string()));
    }

    #[test]
    fn panel_budget_holds_when_every_row_is_human_owned() {
        let rows: Vec<PaneAgent> = (0..200)
            .map(|_| record(AgentStatus::Running, None))
            .collect();
        let managed: BTreeMap<PaneKey, ManagedRow> = rows
            .iter()
            .map(|r| {
                (
                    r.pane,
                    ManagedRow {
                        session: agent_coordination::AgentSessionId::new(),
                        human_owned: true,
                    },
                )
            })
            .collect();
        let tree = panel_tree(&rows, &managed, &[]);
        assert!(node_count(&tree) <= 500, "nodes: {}", node_count(&tree));
    }

    #[test]
    fn managed_section_has_a_header_only_when_sessions_exist() {
        let (texts, _, _) = parts(&panel_tree(&[], &BTreeMap::new(), &[]));
        assert!(!texts.iter().any(|t| t == "Managed sessions"));
        let sessions = vec![managed_session(None, false, false, None)];
        let (texts, _, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        assert!(texts.iter().any(|t| t == "Managed sessions"));
    }

    #[test]
    fn managed_summary_lists_kind_name_writer_binding_and_task() {
        let sessions = vec![ManagedSessionRow {
            kind: AgentKind::Opencode,
            ..managed_session(Some("scout"), false, true, Some(TaskStatus::AwaitingHuman))
        }];
        let (texts, _, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        let summary = texts
            .iter()
            .find(|t| t.contains("scout"))
            .expect("a managed summary row");
        assert!(summary.starts_with("opencode scout"), "kind: {summary}");
        assert!(summary.contains("· Coordinator ·"), "writer: {summary}");
        assert!(summary.contains("· bound ·"), "binding: {summary}");
        assert!(summary.contains("awaiting human"), "task: {summary}");
    }

    #[test]
    fn nameless_managed_row_falls_back_to_the_short_session_id() {
        let session = agent_coordination::AgentSessionId::new();
        let short: String = session.as_uuid().to_string().chars().take(8).collect();
        let sessions = vec![ManagedSessionRow {
            session,
            ..managed_session(None, true, false, None)
        }];
        let (texts, _, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        let summary = texts
            .iter()
            .find(|t| t.contains(&short))
            .expect("a managed summary row");
        assert!(summary.contains("· Human ·"), "writer: {summary}");
        assert!(summary.contains("unbound"), "binding: {summary}");
        assert_eq!(
            summary.matches('·').count(),
            2,
            "no task segment when latest_task is None: {summary}"
        );
    }

    #[test]
    fn managed_section_never_claims_task_success() {
        let sessions: Vec<ManagedSessionRow> = [
            TaskStatus::Accepted,
            TaskStatus::Dispatching,
            TaskStatus::Running,
            TaskStatus::AwaitingHuman,
            TaskStatus::Interrupting,
            TaskStatus::Settled,
            TaskStatus::Unknown,
            TaskStatus::FailedDelivery,
        ]
        .into_iter()
        .map(|status| managed_session(None, false, true, Some(status)))
        .collect();
        let tree = panel_tree(&[], &BTreeMap::new(), &sessions);
        let (texts, btns, badges) = parts(&tree);
        let mut all = texts.join(" ");
        all.push_str(&btns.iter().map(|(s, _, _)| s.as_str()).collect::<String>());
        all.push_str(&badges.iter().map(|(s, _)| s.as_str()).collect::<String>());
        let all = all.to_lowercase();
        for forbidden in [
            "success",
            "succeeded",
            "complete",
            "completed",
            "done",
            "approv",
        ] {
            assert!(
                !all.contains(forbidden),
                "managed section must not contain {forbidden:?}: {all}"
            );
        }
    }

    #[test]
    fn bound_coordinator_owned_session_offers_focus_takeover_interrupt_close() {
        let session = agent_coordination::AgentSessionId::new();
        let sessions = vec![ManagedSessionRow {
            session,
            ..managed_session(None, false, true, Some(TaskStatus::Running))
        }];
        let (_, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        let labels_actions: Vec<_> = btns
            .iter()
            .map(|(s, a, _)| (s.as_str(), a.as_str()))
            .collect();
        assert_eq!(
            labels_actions,
            [
                ("Focus", "managed_focus"),
                ("Take over", "managed_takeover"),
                ("Interrupt", "managed_interrupt"),
                ("Close", "managed_close"),
            ]
        );
        for (_, _, arg) in &btns {
            assert_eq!(*arg, Some(session.as_uuid().to_string()));
        }
    }

    #[test]
    fn interrupt_is_offered_only_while_the_latest_task_is_in_flight() {
        for status in [
            TaskStatus::Accepted,
            TaskStatus::Dispatching,
            TaskStatus::Running,
            TaskStatus::AwaitingHuman,
            TaskStatus::Interrupting,
        ] {
            let sessions = vec![managed_session(None, false, true, Some(status))];
            let (_, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
            assert!(
                btns.iter().any(|(_, a, _)| a == "managed_interrupt"),
                "{status:?} is in flight"
            );
        }
        for status in [
            TaskStatus::Settled,
            TaskStatus::Unknown,
            TaskStatus::FailedDelivery,
        ] {
            let sessions = vec![managed_session(None, false, true, Some(status))];
            let (_, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
            assert!(
                !btns.iter().any(|(_, a, _)| a == "managed_interrupt"),
                "{status:?} is terminal: nothing to interrupt"
            );
        }
        let sessions = vec![managed_session(None, false, true, None)];
        let (_, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        assert!(!btns.iter().any(|(_, a, _)| a == "managed_interrupt"));
    }

    #[test]
    fn closed_sessions_render_dim_text_with_no_actions() {
        let sessions = vec![ManagedSessionRow {
            open: false,
            result_excerpt: Some("files written".into()),
            ..managed_session(None, false, true, Some(TaskStatus::Settled))
        }];
        let (texts, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        assert!(btns.is_empty(), "nothing acts on a closed session");
        let summary = texts.iter().find(|t| t.contains("closed")).expect("row");
        assert!(
            summary.contains("· result: files written"),
            "result excerpt marker: {summary}"
        );
        assert!(
            !summary.to_lowercase().contains("success"),
            "settled is not success: {summary}"
        );
    }

    #[test]
    fn awaiting_human_detail_is_shown_clipped_and_results_are_never_dumped() {
        // Clipping lives in the adapter; the view renders the pre-clipped
        // strings verbatim and never dumps a full result payload.
        let detail = "needs confirmation for the database m…";
        let excerpt = "wrote 3 files, 12 tests pending";
        let sessions = vec![ManagedSessionRow {
            awaiting_detail: Some(detail.into()),
            result_excerpt: Some(excerpt.into()),
            ..managed_session(None, false, true, Some(TaskStatus::AwaitingHuman))
        }];
        let (texts, _, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        let summary = texts
            .iter()
            .find(|t| t.contains("needs confirmation"))
            .expect("detail row");
        assert!(
            summary.contains(detail),
            "pre-clipped detail is shown verbatim: {summary}"
        );
        assert!(
            summary.contains(&format!("· result: {excerpt}")),
            "bounded excerpt, not a dump: {summary}"
        );
        assert!(!summary.contains("twelve"), "unclipped tail is absent");
        for forbidden in ["success", "succeeded", "100%", "complete"] {
            assert!(
                !summary.to_lowercase().contains(forbidden),
                "never a success claim ({forbidden}): {summary}"
            );
        }
    }

    #[test]
    fn human_owned_session_offers_release_and_never_interrupt_or_close() {
        let session = agent_coordination::AgentSessionId::new();
        let sessions = vec![ManagedSessionRow {
            session,
            ..managed_session(None, true, true, None)
        }];
        let (_, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        let labels_actions: Vec<_> = btns
            .iter()
            .map(|(s, a, _)| (s.as_str(), a.as_str()))
            .collect();
        assert_eq!(
            labels_actions,
            [("Focus", "managed_focus"), ("Release", "release")]
        );
        for (_, _, arg) in &btns {
            assert_eq!(*arg, Some(session.as_uuid().to_string()));
        }
        assert!(
            !btns
                .iter()
                .any(|(_, a, _)| a == "managed_interrupt" || a == "managed_close"),
            "the human's own sessions are theirs — no interrupt/close: {btns:?}"
        );
    }

    #[test]
    fn unbound_session_has_no_focus_button() {
        let sessions = vec![managed_session(
            None,
            false,
            false,
            Some(TaskStatus::Running),
        )];
        let (_, btns, _) = parts(&panel_tree(&[], &BTreeMap::new(), &sessions));
        assert!(!btns.iter().any(|(_, a, _)| a == "managed_focus"));
        assert_eq!(btns.len(), 3, "take over / interrupt / close remain");
    }

    #[test]
    fn panel_budget_is_shared_between_observer_and_managed_sections() {
        // 100 human-owned observer rows (3 nodes each = 300) plus 20 fully
        // buttoned managed rows (6 each = 120) fit the shared 440-node row
        // budget: both sections render in full, no truncation.
        let rows: Vec<PaneAgent> = (0..100)
            .map(|_| record(AgentStatus::Running, None))
            .collect();
        let managed: BTreeMap<PaneKey, ManagedRow> = rows
            .iter()
            .map(|r| {
                (
                    r.pane,
                    ManagedRow {
                        session: agent_coordination::AgentSessionId::new(),
                        human_owned: true,
                    },
                )
            })
            .collect();
        let sessions: Vec<ManagedSessionRow> = (0..20)
            .map(|_| managed_session(None, false, true, Some(TaskStatus::Running)))
            .collect();
        let tree = panel_tree(&rows, &managed, &sessions);
        assert!(node_count(&tree) <= 500, "nodes: {}", node_count(&tree));
        let (texts, _, _) = parts(&tree);
        assert!(texts.iter().any(|t| t == "Managed sessions"));
        assert!(
            !texts.iter().any(|t| t.contains("truncated")),
            "everything fits: {texts:?}"
        );
        let summaries = texts.iter().filter(|t| t.contains("· bound ·")).count();
        assert_eq!(summaries, 20);
    }

    #[test]
    fn managed_section_yields_to_the_shared_budget() {
        // 100 observer rows (300 nodes) leave 140; at 6 nodes per managed row
        // only 23 of 30 sessions fit, with one truncation note for the panel.
        let rows: Vec<PaneAgent> = (0..100)
            .map(|_| record(AgentStatus::Running, None))
            .collect();
        let sessions: Vec<ManagedSessionRow> = (0..30)
            .map(|_| managed_session(None, false, true, Some(TaskStatus::Running)))
            .collect();
        let tree = panel_tree(&rows, &BTreeMap::new(), &sessions);
        assert!(node_count(&tree) <= 500, "nodes: {}", node_count(&tree));
        let (texts, _, _) = parts(&tree);
        let summaries = texts.iter().filter(|t| t.contains("· bound ·")).count();
        assert_eq!(summaries, 23);
        assert!(
            texts.iter().any(|t| t.contains("truncated")),
            "a truncation note tells the user sessions were dropped"
        );
    }

    #[test]
    fn exit_notice_names_the_agent_and_cwd_basename() {
        let outcome = RunFinishOutcome::Exited {
            agent: "claude".into(),
            cwd: Some("/work/sleipnir".into()),
            seen: false,
        };
        let (title, body) = exit_notice(&outcome).unwrap();
        assert_eq!(title, "claude session exited");
        assert_eq!(body, "Output in sleipnir is ready to review.");
    }

    #[test]
    fn exit_notice_omits_the_cwd_when_unknown() {
        let outcome = RunFinishOutcome::Exited {
            agent: "codex".into(),
            cwd: None,
            seen: false,
        };
        let (title, body) = exit_notice(&outcome).unwrap();
        assert_eq!(title, "codex session exited");
        assert_eq!(body, "Output is ready to review.");
    }

    #[test]
    fn exit_notice_never_claims_success_or_task_state() {
        let outcome = RunFinishOutcome::Exited {
            agent: "gemini".into(),
            cwd: Some("/work/repo".into()),
            seen: false,
        };
        let (title, body) = exit_notice(&outcome).unwrap();
        for forbidden in [
            "success",
            "succeeded",
            "complete",
            "completed",
            "done",
            "approv",
        ] {
            assert!(
                !format!("{title} {body}").to_lowercase().contains(forbidden),
                "notification must not contain {forbidden:?}: {title} / {body}"
            );
        }
    }

    #[test]
    fn no_notice_for_seen_or_ignored_finishes() {
        let seen = RunFinishOutcome::Exited {
            agent: "claude".into(),
            cwd: None,
            seen: true,
        };
        assert_eq!(exit_notice(&seen), None, "focused exit: the user saw it");
        assert_eq!(exit_notice(&RunFinishOutcome::Ignored), None);
    }

    #[test]
    fn row_summary_uses_status_icon_and_cwd_basename() {
        assert_eq!(
            row_summary(&record(AgentStatus::Running, None)),
            "● claude — sleipnir"
        );
        assert_eq!(
            row_summary(&record(AgentStatus::ExitedUnseen, None)),
            "! claude — sleipnir"
        );
        assert_eq!(
            row_summary(&record(AgentStatus::ExitedSeen, None)),
            "○ claude — sleipnir"
        );
        assert_eq!(
            row_summary(&record(AgentStatus::Unknown, None)),
            "? claude — sleipnir"
        );
        let mut root = record(AgentStatus::ExitedSeen, None);
        root.cwd = Some("/".into());
        assert_eq!(row_summary(&root), "○ claude — /");
        root.cwd = None;
        assert_eq!(row_summary(&root), "○ claude");
    }
}
