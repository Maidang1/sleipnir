//! [`RowGeometry`]: the row ↔ pixel mapping both paint and hit-testing use.
//!
//! Internally this is integer cell-row space. Conversion to pixels is one
//! multiply. `y_for` and `hit` are inverses on cell rows by construction.
//!
//! The mapping is linear: every row is one `line_height` tall. Pixels are a
//! plain `f32`; the host converts `gpui::Pixels` at the boundary.

use crate::Px;

/// One mapping, both directions.
///
/// Host-side. Knows nothing about the grid's internals; the grid knows
/// nothing about it.
#[derive(Clone, Debug)]
pub struct RowGeometry {
    line_height: Px,
    line_count: i32,
}

impl RowGeometry {
    /// Empty document, linear mapping.
    ///
    /// A non-finite or non-positive `line_height` is kept as given for
    /// inspection but treated as zero by every query so we never panic or
    /// invert the axis.
    pub fn new(line_height: Px) -> Self {
        Self {
            line_height,
            line_count: 0,
        }
    }

    pub fn line_height(&self) -> Px {
        self.line_height
    }

    pub fn set_line_height(&mut self, line_height: Px) {
        self.line_height = line_height;
    }

    /// Number of grid lines in the document (`history + screen`). Used only
    /// for [`Self::total_height`]; queries on other lines still work so an
    /// out-of-range anchor cannot panic.
    pub fn line_count(&self) -> i32 {
        self.line_count
    }

    pub fn set_line_count(&mut self, line_count: i32) {
        self.line_count = line_count.max(0);
    }

    /// Pixel y of the top of `abs_line`.
    pub fn y_for(&self, abs_line: i32) -> Px {
        if !usable_line_height(self.line_height) {
            return 0.0;
        }
        (abs_line as Px) * self.line_height
    }

    /// Pixel → row. Replaces `(pos.y / line_height) as i32`.
    ///
    /// Inverse of [`Self::y_for`] on cell rows: `hit(y_for(line))` is that
    /// same line.
    pub fn hit(&self, y: Px) -> i32 {
        if !y.is_finite() || !usable_line_height(self.line_height) {
            return 0;
        }
        let h = self.line_height;
        let mut n = (y / h).floor() as i64;
        for _ in 0..16 {
            let start = self.rows_to_px(n);
            if start > y {
                n = n.saturating_sub(1);
                continue;
            }
            let end = self.rows_to_px(n.saturating_add(1));
            if end <= y {
                n = n.saturating_add(1);
                continue;
            }
            break;
        }
        i32_from_i64(n)
    }

    /// Total document height, for scroll extent. Empty documents are 0.
    pub fn total_height(&self) -> Px {
        if self.line_count <= 0 {
            return 0.0;
        }
        self.y_for(self.line_count)
    }

    /// Pixel height of one grid line.
    pub fn height_of(&self, _abs_line: i32) -> Px {
        self.line_height
    }

    fn rows_to_px(&self, rows: i64) -> Px {
        if !usable_line_height(self.line_height) {
            0.0
        } else {
            (rows as Px) * self.line_height
        }
    }
}

fn usable_line_height(h: Px) -> bool {
    h.is_finite() && h > 0.0
}

pub(crate) fn i32_from_i64(n: i64) -> i32 {
    i32::try_from(n).unwrap_or(if n < 0 { i32::MIN } else { i32::MAX })
}

pub(crate) fn i32_from_usize(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

pub(crate) fn usize_from_i32(n: i32) -> usize {
    usize::try_from(n).unwrap_or(0)
}
