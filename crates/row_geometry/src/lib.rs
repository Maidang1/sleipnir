//! Row ↔ pixel mapping for Block rendering (ADR-0018).
//!
//! Every screen coordinate in the host today is a linear function of one
//! uniform row height. Blocks make that height non-uniform, so both directions
//! must go through one mapping. That mapping is this crate.
//!
//! **The grid stays a uniform character grid.** The audited `* line_height`
//! sites live in host code (`term_element.rs`, `mappings/mouse.rs`,
//! `terminal.rs`), not in the vendored parser. ADR-0007's freeze is therefore
//! safe: nothing here is a parser feature, and nothing here depends on gpui.
//! Pixels are a plain `f32`; the host converts `gpui::Pixels` at the boundary.
//!
//! Block heights are integer cell rows because `sleipnir_widget::layout`
//! sizes widgets in cells (ADR-0017). `y_for` is then a step function of those
//! integers — one multiply, no float accumulation across a long scrollback.
//! Only [`ViewportPosition::sub`] is a fractional remainder, which is the
//! price of pixel scrolling (ADR-0018 decision 2): the grid keeps integer
//! `display_offset`, the host owns the leftover.
//!
//! Alt-screen hides Blocks (decision 4), so geometry collapses to today's
//! linear mapping. Frozen-during-drag pins heights (decision 3) so a divider
//! drag does not re-layout every visible tree each frame (a narrowing of
//! ADR-0003, not a reversal).

mod geometry;
mod viewport;

pub use geometry::{Anchor, Block, HitTarget, RowGeometry};
pub use plugin_protocol::v2::{BlockId, RunId};
pub use viewport::ViewportPosition;

/// Device pixels as a plain `f32`. Not `gpui::Pixels`.
///
/// Row positions are `cell_rows * line_height` (one multiply). Only
/// [`ViewportPosition::sub`] is a fractional remainder.
pub type Px = f32;

#[cfg(test)]
mod tests;
