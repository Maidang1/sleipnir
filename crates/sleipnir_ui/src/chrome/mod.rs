//! Unified macOS chrome: geometry, palette-derived tokens, pure helpers.

mod chrome_tokens;
mod close_index;
mod geometry;
mod tab_badge;
mod tab_strip;

pub use chrome_tokens::{ChromeTokens, contrast_ratio};
pub use close_index::active_after_close;
pub use geometry::ChromeGeometry;
