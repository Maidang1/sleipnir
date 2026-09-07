//! Pixel-art geometry tokens for the chrome. Pixel is the only UI style;
//! this module owns the geometry: radii, border widths, hard offset shadows,
//! and the staircase-corner panel painter. Colors stay with `ChromeTokens`.

use gpui::{Bounds, BoxShadow, Canvas, Hsla, Path, Pixels, Point, Styled, canvas, point, px};

/// Staircase corner size for floating panels.
pub const PANEL_CORNER: Pixels = px(6.0);
/// Border width used by the staircase painter and 2px chrome borders.
pub const PIXEL_BORDER: Pixels = px(2.0);

/// Hard offset drop shadow (no blur, no spread).
pub fn hard_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: Hsla::black().opacity(0.55),
        offset: point(px(6.0), px(6.0)),
        blur_radius: px(0.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

/// Smaller hard shadow for buttons (3px offset).
pub fn button_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: Hsla::black().opacity(0.5),
        offset: point(px(3.0), px(3.0)),
        blur_radius: px(0.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

/// Clockwise staircase-corner polygon points for a `w` x `h` rect at
/// `origin`, with two steps per corner. `corner` is clamped to half the
/// shortest side so small panels degrade to smaller steps instead of
/// self-intersecting. 16 points: 4 staircase vertices per corner.
pub fn staircase_points(
    origin: Point<Pixels>,
    w: Pixels,
    h: Pixels,
    corner: Pixels,
) -> Vec<Point<Pixels>> {
    let c = corner.min(w / 2.0).min(h / 2.0);
    let s = c / 2.0;
    let (x0, y0) = (origin.x, origin.y);
    let (x1, y1) = (x0 + w, y0 + h);
    vec![
        point(x0 + c, y0), // top edge, after the TL cut
        point(x0 + s, y0),
        point(x0 + s, y0 + s),
        point(x0, y0 + s), // TL cut done; down the left edge
        point(x0, y1 - s),
        point(x0 + s, y1 - s),
        point(x0 + s, y1),
        point(x0 + c, y1), // BL cut done; along the bottom
        point(x1 - c, y1),
        point(x1 - s, y1),
        point(x1 - s, y1 - s),
        point(x1, y1 - s), // BR cut done; up the right edge
        point(x1, y0 + s),
        point(x1 - s, y0 + s),
        point(x1 - s, y0),
        point(x0 + c, y0), // TR cut done; close along the top
    ]
}

fn staircase_path(bounds: Bounds<Pixels>, inset: Pixels, corner: Pixels) -> Path<Pixels> {
    let origin = bounds.origin + point(inset, inset);
    let w = bounds.size.width - inset * 2.0;
    let h = bounds.size.height - inset * 2.0;
    let pts = staircase_points(origin, w, h, corner);
    let mut path = Path::new(pts[0]);
    for p in &pts[1..] {
        path.line_to(*p);
    }
    path
}

/// Absolute inset-0 canvas that paints a staircase-cornered panel
/// background: border pass first, then the inset fill pass on top. The
/// caller's panel div keeps its own (hard) box shadow and must NOT set
/// `.bg()` / `.border_*()`.
pub fn pixel_panel_bg(fill: Hsla, border: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let corner = PANEL_CORNER
                .min(bounds.size.width / 4.0)
                .min(bounds.size.height / 4.0);
            window.paint_path(staircase_path(bounds, px(0.0), corner), border);
            window.paint_path(
                staircase_path(bounds, PIXEL_BORDER, corner - PIXEL_BORDER),
                fill,
            );
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staircase_polygon_cuts_two_steps_per_corner() {
        let pts = staircase_points(point(px(0.0), px(0.0)), px(100.0), px(60.0), px(6.0));
        // 4 corners x 4 staircase vertices = 16 points.
        assert_eq!(pts.len(), 16);
        // Starts just right of the top-left corner cut.
        assert_eq!(pts[0], point(px(6.0), px(0.0)));
        // Then steps down-left toward the corner.
        assert_eq!(pts[1], point(px(3.0), px(0.0)));
        assert_eq!(pts[2], point(px(3.0), px(3.0)));
        assert_eq!(pts[3], point(px(0.0), px(3.0)));
        // Closes back at the start.
        assert_eq!(pts[15], pts[0]);
    }

    #[test]
    fn staircase_corner_clamps_on_tiny_panels() {
        let pts = staircase_points(point(px(0.0), px(0.0)), px(8.0), px(8.0), px(6.0));
        // Corner clamped to half the smallest side: no degenerate crossing.
        assert!(pts.iter().all(|p| p.x >= px(0.0) && p.y >= px(0.0)));
        assert!(pts.iter().all(|p| p.x <= px(8.0) && p.y <= px(8.0)));
        assert!(pts.iter().all(|p| p.x <= px(8.0) && p.y <= px(8.0)));
    }

    #[test]
    fn pixel_shadow_is_hard_and_offset() {
        let shadows = hard_shadow();
        assert_eq!(shadows.len(), 1);
        let s = &shadows[0];
        assert_eq!(s.blur_radius, px(0.0));
        assert_eq!(s.spread_radius, px(0.0));
        assert!(s.offset.x > px(0.0) && s.offset.y > px(0.0));
        assert!(!s.inset);

        let b = &button_shadow()[0];
        assert_eq!(b.blur_radius, px(0.0));
        assert!(b.offset.x < s.offset.x, "button shadow is tighter than panel shadow");
    }
}
