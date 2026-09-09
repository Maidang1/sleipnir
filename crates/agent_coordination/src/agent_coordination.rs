//! Local JSON-lines coordination protocol, in-memory registry, and Unix
//! socket server.
//!
//! `sleipnir-agentctl` speaks this dialect to a local server. Coordinator
//! requests acknowledge **acceptance** only. Execution payloads live on a
//! durable adapter [`Effect`] log until an adapter (not this crate) acks
//! them. This crate does not spawn agents, drive a PTY, or consume effects.
//! Native agent approvals are never a coordinator operation. Task status
//! has no success/failure — `Settled` is not success; delivery failure is
//! `FailedDelivery`.

pub mod client;
pub mod line;
pub mod protocol;
pub mod registry;
pub mod server;

pub use client::{ClientError, call};
pub use line::MAX_LINE_BYTES;
pub use protocol::{
    AdapterUpdate, AgentKind, AgentSessionId, CoordinationTaskId, Effect, EffectBody, Event, Fact,
    FactBatch, PROTOCOL_VERSION, RegistryStats, Request, Response, SessionSnapshot, TaskSnapshot,
    TaskStatus, WireRequest, WireResponse, Writer, decode_request_line, decode_response_line,
    encode_effect_line, encode_event_line, encode_fact_line, encode_request_line,
    encode_response_line,
};
pub use registry::{Limits, Registry};
pub use server::{CLIENT_READ_TIMEOUT, MAX_CLIENTS, Server, ServerError, default_socket_path};
