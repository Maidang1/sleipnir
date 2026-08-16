//! Coupled chrome geometry for the unified title-tab band.

use gpui::{Pixels, Point, point, px};

/// Local copy of Zed `ui::TRAFFIC_LIGHT_PADDING` spirit — do not depend on `ui`.
///
/// Zed: 71 default; 78 when `macos_sdk_26_or_later` (requires package-local `build.rs`).
pub struct ChromeGeometry {
    pub height: Pixels,
    pub traffic_light_position: Point<Pixels>,
    pub leading_pad: Pixels,
    pub tab_height: Pixels,
    /// Two-line side-rail row (title + branch). Top-strip chips stay on `tab_height`.
    pub rail_row_height: Pixels,
    pub tab_radius: Pixels,
    pub tab_min_width: Pixels,
    pub tab_max_width: Pixels,
    pub tab_gap: Pixels,
    pub tab_px: Pixels,
    pub after_lights_gap: Pixels,
    pub new_tab_hit: Pixels,
    pub close_hit: Pixels,
    /// Radius of the macOS window's rounded corners; the content is clipped to
    /// this so the opaque terminal background does not square off the corners.
    pub window_radius: Pixels,
    /// Trailing inset so tab chips stay clear of system caption buttons.
    pub trailing_pad: Pixels,
    /// Left tab rail width (side placement).
    pub sidebar_width: Pixels,
    /// Top of the rail reserved for traffic lights + window drag.
    pub sidebar_header: Pixels,
    /// Thin content-column title / drag bar (side placement).
    pub content_title_height: Pixels,
}

impl ChromeGeometry {
    pub fn standard() -> Self {
        Self {
            height: px(40.0),
            traffic_light_position: point(px(12.0), px(12.0)),
            leading_pad: traffic_light_leading_pad(),
            tab_height: px(28.0),
            rail_row_height: px(48.0),
            tab_radius: px(6.0),
            tab_min_width: px(80.0),
            tab_max_width: px(220.0),
            tab_gap: px(2.0),
            tab_px: px(10.0),
            after_lights_gap: px(8.0),
            new_tab_hit: px(28.0),
            close_hit: px(24.0),
            window_radius: px(10.0),
            trailing_pad: px(8.0),
            sidebar_width: px(200.0),
            sidebar_header: px(40.0),
            content_title_height: px(28.0),
        }
    }

    /// Apply a settings-clamped rail width.
    pub fn with_sidebar_width(mut self, width: f32) -> Self {
        self.sidebar_width = px(width);
        self
    }

    /// Leading pad when the window is fullscreen (traffic lights restored by platform).
    pub fn fullscreen_leading_pad() -> Pixels {
        px(8.0)
    }
}

#[inline]
pub fn traffic_light_leading_pad() -> Pixels {
    if cfg!(macos_sdk_26_or_later) {
        // Match Zed ui::TRAFFIC_LIGHT_PADDING without depending on ui.
        px(78.0)
    } else {
        px(71.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_geometry_is_coupled() {
        let g = ChromeGeometry::standard();
        assert_eq!(g.height, px(40.0));
        assert_eq!(g.traffic_light_position, point(px(12.0), px(12.0)));
        assert_eq!(g.close_hit, px(24.0));
        assert_eq!(g.new_tab_hit, px(28.0));
        let pad = traffic_light_leading_pad();
        assert!(pad == px(71.0) || pad == px(78.0));
        assert_eq!(g.trailing_pad, px(8.0));
        assert!(g.leading_pad >= px(71.0));
        assert_eq!(g.sidebar_width, px(200.0));
        assert_eq!(g.sidebar_header, px(40.0));
        assert_eq!(g.content_title_height, px(28.0));
        assert_eq!(g.rail_row_height, px(48.0));
        assert!(g.rail_row_height > g.tab_height);
    }
}
