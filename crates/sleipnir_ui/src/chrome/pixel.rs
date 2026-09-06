//! Pixel-art geometry tokens for the `ui_style = "pixel"` chrome skin.
//!
//! Colors stay with `ChromeTokens`; this module owns *geometry*: radii,
//! border widths, and hard offset shadows. Every function takes the active
//! `UiStyle` and returns today's exact values for `UiStyle::Default`, so
//! default mode is visually unchanged by construction.

use gpui::{BoxShadow, Hsla, Pixels, point, px};
use sleipnir_settings::UiStyle;

/// Staircase corner size for floating panels in pixel mode.
pub const PANEL_CORNER: Pixels = px(6.0);
/// Border width used by the staircase painter and 2px chrome borders.
pub const PIXEL_BORDER: Pixels = px(2.0);

/// Corner radius: pixel mode is always square.
pub fn radius(style: UiStyle, default: Pixels) -> Pixels {
    match style {
        UiStyle::Pixel => px(0.0),
        UiStyle::Default => default,
    }
}

/// Border width: pixel mode doubles to a chunky 2px line.
pub fn border_width(style: UiStyle, default: Pixels) -> Pixels {
    match style {
        UiStyle::Pixel => PIXEL_BORDER,
        UiStyle::Default => default,
    }
}

/// Hard offset drop shadow (no blur, no spread). Empty in default mode so
/// callers keep their existing `.shadow_lg()` etc. via `.when(...)`.
pub fn hard_shadow(style: UiStyle) -> Vec<BoxShadow> {
    match style {
        UiStyle::Default => Vec::new(),
        UiStyle::Pixel => vec![BoxShadow {
            color: Hsla::black().opacity(0.55),
            offset: point(px(6.0), px(6.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
            inset: false,
        }],
    }
}

/// Smaller hard shadow for buttons (3px offset).
pub fn button_shadow(style: UiStyle) -> Vec<BoxShadow> {
    match style {
        UiStyle::Default => Vec::new(),
        UiStyle::Pixel => vec![BoxShadow {
            color: Hsla::black().opacity(0.5),
            offset: point(px(3.0), px(3.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
            inset: false,
        }],
    }
}

/// Read the active style from global settings. Shorthand for call sites.
pub fn active_style(cx: &gpui::App) -> UiStyle {
    sleipnir_settings::TerminalSettings::get_global(cx).ui_style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_mode_squares_corners_and_thickens_borders() {
        assert_eq!(radius(UiStyle::Pixel, px(6.0)), px(0.0));
        assert_eq!(border_width(UiStyle::Pixel, px(1.0)), px(2.0));
    }

    #[test]
    fn default_mode_passes_values_through() {
        // Golden values: default mode must not drift from today's look.
        assert_eq!(radius(UiStyle::Default, px(6.0)), px(6.0));
        assert_eq!(border_width(UiStyle::Default, px(1.0)), px(1.0));
        assert!(hard_shadow(UiStyle::Default).is_empty());
        assert!(button_shadow(UiStyle::Default).is_empty());
    }

    #[test]
    fn pixel_shadow_is_hard_and_offset() {
        let shadows = hard_shadow(UiStyle::Pixel);
        assert_eq!(shadows.len(), 1);
        let s = &shadows[0];
        assert_eq!(s.blur_radius, px(0.0));
        assert_eq!(s.spread_radius, px(0.0));
        assert!(s.offset.x > px(0.0) && s.offset.y > px(0.0));
        assert!(!s.inset);

        let b = &button_shadow(UiStyle::Pixel)[0];
        assert_eq!(b.blur_radius, px(0.0));
        assert!(b.offset.x < s.offset.x, "button shadow is tighter than panel shadow");
    }
}
