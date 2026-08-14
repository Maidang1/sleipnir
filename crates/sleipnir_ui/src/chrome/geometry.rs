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
}

impl ChromeGeometry {
    pub fn standard() -> Self {
        Self::standard_for(cfg!(windows))
    }

    /// Chrome insets for a given OS family. `windows = true` uses a small
    /// leading pad and reserves trailing space for caption buttons.
    pub fn standard_for(windows: bool) -> Self {
        Self {
            height: px(40.0),
            traffic_light_position: point(px(12.0), px(12.0)),
            leading_pad: leading_pad_for(windows),
            tab_height: px(28.0),
            tab_radius: px(6.0),
            tab_min_width: px(80.0),
            tab_max_width: px(220.0),
            tab_gap: px(2.0),
            tab_px: px(10.0),
            after_lights_gap: px(8.0),
            new_tab_hit: px(28.0),
            close_hit: px(24.0),
            window_radius: px(10.0),
            trailing_pad: trailing_pad_for(windows),
        }
    }

    /// Leading pad when the window is fullscreen (traffic lights restored by platform).
    pub fn fullscreen_leading_pad() -> Pixels {
        px(8.0)
    }
}

#[inline]
#[allow(dead_code)]
pub fn traffic_light_leading_pad() -> Pixels {
    leading_pad_for(cfg!(windows))
}

#[inline]
pub fn leading_pad_for(windows: bool) -> Pixels {
    if windows {
        px(8.0)
    } else if cfg!(target_os = "linux") {
        // No traffic lights on Linux: the custom chrome band starts flush with
        // the window edge, matching the Windows layout.
        px(8.0)
    } else if cfg!(macos_sdk_26_or_later) {
        // Match Zed ui::TRAFFIC_LIGHT_PADDING without depending on ui.
        px(78.0)
    } else {
        px(71.0)
    }
}

/// Space reserved on the right for system window controls.
#[inline]
pub fn trailing_pad_for(windows: bool) -> Pixels {
    if windows {
        px(138.0)
    } else {
        px(8.0)
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
        // Without SDK 26 cfg in unit test host, pad is 71 unless build set the cfg.
        let pad = traffic_light_leading_pad();
        if cfg!(windows) {
            assert_eq!(pad, px(8.0));
            assert_eq!(g.trailing_pad, px(138.0));
        } else if cfg!(target_os = "linux") {
            // No traffic lights: flush leading pad, no caption-button trail.
            assert_eq!(pad, px(8.0));
            assert_eq!(g.trailing_pad, px(8.0));
        } else {
            assert!(pad == px(71.0) || pad == px(78.0));
            assert_eq!(g.trailing_pad, px(8.0));
        }
    }

    #[test]
    fn windows_chrome_reserves_caption_not_traffic_lights() {
        let win = ChromeGeometry::standard_for(true);
        assert_eq!(win.leading_pad, px(8.0));
        assert_eq!(win.trailing_pad, px(138.0));
        let mac = ChromeGeometry::standard_for(false);
        if cfg!(target_os = "linux") {
            // Linux has no traffic lights either; the "non-Windows" geometry
            // falls back to a flush leading pad.
            assert_eq!(mac.leading_pad, px(8.0));
        } else {
            assert!(mac.leading_pad >= px(71.0));
        }
        assert_eq!(mac.trailing_pad, px(8.0));
    }
}
