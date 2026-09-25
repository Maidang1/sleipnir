//! Row ↔ pixel mapping for terminal display.
//!
//! Every screen coordinate in the host today is a linear function of one
//! uniform row height. Both directions must go through one mapping. That
//! mapping is this crate.
//!
//! **The grid stays a uniform character grid.** The audited `* line_height`
//! sites live in host code (`term_element.rs`, `mappings/mouse.rs`,
//! `terminal.rs`), not in the vendored parser. ADR-0007's freeze is therefore
//! safe: nothing here is a parser feature, and nothing here depends on gpui.
//! Pixels are a plain `f32`; the host converts `gpui::Pixels` at the boundary.
//!
//! Only [`ViewportPosition::sub`] is a fractional remainder, which is the
//! price of pixel scrolling: the grid keeps integer `display_offset`, the
//! host owns the leftover.

mod geometry;
mod viewport;

pub use geometry::RowGeometry;
pub use viewport::ViewportPosition;

/// Device pixels as a plain `f32`. Not `gpui::Pixels`.
///
/// Row positions are `cell_rows * line_height` (one multiply). Only
/// [`ViewportPosition::sub`] is a fractional remainder.
pub type Px = f32;

#[cfg(test)]
mod tests;
