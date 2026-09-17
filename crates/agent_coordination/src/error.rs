//! Typed registry errors. Wire responses still carry `Error { message }`.

use crate::protocol::TaskStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordError {
    UnknownSession,
    UnknownTask,
    UnknownEffect,
    /// Delivery ack without a live [`crate::ClaimedEffect`] for that seq.
    EffectNotClaimed,
    /// [`crate::Registry::try_claim`] for a seq this session already holds.
    EffectAlreadyClaimed,
    SessionClosed,
    HumanOwnsSession,
    HumanAlreadyOwns,
    CoordinatorAlreadyOwns,
    PaneNotBound,
    PaneAlreadyBound,
    SessionLimit {
        max: usize,
    },
    TaskLimit {
        max: usize,
    },
    EffectQueueFull,
    NothingInFlight,
    TaskNotInFlight,
    PromptNotDispatchable,
    NativeApproval,
    AlreadyInFlight,
    EffectKindMismatch {
        seq: u64,
        actual: &'static str,
        expected: &'static str,
    },
    EffectWrongSession,
    CwdEmpty,
    CwdNul,
    CwdTooLong,
    CwdRelative,
    NameEmpty,
    NameNul,
    NameTooLong,
    NameInvalid,
    TooManyArgs {
        max: usize,
    },
    ArgTooLong,
    ArgNul,
    PayloadEmpty {
        label: &'static str,
    },
    PayloadTooLong {
        label: &'static str,
        max: usize,
    },
    PayloadNul {
        label: &'static str,
    },
    PayloadControl {
        label: &'static str,
    },
    TaskNotRunnable {
        status: TaskStatus,
    },
}

impl std::fmt::Display for CoordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSession => write!(f, "unknown session"),
            Self::UnknownTask => write!(f, "unknown task"),
            Self::UnknownEffect => write!(f, "unknown effect"),
            Self::EffectNotClaimed => write!(f, "effect is not claimed"),
            Self::EffectAlreadyClaimed => write!(f, "effect is already claimed"),
            Self::SessionClosed => write!(f, "session is closed"),
            Self::HumanOwnsSession => write!(f, "human owns this session"),
            Self::HumanAlreadyOwns => write!(f, "human already owns this session"),
            Self::CoordinatorAlreadyOwns => write!(f, "coordinator already owns this session"),
            Self::PaneNotBound => write!(f, "pane is not bound"),
            Self::PaneAlreadyBound => write!(f, "pane is already bound"),
            Self::SessionLimit { max } => write!(f, "session limit {max} reached"),
            Self::TaskLimit { max } => write!(f, "task limit {max} reached"),
            Self::EffectQueueFull => write!(f, "effect queue full"),
            Self::NothingInFlight => write!(f, "nothing in flight"),
            Self::TaskNotInFlight => write!(f, "task is not in flight"),
            Self::PromptNotDispatchable => write!(f, "prompt is no longer dispatchable"),
            Self::NativeApproval => write!(f, "coordinator cannot answer native approvals"),
            Self::AlreadyInFlight => write!(f, "session already has an in-flight task"),
            Self::EffectKindMismatch {
                seq,
                actual,
                expected,
            } => write!(f, "effect {seq} is {actual}, not {expected}"),
            Self::EffectWrongSession => write!(f, "effect belongs to a different session"),
            Self::CwdEmpty => write!(f, "cwd is empty"),
            Self::CwdNul => write!(f, "cwd contains NUL"),
            Self::CwdTooLong => write!(f, "cwd exceeds length cap"),
            Self::CwdRelative => write!(f, "cwd must be absolute"),
            Self::NameEmpty => write!(f, "name is empty"),
            Self::NameNul => write!(f, "name contains NUL"),
            Self::NameTooLong => write!(f, "name exceeds length cap"),
            Self::NameInvalid => {
                write!(
                    f,
                    "name must start with a letter, then letters, digits, '_' or '-'"
                )
            }
            Self::TooManyArgs { max } => write!(f, "too many args (max {max})"),
            Self::ArgTooLong => write!(f, "arg exceeds length cap"),
            Self::ArgNul => write!(f, "arg contains NUL"),
            Self::PayloadEmpty { label } => write!(f, "{label} is empty"),
            Self::PayloadTooLong { label, max } => {
                write!(f, "{label} exceeds length cap of {max} characters")
            }
            Self::PayloadNul { label } => write!(f, "{label} contains NUL"),
            Self::PayloadControl { label } => write!(f, "{label} contains control characters"),
            Self::TaskNotRunnable { status } => write!(f, "task is {status:?}, not runnable"),
        }
    }
}

impl std::error::Error for CoordError {}
