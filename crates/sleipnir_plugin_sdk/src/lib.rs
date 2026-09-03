//! Official Rust SDK for Sleipnir plugins (ADR-0015, ADR-0016).
//!
//! A plugin author writes plain Rust: implement [`Plugin`] and call [`run`]
//! from `main`. The SDK owns the wire protocol — the handshake, the
//! read/decode/dispatch/encode loop over stdin/stdout, and serialization — so
//! the author never touches JSON or process plumbing.
//!
//! The protocol is the resident, event-driven, rendering dialect of ADR-0016
//! (the only one the host speaks). The whole surface is re-exported at the
//! crate root; the [`v2`] module path remains as the versioned namespace a
//! future dialect will sit next to.
//!
//! ```no_run
//! use sleipnir_plugin::{
//!     run, Capability, CommandSpec, Context, Invoke, Lifecycle, Manifest, Output, Plugin,
//! };
//!
//! struct Echo;
//!
//! impl Plugin for Echo {
//!     fn manifest(&self) -> Manifest {
//!         Manifest {
//!             id: "echo".into(),
//!             name: "Echo".into(),
//!             version: "0.1.0".into(),
//!             description: "Echo the selection back into the pane".into(),
//!             lifecycle: Lifecycle::OnDemand,
//!             commands: vec![CommandSpec {
//!                 id: "echo".into(),
//!                 title: "Echo Selection".into(),
//!                 description: String::new(),
//!                 keywords: vec![],
//!                 capabilities: vec![Capability::ReadSelection],
//!             }],
//!         }
//!     }
//!
//!     fn requests(&self) -> Vec<Capability> {
//!         vec![Capability::ReadSelection]
//!     }
//!
//!     fn invoke(&mut self, req: Invoke, _ctx: &mut Context<'_>) -> Result<Output, String> {
//!         Ok(Output::insert(req.context.selection.unwrap_or_default()))
//!     }
//! }
//!
//! fn main() { run(Echo); }
//! ```

pub mod v2;
mod widgets;

pub use v2::*;
