//! The generic adapter: consumes the `agent_coordination` effect log and
//! drives the host's plugin calls. Pure with respect to the host — every
//! side effect goes through the [`HostCalls`] trait, so tests run against a
//! fake without a live terminal.
//!
//! Discipline:
//!
//! - One oldest pending effect per call to [`Adapter::process_one`]; the
//!   plugin's ~50ms tick calls it once, so effects are delivered in order.
//! - Every processed effect gets exactly one acknowledgement naming its
//!   exact `seq`: `BindPane` / `PromptDelivered` / `InterruptDelivered` /
//!   `FocusDelivered` / `CloseDelivered` on success, `DeliveryFailed` on any
//!   refusal or host error. An effect is never left un-acked — except a
//!   host rate limit, which leaves the effect queued (same seq) for a
//!   bounded backoff retry; only after [`MAX_RATE_LIMIT_RETRIES`] consecutive
//!   rate-limit failures does the effect fail delivery.
//! - The registry is the ownership gate: prompts, interrupts, and closes are
//!   re-checked at execution time and fail delivery when the session is
//!   human-owned, closed, or unbound. There is no approval operation —
//!   nothing here ever types yes/no into a native dialog.
//! - A prompt is delivered as an envelope: self-report instructions plus the
//!   original text, unmodified, between `----- task -----` markers. An
//!   envelope over the host's send-text cap is rejected before the host call
//!   and never truncated. A prompt task stays `Running` after delivery. Only
//!   a worker `report-result` / future native `TaskResult`, an interrupt
//!   delivery, or a session close moves it; this adapter never claims turn
//!   completion.
//! - A bound pane whose agent is never detected within
//!   [`LAUNCH_DETECT_TIMEOUT_MS`] closes its coordination session (launch
//!   task → `Unknown`, a terminal non-success state) and stops being
//!   managed; the pane itself is never auto-closed.

use std::collections::BTreeMap;

use agent_coordination::{
    AdapterUpdate, AgentKind, AgentSessionId, CoordinationTaskId, Effect, EffectBody, Registry,
    Request, Response, WireRequest, Writer,
};
use sleipnir_plugin::{Capability, PaneKey};

/// The executable each agent kind maps to. Direct program name, never a
/// shell line — the host spawns `program` + `args` via `open_pane_argv`.
pub fn executable_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Codex => "codex",
        AgentKind::Claude => "claude",
        AgentKind::Gemini => "gemini",
        AgentKind::Opencode => "opencode",
    }
}

/// True when a host-reported foreground agent id names this kind. The host
/// catalog ids match the executable names for the four first-wave kinds.
pub fn kind_matches_agent(kind: AgentKind, agent: &str) -> bool {
    agent == executable_name(kind)
}

/// The neutral launch-detection result. Records only that the agent process
/// was observed — not success, not task completion.
pub const LAUNCH_DETECTED_RESULT: &str = "agent process detected";

/// How long a bound pane may go without a matching foreground detection
/// before the launch is declared undetected (session closes, launch task
/// becomes `Unknown`). Missing binary, instant exit, and a permanently
/// mismatched catalog id all land here.
pub const LAUNCH_DETECT_TIMEOUT_MS: u64 = 15_000;

/// Backoff after a host "rate limited" error. The host limiter window is
/// 5s; 1s retries ride the slide without hammering.
pub const RATE_LIMIT_BACKOFF_MS: u64 = 1_000;

/// Consecutive rate-limit failures on the same effect before it fails
/// delivery for real.
pub const MAX_RATE_LIMIT_RETRIES: u32 = 10;

/// The host message the planner uses for a rejected call
/// (`plugin_host_calls` rate limiter). Matched exactly: a rate limit is a
/// transient retry, anything else is a delivery failure.
pub fn is_rate_limited(message: &str) -> bool {
    message == "rate limited"
}

/// The host's SendText payload cap, in characters (mirrors
/// `plugin_host_calls::MAX_SEND_TEXT_CHARS`, which the plugin cannot import).
/// An envelope that would exceed it is rejected *before* the host call —
/// never truncated.
pub const SEND_TEXT_CAP_CHARS: usize = 8 * 1024;

/// Single-quote a path for display inside shell instructions. The envelope
/// is pasted into an agent's UI, not executed by a shell we control — but
/// the worker will run these commands, so the path must survive one shell
/// word-split.
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Build the prompt envelope pasted into a managed worker's pane: explicit
/// self-report instructions plus the coordinator's original text, unmodified,
/// in a clearly delimited block. Returns `Err` when the total would exceed
/// the host's send-text cap; nothing is ever truncated.
///
/// This text is delivered through the host's paste-aware path; it is not a
/// shell command line and is never executed by us.
pub fn build_prompt_envelope(
    session: AgentSessionId,
    task: CoordinationTaskId,
    text: &str,
    socket: Option<&str>,
) -> Result<String, String> {
    build_prompt_envelope_with_command(session, task, text, socket, "sleipnir-agentctl")
}

fn build_prompt_envelope_with_command(
    session: AgentSessionId,
    task: CoordinationTaskId,
    text: &str,
    socket: Option<&str>,
    command: &str,
) -> Result<String, String> {
    // `session` rides along because report-session-closed names the session,
    // not the task.
    let ctl = match socket {
        Some(path) => format!("{command} --socket {}", shell_quote(path)),
        None => command.to_string(),
    };
    let sid = session.as_uuid();
    let tid = task.as_uuid();
    let envelope = format!(
        "[Sleipnir coordination — worker session {sid}, task {tid}]\n\
         You are a managed worker agent in Sleipnir. Self-report progress with\n\
         the commands below so the coordinator can track this task. These reports\n\
         are coordination metadata only — NEVER use them to answer approval\n\
         prompts; approvals stay with the human watching this pane.\n\
         \n\
         - Starting: no report needed; delivery already marks the task running.\n\
         - Blocked on a person: {ctl} report-awaiting-human {tid} <what you need>\n\
         - Result ready: {ctl} report-result {tid} --stdin (pipe the result on stdin)\n\
         - Actually exiting: {ctl} report-session-closed {sid}\n\
         \n\
         Your task follows, unmodified:\n\
         ----- task -----\n\
         {text}\n\
         ----- end task -----"
    );
    if envelope.chars().count() > SEND_TEXT_CAP_CHARS {
        return Err(format!(
            "prompt envelope exceeds the host send-text cap of {SEND_TEXT_CAP_CHARS} characters"
        ));
    }
    Ok(envelope)
}

/// Panel clip for a worker's `report-awaiting-human` note.
pub const AWAITING_DETAIL_CLIP_CHARS: usize = 40;
/// Panel clip for a recorded task result excerpt. Never the full payload.
pub const RESULT_EXCERPT_CLIP_CHARS: usize = 48;

/// Clip a worker-reported string for panel display: newlines/tabs become
/// spaces and the result is capped at `max` chars with an ellipsis.
pub fn display_clip(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            c => c,
        })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max).collect::<String>())
}

/// The grants effect delivery needs. The coordination socket must not come
/// up without all of them: a coordinator that can launch workers but cannot
/// interrupt or close them is a trap.
pub const DELIVERY_GRANTS: &[Capability] = &[
    Capability::HostCallOpenPane,
    Capability::HostCallSendText,
    Capability::HostCallSendKey,
    Capability::HostCallFocusPane,
    Capability::HostCallRequestClosePane,
];

/// Grants from [`DELIVERY_GRANTS`] the host did not grant. Empty means the
/// coordination socket may come up.
pub fn missing_delivery_grants(granted: &[Capability]) -> Vec<Capability> {
    DELIVERY_GRANTS
        .iter()
        .copied()
        .filter(|cap| !granted.contains(cap))
        .collect()
}

/// The host surface the adapter needs. Production wraps the plugin
/// `Context`; tests use a fake.
pub trait HostCalls {
    fn open_pane_argv(
        &mut self,
        cwd: Option<String>,
        program: &str,
        args: Vec<String>,
    ) -> Result<PaneKey, String>;
    /// Paste-aware text insertion followed by Enter.
    fn send_text_enter(&mut self, pane: PaneKey, text: &str) -> Result<(), String>;
    fn send_key(&mut self, pane: PaneKey, key: &str) -> Result<(), String>;
    fn focus_pane(&mut self, pane: PaneKey) -> Result<(), String>;
    fn request_close_pane(&mut self, pane: PaneKey) -> Result<(), String>;
}

/// One session this adapter launched and tracks.
#[derive(Clone, Debug)]
struct ManagedSession {
    pane: PaneKey,
    kind: AgentKind,
    launch_task: CoordinationTaskId,
    /// When the pane was bound (adapter clock). Drives the launch-detection
    /// timeout.
    bound_at_ms: u64,
    /// Set once the launch task was settled by foreground detection, so a
    /// re-report does not produce a duplicate result.
    detected: bool,
}

/// Display/ownership facts about a managed session, for the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedRow {
    pub session: AgentSessionId,
    pub human_owned: bool,
}

/// The result of executing one effect against the host.
enum ExecOutcome {
    /// Ack the effect's exact seq with these updates, in order. The task is
    /// marked `Running` only after a successful `PromptDelivered` ack.
    Ack(Vec<AdapterUpdate>, Option<CoordinationTaskId>),
    /// The host rate-limited the call. The effect stays queued under the
    /// same seq for a bounded retry; no state changes.
    RateLimited,
}

/// The adapter: owns the shared registry handle and the session↔pane map.
pub struct Adapter {
    registry: Registry,
    sessions: BTreeMap<AgentSessionId, ManagedSession>,
    panes: BTreeMap<PaneKey, AgentSessionId>,
    /// `SLEIPNIR_AGENT_CONTROL_SOCKET` when set: workers get a `--socket`
    /// argument in their prompt envelope.
    socket_override: Option<String>,
    control_command: String,
    /// No effect processing before this time (rate-limit backoff).
    backoff_until_ms: u64,
    /// The seq currently being rate-limited, and how many consecutive times.
    rate_limited_seq: Option<u64>,
    rate_retries: u32,
}

impl Adapter {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            sessions: BTreeMap::new(),
            panes: BTreeMap::new(),
            socket_override: None,
            control_command: "sleipnir-agentctl".into(),
            backoff_until_ms: 0,
            rate_limited_seq: None,
            rate_retries: 0,
        }
    }

    /// The socket path override workers should report to, when the server
    /// bound a non-default path via `SLEIPNIR_AGENT_CONTROL_SOCKET`.
    /// Empty strings are treated as unset so the envelope can use the bare
    /// `sleipnir-agentctl` command (which already honors the same default).
    pub fn set_socket_override(&mut self, socket: Option<String>) {
        self.socket_override = socket.filter(|s| !s.is_empty());
    }

    /// Use the exact shipped executable, not a separately installed PATH tool.
    pub fn set_builtin_executable(&mut self, executable: &std::path::Path) {
        self.control_command = format!("{} agentctl", shell_quote(&executable.to_string_lossy()));
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Process at most one oldest pending effect. Returns true when one was
    /// attempted. Every attempted effect is acked with its exact seq —
    /// success or `DeliveryFailed` — unless the host rate-limited the call,
    /// which leaves the effect queued (same seq) for a bounded retry.
    pub fn process_one(&mut self, host: &mut dyn HostCalls, now_ms: u64) -> bool {
        if now_ms < self.backoff_until_ms {
            return false;
        }
        let Some(effect) = self.registry.peek_effects().into_iter().next() else {
            return false;
        };
        let seq = effect.seq;
        match self.execute(effect, host, now_ms) {
            ExecOutcome::RateLimited => {
                if self.note_rate_limited(seq, now_ms) {
                    // Same seq rate-limited too many times: give up for real.
                    self.fail_delivery(seq, now_ms);
                }
                true
            }
            ExecOutcome::Ack(updates, prompt_task) => {
                self.rate_limited_seq = None;
                self.rate_retries = 0;
                for update in updates {
                    if let Err(err) = self.registry.apply(update, now_ms) {
                        // The registry rejected our ack. Retire the effect so
                        // the next tick cannot re-execute a host side effect
                        // that already ran (a duplicate pane/prompt is worse
                        // than a FailedDelivery mark).
                        eprintln!("agents: coordination ack rejected: {err}");
                        let _ = self
                            .registry
                            .apply(AdapterUpdate::DeliveryFailed { seq }, now_ms);
                        return true;
                    }
                }
                if let Some(task) = prompt_task {
                    // Prompt delivered; the task stays in flight as Running
                    // until a native adapter result, an interrupt, or a
                    // session close moves it.
                    self.mark_prompt_running(task, now_ms);
                }
                true
            }
        }
    }

    /// Count a rate-limit failure for `seq`. Returns true when the retry
    /// budget is exhausted and the caller should fail delivery.
    fn note_rate_limited(&mut self, seq: u64, now_ms: u64) -> bool {
        if self.rate_limited_seq == Some(seq) {
            self.rate_retries += 1;
        } else {
            self.rate_limited_seq = Some(seq);
            self.rate_retries = 1;
        }
        self.backoff_until_ms = now_ms + RATE_LIMIT_BACKOFF_MS;
        self.rate_retries >= MAX_RATE_LIMIT_RETRIES
    }

    fn fail_delivery(&mut self, seq: u64, now_ms: u64) {
        if let Err(err) = self
            .registry
            .apply(AdapterUpdate::DeliveryFailed { seq }, now_ms)
        {
            eprintln!("agents: could not fail delivery: {err}");
        }
    }

    /// Execute one effect against the host and produce its acknowledgement.
    fn execute(&mut self, effect: Effect, host: &mut dyn HostCalls, now_ms: u64) -> ExecOutcome {
        let registry = self.registry.clone();
        let seq = effect.seq;
        registry
            .deliver(&effect, |delivery| {
                match self.execute_claimed(effect.clone(), host, now_ms) {
                    ExecOutcome::Ack(updates, prompt_task) => {
                        for update in updates {
                            if let Err(err) = delivery.apply(update, now_ms) {
                                eprintln!("agents: coordination ack rejected: {err}");
                                let _ =
                                    delivery.apply(AdapterUpdate::DeliveryFailed { seq }, now_ms);
                                return ExecOutcome::Ack(vec![], None);
                            }
                        }
                        if let Some(task) = prompt_task {
                            let _ = delivery.apply(AdapterUpdate::TaskRunning { task }, now_ms);
                        }
                        ExecOutcome::Ack(vec![], None)
                    }
                    ExecOutcome::RateLimited => ExecOutcome::RateLimited,
                }
            })
            .unwrap_or_else(|_| ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None))
    }

    fn execute_claimed(
        &mut self,
        effect: Effect,
        host: &mut dyn HostCalls,
        now_ms: u64,
    ) -> ExecOutcome {
        let seq = effect.seq;
        match effect.body {
            EffectBody::LaunchRequested {
                session,
                task,
                kind,
                cwd,
                args,
                ..
            } => match host.open_pane_argv(Some(cwd), executable_name(kind), args) {
                Ok(pane) => {
                    self.sessions.insert(
                        session,
                        ManagedSession {
                            pane,
                            kind,
                            launch_task: task,
                            bound_at_ms: now_ms,
                            detected: false,
                        },
                    );
                    self.panes.insert(pane, session);
                    ExecOutcome::Ack(vec![AdapterUpdate::BindPane { seq, session, pane }], None)
                }
                Err(message) if is_rate_limited(&message) => ExecOutcome::RateLimited,
                Err(message) => {
                    eprintln!("agents: launch delivery failed: {message}");
                    // Canonical registry failure also closes an unbound launch session.
                    ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None)
                }
            },
            EffectBody::PromptRequested {
                session,
                task,
                text,
                ..
            } => match self.writable_pane(session) {
                None => ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None),
                Some(pane) => {
                    let envelope = build_prompt_envelope_with_command(
                        session,
                        task,
                        &text,
                        self.socket_override.as_deref(),
                        &self.control_command,
                    );
                    match envelope {
                        // Oversize is rejected before the host call; the
                        // coordinator's text is never truncated.
                        Err(message) => {
                            eprintln!("agents: prompt delivery failed: {message}");
                            ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None)
                        }
                        Ok(envelope) => match host.send_text_enter(pane, &envelope) {
                            Ok(()) => ExecOutcome::Ack(
                                vec![AdapterUpdate::PromptDelivered { seq }],
                                Some(task),
                            ),
                            Err(message) if is_rate_limited(&message) => ExecOutcome::RateLimited,
                            Err(message) => {
                                eprintln!("agents: prompt delivery failed: {message}");
                                ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None)
                            }
                        },
                    }
                }
            },
            EffectBody::InterruptRequested { session, .. } => match self.writable_pane(session) {
                None => ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None),
                Some(pane) => match host.send_key(pane, "ctrl-c") {
                    Ok(()) => {
                        ExecOutcome::Ack(vec![AdapterUpdate::InterruptDelivered { seq }], None)
                    }
                    Err(message) if is_rate_limited(&message) => ExecOutcome::RateLimited,
                    Err(message) => {
                        eprintln!("agents: interrupt delivery failed: {message}");
                        ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None)
                    }
                },
            },
            // Focus is visibility-only: no ownership gate, but the pane must
            // still be one we bound.
            EffectBody::FocusRequested { session } => match self.pane_for(session) {
                None => ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None),
                Some(pane) => match host.focus_pane(pane) {
                    Ok(()) => ExecOutcome::Ack(vec![AdapterUpdate::FocusDelivered { seq }], None),
                    Err(message) if is_rate_limited(&message) => ExecOutcome::RateLimited,
                    Err(message) => {
                        eprintln!("agents: focus delivery failed: {message}");
                        ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None)
                    }
                },
            },
            EffectBody::CloseRequested { session } => {
                match self.writable_pane(session) {
                    None => ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None),
                    Some(pane) => match host.request_close_pane(pane) {
                        // Ok means the close *request* was accepted (a busy
                        // pane may show a confirm the user can cancel). The
                        // session closes when the real PaneClosed arrives.
                        Ok(()) => {
                            ExecOutcome::Ack(vec![AdapterUpdate::CloseDelivered { seq }], None)
                        }
                        Err(message) if is_rate_limited(&message) => ExecOutcome::RateLimited,
                        Err(message) => {
                            eprintln!("agents: close delivery failed: {message}");
                            ExecOutcome::Ack(vec![AdapterUpdate::DeliveryFailed { seq }], None)
                        }
                    },
                }
            }
        }
    }

    /// The pane a write may target: session open, coordinator-owned, pane
    /// bound in the registry, and the registry's binding agrees with ours.
    /// Re-checked at execution time because a takeover can land between the
    /// coordinator's request and this tick. Gates prompts, interrupts, and
    /// closes; focus stays visibility-only.
    fn writable_pane(&self, session: AgentSessionId) -> Option<PaneKey> {
        let ours = *self.sessions.get(&session).map(|m| &m.pane)?;
        let snap = self.inspect(session)?;
        if !snap.open || snap.writer != Writer::Coordinator {
            return None;
        }
        if snap.pane != Some(ours) {
            return None;
        }
        Some(ours)
    }

    fn pane_for(&self, session: AgentSessionId) -> Option<PaneKey> {
        self.sessions.get(&session).map(|m| m.pane)
    }

    fn inspect(&self, session: AgentSessionId) -> Option<agent_coordination::SessionSnapshot> {
        let resp = self.registry.handle(
            WireRequest {
                id: 0,
                body: Request::Inspect { session },
            },
            0,
        );
        match resp.body {
            Response::Inspect { session } => Some(session),
            _ => None,
        }
    }

    /// After a successful `PromptDelivered` ack, move the task to Running.
    /// Split from the ack so the effect log is cleared first.
    pub fn mark_prompt_running(&self, task: CoordinationTaskId, now_ms: u64) {
        if let Err(err) = self
            .registry
            .apply(AdapterUpdate::TaskRunning { task }, now_ms)
        {
            eprintln!("agents: could not mark prompt running: {err}");
        }
    }

    /// `ForegroundChanged` on a managed pane: when the reported agent matches
    /// the session's expected kind, settle the launch task with the neutral
    /// detection result. Anything else (transient child, different tool) is
    /// ignored.
    pub fn foreground_changed(&mut self, pane: PaneKey, agent: Option<&str>, now_ms: u64) {
        let Some(agent) = agent else { return };
        let Some(session) = self.panes.get(&pane).copied() else {
            return;
        };
        let Some(managed) = self.sessions.get_mut(&session) else {
            return;
        };
        if managed.detected || !kind_matches_agent(managed.kind, agent) {
            return;
        }
        managed.detected = true;
        let task = managed.launch_task;
        if let Err(err) = self.registry.apply(
            AdapterUpdate::TaskResult {
                task,
                text: LAUNCH_DETECTED_RESULT.into(),
            },
            now_ms,
        ) {
            eprintln!("agents: could not record launch detection: {err}");
        }
    }

    /// `PaneClosed` on a managed pane: the pane is gone, so the coordination
    /// session closes and its in-flight tasks become `Unknown`.
    pub fn pane_closed(&mut self, pane: PaneKey, now_ms: u64) {
        let Some(session) = self.panes.remove(&pane) else {
            return;
        };
        self.sessions.remove(&session);
        self.close_session(session, now_ms);
    }

    /// The agent's containing run finished (the observer only reports this
    /// for the pinned run): the agent process exited, so the coordination
    /// session closes. Never used to settle a prompt task — in-flight tasks
    /// become `Unknown` via the close.
    pub fn containing_run_exited(&mut self, pane: PaneKey, now_ms: u64) {
        let Some(session) = self.panes.remove(&pane) else {
            return;
        };
        self.sessions.remove(&session);
        self.close_session(session, now_ms);
    }

    fn close_session(&mut self, session: AgentSessionId, now_ms: u64) {
        if let Err(err) = self
            .registry
            .apply(AdapterUpdate::SessionClosed { session }, now_ms)
        {
            eprintln!("agents: could not close session: {err}");
        }
    }

    /// Per-tick housekeeping, called whether or not an effect was processed.
    /// A bound pane whose expected agent was never detected within
    /// [`LAUNCH_DETECT_TIMEOUT_MS`] (missing binary, instant exit, or a
    /// permanently mismatched catalog id) closes its coordination session —
    /// the launch task becomes `Unknown`, a terminal non-success state — and
    /// stops being managed. The pane itself is never auto-closed: whatever
    /// the failed launch left on screen belongs to the user. Returns true
    /// when a session timed out.
    pub fn housekeeping(&mut self, now_ms: u64) -> bool {
        let stale: Vec<AgentSessionId> = self
            .sessions
            .iter()
            .filter(|(_, m)| {
                !m.detected && now_ms.saturating_sub(m.bound_at_ms) >= LAUNCH_DETECT_TIMEOUT_MS
            })
            .map(|(id, _)| *id)
            .collect();
        if stale.is_empty() {
            return false;
        }
        for session in stale {
            if let Some(managed) = self.sessions.remove(&session) {
                eprintln!(
                    "agents: launch of session {} never detected a {} process; closing the \
                     coordination session (pane left open)",
                    session.as_uuid(),
                    executable_name(managed.kind)
                );
                self.panes.remove(&managed.pane);
            }
            self.close_session(session, now_ms);
        }
        true
    }

    /// Human-originated release of a human-owned session back to the
    /// coordinator. This is a host-side adapter update, never a
    /// coordinator-wire op.
    pub fn release_to_coordinator(&self, session: AgentSessionId, now_ms: u64) {
        if let Err(err) = self
            .registry
            .apply(AdapterUpdate::ReleaseToCoordinator { session }, now_ms)
        {
            eprintln!("agents: release rejected: {err}");
        }
    }

    /// Display rows for every known session — open and closed — open first
    /// (then name/kind/session id for stability). Latest task = the last
    /// entry in the snapshot's task list; awaiting-human detail and a
    /// flattened, clipped result excerpt ride along (the panel never dumps
    /// full result text and never claims success).
    pub fn managed_session_rows(&self) -> Vec<crate::view::ManagedSessionRow> {
        let mut agents = self.registry.session_summaries();
        agents.sort_by(|a, b| {
            (!a.open, &a.name, executable_name(a.kind), &a.session).cmp(&(
                !b.open,
                &b.name,
                executable_name(b.kind),
                &b.session,
            ))
        });
        agents
            .into_iter()
            .map(|snap| {
                let latest = snap.tasks.last();
                crate::view::ManagedSessionRow {
                    session: snap.session,
                    kind: snap.kind,
                    name: snap.name,
                    human_owned: snap.writer == Writer::Human,
                    open: snap.open,
                    bound: snap.pane.is_some(),
                    latest_task: latest.map(|task| task.status),
                    // Both worker-reported strings are flattened and clipped
                    // here, so the panel never renders raw multi-line text.
                    awaiting_detail: latest.and_then(|task| {
                        (task.status == agent_coordination::TaskStatus::AwaitingHuman)
                            .then(|| task.detail.clone())
                            .flatten()
                            .map(|d| display_clip(&d, AWAITING_DETAIL_CLIP_CHARS))
                    }),
                    result_excerpt: latest.and_then(|task| {
                        self.registry
                            .result_excerpt(task.task, RESULT_EXCERPT_CLIP_CHARS + 1)
                            .map(|r| display_clip(&r, RESULT_EXCERPT_CLIP_CHARS))
                    }),
                }
            })
            .collect()
    }

    /// Open sessions whose latest task is `AwaitingHuman` — the strip's
    /// awaiting-human count.
    pub fn awaiting_human_count(&self) -> usize {
        let agents = self.registry.session_summaries();
        agents
            .iter()
            .filter(|snap| {
                snap.open
                    && snap.tasks.last().map(|task| task.status)
                        == Some(agent_coordination::TaskStatus::AwaitingHuman)
            })
            .count()
    }

    /// The pane this adapter bound for `session`, if any.
    pub fn pane_of(&self, session: AgentSessionId) -> Option<PaneKey> {
        self.pane_for(session)
    }

    /// The human takes over: submit `Request::HumanTakeover` through the
    /// registry (the same op a coordinator would send). The registry's
    /// response error, if any, is returned for the caller to surface.
    pub fn take_over(&self, session: AgentSessionId, now_ms: u64) -> Result<(), String> {
        self.submit(Request::HumanTakeover { session }, now_ms)
    }

    /// Submit `Request::Focus` / `Request::Interrupt` / `Request::Close`
    /// through the registry — never a direct host call — so the
    /// request-level gates and the effect log stay authoritative; delivery
    /// happens on the adapter tick. The registry's response error, if any,
    /// is returned for the caller to surface.
    pub fn request_focus(&self, session: AgentSessionId, now_ms: u64) -> Result<(), String> {
        self.submit(Request::Focus { session }, now_ms)
    }

    /// See [`Self::request_focus`].
    pub fn request_interrupt(&self, session: AgentSessionId, now_ms: u64) -> Result<(), String> {
        self.submit(Request::Interrupt { session }, now_ms)
    }

    /// See [`Self::request_focus`].
    pub fn request_close(&self, session: AgentSessionId, now_ms: u64) -> Result<(), String> {
        self.submit(Request::Close { session }, now_ms)
    }

    /// Send one request through the registry's wire path; surface a
    /// rejection as `Err`.
    fn submit(&self, body: Request, now_ms: u64) -> Result<(), String> {
        let op = match &body {
            Request::HumanTakeover { .. } => "takeover",
            Request::Focus { .. } => "focus",
            Request::Interrupt { .. } => "interrupt",
            Request::Close { .. } => "close",
            _ => "request",
        };
        let resp = self.registry.handle(WireRequest { id: 0, body }, now_ms);
        match resp.body {
            Response::Error { message } => Err(format!("agents: {op} rejected: {message}")),
            _ => Ok(()),
        }
    }

    /// Managed rows for the panel: pane → (session, human-owned), read live
    /// from the registry so ownership is never stale.
    pub fn managed_rows(&self) -> BTreeMap<PaneKey, ManagedRow> {
        self.sessions
            .iter()
            .map(|(session, managed)| {
                let human_owned = self
                    .inspect(*session)
                    .is_some_and(|snap| snap.writer == Writer::Human);
                (
                    managed.pane,
                    ManagedRow {
                        session: *session,
                        human_owned,
                    },
                )
            })
            .collect()
    }

    /// Whether the pane belongs to a managed session.
    pub fn is_managed(&self, pane: PaneKey) -> bool {
        self.panes.contains_key(&pane)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_coordination::{TaskStatus, WireRequest};

    fn ms() -> u64 {
        1_000
    }

    /// Platform-absolute cwd for launch requests: Unix paths verbatim,
    /// `C:\...` on Windows where a leading `/` is not absolute.
    fn abs(path: &str) -> String {
        if cfg!(windows) {
            if let Some(rest) = path.strip_prefix('/') {
                format!(r"C:\{}", rest.replace('/', "\\"))
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        }
    }

    /// Records every call; `fail` makes each host call error with that
    /// message; `rate_limit_remaining` makes the next N calls fail with the
    /// host's exact rate-limit message. Each open gets a fresh pane key.
    #[derive(Default)]
    struct FakeHost {
        opened: Vec<(Option<String>, String, Vec<String>)>,
        texts: Vec<(PaneKey, String)>,
        keys: Vec<(PaneKey, String)>,
        focuses: Vec<PaneKey>,
        closes: Vec<PaneKey>,
        fail: Option<String>,
        rate_limit_remaining: usize,
        delivery_barrier: Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>,
    }

    impl FakeHost {
        fn failing(message: &str) -> Self {
            Self {
                fail: Some(message.into()),
                ..Self::default()
            }
        }

        fn rate_limited(times: usize) -> Self {
            Self {
                rate_limit_remaining: times,
                ..Self::default()
            }
        }

        fn check<T>(&mut self, value: T) -> Result<T, String> {
            if self.rate_limit_remaining > 0 {
                self.rate_limit_remaining -= 1;
                return Err("rate limited".into());
            }
            if let Some(message) = &self.fail {
                return Err(message.clone());
            }
            Ok(value)
        }
    }

    impl HostCalls for FakeHost {
        fn open_pane_argv(
            &mut self,
            cwd: Option<String>,
            program: &str,
            args: Vec<String>,
        ) -> Result<PaneKey, String> {
            self.check(())?;
            self.opened.push((cwd, program.to_string(), args));
            Ok(PaneKey::new_v4())
        }

        fn send_text_enter(&mut self, pane: PaneKey, text: &str) -> Result<(), String> {
            if let Some((entered, release)) = self.delivery_barrier.take() {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
            self.check(())?;
            self.texts.push((pane, text.to_string()));
            Ok(())
        }

        fn send_key(&mut self, pane: PaneKey, key: &str) -> Result<(), String> {
            self.check(())?;
            self.keys.push((pane, key.to_string()));
            Ok(())
        }

        fn focus_pane(&mut self, pane: PaneKey) -> Result<(), String> {
            self.check(())?;
            self.focuses.push(pane);
            Ok(())
        }

        fn request_close_pane(&mut self, pane: PaneKey) -> Result<(), String> {
            self.check(())?;
            self.closes.push(pane);
            Ok(())
        }
    }

    fn call(reg: &Registry, body: Request) -> Response {
        reg.handle(WireRequest { id: 0, body }, ms()).body
    }

    fn launch(reg: &Registry, kind: AgentKind) -> (AgentSessionId, CoordinationTaskId) {
        match call(
            reg,
            Request::Launch {
                kind,
                cwd: abs("/work/repo"),
                name: Some("w".into()),
                args: vec!["--fast".into()],
            },
        ) {
            Response::LaunchAccepted { session, task } => (session, task),
            other => panic!("expected LaunchAccepted, got {other:?}"),
        }
    }

    fn snapshot(reg: &Registry, session: AgentSessionId) -> agent_coordination::SessionSnapshot {
        match call(reg, Request::Inspect { session }) {
            Response::Inspect { session } => session,
            other => panic!("expected Inspect, got {other:?}"),
        }
    }

    fn task_status(reg: &Registry, task: CoordinationTaskId) -> TaskStatus {
        match call(reg, Request::Wait { task }) {
            Response::Wait { status, .. } => status,
            other => panic!("expected Wait, got {other:?}"),
        }
    }

    /// Launch, deliver, and detect; returns session, launch task, pane.
    fn launched_and_detected_pane(
        adapter: &mut Adapter,
        host: &mut FakeHost,
    ) -> (AgentSessionId, CoordinationTaskId, PaneKey) {
        let (session, task) = launch(adapter.registry(), AgentKind::Codex);
        assert!(adapter.process_one(host, ms()));
        let pane = bound_pane(adapter, session);
        adapter.foreground_changed(pane, Some("codex"), ms());
        assert_eq!(task_status(adapter.registry(), task), TaskStatus::Settled);
        (session, task, pane)
    }

    /// The pane the adapter bound for `session`.
    fn bound_pane(adapter: &Adapter, session: AgentSessionId) -> PaneKey {
        snapshot(adapter.registry(), session)
            .pane
            .expect("bound pane")
    }

    #[test]
    fn awaiting_human_report_prevents_an_older_prompt_from_typing() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        let task = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        call(
            adapter.registry(),
            Request::ReportAwaitingHuman {
                task,
                detail: Some("native approval".into()),
            },
        );
        adapter.process_one(&mut host, ms());
        assert!(
            host.texts.is_empty(),
            "never deliver queued input into a native approval"
        );
    }

    #[test]
    fn stale_prompt_snapshot_is_refused_before_host_io() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        let task = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        let stale = adapter.registry().peek_effects()[0].clone();
        call(
            adapter.registry(),
            Request::ReportResult {
                task,
                text: "already done".into(),
            },
        );
        adapter.execute(stale, &mut host, ms());
        assert!(
            host.texts.is_empty(),
            "stale effect must never type into the pane"
        );
    }

    #[test]
    fn takeover_ack_waits_for_prior_host_delivery_without_blocking_reads() {
        use std::sync::mpsc;
        use std::time::Duration;
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        );
        let registry = adapter.registry().clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        host.delivery_barrier = Some((entered_tx, release_rx));
        let delivering = std::thread::spawn(move || adapter.process_one(&mut host, ms()));
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // The global registry lock must not be held during blocked host I/O.
        assert!(matches!(
            call(&registry, Request::List),
            Response::Agents { .. }
        ));
        let (started_tx, started_rx) = mpsc::channel();
        let (ack_tx, ack_rx) = mpsc::channel();
        let taking_over = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            ack_tx
                .send(call(&registry, Request::HumanTakeover { session }))
                .unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let early = ack_rx.recv_timeout(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        delivering.join().unwrap();
        taking_over.join().unwrap();
        assert!(
            early.is_err(),
            "takeover acknowledged while prior input was still delivering"
        );
        assert!(matches!(
            ack_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Response::TakenOver { .. }
        ));
    }

    #[test]
    fn executable_names_map_to_the_four_binaries() {
        assert_eq!(executable_name(AgentKind::Codex), "codex");
        assert_eq!(executable_name(AgentKind::Claude), "claude");
        assert_eq!(executable_name(AgentKind::Gemini), "gemini");
        assert_eq!(executable_name(AgentKind::Opencode), "opencode");
        assert!(kind_matches_agent(AgentKind::Claude, "claude"));
        assert!(!kind_matches_agent(AgentKind::Claude, "codex"));
    }

    #[test]
    fn launch_opens_pane_with_argv_and_binds_the_exact_seq() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, task) = launch(adapter.registry(), AgentKind::Codex);
        let seq = adapter.registry().peek_effects()[0].seq;

        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(
            host.opened,
            vec![(
                Some(abs("/work/repo")),
                "codex".into(),
                vec!["--fast".into()]
            )]
        );
        assert!(
            adapter.registry().peek_effects().is_empty(),
            "BindPane acked the exact seq"
        );
        let pane = bound_pane(&adapter, session);
        assert!(adapter.is_managed(pane));
        assert_eq!(snapshot(adapter.registry(), session).pane, Some(pane));
        // Bound but not yet detected: the launch task is still in flight.
        assert_eq!(
            task_status(adapter.registry(), task),
            TaskStatus::Dispatching
        );
        // The binding is pinned: a rebind to a *different* pane is an error,
        // and acking an unknown seq is an error.
        let err = adapter
            .registry()
            .apply(
                AdapterUpdate::BindPane {
                    seq,
                    session,
                    pane: PaneKey::new_v4(),
                },
                ms(),
            )
            .unwrap_err();
        assert!(err.contains("already bound"), "{err}");
        let err = adapter
            .registry()
            .apply(AdapterUpdate::PromptDelivered { seq: seq + 100 }, ms())
            .unwrap_err();
        assert!(err.contains("unknown effect"), "{err}");
    }

    #[test]
    fn one_effect_per_tick_oldest_first() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _) = launch(adapter.registry(), AgentKind::Claude);
        assert!(adapter.process_one(&mut host, ms()));
        adapter.foreground_changed(bound_pane(&adapter, session), Some("claude"), ms());
        // Queue a focus behind anything else: only the oldest moves per tick.
        assert!(matches!(
            call(adapter.registry(), Request::Focus { session }),
            Response::FocusAccepted { .. }
        ));
        assert!(matches!(
            call(adapter.registry(), Request::Interrupt { session }),
            Response::InterruptAccepted { .. }
        ));
        let pending = adapter.registry().peek_effects();
        assert_eq!(pending.len(), 2);
        let first = pending[0].seq;
        assert!(matches!(pending[0].body, EffectBody::FocusRequested { .. }));
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.focuses.len(), 1);
        assert_eq!(host.keys.len(), 0, "interrupt not processed yet");
        let remaining = adapter.registry().peek_effects();
        assert_eq!(remaining.len(), 1);
        assert_ne!(
            remaining[0].seq, first,
            "the oldest was acked, not the newest"
        );
    }

    #[test]
    fn launch_host_error_fails_delivery_and_reaps_the_session() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::failing("spawn denied");
        let (session, task) = launch(adapter.registry(), AgentKind::Gemini);
        assert!(adapter.process_one(&mut host, ms()));
        assert!(adapter.registry().peek_effects().is_empty());
        assert_eq!(
            task_status(adapter.registry(), task),
            TaskStatus::FailedDelivery
        );
        // H1: the pane-less session is closed with the failure, not left as
        // an open zombie against the session cap.
        assert!(!snapshot(adapter.registry(), session).open);
        match call(adapter.registry(), Request::Close { session }) {
            Response::Error { message } => assert!(message.contains("closed"), "{message}"),
            other => panic!("closing a reaped session must error: {other:?}"),
        }
    }

    #[test]
    fn interrupt_after_takeover_never_touches_the_human_pane() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        adapter.process_one(&mut host, ms());
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
        assert!(matches!(
            call(adapter.registry(), Request::Interrupt { session }),
            Response::InterruptAccepted { .. }
        ));
        // The human takes over between acceptance and delivery.
        call(adapter.registry(), Request::HumanTakeover { session });
        assert!(adapter.process_one(&mut host, ms()));
        assert!(host.keys.is_empty(), "never interrupt a human-owned pane");
        let _ = pane;
        assert!(adapter.registry().peek_effects().is_empty());
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Unknown);
    }

    #[test]
    fn close_after_takeover_never_touches_the_human_pane() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        assert!(matches!(
            call(adapter.registry(), Request::Close { session }),
            Response::CloseAccepted { .. }
        ));
        call(adapter.registry(), Request::HumanTakeover { session });
        assert!(adapter.process_one(&mut host, ms()));
        assert!(host.closes.is_empty(), "never close a human-owned pane");
        assert!(adapter.registry().peek_effects().is_empty());
        // The failed close does not close the session; the human keeps it.
        assert!(snapshot(adapter.registry(), session).open);
    }

    #[test]
    fn full_grant_set_is_required_for_coordination() {
        let all: Vec<Capability> = DELIVERY_GRANTS.to_vec();
        assert!(missing_delivery_grants(&all).is_empty());
        let missing_one: Vec<Capability> = all
            .iter()
            .copied()
            .filter(|c| *c != Capability::HostCallSendKey)
            .collect();
        assert_eq!(
            missing_delivery_grants(&missing_one),
            vec![Capability::HostCallSendKey]
        );
        let observer_only = [
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderPanel,
        ];
        assert_eq!(
            missing_delivery_grants(&observer_only).len(),
            DELIVERY_GRANTS.len()
        );
    }

    #[test]
    fn undetected_launch_times_out_and_closes_the_session_without_touching_the_pane() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, task) = launch(adapter.registry(), AgentKind::Opencode);
        let bound_at = 1_000;
        assert!(adapter.process_one(&mut host, bound_at));
        let pane = bound_pane(&adapter, session);
        // A different foreground does not count as detection.
        adapter.foreground_changed(pane, Some("bash"), bound_at + 5_000);
        assert!(!adapter.housekeeping(bound_at + LAUNCH_DETECT_TIMEOUT_MS - 1));
        assert!(adapter.is_managed(pane));
        assert_eq!(
            task_status(adapter.registry(), task),
            TaskStatus::Dispatching
        );
        // At the deadline: terminal non-success, session closed, pane left
        // open and unmanaged.
        assert!(adapter.housekeeping(bound_at + LAUNCH_DETECT_TIMEOUT_MS));
        assert_eq!(task_status(adapter.registry(), task), TaskStatus::Unknown);
        assert!(!snapshot(adapter.registry(), session).open);
        assert!(!adapter.is_managed(pane));
        assert!(host.closes.is_empty(), "the pane is never auto-closed");
        // Housekeeping is idempotent afterwards.
        assert!(!adapter.housekeeping(bound_at + LAUNCH_DETECT_TIMEOUT_MS + 1));
    }

    #[test]
    fn detected_launch_never_times_out() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, task, pane) = {
            let (session, task) = launch(adapter.registry(), AgentKind::Claude);
            assert!(adapter.process_one(&mut host, 1_000));
            let pane = bound_pane(&adapter, session);
            adapter.foreground_changed(pane, Some("claude"), 2_000);
            (session, task, pane)
        };
        assert!(!adapter.housekeeping(1_000 + LAUNCH_DETECT_TIMEOUT_MS * 2));
        assert!(adapter.is_managed(pane));
        assert_eq!(task_status(adapter.registry(), task), TaskStatus::Settled);
        assert!(snapshot(adapter.registry(), session).open);
    }

    #[test]
    fn rate_limited_effect_stays_queued_and_retries_with_the_same_seq() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::rate_limited(2);
        let (session, _, pane) = {
            let mut ok = FakeHost::default();
            launched_and_detected_pane(&mut adapter, &mut ok)
        };
        assert!(matches!(
            call(adapter.registry(), Request::Focus { session }),
            Response::FocusAccepted { .. }
        ));
        let seq = adapter.registry().peek_effects()[0].seq;
        let mut now = 10_000;
        // First attempt: rate limited, effect NOT acked, nothing recorded.
        assert!(adapter.process_one(&mut host, now));
        assert_eq!(adapter.registry().peek_effects()[0].seq, seq);
        assert!(host.focuses.is_empty());
        // Backoff suppresses the immediate next tick.
        assert!(!adapter.process_one(&mut host, now + 100));
        // After the backoff, retry (second rate limit), then success with
        // the same seq.
        now += RATE_LIMIT_BACKOFF_MS;
        assert!(adapter.process_one(&mut host, now));
        assert_eq!(adapter.registry().peek_effects()[0].seq, seq);
        now += RATE_LIMIT_BACKOFF_MS;
        assert!(adapter.process_one(&mut host, now));
        assert_eq!(host.focuses, vec![pane]);
        assert!(
            adapter.registry().peek_effects().is_empty(),
            "acked with the same seq after retries"
        );
    }

    #[test]
    fn persistent_rate_limit_eventually_fails_delivery() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::rate_limited(MAX_RATE_LIMIT_RETRIES as usize + 5);
        let (session, _, _) = {
            let mut ok = FakeHost::default();
            launched_and_detected_pane(&mut adapter, &mut ok)
        };
        assert!(matches!(
            call(adapter.registry(), Request::Focus { session }),
            Response::FocusAccepted { .. }
        ));
        let mut now = 10_000;
        for attempt in 0..MAX_RATE_LIMIT_RETRIES {
            assert!(adapter.process_one(&mut host, now), "attempt {attempt}");
            now += RATE_LIMIT_BACKOFF_MS;
        }
        assert!(
            adapter.registry().peek_effects().is_empty(),
            "retry budget exhausted: effect failed out"
        );
    }

    #[test]
    fn prompt_delivers_text_with_enter_and_stays_running() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "implement the tests".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        assert!(adapter.process_one(&mut host, ms()));
        let expected = build_prompt_envelope(session, prompt, "implement the tests", None).unwrap();
        assert_eq!(host.texts, vec![(pane, expected)]);
        assert!(adapter.registry().peek_effects().is_empty());
        // Delivered, not done: Running until a native result/interrupt/close.
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
    }

    #[test]
    fn prompt_after_takeover_fails_delivery_without_touching_the_host() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        // The human takes over between acceptance and delivery.
        assert!(matches!(
            call(adapter.registry(), Request::HumanTakeover { session }),
            Response::TakenOver { .. }
        ));
        assert!(adapter.process_one(&mut host, ms()));
        assert!(host.texts.is_empty(), "never type into a human-owned pane");
        assert!(adapter.registry().peek_effects().is_empty());
        assert_eq!(
            task_status(adapter.registry(), prompt),
            TaskStatus::FailedDelivery
        );
    }

    #[test]
    fn prompt_is_rejected_at_request_time_when_human_owns() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        assert!(matches!(
            call(adapter.registry(), Request::HumanTakeover { session }),
            Response::TakenOver { .. }
        ));
        match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "yes".into(),
            },
        ) {
            Response::Error { message } => assert!(message.contains("human owns")),
            other => panic!("{other:?}"),
        }
        assert!(adapter.registry().peek_effects().is_empty());
    }

    #[test]
    fn interrupt_sends_ctrl_c_but_cannot_prove_worker_completion() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        adapter.process_one(&mut host, ms());
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
        assert!(matches!(
            call(adapter.registry(), Request::Interrupt { session }),
            Response::InterruptAccepted { .. }
        ));
        // The interrupt request alone does not settle; delivery does.
        assert_eq!(
            task_status(adapter.registry(), prompt),
            TaskStatus::Interrupting
        );
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.keys, vec![(pane, "ctrl-c".into())]);
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Unknown);
    }

    #[test]
    fn interrupt_host_error_marks_in_flight_unknown() {
        let mut adapter = Adapter::new(Registry::new());
        let (session, _, _) = {
            let mut host = FakeHost::default();
            launched_and_detected_pane(&mut adapter, &mut host)
        };
        let mut host = FakeHost::failing("vi mode");
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        // Deliver the prompt with a working host, then interrupt with a
        // failing one.
        let mut ok_host = FakeHost::default();
        assert!(adapter.process_one(&mut ok_host, ms()));
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
        call(adapter.registry(), Request::Interrupt { session });
        adapter.process_one(&mut host, ms());
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Unknown);
    }

    #[test]
    fn focus_delivers_and_close_waits_for_the_real_pane_closed() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        assert!(matches!(
            call(adapter.registry(), Request::Focus { session }),
            Response::FocusAccepted { .. }
        ));
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.focuses, vec![pane]);

        assert!(matches!(
            call(adapter.registry(), Request::Close { session }),
            Response::CloseAccepted { .. }
        ));
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.closes, vec![pane]);
        // CloseDelivered acked the request; the session is still open until
        // the real PaneClosed event arrives.
        assert!(snapshot(adapter.registry(), session).open);
        adapter.pane_closed(pane, ms());
        assert!(!snapshot(adapter.registry(), session).open);
        assert!(!adapter.is_managed(pane));
    }

    #[test]
    fn foreground_detection_settles_launch_once_with_a_neutral_result() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, task) = launch(adapter.registry(), AgentKind::Opencode);
        adapter.process_one(&mut host, ms());
        let pane = bound_pane(&adapter, session);
        // A transient child process first: not the agent, no settle.
        adapter.foreground_changed(pane, Some("bash"), ms());
        assert_eq!(
            task_status(adapter.registry(), task),
            TaskStatus::Dispatching
        );
        adapter.foreground_changed(pane, Some("opencode"), ms());
        let snap = snapshot(adapter.registry(), session);
        assert_eq!(snap.tasks[0].status, TaskStatus::Settled);
        assert!(
            snap.tasks[0].result.is_none(),
            "summaries omit result payloads"
        );
        match call(adapter.registry(), Request::Wait { task }) {
            Response::Wait { result, .. } => {
                assert_eq!(result.as_deref(), Some(LAUNCH_DETECTED_RESULT))
            }
            other => panic!("{other:?}"),
        }
        // A re-report must not duplicate.
        adapter.foreground_changed(pane, Some("opencode"), ms());
        let facts = adapter.registry().facts_since(0);
        assert!(
            !facts
                .facts
                .iter()
                .any(|f| matches!(f.event, agent_coordination::Event::DuplicateResult { .. })),
            "detection must not fire twice"
        );
    }

    #[test]
    fn containing_run_exit_closes_the_session_and_unknowns_the_prompt() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        adapter.process_one(&mut host, ms());
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
        // The agent process exited (pinned containing run finished): the
        // session closes; the prompt must NOT settle as if it completed.
        adapter.containing_run_exited(pane, ms());
        assert!(!snapshot(adapter.registry(), session).open);
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Unknown);
    }

    #[test]
    fn release_to_coordinator_is_host_side() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        call(adapter.registry(), Request::HumanTakeover { session });
        let rows = adapter.managed_rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[&pane].human_owned);
        adapter.release_to_coordinator(session, ms());
        let rows = adapter.managed_rows();
        assert!(!rows[&pane].human_owned);
    }

    #[cfg(unix)]
    #[test]
    fn server_on_a_temp_socket_serves_the_shared_registry() {
        use agent_coordination::{Server, call as client_call};
        use std::sync::atomic::{AtomicU64, Ordering};

        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "agents-adapter-test-{}-{}.sock",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let mut adapter = Adapter::new(Registry::new());
        let server = Server::bind(adapter.registry().clone(), &path).unwrap();
        let launched = client_call(
            &path,
            &WireRequest {
                id: 1,
                body: Request::Launch {
                    kind: AgentKind::Codex,
                    cwd: abs("/work"),
                    name: None,
                    args: vec![],
                },
            },
        )
        .unwrap();
        let session = match launched.body {
            Response::LaunchAccepted { session, .. } => session,
            other => panic!("{other:?}"),
        };
        let mut host = FakeHost::default();
        assert!(adapter.process_one(&mut host, ms()));
        let listed = client_call(
            &path,
            &WireRequest {
                id: 2,
                body: Request::List,
            },
        )
        .unwrap();
        match listed.body {
            Response::Agents { agents, .. } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].session, session);
                assert!(agents[0].pane.is_some(), "adapter bound the pane");
            }
            other => panic!("{other:?}"),
        }
        server.stop();
        assert!(!path.exists());
    }

    #[test]
    fn managed_session_rows_reflects_registry() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        let rows = adapter.managed_session_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session, session);
        assert_eq!(rows[0].kind, AgentKind::Codex);
        assert_eq!(rows[0].name.as_deref(), Some("w"));
        assert!(!rows[0].human_owned);
        assert!(rows[0].bound);
        assert_eq!(rows[0].latest_task, Some(TaskStatus::Settled));

        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        adapter.process_one(&mut host, ms());
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
        let rows = adapter.managed_session_rows();
        assert_eq!(rows[0].latest_task, Some(TaskStatus::Running));
    }

    #[test]
    fn take_over_flips_writer_and_release_returns() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        adapter.take_over(session, ms()).unwrap();
        assert_eq!(snapshot(adapter.registry(), session).writer, Writer::Human);
        assert!(adapter.managed_session_rows()[0].human_owned);
        adapter.release_to_coordinator(session, ms());
        assert_eq!(
            snapshot(adapter.registry(), session).writer,
            Writer::Coordinator
        );
        assert!(!adapter.managed_session_rows()[0].human_owned);
    }

    #[test]
    fn request_interrupt_and_close_submit_effects_not_host_calls() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { .. } => {}
            other => panic!("{other:?}"),
        }
        adapter.process_one(&mut host, ms());

        adapter.request_interrupt(session, ms()).unwrap();
        let pending = adapter.registry().peek_effects();
        assert_eq!(pending.len(), 1);
        assert!(matches!(
            pending[0].body,
            EffectBody::InterruptRequested { .. }
        ));
        assert!(host.keys.is_empty(), "queued, not yet delivered");
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.keys, vec![(pane, "ctrl-c".into())]);

        adapter.request_close(session, ms()).unwrap();
        let pending = adapter.registry().peek_effects();
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0].body, EffectBody::CloseRequested { .. }));
        assert!(host.closes.is_empty(), "queued, not yet delivered");
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.closes, vec![pane]);
    }

    #[test]
    fn request_interrupt_after_takeover_queues_nothing() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        adapter.take_over(session, ms()).unwrap();
        let queued = adapter.registry().peek_effects().len();
        assert!(adapter.request_interrupt(session, ms()).is_err());
        assert!(adapter.request_close(session, ms()).is_err());
        assert_eq!(
            adapter.registry().peek_effects().len(),
            queued,
            "the registry rejects writes on a human-owned session"
        );
        assert!(host.keys.is_empty() && host.closes.is_empty());
    }

    #[test]
    fn awaiting_human_count_tracks_latest_task() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        assert_eq!(adapter.awaiting_human_count(), 0);
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        assert_eq!(adapter.awaiting_human_count(), 0);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        adapter.process_one(&mut host, ms());
        adapter
            .registry()
            .apply(AdapterUpdate::TaskAwaitingHuman { task: prompt }, ms())
            .unwrap();
        assert_eq!(adapter.awaiting_human_count(), 1);
        adapter
            .registry()
            .apply(AdapterUpdate::TaskRunning { task: prompt }, ms())
            .unwrap();
        assert_eq!(adapter.awaiting_human_count(), 0);
        // A closed session does not count.
        adapter
            .registry()
            .apply(AdapterUpdate::TaskAwaitingHuman { task: prompt }, ms())
            .unwrap();
        assert_eq!(adapter.awaiting_human_count(), 1);
        adapter.pane_closed(pane, ms());
        assert_eq!(adapter.awaiting_human_count(), 0);
    }

    #[test]
    fn request_focus_queues_a_focus_effect_not_a_host_call() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        adapter.request_focus(session, ms()).unwrap();
        let pending = adapter.registry().peek_effects();
        assert_eq!(pending.len(), 1);
        assert!(matches!(pending[0].body, EffectBody::FocusRequested { .. }));
        assert!(host.focuses.is_empty(), "queued, not yet delivered");
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.focuses, vec![pane]);
    }

    #[test]
    fn request_focus_on_an_unbound_session_is_an_error() {
        let adapter = Adapter::new(Registry::new());
        // A session whose launch effect was never delivered has no bound
        // pane: the registry refuses focus.
        let (session, _) = launch(adapter.registry(), AgentKind::Claude);
        assert!(adapter.request_focus(session, ms()).is_err());
        assert!(
            !adapter
                .registry()
                .peek_effects()
                .iter()
                .any(|e| matches!(e.body, EffectBody::FocusRequested { .. })),
            "no focus effect queued (the launch effect remains, unrelated)"
        );
    }

    #[test]
    fn rows_carry_open_detail_and_result_presence_without_result_text() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        adapter.process_one(&mut host, ms());
        // A worker self-report is flattened and clipped; the panel never
        // renders the raw multi-line note.
        let raw_detail = format!("native\ntool dialog open — {}", "d".repeat(60));
        call(
            adapter.registry(),
            Request::ReportAwaitingHuman {
                task: prompt,
                detail: Some(raw_detail.clone()),
            },
        );
        let rows = adapter.managed_session_rows();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].open);
        let detail = rows[0].awaiting_detail.as_deref().expect("awaiting detail");
        assert!(!detail.contains('\n') && !detail.contains('\t'));
        assert!(
            detail.chars().count() <= AWAITING_DETAIL_CLIP_CHARS + 1,
            "clipped: {detail}"
        );
        assert!(detail.contains('…'), "long notes are clipped: {detail}");
        assert!(
            !detail.contains(&raw_detail),
            "raw multi-line detail is absent: {detail}"
        );
        assert!(rows[0].result_excerpt.is_none());

        // A worker result yields a flattened, clipped excerpt — never the
        // full payload, never a success claim.
        let payload = format!("line1\nline2\t{}", "w".repeat(80));
        adapter
            .registry()
            .apply(
                AdapterUpdate::TaskResult {
                    task: prompt,
                    text: payload.clone(),
                },
                ms(),
            )
            .unwrap();
        let rows = adapter.managed_session_rows();
        let excerpt = rows[0].result_excerpt.as_deref().expect("result excerpt");
        assert!(!excerpt.contains('\n') && !excerpt.contains('\t'));
        assert!(
            excerpt.chars().count() <= RESULT_EXCERPT_CLIP_CHARS + 1,
            "clipped: {excerpt}"
        );
        assert!(excerpt.contains('…'), "long results are clipped: {excerpt}");
        assert!(
            !excerpt.contains(&payload),
            "full payload is absent: {excerpt}"
        );
        assert!(
            !excerpt.to_lowercase().contains("success"),
            "never a success claim: {excerpt}"
        );
        assert!(rows[0].awaiting_detail.is_none(), "no longer awaiting");

        // Closed sessions stay listed, marked closed.
        adapter.pane_closed(pane, ms());
        let rows = adapter.managed_session_rows();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].open);
    }

    /// Envelope template size minus the user text, for a given socket option.
    /// UUID spellings are fixed-width, so any session/task pair has the same
    /// overhead.
    fn envelope_overhead(
        session: AgentSessionId,
        task: CoordinationTaskId,
        socket: Option<&str>,
    ) -> usize {
        let probe = "x";
        let env = build_prompt_envelope(session, task, probe, socket).unwrap();
        env.chars().count() - probe.chars().count()
    }

    #[test]
    fn prompt_envelope_carries_ids_disclaimer_and_verbatim_task() {
        let session = AgentSessionId::new();
        let task = CoordinationTaskId::new();
        let text = "do the thing\nwith a 'quote' and ----- task ----- marker";
        let envelope = build_prompt_envelope(session, task, text, None).unwrap();

        let sid = session.as_uuid().to_string();
        let tid = task.as_uuid().to_string();
        assert!(
            envelope.contains(&format!("worker session {sid}, task {tid}")),
            "exact ids: {envelope}"
        );

        assert!(
            envelope.contains("coordination metadata"),
            "disclaimer: {envelope}"
        );
        assert!(
            envelope.contains("NEVER use them to answer approval"),
            "approval disclaimer: {envelope}"
        );
        let lower = envelope.to_lowercase();
        for forbidden in ["answer the approval", "approve with", "type yes"] {
            assert!(
                !lower.contains(forbidden),
                "must not instruct using reports as approval answers ({forbidden}): {envelope}"
            );
        }

        assert!(
            envelope.contains(&format!("sleipnir-agentctl report-awaiting-human {tid}")),
            "{envelope}"
        );
        assert!(
            envelope.contains(&format!("sleipnir-agentctl report-result {tid} --stdin")),
            "{envelope}"
        );
        assert!(
            envelope.contains(&format!("sleipnir-agentctl report-session-closed {sid}")),
            "{envelope}"
        );
        assert!(
            !envelope.contains("--socket"),
            "default path uses the bare command: {envelope}"
        );

        let start_marker = "----- task -----\n";
        let end_marker = "\n----- end task -----";
        let start = envelope.find(start_marker).expect("start delimiter") + start_marker.len();
        let end = envelope.rfind(end_marker).expect("end delimiter");
        assert_eq!(&envelope[start..end], text);
    }

    #[test]
    fn builtin_prompt_uses_the_shipped_executable_without_path_dependencies() {
        let mut adapter = Adapter::new(Registry::new());
        adapter.set_builtin_executable(std::path::Path::new("/Applications/O'Reilly App/sleipnir"));
        let session = AgentSessionId::new();
        let task = CoordinationTaskId::new();
        let text = "implement the tests";
        let envelope = build_prompt_envelope_with_command(
            session,
            task,
            text,
            Some("/tmp/agent socket"),
            &adapter.control_command,
        )
        .unwrap();
        assert!(envelope.contains("'/Applications/O'\\''Reilly App/sleipnir' agentctl --socket '/tmp/agent socket' report-result"));
        assert!(!envelope.contains("sleipnir-agentctl"));
        assert!(envelope.contains(&format!("----- task -----\n{text}\n----- end task -----")));
    }

    #[test]
    fn prompt_envelope_socket_override_is_shell_quoted() {
        let session = AgentSessionId::new();
        let task = CoordinationTaskId::new();
        let envelope =
            build_prompt_envelope(session, task, "work", Some("/tmp/o'reilly/agent.sock")).unwrap();
        let quoted = "sleipnir-agentctl --socket '/tmp/o'\\''reilly/agent.sock'";
        assert!(envelope.contains(quoted), "{envelope}");
        assert!(
            envelope.contains(&format!("{quoted} report-awaiting-human")),
            "{envelope}"
        );
        assert!(
            envelope.contains(&format!("{quoted} report-result")),
            "{envelope}"
        );
        assert!(
            envelope.contains(&format!("{quoted} report-session-closed")),
            "{envelope}"
        );
    }

    #[test]
    fn prompt_envelope_rejects_oversize_without_truncating() {
        let session = AgentSessionId::new();
        let task = CoordinationTaskId::new();
        let overhead = envelope_overhead(session, task, None);
        let fits = "x".repeat(SEND_TEXT_CAP_CHARS - overhead);
        let ok = build_prompt_envelope(session, task, &fits, None).unwrap();
        assert_eq!(ok.chars().count(), SEND_TEXT_CAP_CHARS);
        assert!(ok.contains(&fits), "user text is never truncated");

        let over = "x".repeat(SEND_TEXT_CAP_CHARS - overhead + 1);
        let err = build_prompt_envelope(session, task, &over, None).unwrap_err();
        assert!(
            err.contains(&SEND_TEXT_CAP_CHARS.to_string()),
            "cap named in the error: {err}"
        );
    }

    #[test]
    fn display_clip_flattens_and_caps() {
        assert_eq!(display_clip("short", 40), "short");
        assert_eq!(display_clip("a\nb\tc", 40), "a b c");
        let long = "x".repeat(50);
        let clipped = display_clip(&long, 40);
        assert_eq!(clipped.chars().count(), 41);
        assert!(clipped.ends_with('…'));
        assert!(!clipped.contains(&long));
        assert_eq!(display_clip(&"y".repeat(40), 40).chars().count(), 40);
    }

    #[test]
    fn oversize_prompt_fails_delivery_without_a_host_call() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        let overhead = envelope_overhead(session, CoordinationTaskId::new(), None);
        let over = "y".repeat(SEND_TEXT_CAP_CHARS - overhead + 1);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: over,
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        assert!(adapter.process_one(&mut host, ms()));
        assert!(host.texts.is_empty(), "never send an oversize envelope");
        assert!(
            adapter.registry().peek_effects().is_empty(),
            "oversize is acked as DeliveryFailed"
        );
        assert_eq!(
            task_status(adapter.registry(), prompt),
            TaskStatus::FailedDelivery
        );
    }

    #[test]
    fn just_under_cap_prompt_is_delivered() {
        let mut adapter = Adapter::new(Registry::new());
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let overhead = envelope_overhead(session, CoordinationTaskId::new(), None);
        let fits = "z".repeat(SEND_TEXT_CAP_CHARS - overhead);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: fits.clone(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        assert!(adapter.process_one(&mut host, ms()));
        assert_eq!(host.texts.len(), 1);
        assert_eq!(host.texts[0].0, pane);
        assert_eq!(host.texts[0].1.chars().count(), SEND_TEXT_CAP_CHARS);
        assert!(host.texts[0].1.contains(&fits));
        assert_eq!(task_status(adapter.registry(), prompt), TaskStatus::Running);
    }

    #[test]
    fn prompt_delivery_uses_socket_override() {
        let mut adapter = Adapter::new(Registry::new());
        adapter.set_socket_override(Some("/tmp/o'reilly.sock".into()));
        let mut host = FakeHost::default();
        let (session, _, pane) = launched_and_detected_pane(&mut adapter, &mut host);
        let prompt = match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        assert!(adapter.process_one(&mut host, ms()));
        let expected =
            build_prompt_envelope(session, prompt, "work", Some("/tmp/o'reilly.sock")).unwrap();
        assert_eq!(host.texts, vec![(pane, expected)]);
    }

    #[test]
    fn empty_socket_override_uses_the_bare_command() {
        let mut adapter = Adapter::new(Registry::new());
        adapter.set_socket_override(Some(String::new()));
        let mut host = FakeHost::default();
        let (session, _, _) = launched_and_detected_pane(&mut adapter, &mut host);
        match call(
            adapter.registry(),
            Request::Prompt {
                session,
                text: "work".into(),
            },
        ) {
            Response::PromptAccepted { .. } => {}
            other => panic!("{other:?}"),
        }
        assert!(adapter.process_one(&mut host, ms()));
        assert!(
            !host.texts[0].1.contains("--socket"),
            "empty override is treated as unset: {}",
            host.texts[0].1
        );
    }
}
