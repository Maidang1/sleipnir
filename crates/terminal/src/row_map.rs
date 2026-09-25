//! Display-line ↔ pixel mapping through [`row_geometry::RowGeometry`] (ADR-0018).
//!
//! This is the only host-side path from a grid line to a y coordinate and
//! back. The vendored parser is not involved: the grid stays a uniform
//! character grid, and only this mapping changes. A site that multiplies a
//! line by `line_height` for a y, or divides y by `line_height` for a line,
//! is a mouse-versus-paint drift bug.

use row_geometry::RowGeometry;

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

/// Rows dropped from scrollback between two observations, saturating.
///
/// A shrink means every stored absolute line is stale by this much. History
/// belongs to the terminal, so this is computed once there rather than
/// re-derived by each mount that stores absolute lines (gutter markers,
/// prompt markers) — two trackers of the same fact drift, and a drift here
/// strands content on the wrong row.
pub fn history_shrink(previous: usize, current: usize) -> i32 {
    i32::try_from(previous.saturating_sub(current)).unwrap_or(i32::MAX)
}

/// Pixel y of a display line relative to the viewport origin, including the
/// host `sub` remainder. Paint adds `origin.y`.
pub fn y_for_display(geom: &RowGeometry, display_line: i32, top_abs: i32, sub: f32) -> f32 {
    geom.y_for(top_abs.saturating_add(display_line)) - geom.y_for(top_abs) - sub
}

/// Inverse of [`y_for_display`]: absolute line for a y relative to the
/// viewport origin.
pub fn hit_display(geom: &RowGeometry, local_y: f32, top_abs: i32, sub: f32) -> i32 {
    if !local_y.is_finite() {
        return top_abs;
    }
    geom.hit(geom.y_for(top_abs) + sub + local_y)
}

/// Alacritty `Point.line` for an absolute scrollback line.
///
/// Plain subtraction, not `saturating_sub`: scrollback grid lines above the
/// viewport are legitimately negative in alacritty's coordinate space, and
/// `abs` is always `>= 0` here (it comes from a recorded `cursor + history`),
/// so this cannot underflow.
pub fn abs_to_grid_point_line(abs: i32, history_size: i32) -> i32 {
    abs - history_size
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

        let abs = hit_display(self.geometry, f32::from(pos.y), self.top_abs(), self.sub);
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

    pub fn hit(self, pos: GpuiPoint<Pixels>) -> i32 {
        hit_display(self.geometry, f32::from(pos.y), self.top_abs(), self.sub)
    }

    /// Cell index into `Content.cells` (row-major display). Used for hyperlink
    /// lookup.
    pub fn content_index(self, pos: GpuiPoint<Pixels>) -> usize {
        let col = (pos.x / self.size.cell_width()).round() as usize;
        let clamped_col = min(col, self.size.num_columns().saturating_sub(1));
        let abs = self.hit(pos);
        let display = abs
            .saturating_sub(self.history_size)
            .saturating_add(i32::try_from(self.display_offset).unwrap_or(0));
        let row = if display < 0 { 0 } else { display as usize };
        let clamped_row = min(row, self.size.num_lines().saturating_sub(1));
        clamped_row * self.size.num_columns() + clamped_col
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use row_geometry::ViewportPosition;

    fn geom(lh: f32) -> RowGeometry {
        let mut g = RowGeometry::new(lh);
        g.set_line_count(80);
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
        let g = geom(16.0);
        let top = 40;
        for d in 0..10 {
            assert_eq!(y_for_display(&g, d, top, 0.0), d as f32 * 16.0);
        }
    }

    #[test]
    fn mouse_and_paint_agree_across_geometries() {
        for &lh in &[16.0, 17.0, 18.5] {
            let g = geom(lh);
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
                let abs = hit_display(&g, y, top, sub);
                assert_eq!(abs, top.saturating_add(display), "lh={lh} y={y}");
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
    fn sub_shifts_paint_and_hit_together() {
        let g = geom(16.0);
        let top = 10;
        let y0 = y_for_display(&g, 0, top, 4.0);
        assert_eq!(y0, -4.0);
        assert_eq!(hit_display(&g, 0.0, top, 4.0), top);
    }

    #[test]
    fn wheel_delta_preserves_the_legacy_scroll_delta_direction() {
        let g = geom(16.0);
        for (initial_sub, delta_px, expected_display_offset_delta) in
            [(0.0, 40.0, 2), (4.0, -20.0, -1)]
        {
            let mut viewport = ViewportPosition {
                row: viewport_top_abs(40, 2) as usize,
                sub: initial_sub,
            };
            let absolute_line_delta = viewport.apply_pixel_delta(delta_px, &g);

            assert_eq!(
                absolute_line_delta, expected_display_offset_delta,
                "pixel delta {delta_px} must retain the legacy Scroll::Delta sign"
            );
        }
    }

    #[test]
    fn wheel_route_characterization() {
        struct Case {
            name: &'static str,
            history_size: i32,
            display_offset: usize,
            initial_sub: f32,
            delta_px: f32,
            expected_absolute_delta: i32,
            expected_display_offset_delta: i32,
            expected_clamped_offset: i32,
            expected_sub: f32,
        }

        let cases = [
            Case {
                name: "positive delta",
                history_size: 40,
                display_offset: 0,
                initial_sub: 0.0,
                delta_px: 40.0,
                expected_absolute_delta: 2,
                expected_display_offset_delta: 2,
                expected_clamped_offset: 2,
                expected_sub: 8.0,
            },
            Case {
                name: "negative delta",
                history_size: 40,
                display_offset: 2,
                initial_sub: 4.0,
                delta_px: -20.0,
                expected_absolute_delta: -1,
                expected_display_offset_delta: -1,
                expected_clamped_offset: 1,
                expected_sub: 0.0,
            },
            Case {
                name: "clamped before absolute line zero",
                history_size: 40,
                display_offset: 40,
                initial_sub: 0.0,
                delta_px: -20.0,
                expected_absolute_delta: 0,
                expected_display_offset_delta: 0,
                expected_clamped_offset: 40,
                expected_sub: 0.0,
            },
            Case {
                name: "clamped at bottom display offset",
                history_size: 40,
                display_offset: 0,
                initial_sub: 0.0,
                delta_px: 20.0,
                expected_absolute_delta: 1,
                expected_display_offset_delta: 1,
                expected_clamped_offset: 1,
                expected_sub: 4.0,
            },
            Case {
                name: "NaN is ignored",
                history_size: 40,
                display_offset: 2,
                initial_sub: 3.0,
                delta_px: f32::NAN,
                expected_absolute_delta: 0,
                expected_display_offset_delta: 0,
                expected_clamped_offset: 2,
                expected_sub: 3.0,
            },
            Case {
                name: "infinity is ignored",
                history_size: 40,
                display_offset: 2,
                initial_sub: 3.0,
                delta_px: f32::INFINITY,
                expected_absolute_delta: 0,
                expected_display_offset_delta: 0,
                expected_clamped_offset: 2,
                expected_sub: 3.0,
            },
        ];

        for case in cases {
            let g = geom(16.0);
            let top_abs = viewport_top_abs(case.history_size, case.display_offset);
            let mut viewport = ViewportPosition {
                row: usize::try_from(top_abs).unwrap_or(0),
                sub: case.initial_sub,
            };
            let absolute_delta = viewport.apply_pixel_delta(case.delta_px, &g);
            let display_offset_delta = absolute_delta;
            let clamped_offset = i32::try_from(case.display_offset)
                .unwrap_or(i32::MAX)
                .saturating_add(display_offset_delta)
                .clamp(0, case.history_size.max(0));

            assert_eq!(
                absolute_delta, case.expected_absolute_delta,
                "{} absolute-line delta",
                case.name
            );
            assert_eq!(
                display_offset_delta, case.expected_display_offset_delta,
                "{} display-offset delta",
                case.name
            );
            assert_eq!(
                clamped_offset, case.expected_clamped_offset,
                "{} clamped offset",
                case.name
            );
            assert_eq!(viewport.sub, case.expected_sub, "{} remainder", case.name);
        }
    }

    // -----------------------------------------------------------------------
    // History shrink accounting
    // -----------------------------------------------------------------------

    #[test]
    fn history_shrink_reports_only_shrink() {
        assert_eq!(history_shrink(40, 10), 30, "30 rows dropped");
        assert_eq!(history_shrink(10, 10), 0, "steady history is not a shrink");
        assert_eq!(history_shrink(10, 40), 0, "growth is not a shrink");
    }

    #[test]
    fn history_shrink_saturates_instead_of_wrapping() {
        // usize::MAX -> 0 exceeds i32; it must clamp, not wrap negative, or a
        // rebase would shift anchors the wrong way.
        assert_eq!(history_shrink(usize::MAX, 0), i32::MAX);
    }
}
