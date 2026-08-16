//! Run Ledger: the app's record of "what ran here" (spec 2026-08-16).
//!
//! Pure data + state machine — no gpui, no terminal, no I/O beyond `store`.

pub mod ledger;
pub mod redact;
pub mod run;
pub mod store;

pub use ledger::{Badge, BadgeKind, Ledger};
pub use redact::redact_command;
pub use run::{LaunchId, PaneKey, Run, RunEvent, RunId, RunState};
