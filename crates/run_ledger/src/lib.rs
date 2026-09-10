//! Run Ledger: the app's record of "what ran here" (spec 2026-08-16).
//!
//! Pure data + state machine — no gpui, no terminal, no I/O beyond `store`.

pub mod ledger;
pub mod run;
pub mod store;

pub use ledger::{Badge, BadgeKind, Ledger, Retention};
// The implementation lives in `plugin_protocol::redact` (the wire-safety
// invariant's single home); re-exported here so the capture-time call sites
// and existing references keep working.
pub use plugin_protocol::redact_command;
pub use run::{Anchor, LaunchId, PaneKey, Run, RunEvent, RunId, RunState};
pub use store::{RUNS_VERSION, RunsFile, default_runs_path, load_runs, save_runs};
