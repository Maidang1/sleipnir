//! Unified macOS chrome: geometry, palette-derived tokens, pure helpers.

pub(crate) mod agent;
mod chrome_tokens;
pub(crate) mod close_copy;
mod close_index;
mod geometry;
pub(crate) mod git_status;
pub(crate) mod history_search;
pub(crate) mod pane_facts;
pub(crate) mod send_context;
pub(crate) mod tombstone;
mod tab_strip;
pub(crate) mod window_controls;
pub(crate) mod workspace;

pub use chrome_tokens::{ChromeTokens, contrast_ratio};
pub use close_index::active_after_close;
pub use geometry::ChromeGeometry;
