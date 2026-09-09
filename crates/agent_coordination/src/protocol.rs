//! JSON-lines wire types for `sleipnir-agentctl`.
//!
//! One object per line, tagged unions, `snake_case`, `#[serde(default)]` on
//! additions — the same house IPC style as `sleipnir_ctl` (ADR-0011).
//! There is no `approve` / `deny` operation: native agent approvals stay a
//! human action in the visible pane.
//!
//! Coordinator requests acknowledge **acceptance**, never execution.
//! Adapter work is dispatched on a separate durable [`Effect`] log.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Protocol version this crate speaks. The future socket server will
/// advertise this on connect; a mismatch is a refuse, not a silent coerce.
pub const PROTOCOL_VERSION: u32 = 1;

/// Opaque id of one visible agent session (one pane's integrated agent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentSessionId(Uuid);

impl AgentSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// Opaque id of one coordination assignment (launch or prompt).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoordinationTaskId(Uuid);

impl CoordinationTaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

/// First-wave integrated agent kinds. Unknown names fail to deserialize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    Claude,
    Gemini,
    Opencode,
}

/// Who may write into the session's visible pane. Exactly one writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Writer {
    Coordinator,
    Human,
}

/// Assignment status. Never task success or approval state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Registry accepted the request; the adapter effect is not yet queued
    /// (unused at rest: handle() moves to [`Dispatching`] in the same step).
    Accepted,
    /// An adapter effect is pending or in delivery.
    Dispatching,
    /// Work is in flight in the visible pane.
    Running,
    /// The native agent is waiting on a human. The coordinator must not
    /// answer that prompt.
    AwaitingHuman,
    /// An interrupt effect is pending or unconfirmed. Still in flight.
    /// [`AdapterUpdate::InterruptDelivered`] settles these; the interrupt
    /// *request* does not.
    Interrupting,
    /// The assignment is no longer in flight. This is not success.
    Settled,
    /// The registry cannot prove more (pane/session closed mid-flight).
    Unknown,
    /// The adapter reported it could not deliver the effect. Not success.
    FailedDelivery,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Unknown | Self::FailedDelivery)
    }

    pub fn is_in_flight(self) -> bool {
        matches!(
            self,
            Self::Accepted
                | Self::Dispatching
                | Self::Running
                | Self::AwaitingHuman
                | Self::Interrupting
        )
    }
}

/// Coordinator → registry. Flattened onto [`WireRequest`] as `op`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    List,
    Launch {
        kind: AgentKind,
        cwd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Argv for the agent process, never a shell line.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    Prompt {
        session: AgentSessionId,
        text: String,
    },
    /// Immediate status snapshot. The client owns any real waiting.
    Wait {
        task: CoordinationTaskId,
    },
    Interrupt {
        session: AgentSessionId,
    },
    Focus {
        session: AgentSessionId,
    },
    Inspect {
        session: AgentSessionId,
    },
    HumanTakeover {
        session: AgentSessionId,
    },
    Close {
        session: AgentSessionId,
    },
    /// Worker/adapter self-report: task is in flight in the pane.
    /// Same-user socket clients can spoof this; it is not an approval answer.
    ReportRunning {
        task: CoordinationTaskId,
    },
    /// Worker/adapter self-report: native agent is waiting on a human.
    /// Coordinator prompts stay refused while this status holds.
    ReportAwaitingHuman {
        task: CoordinationTaskId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// Worker/adapter self-report: assignment is no longer in flight.
    /// `text` is the result payload, not a success flag.
    ReportResult {
        task: CoordinationTaskId,
        text: String,
    },
    /// Worker/adapter self-report: the session's pane/process is gone.
    ReportSessionClosed {
        session: AgentSessionId,
    },
    /// Diagnostic: peek the adapter effect log without consuming it.
    Effects,
    /// Diagnostic: coordinator fact ring after `cursor` (0 = start).
    Facts {
        #[serde(default)]
        cursor: u64,
    },
}

/// One JSON-lines request. `id` correlates the matching [`WireResponse`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRequest {
    pub id: u64,
    #[serde(flatten)]
    pub body: Request,
}

/// Registry → coordinator. Flattened onto [`WireResponse`] as `op`.
///
/// `*_accepted` names mean the registry accepted and queued work, not that
/// a pane was spawned or a key was delivered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    Error {
        message: String,
    },
    Agents {
        agents: Vec<SessionSnapshot>,
    },
    LaunchAccepted {
        session: AgentSessionId,
        task: CoordinationTaskId,
    },
    PromptAccepted {
        task: CoordinationTaskId,
    },
    Wait {
        task: CoordinationTaskId,
        status: TaskStatus,
        /// True when `status` is terminal. The client should stop polling.
        terminal: bool,
        /// Last worker-reported result. Absence is not failure.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        /// Last `report_awaiting_human` note, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    ReportedRunning {
        task: CoordinationTaskId,
    },
    ReportedAwaitingHuman {
        task: CoordinationTaskId,
    },
    ReportedResult {
        task: CoordinationTaskId,
    },
    ReportedSessionClosed {
        session: AgentSessionId,
    },
    InterruptAccepted {
        session: AgentSessionId,
    },
    FocusAccepted {
        session: AgentSessionId,
    },
    Inspect {
        session: SessionSnapshot,
    },
    TakenOver {
        session: AgentSessionId,
    },
    CloseAccepted {
        session: AgentSessionId,
    },
    Effects {
        effects: Vec<Effect>,
    },
    Facts {
        facts: Vec<Fact>,
        next_cursor: u64,
        missed: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireResponse {
    pub id: u64,
    #[serde(flatten)]
    pub body: Response,
}

/// Coordinator-facing facts. Independent of the adapter [`Effect`] log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    SessionOpened {
        session: AgentSessionId,
    },
    TaskAccepted {
        session: AgentSessionId,
        task: CoordinationTaskId,
    },
    TaskDispatching {
        task: CoordinationTaskId,
    },
    TaskRunning {
        task: CoordinationTaskId,
    },
    TaskAwaitingHuman {
        task: CoordinationTaskId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    TaskInterrupting {
        task: CoordinationTaskId,
    },
    TaskSettled {
        task: CoordinationTaskId,
    },
    TaskUnknown {
        task: CoordinationTaskId,
    },
    TaskFailedDelivery {
        task: CoordinationTaskId,
    },
    SessionClosed {
        session: AgentSessionId,
    },
    OwnershipChanged {
        session: AgentSessionId,
        writer: Writer,
    },
    LateResult {
        task: CoordinationTaskId,
    },
    DuplicateResult {
        task: CoordinationTaskId,
    },
}

/// One fact on the subscriber stream, with a monotonic sequence id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
}

/// Adapter-facing intent. Payloads live here until the adapter acks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub seq: u64,
    #[serde(flatten)]
    pub body: EffectBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum EffectBody {
    LaunchRequested {
        session: AgentSessionId,
        task: CoordinationTaskId,
        kind: AgentKind,
        cwd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    PromptRequested {
        session: AgentSessionId,
        task: CoordinationTaskId,
        text: String,
    },
    InterruptRequested {
        session: AgentSessionId,
    },
    FocusRequested {
        session: AgentSessionId,
    },
    CloseRequested {
        session: AgentSessionId,
    },
}

impl EffectBody {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::LaunchRequested { .. } => "launch_requested",
            Self::PromptRequested { .. } => "prompt_requested",
            Self::InterruptRequested { .. } => "interrupt_requested",
            Self::FocusRequested { .. } => "focus_requested",
            Self::CloseRequested { .. } => "close_requested",
        }
    }

    pub fn session(&self) -> AgentSessionId {
        match self {
            Self::LaunchRequested { session, .. }
            | Self::PromptRequested { session, .. }
            | Self::InterruptRequested { session }
            | Self::FocusRequested { session }
            | Self::CloseRequested { session } => *session,
        }
    }
}

/// Adapter/host → registry. Not a coordinator wire op.
///
/// Delivery acknowledgements name the **exact effect `seq`**. A later queued
/// prompt/focus/interrupt cannot be acked by a stale delivery for an earlier
/// request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterUpdate {
    /// Bind the session to a host pane and ack `LaunchRequested` at `seq`.
    /// Same pane is idempotent; a different pane is an error.
    BindPane {
        seq: u64,
        session: AgentSessionId,
        pane: Uuid,
    },
    PromptDelivered {
        seq: u64,
    },
    /// Ack `InterruptRequested` at `seq` and settle in-flight tasks on that
    /// session. Settled means no longer in flight, not success.
    InterruptDelivered {
        seq: u64,
    },
    FocusDelivered {
        seq: u64,
    },
    CloseDelivered {
        seq: u64,
    },
    /// Adapter could not carry out the effect. Related in-flight work
    /// becomes [`TaskStatus::FailedDelivery`] or [`TaskStatus::Unknown`].
    DeliveryFailed {
        seq: u64,
    },
    TaskRunning {
        task: CoordinationTaskId,
    },
    TaskAwaitingHuman {
        task: CoordinationTaskId,
    },
    TaskResult {
        task: CoordinationTaskId,
        text: String,
    },
    SessionClosed {
        session: AgentSessionId,
    },
    /// Host/human originated. Not a coordinator request.
    ReleaseToCoordinator {
        session: AgentSessionId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session: AgentSessionId,
    pub kind: AgentKind,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub writer: Writer,
    pub open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TaskSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task: CoordinationTaskId,
    pub session: AgentSessionId,
    pub status: TaskStatus,
    pub accepted_at_ms: u64,
    /// Present only when a worker/adapter recorded a result. Absence is not failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Last `report_awaiting_human` note. Not an approval prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Independent read of the fact stream from `cursor` (last seen seq, 0 = start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactBatch {
    pub facts: Vec<Fact>,
    /// Pass this back as the next `cursor`.
    pub next_cursor: u64,
    /// Sequence numbers skipped because the ring overflowed since `cursor`.
    pub missed: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistryStats {
    pub facts_dropped: u64,
    pub effects_rejected: u64,
    /// Effects discarded because their session closed (not a queue-full reject).
    pub effects_drained: u64,
    pub pruned_sessions: u64,
    pub pruned_tasks: u64,
}

pub fn decode_request_line(line: &str) -> Result<WireRequest, String> {
    serde_json::from_str(line.trim()).map_err(|err| err.to_string())
}

pub fn encode_request_line(req: &WireRequest) -> Result<String, String> {
    serde_json::to_string(req).map_err(|err| err.to_string())
}

pub fn encode_response_line(resp: &WireResponse) -> Result<String, String> {
    serde_json::to_string(resp).map_err(|err| err.to_string())
}

pub fn decode_response_line(line: &str) -> Result<WireResponse, String> {
    serde_json::from_str(line.trim()).map_err(|err| err.to_string())
}

pub fn encode_event_line(event: &Event) -> Result<String, String> {
    serde_json::to_string(event).map_err(|err| err.to_string())
}

pub fn encode_effect_line(effect: &Effect) -> Result<String, String> {
    serde_json::to_string(effect).map_err(|err| err.to_string())
}

pub fn encode_fact_line(fact: &Fact) -> Result<String, String> {
    serde_json::to_string(fact).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u128) -> AgentSessionId {
        AgentSessionId::from_uuid(Uuid::from_u128(n))
    }

    fn tid(n: u128) -> CoordinationTaskId {
        CoordinationTaskId::from_uuid(Uuid::from_u128(n))
    }

    #[test]
    fn each_request_round_trips() {
        let session = sid(1);
        let task = tid(2);
        let cases = [
            Request::List,
            Request::Launch {
                kind: AgentKind::Codex,
                cwd: "/work".into(),
                name: Some("w1".into()),
                args: vec!["--foo".into()],
            },
            Request::Prompt {
                session,
                text: "do the thing".into(),
            },
            Request::Wait { task },
            Request::Interrupt { session },
            Request::Focus { session },
            Request::Inspect { session },
            Request::HumanTakeover { session },
            Request::Close { session },
            Request::ReportRunning { task },
            Request::ReportAwaitingHuman {
                task,
                detail: Some("native dialog".into()),
            },
            Request::ReportResult {
                task,
                text: "diff applied".into(),
            },
            Request::ReportSessionClosed { session },
            Request::Effects,
            Request::Facts { cursor: 3 },
        ];
        for body in cases {
            let req = WireRequest { id: 7, body };
            let line = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<WireRequest>(&line).unwrap(), req);
            assert!(line.contains(r#""id":7"#));
            assert!(line.contains(r#""op":"#));
        }
    }

    #[test]
    fn release_to_coordinator_is_not_a_coordinator_op() {
        let raw = format!(
            r#"{{"id":1,"op":"release_to_coordinator","session":"{}"}}"#,
            Uuid::nil()
        );
        assert!(serde_json::from_str::<WireRequest>(&raw).is_err());
    }

    #[test]
    fn launch_omits_empty_optional_fields() {
        let req = WireRequest {
            id: 1,
            body: Request::Launch {
                kind: AgentKind::Claude,
                cwd: "/tmp".into(),
                name: None,
                args: vec![],
            },
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(!line.contains("name"));
        assert!(!line.contains("args"));
        assert_eq!(serde_json::from_str::<WireRequest>(&line).unwrap(), req);
    }

    #[test]
    fn unknown_agent_kind_fails_to_decode() {
        let raw = r#"{"id":1,"op":"launch","kind":"cursor","cwd":"/tmp"}"#;
        assert!(serde_json::from_str::<WireRequest>(raw).is_err());
    }

    #[test]
    fn approve_is_not_a_protocol_operation() {
        for op in ["approve", "deny", "answer_approval"] {
            let raw = format!(r#"{{"id":1,"op":"{op}","session":"{}"}}"#, Uuid::nil());
            assert!(
                serde_json::from_str::<WireRequest>(&raw).is_err(),
                "{op} must not decode"
            );
        }
    }

    #[test]
    fn agent_kind_wire_names() {
        for (kind, name) in [
            (AgentKind::Codex, "codex"),
            (AgentKind::Claude, "claude"),
            (AgentKind::Gemini, "gemini"),
            (AgentKind::Opencode, "opencode"),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn task_status_has_no_success_variant() {
        let line = serde_json::to_string(&TaskStatus::Settled).unwrap();
        assert_eq!(line, "\"settled\"");
        for forbidden in ["success", "succeeded", "failed", "complete"] {
            assert!(!line.contains(forbidden));
        }
        assert!(TaskStatus::Settled.is_terminal());
        assert!(TaskStatus::FailedDelivery.is_terminal());
        assert!(!TaskStatus::Settled.is_in_flight());
        assert!(TaskStatus::Dispatching.is_in_flight());
        assert!(TaskStatus::Interrupting.is_in_flight());
        assert!(TaskStatus::AwaitingHuman.is_in_flight());
    }

    #[test]
    fn snapshot_omits_success_fields() {
        let snap = TaskSnapshot {
            task: tid(1),
            session: sid(1),
            status: TaskStatus::Settled,
            accepted_at_ms: 10,
            result: Some("output".into()),
            detail: None,
        };
        let line = serde_json::to_string(&snap).unwrap();
        assert!(line.contains("\"result\":\"output\""));
        assert!(!line.contains("success"));
        assert!(!line.contains("exit_code"));
        assert!(!line.contains("detail"));
    }

    #[test]
    fn worker_report_ops_round_trip_and_are_not_approvals() {
        let task = tid(2);
        let session = sid(1);
        for body in [
            Request::ReportRunning { task },
            Request::ReportAwaitingHuman { task, detail: None },
            Request::ReportResult {
                task,
                text: String::new(),
            },
            Request::ReportSessionClosed { session },
        ] {
            let req = WireRequest { id: 3, body };
            let line = serde_json::to_string(&req).unwrap();
            assert_eq!(serde_json::from_str::<WireRequest>(&line).unwrap(), req);
            assert!(!line.contains("approv"));
            assert!(!line.contains("success"));
        }
        let wait = WireResponse {
            id: 4,
            body: Response::Wait {
                task,
                status: TaskStatus::Settled,
                terminal: true,
                result: Some("files written".into()),
                detail: None,
            },
        };
        let line = encode_response_line(&wait).unwrap();
        assert!(line.contains("files written"));
        assert!(!line.contains("success"));
        assert_eq!(serde_json::from_str::<WireResponse>(&line).unwrap(), wait);
    }

    #[test]
    fn responses_name_acceptance_not_execution() {
        let session = sid(3);
        let task = tid(4);
        for (body, tag) in [
            (
                Response::LaunchAccepted { session, task },
                "launch_accepted",
            ),
            (Response::PromptAccepted { task }, "prompt_accepted"),
            (
                Response::InterruptAccepted { session },
                "interrupt_accepted",
            ),
            (Response::FocusAccepted { session }, "focus_accepted"),
            (Response::CloseAccepted { session }, "close_accepted"),
        ] {
            let line = serde_json::to_string(&WireResponse { id: 1, body }).unwrap();
            assert!(line.contains(&format!(r#""op":"{tag}""#)), "{line}");
        }
    }

    #[test]
    fn effect_payloads_round_trip() {
        let effect = Effect {
            seq: 9,
            body: EffectBody::PromptRequested {
                session: sid(1),
                task: tid(2),
                text: "implement the tests".into(),
            },
        };
        let line = encode_effect_line(&effect).unwrap();
        assert!(line.contains(r#""effect":"prompt_requested""#));
        assert!(line.contains("implement the tests"));
        assert_eq!(serde_json::from_str::<Effect>(&line).unwrap(), effect);

        let launch = Effect {
            seq: 1,
            body: EffectBody::LaunchRequested {
                session: sid(1),
                task: tid(2),
                kind: AgentKind::Codex,
                cwd: "/work".into(),
                name: Some("w".into()),
                args: vec!["--foo".into()],
            },
        };
        let line = encode_effect_line(&launch).unwrap();
        assert!(line.contains(r#""effect":"launch_requested""#));
        assert!(line.contains("--foo"));
        assert_eq!(serde_json::from_str::<Effect>(&line).unwrap(), launch);
    }

    #[test]
    fn response_and_event_round_trips() {
        let session = sid(3);
        let task = tid(4);
        let inspect = WireResponse {
            id: 2,
            body: Response::Inspect {
                session: SessionSnapshot {
                    session,
                    kind: AgentKind::Gemini,
                    cwd: "/repo".into(),
                    name: None,
                    writer: Writer::Human,
                    open: true,
                    pane: Some(Uuid::from_u128(9)),
                    tasks: vec![],
                },
            },
        };
        let line = encode_response_line(&inspect).unwrap();
        assert_eq!(
            serde_json::from_str::<WireResponse>(&line).unwrap(),
            inspect
        );

        let event = Event::LateResult { task };
        let eline = encode_event_line(&event).unwrap();
        assert!(eline.contains(r#""event":"late_result""#));
        assert_eq!(serde_json::from_str::<Event>(&eline).unwrap(), event);

        let fact = Fact { seq: 3, event };
        let fline = encode_fact_line(&fact).unwrap();
        assert!(fline.contains(r#""seq":3"#));
        assert_eq!(serde_json::from_str::<Fact>(&fline).unwrap(), fact);
    }

    #[test]
    fn decode_request_line_rejects_garbage() {
        assert!(decode_request_line("not json").is_err());
        let ok = decode_request_line(r#"{"id":1,"op":"list"}"#).unwrap();
        assert_eq!(ok.id, 1);
        assert_eq!(ok.body, Request::List);
        let encoded = encode_request_line(&ok).unwrap();
        assert_eq!(decode_request_line(&encoded).unwrap(), ok);
    }

    #[test]
    fn ids_are_opaque_uuids_on_the_wire() {
        let id = sid(0xabc);
        let line = serde_json::to_string(&id).unwrap();
        assert!(line.starts_with('"'));
        assert_eq!(serde_json::from_str::<AgentSessionId>(&line).unwrap(), id);
        assert_ne!(AgentSessionId::new(), AgentSessionId::new());
        assert_ne!(CoordinationTaskId::new(), CoordinationTaskId::new());
    }
}
