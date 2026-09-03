//! Official Rust SDK for Sleipnir plugins (ADR-0015, ADR-0016).
//!
//! A plugin author writes plain Rust: implement [`v2::Plugin`] and call
//! [`v2::run`] from `main`. The SDK owns the wire protocol — the handshake, the
//! read/decode/dispatch/encode loop over stdin/stdout, and serialization — so
//! the author never touches JSON or process plumbing.
//!
//! [`v2`] is the resident, event-driven, rendering surface (ADR-0016) and the
//! only supported dialect; the v1 request/response API was removed.
//!
//! ```no_run
//! use sleipnir_plugin::v2::{
//!     run, Plugin, Context, Manifest, Lifecycle, Capability, EventFilter,
//! };
//!
//! struct Echo;
//! impl Plugin for Echo {
//!     fn manifest(&self) -> Manifest {
//!         Manifest {
//!             id: "echo".into(),
//!             name: "Echo".into(),
//!             version: "0.1.0".into(),
//!             description: "Echo the selection".into(),
//!             lifecycle: Lifecycle::OnDemand,
//!             commands: vec![],
//!         }
//!     }
//!     fn requests(&self) -> Vec<Capability> {
//!         vec![]
//!     }
//!     fn event_filter(&self) -> EventFilter {
//!         EventFilter::default()
//!     }
//! }
//!
//! fn main() { run(Echo); }
//! ```

pub mod v2;
mod widgets;
