//! Plugin-owned agent state: which pane runs (or last ran) a known coding
//! agent, and the most conservative status the observed facts support.
//!
//! Everything here is derived from six host events (`ForegroundChanged`,
//! `RunStarted`, `RunFinished`, `PaneFocused`, `PaneClosed`, `CwdChanged`).
//! Those events prove very little about an agent's inner life, so the state
//! machine is deliberately pessimistic (see the tickets under
//! `.scratch/agent-collaboration/issues/`):
//!
//! - Every status is **process/session status, never turn or task progress**.
//!   A quiet pane, a restored prompt, or a live process do not prove a task
//!   finished or succeeded. There is no "done %", no "ready for work", and
//!   no "waiting for approval" state: the current protocol carries no such
//!   fact, so this plugin never shows one.
//! - [`AgentStatus::Running`] means exactly that the shell Run containing
//!   the agent process is still open. An interactive agent's launch Run
//!   stays open the whole session — including while the agent sits at its
//!   prompt waiting for input — so `Running` does not mean the agent is
//!   computing.
//! - [`AgentStatus::ExitedUnseen`] / [`AgentStatus::ExitedSeen`] mean the
//!   latest agent session in the pane ended, proven by the pinned containing
//!   Run's `RunFinished`. Exit implies nothing about success — interrupts,
//!   denials, and crashes also end sessions.
//! - [`AgentStatus::Unknown`] covers a pane where an agent is foreground but
//!   no containing run is pinned. With no pinned run, *no* `RunFinished` can
//!   prove an exit there — an arbitrary finish leaves the pane `Unknown`.
//!
//! ## Durable exit records
//!
//! An exit is recorded only when the Run *containing* the agent process
//! finishes — a matching `RunFinished` is the one fact that proves the
//! session ended. `ForegroundChanged(None)` does **not** prove an exit: the
//! host identity is only the current foreground command, and a transient
//! child/tool process can briefly hold the foreground while the agent stays
//! alive. `None` therefore changes nothing — a `Running` record stays
//! `Running`, an `Unknown` record stays `Unknown`, and the cached active run
//! is preserved, so the matching `RunFinished` can still settle
//! `Running → Exited*` whenever it arrives (host event order is preserved, so
//! a `None` that precedes the finish is just a quiet window). A record never
//! flips from seen back to unseen, and it leaves the panel only when a
//! different agent takes the foreground (a fresh session replaces it) or the
//! pane closes.
//!
//! ## Observation cache
//!
//! The host emits `CwdChanged` before `ForegroundChanged` on first
//! observation, and a `RunStarted` can arrive up to a full foreground-poll
//! interval before the agent is recognized. Events for a not-yet-recognized
//! pane are therefore cached per pane ([`Observation`]) instead of dropped:
//! when `ForegroundChanged(Some(agent))` arrives, the record is initialized
//! from the cached cwd and active run, so an agent whose launch command was
//! already observed starts as `Running`, not `Unknown`.
//!
//! Two pinning rules keep that cache honest:
//!
//! - The containing run is pinned once, at the `Unknown → Running`
//!   transition (recognition adoption or the first `RunStarted` while
//!   tracked), and never replaced while `Running`. The host ledger abandons
//!   a pane's previous run on a new start without delivering a finish for
//!   it, so chasing later starts would let a nested OSC 133 run or a
//!   busy-probe guess masquerade as the session and false-exit it (with a
//!   notification) when the nested run ends.
//! - On identity replacement (a different agent, or the same agent
//!   restarted after a proven exit), a cached run that was the previous
//!   record's pinned containing run is never inherited — the fresh session
//!   starts `Unknown` until a `RunStarted` that postdates the switch.
//!
//! Cleanup rules:
//!
//! - `PaneClosed` drops the record *and* the observation — the pane is gone.
//! - `ForegroundChanged(None)` clears nothing: it is only a loss of
//!   foreground detection (possibly transient), so the record and the cached
//!   run are preserved for the matching `RunFinished` or the agent's
//!   re-detection. A run that starts *after* the `None` — e.g. the next
//!   agent's own launch command after a shell interlude — replaces the
//!   cached run and is adopted normally: in a single-foreground pane, a run
//!   still open when an agent is detected is that agent's launch, so
//!   `Running` is honest there.

use std::collections::BTreeMap;

use sleipnir_plugin::{PaneKey, RunId};

/// What the plugin can honestly claim about one agent pane. Process/session
/// status only — never turn or task progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentStatus {
    /// The shell Run containing the agent process is still open. Says
    /// nothing about whether the agent is computing or waiting at its
    /// prompt — the containing Run stays open either way.
    Running,
    /// The latest agent session in the pane ended and the pane has not been
    /// focused since. "Exited" implies nothing about success.
    ExitedUnseen,
    /// The latest agent session ended and the pane was focused at the time
    /// (or has been focused since), so the human has seen the exit.
    ExitedSeen,
    /// An agent is foreground but no active run has been observed since the
    /// plugin started (or since the agent took the foreground). Busy, idle,
    /// and mid-turn are all possible.
    Unknown,
}

/// The result of applying a `RunFinished`. Carries exactly the presentation
/// data a notification may safely use — the agent id and cwd, both facts the
/// host reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunFinishOutcome {
    /// The finish did not apply to the record: a mismatched/stale run id, a
    /// duplicate finish, shell interlude after the exit was recorded, a pane
    /// with no pinned containing run (`Unknown`), or an untracked pane.
    Ignored,
    /// A tracked session's pinned containing run finished:
    /// `Running → Exited*`. `seen` is true when the pane was focused
    /// (`ExitedSeen`).
    Exited {
        agent: String,
        cwd: Option<String>,
        seen: bool,
    },
}

/// One tracked pane: the current agent process, or — once it has exited —
/// the latest detected agent session, kept until a new agent replaces it or
/// the pane closes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneAgent {
    pub pane: PaneKey,
    /// Agent identity as reported by the host (`foreground_changed.agent`),
    /// e.g. `"claude"`, `"codex"`, `"gemini"`, `"opencode"`. Kept verbatim:
    /// the host owns the catalog, and an unrecognized id is still displayed.
    pub agent: String,
    pub cwd: Option<String>,
    pub status: AgentStatus,
    /// The pinned containing run: the host run id that was open when this
    /// session became `Running` (the agent's launch command, which stays
    /// open while the agent runs). Pinned once and never moved — a later
    /// `RunStarted` in the same pane abandons the previous run in the host
    /// ledger without a finish event, so following it would let a nested or
    /// inferred run falsely "end" the session. Only this id's `RunFinished`
    /// proves the exit; it is also the `scroll_to_run` jump target.
    pub last_run: Option<RunId>,
}

/// Facts observed for a pane regardless of whether an agent has been
/// recognized in it yet. See the module docs for why this exists and when
/// each field is cleared.
#[derive(Default)]
struct Observation {
    cwd: Option<String>,
    active_run: Option<RunId>,
}

/// The whole plugin state: agent panes, per-pane observations, and which
/// pane is focused.
#[derive(Default)]
pub struct AgentState {
    panes: BTreeMap<PaneKey, PaneAgent>,
    observations: BTreeMap<PaneKey, Observation>,
    focused: Option<PaneKey>,
}

impl AgentState {
    pub fn new() -> Self {
        Self::default()
    }

    /// `ForegroundChanged`. `Some(agent)` tracks the pane, seeding a fresh
    /// record from the observation cache. `None` only means the foreground
    /// command is not a known agent right now — possibly a transient
    /// child/tool process while the agent stays alive — so it changes
    /// nothing: the record keeps its status and the cached run survives
    /// until its matching `RunFinished` or a new `RunStarted`.
    pub fn foreground_changed(&mut self, pane: PaneKey, agent: Option<String>) {
        let Some(agent) = agent else {
            return;
        };
        let observation = self.observations.entry(pane).or_default();
        match self.panes.get_mut(&pane) {
            // Same agent, session still live: keep the derived status,
            // refresh the cwd from the cache.
            Some(record)
                if record.agent == agent
                    && matches!(record.status, AgentStatus::Running | AgentStatus::Unknown) =>
            {
                record.cwd = observation.cwd.clone();
            }
            // First sighting, a different agent, or the same agent restarted
            // after its exit was recorded: a fresh session. Adopt the cached
            // cwd. Adopt the cached active run only when it is not the
            // previous record's pinned containing run — a run opened by the
            // outgoing agent (or a transient child) must not become the new
            // agent's session evidence. A run started after the switch,
            // e.g. the new agent's own launch command, differs and is
            // adopted normally.
            _ => {
                let previous_run = self.panes.get(&pane).and_then(|r| r.last_run);
                let (status, last_run) = match observation.active_run {
                    Some(run) if previous_run != Some(run) => (AgentStatus::Running, Some(run)),
                    _ => (AgentStatus::Unknown, None),
                };
                self.panes.insert(
                    pane,
                    PaneAgent {
                        pane,
                        agent,
                        cwd: observation.cwd.clone(),
                        status,
                        last_run,
                    },
                );
            }
        }
    }

    /// `RunStarted`. Always cached — the pane's agent may only be recognized
    /// a poll interval later. A tracked `Unknown` pane pins this run as its
    /// containing run and becomes `Running`. A pane already `Running` keeps
    /// its pinned run: the host ledger abandons the old run on a new start
    /// without a finish event, so following the new id would let a nested
    /// OSC 133 run or a busy-probe guess falsely "end" the session later.
    /// A run starting in a pane whose record is already `Exited*` is shell
    /// interlude, not the departed agent: the record is untouched and the
    /// run is only cached, ready to be adopted if it turns out to be the
    /// next agent's launch.
    pub fn run_started(&mut self, pane: PaneKey, run_id: RunId) {
        self.observations.entry(pane).or_default().active_run = Some(run_id);
        if let Some(record) = self.panes.get_mut(&pane) {
            if record.status == AgentStatus::Unknown {
                record.status = AgentStatus::Running;
                record.last_run = Some(run_id);
            }
        }
    }

    /// `RunFinished`. Clears the matching cached run. Only the finish of a
    /// tracked pane's pinned containing run records an exit and reports
    /// [`RunFinishOutcome::Exited`]. Everything else is `Ignored`: a
    /// mismatched/stale run id (including the finish of a run that replaced
    /// nothing — the pinned run stays the evidence), a duplicate finish,
    /// shell interlude after the exit, a pane with no pinned run (`Unknown`
    /// has no containing run, so an arbitrary finish proves nothing), and
    /// untracked panes. Seen-ness is preserved: an already-seen record never
    /// goes back. The record keeps `last_run` so an exited row stays
    /// jumpable.
    pub fn run_finished(&mut self, pane: PaneKey, run_id: RunId) -> RunFinishOutcome {
        if let Some(observation) = self.observations.get_mut(&pane) {
            if observation.active_run == Some(run_id) {
                observation.active_run = None;
            }
        }
        let Some(record) = self.panes.get(&pane) else {
            return RunFinishOutcome::Ignored;
        };
        match record.status {
            AgentStatus::Running if record.last_run == Some(run_id) => {
                let seen = self.focused == Some(pane);
                let outcome = RunFinishOutcome::Exited {
                    agent: record.agent.clone(),
                    cwd: record.cwd.clone(),
                    seen,
                };
                self.exit(pane);
                outcome
            }
            _ => RunFinishOutcome::Ignored,
        }
    }

    /// The exit transition, reached only by a `RunFinished` matching the
    /// pinned containing run: exited, seen only if the pane is focused or
    /// the exit was already seen. Never a downgrade back to unseen.
    fn exit(&mut self, pane: PaneKey) {
        let focused = self.focused == Some(pane);
        if let Some(record) = self.panes.get_mut(&pane) {
            record.status = match record.status {
                AgentStatus::ExitedSeen => AgentStatus::ExitedSeen,
                _ if focused => AgentStatus::ExitedSeen,
                AgentStatus::ExitedUnseen => AgentStatus::ExitedUnseen,
                _ => AgentStatus::ExitedUnseen,
            };
        }
    }

    /// `PaneFocused`: remember the focus and mark the pane seen.
    pub fn pane_focused(&mut self, pane: PaneKey) {
        self.focused = Some(pane);
        self.mark_seen(pane);
    }

    /// `PaneClosed`: the pane is gone; drop its record, its observation, and
    /// any focus on it.
    pub fn pane_closed(&mut self, pane: PaneKey) {
        self.panes.remove(&pane);
        self.observations.remove(&pane);
        if self.focused == Some(pane) {
            self.focused = None;
        }
    }

    /// `CwdChanged`: display context only; never affects status. Cached even
    /// for unrecognized panes — the host reports cwd before the agent.
    pub fn cwd_changed(&mut self, pane: PaneKey, cwd: String) {
        self.observations.entry(pane).or_default().cwd = Some(cwd.clone());
        if let Some(record) = self.panes.get_mut(&pane) {
            record.cwd = Some(cwd);
        }
    }

    /// An exited-unseen pane the human has now looked at becomes
    /// `ExitedSeen`. Running and Unknown are untouched: looking at a pane
    /// does not change what the process evidence says.
    pub fn mark_seen(&mut self, pane: PaneKey) {
        if let Some(record) = self.panes.get_mut(&pane) {
            if record.status == AgentStatus::ExitedUnseen {
                record.status = AgentStatus::ExitedSeen;
            }
        }
    }

    /// The pane whose latest observed run is `run_id` (panel jump).
    pub fn pane_for_run(&self, run_id: RunId) -> Option<PaneKey> {
        self.panes
            .values()
            .find(|record| record.last_run == Some(run_id))
            .map(|record| record.pane)
    }

    /// `(running, exited_unseen)` — the only numbers the strip shows.
    pub fn summary(&self) -> (usize, usize) {
        let running = self
            .panes
            .values()
            .filter(|r| r.status == AgentStatus::Running)
            .count();
        let exited = self
            .panes
            .values()
            .filter(|r| r.status == AgentStatus::ExitedUnseen)
            .count();
        (running, exited)
    }

    /// Tracked panes in a stable order: Running first, then ExitedUnseen,
    /// ExitedSeen, Unknown; ties broken by agent id so re-renders do not
    /// shuffle.
    pub fn rows(&self) -> Vec<PaneAgent> {
        let mut rows: Vec<PaneAgent> = self.panes.values().cloned().collect();
        rows.sort_by(|a, b| {
            a.status
                .cmp(&b.status)
                .then_with(|| a.agent.cmp(&b.agent))
                .then_with(|| a.pane.cmp(&b.pane))
        });
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> PaneKey {
        PaneKey::new_v4()
    }

    fn run() -> RunId {
        RunId::new_v4()
    }

    fn only_row(state: &AgentState) -> PaneAgent {
        let rows = state.rows();
        assert_eq!(rows.len(), 1);
        rows.into_iter().next().unwrap()
    }

    #[test]
    fn an_agent_foreground_with_no_run_evidence_is_unknown() {
        let mut state = AgentState::new();
        state.foreground_changed(pane(), Some("claude".into()));
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Unknown);
        assert_eq!(row.agent, "claude");
        assert_eq!(row.last_run, None);
    }

    #[test]
    fn run_started_makes_a_known_agent_running() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("codex".into()));
        state.run_started(p, r);
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(r));
    }

    #[test]
    fn run_started_before_agent_recognition_is_adopted() {
        // The agent's launch command is observed up to a poll interval
        // before ForegroundChanged catches up.
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.run_started(p, r);
        state.foreground_changed(p, Some("claude".into()));
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(r));
    }

    #[test]
    fn cwd_and_run_observed_before_recognition_are_adopted() {
        // Host order on first observation: CwdChanged, then RunStarted, then
        // (up to a second later) ForegroundChanged.
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.cwd_changed(p, "/work/repo".into());
        state.run_started(p, r);
        state.foreground_changed(p, Some("gemini".into()));
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(r));
        assert_eq!(row.cwd.as_deref(), Some("/work/repo"));
    }

    #[test]
    fn cwd_observed_before_recognition_survives_an_unknown_start() {
        let mut state = AgentState::new();
        let p = pane();
        state.cwd_changed(p, "/work/repo".into());
        state.foreground_changed(p, Some("claude".into()));
        let row = only_row(&state);
        assert_eq!(row.cwd.as_deref(), Some("/work/repo"));
        assert_eq!(row.status, AgentStatus::Unknown);
    }

    #[test]
    fn a_run_finished_before_recognition_is_not_adopted() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.run_started(p, r);
        state.run_finished(p, r);
        state.foreground_changed(p, Some("claude".into()));
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Unknown);
        assert_eq!(row.last_run, None);
    }

    #[test]
    fn run_finished_clears_only_the_matching_cached_run() {
        let mut state = AgentState::new();
        let p = pane();
        let old = run();
        let current = run();
        state.run_started(p, old);
        state.run_started(p, current);
        // A late finish for the superseded run must not clear the live one.
        state.run_finished(p, old);
        state.foreground_changed(p, Some("codex".into()));
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(current));
    }

    #[test]
    fn exit_is_recorded_by_run_finished_with_none_before_or_after() {
        for run_finished_first in [true, false] {
            let mut state = AgentState::new();
            let p = pane();
            let r = run();
            state.foreground_changed(p, Some("claude".into()));
            state.run_started(p, r);
            if run_finished_first {
                state.run_finished(p, r);
                assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
                state.foreground_changed(p, None);
            } else {
                // The poll loses the foreground before the finish arrives:
                // nothing exits yet — None alone never proves an exit.
                state.foreground_changed(p, None);
                assert_eq!(only_row(&state).status, AgentStatus::Running);
                state.run_finished(p, r);
            }
            let row = only_row(&state);
            assert_eq!(
                row.status,
                AgentStatus::ExitedUnseen,
                "run_finished_first={run_finished_first}"
            );
            assert_eq!(row.agent, "claude", "the exit record is durable");
            assert_eq!(row.last_run, Some(r), "the exited row stays jumpable");
        }
    }

    #[test]
    fn exit_under_focus_is_seen_in_either_event_order() {
        for run_finished_first in [true, false] {
            let mut state = AgentState::new();
            let p = pane();
            let r = run();
            state.foreground_changed(p, Some("codex".into()));
            state.run_started(p, r);
            state.pane_focused(p);
            if run_finished_first {
                state.run_finished(p, r);
                state.foreground_changed(p, None);
            } else {
                state.foreground_changed(p, None);
                state.run_finished(p, r);
            }
            assert_eq!(
                only_row(&state).status,
                AgentStatus::ExitedSeen,
                "run_finished_first={run_finished_first}"
            );
        }
    }

    #[test]
    fn focus_between_the_exit_and_the_poll_is_not_downgraded() {
        // RunFinished unseen, then the user looks, then the foreground poll
        // fires None: the record must stay seen.
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        state.run_finished(p, r);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
        state.pane_focused(p);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedSeen);
        state.foreground_changed(p, None);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedSeen);
    }

    #[test]
    fn a_transient_none_preserves_running_and_the_cached_run() {
        // A child/tool process briefly holds the foreground while the agent
        // stays alive: the record must not exit, and the containing run must
        // survive so its later RunFinished can still settle the exit.
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        state.foreground_changed(p, None);
        assert_eq!(only_row(&state).status, AgentStatus::Running);
        // The agent reappears: same live session, still the same record.
        state.foreground_changed(p, Some("claude".into()));
        assert_eq!(only_row(&state).status, AgentStatus::Running);
        // The containing run finally finishes: now the exit is proven.
        state.run_finished(p, r);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
    }

    #[test]
    fn foreground_none_keeps_an_unknown_record_unknown() {
        // No run was ever observed; losing the foreground detection says
        // nothing about an exit either. Unknown stays Unknown — and with no
        // pinned run, a later finish still proves nothing.
        let mut state = AgentState::new();
        let p = pane();
        state.foreground_changed(p, Some("gemini".into()));
        state.foreground_changed(p, None);
        assert_eq!(only_row(&state).status, AgentStatus::Unknown);
        assert_eq!(state.run_finished(p, run()), RunFinishOutcome::Ignored);
        assert_eq!(only_row(&state).status, AgentStatus::Unknown);
    }

    #[test]
    fn focusing_after_exit_marks_the_record_seen() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("gemini".into()));
        state.run_started(p, r);
        state.run_finished(p, r);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
        state.pane_focused(p);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedSeen);
    }

    #[test]
    fn a_later_run_started_does_not_move_the_pinned_containing_run() {
        // Nested OSC 133 (or a busy-probe guess) starts a second run while
        // the agent's launch run is open. The host ledger abandons the
        // first run without a finish; the pinned run must not follow, and
        // the second run's finish must not read as the session exit.
        let mut state = AgentState::new();
        let p = pane();
        let launch = run();
        let nested = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, launch);
        state.run_started(p, nested);
        assert_eq!(only_row(&state).last_run, Some(launch), "pinned");
        assert_eq!(state.run_finished(p, nested), RunFinishOutcome::Ignored);
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(launch));
        // The pinned containing run's finish is still the exit proof.
        assert!(matches!(
            state.run_finished(p, launch),
            RunFinishOutcome::Exited { seen: false, .. }
        ));
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
    }

    #[test]
    fn an_unknown_pane_finish_proves_nothing() {
        // No pinned containing run: no RunFinished can prove an exit. A
        // delayed finish from a previous command in the pane, delivered
        // after recognition, must not mark a live agent exited.
        let mut state = AgentState::new();
        let p = pane();
        state.foreground_changed(p, Some("claude".into()));
        assert_eq!(state.run_finished(p, run()), RunFinishOutcome::Ignored);
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Unknown);
        assert_eq!(row.last_run, None);
    }

    #[test]
    fn a_matching_finish_reports_exited_with_presentation_data() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.cwd_changed(p, "/work/repo".into());
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        let outcome = state.run_finished(p, r);
        assert_eq!(
            outcome,
            RunFinishOutcome::Exited {
                agent: "claude".into(),
                cwd: Some("/work/repo".into()),
                seen: false,
            }
        );
    }

    #[test]
    fn a_matching_finish_in_the_focused_pane_reports_seen() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("codex".into()));
        state.run_started(p, r);
        state.pane_focused(p);
        let outcome = state.run_finished(p, r);
        assert!(matches!(
            outcome,
            RunFinishOutcome::Exited { seen: true, .. }
        ));
    }

    #[test]
    fn mismatched_duplicate_and_untracked_finishes_are_ignored() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        // Stale/foreign run id: the containing run is still open.
        assert_eq!(state.run_finished(p, run()), RunFinishOutcome::Ignored);
        assert_eq!(only_row(&state).status, AgentStatus::Running);
        // The real exit, then a duplicate delivery of the same finish:
        // Ignored the second time, so a notifier fires exactly once.
        assert!(matches!(
            state.run_finished(p, r),
            RunFinishOutcome::Exited { seen: false, .. }
        ));
        assert_eq!(state.run_finished(p, r), RunFinishOutcome::Ignored);
        // An untracked pane's finish is never notification data.
        assert_eq!(state.run_finished(pane(), run()), RunFinishOutcome::Ignored);
    }

    #[test]
    fn shell_interlude_finishes_are_ignored_after_the_exit() {
        let mut state = AgentState::new();
        let p = pane();
        let agent_run = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, agent_run);
        state.run_finished(p, agent_run);
        let shell = run();
        state.run_started(p, shell);
        assert_eq!(state.run_finished(p, shell), RunFinishOutcome::Ignored);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
    }

    #[test]
    fn shell_interlude_does_not_disturb_the_exit_record() {
        // After the agent exits, plain shell runs are cached (so the next
        // agent can adopt its launch) but must not touch the record.
        let mut state = AgentState::new();
        let p = pane();
        let agent_run = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, agent_run);
        state.run_finished(p, agent_run);
        state.foreground_changed(p, None);
        let shell = run();
        state.run_started(p, shell);
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::ExitedUnseen);
        assert_eq!(row.last_run, Some(agent_run));
        state.run_finished(p, shell);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
    }

    #[test]
    fn a_new_agent_replaces_the_exit_record_and_adopts_its_launch_run() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.cwd_changed(p, "/work/repo".into());
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        state.run_finished(p, r); // the containing run finished: exit proven
        state.foreground_changed(p, None);
        // Shell interlude, then the next agent launches.
        let launch = run();
        state.run_started(p, launch);
        state.foreground_changed(p, Some("codex".into()));
        let row = only_row(&state);
        assert_eq!(row.agent, "codex");
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(launch));
        assert_eq!(row.cwd.as_deref(), Some("/work/repo"));
    }

    #[test]
    fn the_same_agent_restarted_after_exit_is_a_fresh_session() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        state.run_finished(p, r);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
        // claude again: not a continuation of the exited record.
        let relaunch = run();
        state.run_started(p, relaunch);
        state.foreground_changed(p, Some("claude".into()));
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(relaunch));
    }

    #[test]
    fn identity_replacement_never_inherits_the_previous_agents_run() {
        // claude is Running with R1 still cached; the poll switches the
        // foreground to codex (transient child, suspend, or a fast swap)
        // before any new RunStarted. R1 is claude's containing run — codex
        // must start Unknown, and R1's finish must not become "codex
        // session exited" (which would also notify under the wrong name).
        let mut state = AgentState::new();
        let p = pane();
        let r1 = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r1);
        state.foreground_changed(p, None); // transient: clears nothing
        state.foreground_changed(p, Some("codex".into()));
        let row = only_row(&state);
        assert_eq!(row.agent, "codex");
        assert_eq!(row.status, AgentStatus::Unknown);
        assert_eq!(row.last_run, None);
        assert_eq!(state.run_finished(p, r1), RunFinishOutcome::Ignored);
        assert_eq!(only_row(&state).status, AgentStatus::Unknown);
        // A run started after the switch (codex's real launch) pins normally.
        let r2 = run();
        state.run_started(p, r2);
        let row = only_row(&state);
        assert_eq!(row.status, AgentStatus::Running);
        assert_eq!(row.last_run, Some(r2));
    }

    #[test]
    fn the_same_live_agent_re_reported_keeps_its_status() {
        let mut state = AgentState::new();
        let p = pane();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, run());
        state.foreground_changed(p, Some("claude".into()));
        assert_eq!(only_row(&state).status, AgentStatus::Running);
    }

    #[test]
    fn an_unrecognized_agent_id_is_tracked_verbatim() {
        let mut state = AgentState::new();
        state.foreground_changed(pane(), Some("some-future-agent".into()));
        let row = only_row(&state);
        assert_eq!(row.agent, "some-future-agent");
        assert_eq!(row.status, AgentStatus::Unknown);
    }

    #[test]
    fn pane_closed_drops_the_exit_record_the_observation_and_the_focus() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.cwd_changed(p, "/work/repo".into());
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        state.run_finished(p, r);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
        state.pane_closed(p);
        assert!(state.rows().is_empty(), "even a durable exit record goes");
        // Focus must not linger on a dead pane: a proven exit elsewhere is
        // unseen.
        let other = pane();
        let r2 = run();
        state.foreground_changed(other, Some("codex".into()));
        state.run_started(other, r2);
        state.run_finished(other, r2);
        assert_eq!(only_row(&state).status, AgentStatus::ExitedUnseen);
    }

    #[test]
    fn a_reused_pane_key_inherits_nothing_from_the_closed_pane() {
        let mut state = AgentState::new();
        let p = pane();
        state.cwd_changed(p, "/old/dir".into());
        state.run_started(p, run());
        state.foreground_changed(p, Some("claude".into()));
        state.pane_closed(p);
        state.foreground_changed(p, Some("codex".into()));
        let row = only_row(&state);
        assert_eq!(row.agent, "codex");
        assert_eq!(row.status, AgentStatus::Unknown);
        assert_eq!(row.cwd, None);
        assert_eq!(row.last_run, None);
    }

    #[test]
    fn cwd_is_cached_for_unrecognized_panes_without_creating_a_row() {
        let mut state = AgentState::new();
        state.cwd_changed(pane(), "/elsewhere".into());
        assert!(state.rows().is_empty());
    }

    #[test]
    fn pane_for_run_finds_the_jumped_pane() {
        let mut state = AgentState::new();
        let p = pane();
        let r = run();
        state.foreground_changed(p, Some("claude".into()));
        state.run_started(p, r);
        assert_eq!(state.pane_for_run(r), Some(p));
        assert_eq!(state.pane_for_run(run()), None);
    }

    #[test]
    fn summary_counts_running_and_exited_unseen() {
        let mut state = AgentState::new();
        let running = pane();
        let exited_unseen = pane();
        let exited_seen = pane();
        let unknown = pane();
        state.foreground_changed(running, Some("claude".into()));
        state.foreground_changed(exited_unseen, Some("codex".into()));
        state.foreground_changed(exited_seen, Some("gemini".into()));
        state.foreground_changed(unknown, Some("opencode".into()));
        state.run_started(running, run());
        let r1 = run();
        state.run_started(exited_unseen, r1);
        state.run_finished(exited_unseen, r1);
        let r2 = run();
        state.run_started(exited_seen, r2);
        state.run_finished(exited_seen, r2);
        state.pane_focused(exited_seen);
        assert_eq!(state.summary(), (1, 1));
    }

    #[test]
    fn rows_are_grouped_by_status_then_agent() {
        let mut state = AgentState::new();
        let unknown = pane();
        let running = pane();
        let exited = pane();
        state.foreground_changed(unknown, Some("zzz".into()));
        state.foreground_changed(running, Some("codex".into()));
        state.foreground_changed(exited, Some("claude".into()));
        state.run_started(running, run());
        let r = run();
        state.run_started(exited, r);
        state.run_finished(exited, r);
        let order: Vec<_> = state.rows().iter().map(|r| r.status).collect();
        assert_eq!(
            order,
            [
                AgentStatus::Running,
                AgentStatus::ExitedUnseen,
                AgentStatus::Unknown
            ]
        );
    }
}
