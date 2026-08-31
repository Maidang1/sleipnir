//! Host-side sub-row scroll offset (ADR-0018 decision 2).
//!
//! `display_offset` is owned by the grid and is an integer row count.
//! Pixel-accurate scrolling over variable-height Blocks cannot be expressed
//! that way, so the host keeps a remainder `sub` that is never sent to the
//! grid. Wheel deltas accumulate here; whole rows spill as a line delta the
//! way they do today, but the leftover is retained instead of discarded
//! (`terminal.rs` currently takes `scroll_px` modulo the viewport height).
//!
//! `row` is the absolute line at the viewport origin, not the grid's
//! `display_offset`. Each spilled whole row is an absolute-line delta; the
//! host converts it at the `Scroll::Delta` boundary.

use crate::geometry::{RowGeometry, i32_from_usize, usize_from_i32};
use crate::{HitTarget, Px};

/// Integer row plus a pixel remainder within that row's height.
///
/// `sub` is always in `[0, height_of(row))` after [`Self::apply_pixel_delta`].
/// When the current row is a tall Block, that interval is the Block's full
/// pixel height, which is what makes scrolling over it pixel-accurate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportPosition {
    /// Absolute line at the viewport origin. This is not a `display_offset`;
    /// the two coordinate systems point in opposite directions.
    pub row: usize,
    /// Host-side remainder. Never sent to the grid.
    pub sub: Px,
}

impl ViewportPosition {
    pub fn new(row: usize) -> Self {
        Self { row, sub: 0.0 }
    }

    /// Land `abs_line` flush at the viewport origin. A Block anchored there
    /// sits against the edge rather than clipped (ADR-0018 decision 2).
    pub fn scroll_to_anchor(abs_line: i32) -> Self {
        Self {
            row: usize_from_i32(abs_line.max(0)),
            sub: 0.0,
        }
    }

    /// In-place form of [`Self::scroll_to_anchor`].
    pub fn jump_to_anchor(&mut self, abs_line: i32) {
        *self = Self::scroll_to_anchor(abs_line);
    }

    /// Accumulate a pixel wheel delta. Whole rows spill as a signed line
    /// count; `sub` is left in `[0, height_of(row))`.
    ///
    /// Positive `delta_px` moves forward through the document (later lines
    /// toward the origin). Negative deltas walk backward. Deltas larger than
    /// several rows, including tall Blocks, are handled in one mapping step
    /// rather than a per-row loop, so an absurd delta cannot spin.
    pub fn apply_pixel_delta(&mut self, delta_px: Px, geom: &RowGeometry) -> i32 {
        if !delta_px.is_finite() {
            return 0;
        }
        let old_line = i32_from_usize(self.row);
        let origin_y = geom.y_for(old_line) + self.sub;
        let target_y = origin_y + delta_px;
        if !target_y.is_finite() {
            self.sub = 0.0;
            return 0;
        }

        let hit = geom.hit(target_y);
        let new_line = match hit {
            HitTarget::Cell { line } => line,
            HitTarget::Block { id, .. } => geom.get(id).map(|b| b.anchor.line).unwrap_or(old_line),
        };

        if new_line < 0 {
            let spilled = 0i32.saturating_sub(old_line);
            self.row = 0;
            self.sub = 0.0;
            return spilled;
        }

        let mut sub = target_y - geom.y_for(new_line);
        let h = geom.height_of(new_line);
        if !h.is_finite() || h <= 0.0 {
            sub = 0.0;
        } else if sub < 0.0 {
            sub = 0.0;
        } else if sub >= h {
            // Float edge: `hit` places this y on `new_line`, so keep it
            // inside the half-open interval rather than spilling again.
            sub = last_inside(h);
        }

        let spilled = new_line.saturating_sub(old_line);
        self.row = usize_from_i32(new_line);
        self.sub = sub;
        spilled
    }
}

fn last_inside(h: Px) -> Px {
    // Largest value strictly below `h` that is still non-negative. `h / 2`
    // would clip a jump that landed on a boundary; next_down keeps us at
    // the edge the mapping already chose.
    let next = h.next_down();
    if next.is_finite() && next >= 0.0 && next < h {
        next
    } else {
        0.0
    }
}
