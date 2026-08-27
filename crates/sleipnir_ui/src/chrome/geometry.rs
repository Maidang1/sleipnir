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
        Self::standard_for(cfg!(not(target_os = "macos")))
    }

    /// Chrome insets for a windowed platform family.
    pub fn standard_for(desktop_controls: bool) -> Self {
        Self::for_window(desktop_controls, false)
    }

    /// Chrome insets for the platform and current fullscreen state.
    pub fn for_window(desktop_controls: bool, fullscreen: bool) -> Self {
        Self {
            height: px(32.0),
            traffic_light_position: point(px(12.0), px(8.0)),
            leading_pad: if fullscreen {
                Self::fullscreen_leading_pad()
            } else {
                leading_pad_for(desktop_controls)
            },
            tab_height: px(24.0),
            tab_radius: px(5.0),
            tab_min_width: px(80.0),
            tab_max_width: px(220.0),
            tab_gap: px(2.0),
            tab_px: px(10.0),
            after_lights_gap: px(8.0),
            new_tab_hit: px(28.0),
            close_hit: px(24.0),
            window_radius: px(10.0),
            trailing_pad: if fullscreen {
                px(0.0)
            } else {
                trailing_pad_for(desktop_controls)
            },
        }
    }

    /// Leading pad when the window is fullscreen.
    pub fn fullscreen_leading_pad() -> Pixels {
        px(8.0)
    }
}

#[inline]
#[allow(dead_code)]
pub fn traffic_light_leading_pad() -> Pixels {
    leading_pad_for(cfg!(not(target_os = "macos")))
}

#[inline]
pub fn leading_pad_for(desktop_controls: bool) -> Pixels {
    if desktop_controls {
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
pub fn trailing_pad_for(desktop_controls: bool) -> Pixels {
    if desktop_controls { px(138.0) } else { px(8.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_geometry_is_coupled() {
        let g = ChromeGeometry::standard();
        assert_eq!(g.height, px(32.0));
        assert_eq!(g.traffic_light_position, point(px(12.0), px(8.0)));
        assert_eq!(g.close_hit, px(24.0));
        assert_eq!(g.new_tab_hit, px(28.0));
        let pad = traffic_light_leading_pad();
        if cfg!(target_os = "macos") {
            assert!(pad == px(71.0) || pad == px(78.0));
            assert_eq!(g.trailing_pad, px(8.0));
            assert!(g.leading_pad >= px(71.0));
        } else {
            assert_eq!(pad, px(8.0));
            assert_eq!(g.trailing_pad, px(138.0));
        }
    }

    #[test]
    fn desktop_chrome_reserves_caption_buttons_only_when_windowed() {
        let desktop = ChromeGeometry::for_window(true, false);
        assert_eq!(desktop.leading_pad, px(8.0));
        assert_eq!(desktop.trailing_pad, px(138.0));

        let desktop_fullscreen = ChromeGeometry::for_window(true, true);
        assert_eq!(desktop_fullscreen.trailing_pad, px(0.0));

        let macos = ChromeGeometry::for_window(false, false);
        assert!(macos.leading_pad >= px(71.0));
        assert_eq!(macos.trailing_pad, px(8.0));
    }
}
