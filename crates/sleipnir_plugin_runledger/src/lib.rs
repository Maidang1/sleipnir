//! Run Ledger — the "what ran here" product surface as a resident plugin.
//!
//! The core only emits facts (`RunStarted` / `RunFinished` / `PaneClosed` /
//! `PaneFocused`); this plugin owns everything user-facing that used to live
//! in `sleipnir_ui`: the ledger itself ([`run_ledger::Ledger`]), persistence
//! (`runs.json`, same file the core used to write), the status-strip summary,
//! the grouped panel, and the jump-back-to-output action
//! (`Context::scroll_to_run`). Split into testable parts:
//!
//! - [`rows`]  — grouping and row formatting ported from the deleted core
//!   panel (`run_ledger_panel.rs` at the removal commit). Pure.
//! - [`state`] — the ledger plus the local↔host run-id map and `runs.json`
//!   persistence. I/O is limited to [`run_ledger::store`].
//! - [`view`]  — state → widget trees for the Status strip and the Panel.
//!   Pure, so the node budget is unit testable.

pub mod rows;
pub mod state;
pub mod view;
