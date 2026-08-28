//! Palette-derived opaque chrome colors for the unified title-tab band.

use gpui::Hsla;
use sleipnir_settings::TerminalPalette;

/// Window chrome colors derived from the active terminal palette.
/// Terminal cell colors stay on [`TerminalPalette`]; this is shell-only.
#[derive(Clone, Debug)]
pub struct ChromeTokens {
    pub content_bg: Hsla,
    pub surface: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub fg_disabled: Hsla,
    pub accent: Hsla,
    /// Palette green. Widget `ok` resolves here, never a hardcoded hex.
    pub ok: Hsla,
    /// Palette yellow. Widget `warn` resolves here.
    pub warn: Hsla,
    /// Palette red. Widget `err` resolves here.
    pub err: Hsla,
}

impl ChromeTokens {
    pub fn from_palette(p: &TerminalPalette, window_active: bool) -> Self {
        let content_bg = p.background;
        let is_dark = content_bg.l < 0.5;

        let surface = if is_dark {
            content_bg.blend(p.foreground.opacity(0.06))
        } else {
            content_bg.blend(Hsla::black().opacity(0.06))
        };

        let hover = if is_dark {
            surface.blend(p.foreground.opacity(0.08))
        } else {
            surface.blend(Hsla::black().opacity(0.08))
        };

        let border = if is_dark {
            content_bg.blend(p.foreground.opacity(0.12))
        } else {
            content_bg.blend(Hsla::black().opacity(0.12))
        };

        let fg = p.foreground;
        let fg_muted = if is_dark {
            p.foreground.blend(surface.opacity(0.35))
        } else {
            p.foreground.blend(surface.opacity(0.25))
        };
        let fg_muted = fg_muted.alpha(1.0);
        let surface = surface.alpha(1.0);
        let hover = hover.alpha(1.0);
        let border = border.alpha(1.0);

        let mut tokens = Self {
            content_bg: content_bg.alpha(1.0),
            surface,
            hover,
            border,
            fg: fg.alpha(1.0),
            fg_muted,
            fg_disabled: fg_muted.blend(surface.opacity(0.4)).alpha(1.0),
            accent: p.ansi[4].alpha(1.0),
            ok: p.ansi[2].alpha(1.0),
            warn: p.ansi[3].alpha(1.0),
            err: p.ansi[1].alpha(1.0),
        };

        if !window_active {
            tokens.fg = tokens.fg_disabled;
            tokens.fg_muted = tokens
                .fg_disabled
                .blend(tokens.surface.opacity(0.2))
                .alpha(1.0);
            tokens.surface = tokens
                .surface
                .blend(tokens.content_bg.opacity(0.15))
                .alpha(1.0);
            tokens.hover = tokens.surface;
            tokens.border = tokens.border.blend(tokens.surface.opacity(0.3)).alpha(1.0);
        }

        tokens
    }

    /// Active tab fill — connected to terminal content.
    pub fn active_tab_bg(&self) -> Hsla {
        self.content_bg
    }
}

/// Relative luminance of an opaque-ish HSLA color (sRGB).
pub fn relative_luminance(c: Hsla) -> f32 {
    let rgb = c.to_rgb();
    fn lin(v: f32) -> f32 {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
}

/// WCAG contrast ratio between two colors (assumes opaque presentation).
pub fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let l1 = relative_luminance(a);
    let l2 = relative_luminance(b);
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sleipnir_settings::{Appearance, ThemeName, palette_for_theme};

    #[test]
    fn dark_themes_lift_surface_above_content() {
        for name in [ThemeName::Mocha, ThemeName::Macchiato, ThemeName::Frappe] {
            let p = palette_for_theme(name, Appearance::Dark);
            let t = ChromeTokens::from_palette(&p, true);
            assert!(
                t.surface.l > t.content_bg.l,
                "{name:?}: surface.l ({}) should be > content_bg.l ({})",
                t.surface.l,
                t.content_bg.l
            );
            assert_eq!(t.active_tab_bg().l, t.content_bg.l);
            assert!((t.active_tab_bg().a - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn latte_sinks_surface_below_content() {
        let p = palette_for_theme(ThemeName::Latte, Appearance::Light);
        let t = ChromeTokens::from_palette(&p, true);
        assert!(
            t.surface.l < t.content_bg.l,
            "latte: surface.l ({}) should be < content_bg.l ({})",
            t.surface.l,
            t.content_bg.l
        );
        assert_eq!(t.active_tab_bg().l, t.content_bg.l);
    }

    #[test]
    fn contrast_gates_for_built_in_themes() {
        for name in [
            ThemeName::Mocha,
            ThemeName::Macchiato,
            ThemeName::Frappe,
            ThemeName::Latte,
            ThemeName::Dracula,
            ThemeName::OneDark,
            ThemeName::TokyoNight,
            ThemeName::Nord,
            ThemeName::GruvboxDark,
            ThemeName::GithubDark,
            ThemeName::GithubLight,
        ] {
            let p = palette_for_theme(name, Appearance::Dark);
            let t = ChromeTokens::from_palette(&p, true);
            let active = contrast_ratio(t.fg, t.content_bg);
            let inactive = contrast_ratio(t.fg_muted, t.surface);
            assert!(
                active >= 4.5,
                "{name:?}: active fg/content contrast {active} < 4.5"
            );
            assert!(
                inactive >= 3.0,
                "{name:?}: muted fg/surface contrast {inactive} < 3.0"
            );
        }
    }

    #[test]
    fn inactive_window_dims_foreground() {
        let p = palette_for_theme(ThemeName::Mocha, Appearance::Dark);
        let active = ChromeTokens::from_palette(&p, true);
        let inactive = ChromeTokens::from_palette(&p, false);
        // Inactive chrome uses disabled fg (lower contrast vs content).
        assert!(
            contrast_ratio(inactive.fg, inactive.content_bg)
                <= contrast_ratio(active.fg, active.content_bg) + 0.01
        );
    }

    #[test]
    fn from_palette_is_pure_over_real_palettes() {
        // Drives the shipped entry point — not a reimplementation.
        let p = palette_for_theme(ThemeName::Mocha, Appearance::Dark);
        let t1 = ChromeTokens::from_palette(&p, true);
        let t2 = ChromeTokens::from_palette(&p, true);
        assert_eq!(t1.content_bg.l, t2.content_bg.l);
        assert_eq!(t1.surface.l, t2.surface.l);
        assert_eq!(t1.fg.l, t2.fg.l);
    }
}
