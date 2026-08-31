//! Display-line ↔ pixel mapping through [`row_geometry::RowGeometry`] (ADR-0018).
//!
//! This is the only host-side path from a grid line to a y coordinate and
//! back. The vendored parser is not involved: the grid stays a uniform
//! character grid, and only this mapping changes. A site that multiplies a
//! line by `line_height` for a y, or divides y by `line_height` for a line,
//! is a mouse-versus-paint drift bug.

use row_geometry::{HitTarget, RowGeometry};

use crate::{Point, SelectionSide, TerminalBounds};
use gpui::{Pixels, Point as GpuiPoint, px};
use std::cmp::{self, min};

/// Absolute line currently at the top of the viewport.
///
/// `display_offset` is how many grid rows the viewport has moved into
/// history; the matching absolute line is `history - offset`.
pub fn viewport_top_abs(history_size: i32, display_offset: usize) -> i32 {
    history_size.saturating_sub(i32::try_from(display_offset).unwrap_or(i32::MAX))
}

/// Pixel y of a display line relative to the viewport origin, including the
/// host `sub` remainder. Paint adds `origin.y`.
pub fn y_for_display(geom: &RowGeometry, display_line: i32, top_abs: i32, sub: f32) -> f32 {
    geom.y_for(top_abs.saturating_add(display_line)) - geom.y_for(top_abs) - sub
}

/// Inverse of [`y_for_display`]: a y relative to the viewport origin.
pub fn hit_display(geom: &RowGeometry, local_y: f32, top_abs: i32, sub: f32) -> HitTarget {
    if !local_y.is_finite() {
        return HitTarget::Cell { line: top_abs };
    }
    geom.hit(geom.y_for(top_abs) + sub + local_y)
}

/// Alacritty `Point.line` for an absolute scrollback line.
pub fn abs_to_grid_point_line(abs: i32, history_size: i32) -> i32 {
    abs.saturating_sub(history_size)
}

/// Snapshot the mouse mapper needs. Built from [`crate::Terminal`] so paint
/// and hit share one geometry.
#[derive(Clone, Copy)]
pub(crate) struct PointerMap<'a> {
    pub size: TerminalBounds,
    pub display_offset: usize,
    pub geometry: &'a RowGeometry,
    pub history_size: i32,
    pub sub: f32,
}

impl<'a> PointerMap<'a> {
    pub fn top_abs(self) -> i32 {
        viewport_top_abs(self.history_size, self.display_offset)
    }

    pub fn grid_point(self, pos: GpuiPoint<Pixels>) -> Point {
        self.grid_point_and_side(pos).0
    }

    pub fn grid_point_and_side(self, pos: GpuiPoint<Pixels>) -> (Point, SelectionSide) {
        let mut column = (pos.x / self.size.cell_width) as usize;
        let cell_x = cmp::max(px(0.), pos.x) % self.size.cell_width;
        let half_cell_width = self.size.cell_width / 2.0;
        let mut side = if cell_x > half_cell_width {
            SelectionSide::Right
        } else {
            SelectionSide::Left
        };

        let last_column = self.size.num_columns().saturating_sub(1);
        if column > last_column {
            column = last_column;
            side = SelectionSide::Right;
        }
        let column = min(column, last_column);

        let hit = hit_display(self.geometry, f32::from(pos.y), self.top_abs(), self.sub);
        let abs = match hit {
            HitTarget::Cell { line } => line,
            HitTarget::Block { id, .. } => self
                .geometry
                .get(id)
                .map(|b| b.anchor.line)
                .unwrap_or_else(|| self.top_abs()),
        };
        let mut line = abs_to_grid_point_line(abs, self.history_size);
        let bottommost_line = i32::try_from(self.size.num_lines().saturating_sub(1))
            .unwrap_or(i32::MAX)
            .saturating_sub(i32::try_from(self.display_offset).unwrap_or(0));
        // `bottommost_line` is Point.line of the last visible row:
        // display (num_lines-1) - display_offset.
        let bottommost_display =
            i32::try_from(self.size.num_lines().saturating_sub(1)).unwrap_or(i32::MAX);
        let display = abs
            .saturating_sub(self.history_size)
            .saturating_add(i32::try_from(self.display_offset).unwrap_or(0));
        if display > bottommost_display {
            line = bottommost_line;
            side = SelectionSide::Right;
        } else if display < 0 {
            side = SelectionSide::Left;
        }

        (Point::new(line, column), side)
    }

    pub fn hit(self, pos: GpuiPoint<Pixels>) -> HitTarget {
        hit_display(self.geometry, f32::from(pos.y), self.top_abs(), self.sub)
    }

    /// Cell index into `Content.cells` (row-major display). Used for hyperlink
    /// lookup; a Block hit maps to the anchor's display row.
    pub fn content_index(self, pos: GpuiPoint<Pixels>) -> usize {
        let col = (pos.x / self.size.cell_width()).round() as usize;
        let clamped_col = min(col, self.size.num_columns().saturating_sub(1));
        let hit = self.hit(pos);
        let abs = match hit {
            HitTarget::Cell { line } => line,
            HitTarget::Block { id, .. } => self
                .geometry
                .get(id)
                .map(|b| b.anchor.line)
                .unwrap_or_else(|| self.top_abs()),
        };
        let display = abs
            .saturating_sub(self.history_size)
            .saturating_add(i32::try_from(self.display_offset).unwrap_or(0));
        let row = if display < 0 { 0 } else { display as usize };
        let clamped_row = min(row, self.size.num_lines().saturating_sub(1));
        clamped_row * self.size.num_columns() + clamped_col
    }
}

/// Walk `sub` against successive row heights. The returned i32 is a
/// `display_offset` delta: positive means more into history. `sub` is left
/// in `[0, height_of(current_top))`.
pub fn accumulate_wheel(
    sub: &mut f32,
    delta_px: f32,
    geom: &RowGeometry,
    history_size: i32,
    display_offset: usize,
) -> i32 {
    if !delta_px.is_finite() {
        return 0;
    }
    let mut offset = i32::try_from(display_offset).unwrap_or(i32::MAX);
    let history = history_size.max(0);
    *sub += delta_px;
    if !sub.is_finite() {
        *sub = 0.0;
        return 0;
    }
    let mut spilled = 0i32;
    for _ in 0..10_000 {
        let top_abs = history.saturating_sub(offset);
        let h = geom.height_of(top_abs);
        if !h.is_finite() || h <= 0.0 {
            *sub = 0.0;
            break;
        }
        if *sub >= h {
            *sub -= h;
            spilled = spilled.saturating_add(1);
            offset = offset.saturating_add(1);
            if offset > history {
                *sub = 0.0;
                break;
            }
            continue;
        }
        if *sub < 0.0 {
            if offset <= 0 {
                *sub = 0.0;
                break;
            }
            offset = offset.saturating_sub(1);
            let h_prev = geom.height_of(history.saturating_sub(offset));
            if !h_prev.is_finite() || h_prev <= 0.0 {
                *sub = 0.0;
                break;
            }
            *sub += h_prev;
            spilled = spilled.saturating_sub(1);
            continue;
        }
        break;
    }
    spilled
}

#[cfg(test)]
mod tests {
    use super::*;
    use row_geometry::{Anchor, Block, BlockId, RunId};

    fn bid(n: u128) -> BlockId {
        BlockId::from_u128(n)
    }

    fn geom(lh: f32, blocks: &[(u128, i32, u16)]) -> RowGeometry {
        let mut g = RowGeometry::new(lh);
        g.set_line_count(80);
        for &(n, line, height) in blocks {
            g.upsert(Block {
                id: bid(n),
                run_id: RunId::from_u128(n),
                anchor: Anchor { line, column: 0 },
                height,
            });
        }
        g
    }

    fn bounds(lh: f32) -> TerminalBounds {
        TerminalBounds::new(
            px(lh),
            px(8.0),
            gpui::Bounds {
                origin: gpui::point(px(0.0), px(0.0)),
                size: gpui::size(px(80.0), px(lh * 10.0)),
            },
        )
    }

    #[test]
    fn empty_geometry_matches_linear_y() {
        let g = geom(16.0, &[]);
        let top = 40;
        for d in 0..10 {
            assert_eq!(y_for_display(&g, d, top, 0.0), d as f32 * 16.0);
        }
    }

    #[test]
    fn mouse_and_paint_agree_across_geometries() {
        for &(lh, blocks) in &[
            (16.0, &[][..]),
            (16.0, &[(1, 42, 5)][..]),
            (17.0, &[(1, 40, 3), (2, 45, 2)][..]),
            (18.5, &[(1, 0, 4)][..]),
        ] {
            let g = geom(lh, blocks);
            let history = 40;
            let offset = 5usize;
            let top = viewport_top_abs(history, offset);
            let sub = 3.0;
            let map = PointerMap {
                size: bounds(lh),
                display_offset: offset,
                geometry: &g,
                history_size: history,
                sub,
            };
            for display in 0..8 {
                let y = y_for_display(&g, display, top, sub);
                let hit = hit_display(&g, y, top, sub);
                let abs = match hit {
                    HitTarget::Cell { line } => line,
                    HitTarget::Block { id, local_y } => {
                        assert_eq!(local_y, 0.0);
                        g.get(id).unwrap().anchor.line
                    }
                };
                assert_eq!(
                    abs,
                    top.saturating_add(display),
                    "lh={lh} display={display} y={y}"
                );
                let pos = gpui::point(px(4.0), px(y));
                let point = map.grid_point(pos);
                assert_eq!(
                    point.line,
                    abs_to_grid_point_line(abs, history),
                    "pointer vs paint drift at display {display}"
                );
            }
        }
    }

    #[test]
    fn alt_screen_is_linear_even_with_blocks_stored() {
        let mut g = geom(16.0, &[(1, 42, 8)]);
        g.set_alt_screen(true);
        let top = 40;
        assert_eq!(y_for_display(&g, 2, top, 0.0), 2.0 * 16.0);
        assert_eq!(
            hit_display(&g, 2.0 * 16.0, top, 0.0),
            HitTarget::Cell { line: top + 2 }
        );
    }

    #[test]
    fn sub_shifts_paint_and_hit_together() {
        let g = geom(16.0, &[]);
        let top = 10;
        let y0 = y_for_display(&g, 0, top, 4.0);
        assert_eq!(y0, -4.0);
        match hit_display(&g, 0.0, top, 4.0) {
            HitTarget::Cell { line } => assert_eq!(line, top),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn accumulate_wheel_retains_remainder() {
        let g = geom(16.0, &[]);
        let mut sub = 0.0;
        let spilled = accumulate_wheel(&mut sub, 40.0, &g, 40, 0);
        assert_eq!(spilled, 2);
        assert_eq!(sub, 8.0);
        let spilled = accumulate_wheel(&mut sub, -20.0, &g, 40, 2);
        assert_eq!(spilled, -1);
        assert_eq!(sub, 4.0);
    }

    #[test]
    fn accumulate_wheel_walks_a_tall_block() {
        let g = geom(16.0, &[(1, 38, 5)]);
        // history 40, offset 2 → top_abs = 38, the block.
        let mut sub = 0.0;
        let spilled = accumulate_wheel(&mut sub, 3.0 * 16.0, &g, 40, 2);
        assert_eq!(spilled, 0);
        assert_eq!(sub, 3.0 * 16.0);
        let spilled = accumulate_wheel(&mut sub, 2.0 * 16.0 + 4.0, &g, 40, 2);
        assert_eq!(spilled, 1);
        assert_eq!(sub, 4.0);
    }
}
