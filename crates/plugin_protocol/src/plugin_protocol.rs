//! Wire protocol for out-of-process plugins (ADR-0015, ADR-0016).
//!
//! Pure types. No I/O, no process handling, no host or SDK logic. Both the host
//! (`plugin_host`) and the author-facing SDK (`sleipnir-plugin`) depend on this
//! crate so the contract is defined in exactly one place.
//!
//! Transport is line-delimited JSON, one object per line, matching the house
//! IPC style of the control surface (ADR-0011, `sleipnir_ctl`): tagged unions,
//! `snake_case`, forward-compatible with `#[serde(default)]` on additions.
//!
//! The only supported dialect is [`v2`] (ADR-0016): bidirectional, multiplexed
//! messaging with correlation ids, event push, widget rendering and
//! plugin-initiated host calls. The v1 request/response dialect was removed;
//! manifests declaring `api_version: 1` are rejected at load time.

/// Protocol v2 (ADR-0016): bidirectional, multiplexed messaging with
/// correlation ids, event push, widget rendering and plugin-initiated host
/// calls. The resident supervisor and the SDK's `v2` module speak this
/// dialect.
pub mod v2;
