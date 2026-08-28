//! Integer cell-grid coordinates (ADR-0017 constraint 2).
//!
//! The terminal's unit is the character cell. Widget sizes live here so a Block
//! height is an integer row count (ADR-0018) and font-size changes do not
//! require per-plugin rework. Pixels and `f32` stay out of this crate.

/// A cell on the widget surface. Origin is the top-left of the laid-out tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellPos {
    pub col: u32,
    pub row: u32,
}

impl CellPos {
    pub const ORIGIN: Self = Self { col: 0, row: 0 };

    pub fn new(col: u32, row: u32) -> Self {
        Self { col, row }
    }
}

/// Axis-aligned rectangle in cells: `col`/`row` origin, `width` columns,
/// `height` rows. All arithmetic is saturating so a pathological tree cannot
/// overflow into a panic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CellRect {
    pub col: u32,
    pub row: u32,
    pub width: u32,
    pub height: u32,
}

impl CellRect {
    pub fn new(col: u32, row: u32, width: u32, height: u32) -> Self {
        Self {
            col,
            row,
            width,
            height,
        }
    }

    pub fn at(pos: CellPos, width: u32, height: u32) -> Self {
        Self::new(pos.col, pos.row, width, height)
    }

    pub fn pos(self) -> CellPos {
        CellPos {
            col: self.col,
            row: self.row,
        }
    }

    /// Exclusive right edge.
    pub fn right(self) -> u32 {
        self.col.saturating_add(self.width)
    }

    /// Exclusive bottom edge.
    pub fn bottom(self) -> u32 {
        self.row.saturating_add(self.height)
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains(self, pos: CellPos) -> bool {
        !self.is_empty()
            && pos.col >= self.col
            && pos.row >= self.row
            && pos.col < self.right()
            && pos.row < self.bottom()
    }

    pub fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.col < other.right()
            && other.col < self.right()
            && self.row < other.bottom()
            && other.row < self.bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        let r = CellRect::new(2, 3, 4, 1);
        assert!(r.contains(CellPos::new(2, 3)));
        assert!(r.contains(CellPos::new(5, 3)));
        assert!(!r.contains(CellPos::new(6, 3)));
        assert!(!r.contains(CellPos::new(2, 4)));
        assert!(!r.contains(CellPos::new(1, 3)));
    }

    #[test]
    fn empty_rect_contains_nothing() {
        let r = CellRect::new(0, 0, 0, 4);
        assert!(!r.contains(CellPos::ORIGIN));
        assert!(!r.intersects(CellRect::new(0, 0, 1, 1)));
    }

    #[test]
    fn saturating_edges_do_not_panic() {
        let r = CellRect::new(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        assert_eq!(r.right(), u32::MAX);
        assert_eq!(r.bottom(), u32::MAX);
        assert!(!r.contains(CellPos::ORIGIN));
    }
}
