//! Agents — a resident plugin for agent collaboration: observer display plus
//! a generic coordination adapter.
//!
//! Watches the six facts the host already computes (`ForegroundChanged`,
//! `RunStarted`, `RunFinished`, `PaneFocused`, `PaneClosed`, `CwdChanged`) and
//! answers one question conservatively: which panes run a known coding agent,
//! and what is the strongest claim the observed events support about each?
//!
//! The coordination side serves the `agent_coordination` protocol on a local
//! Unix socket (started only when the full delivery grant set is present) and
//! delivers its effect log through the host's granted calls: launching worker
//! panes, typing coordinator prompts as a task-context envelope, interrupting,
//! focusing, and requesting pane close — all gated on coordinator ownership
//! at request *and* delivery time. Still out of scope: answering native
//! approvals (never), model or provider code, network, persistence. Status is
//! bounded to
//! process/session facts — `running` (the pinned containing shell Run is
//! open), `exited-unseen` / `exited-seen` (the latest agent session ended),
//! `unknown` — and never claims task percentage, turn progress, agent
//! readiness, or an approval-blocked state, because the current protocol
//! proves none of those.
//!
//! Beyond rendering and coordination delivery, the plugin posts a single OS
//! notification (`host_call_notify`): exactly once when a tracked session's
//! pinned containing run finishes while its pane is unfocused. The wording
//! says only that the session exited and output is ready to review; a denied
//! or failed notify changes nothing and is not retried.
//!
//! - [`state`] — the per-pane observer state machine. Pure.
//! - [`view`]  — state → Status strip / Panel widget trees. Pure, so the
//!   node budget is unit testable.
//! - [`adapter`] — the generic coordination adapter: consumes the
//!   `agent_coordination` effect log through a [`adapter::HostCalls`]
//!   abstraction, so delivery logic is unit testable without a live host.

pub mod adapter;
mod session;
pub mod state;
pub mod view;

pub use session::{run_builtin, run_plugin};
