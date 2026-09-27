//! Palette-derived opaque chrome colors for the unified title-tab band.
//!
//! Tokens are contrast-paired: a light palette and its dark counterpart land
//! within 1.0 contrast ratio of each other against their own backgrounds.
//! The direction of a surface (lift or sink) follows which of black or white
//! contrasts more with the content background, not a raw HSL lightness cut.

use gpui::Hsla;
use sleipnir_settings::TerminalPalette;

use crate::theme_color;

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

/// White contrasts at least as well as black, so ink on this background is light.
fn prefers_light_ink(bg: Hsla) -> bool {
    theme_color::contrast_ratio(Hsla::white(), bg) >= theme_color::contrast_ratio(Hsla::black(), bg)
}

/// Blend `ink` over `bg` until the result's contrast against `bg` meets `target`.
///
/// `t = 0` is `bg` (contrast 1). `t = 1` is opaque `ink`. When `ink` cannot
/// reach `target`, the opaque ink is returned — that is the most contrast this
/// pair can make.
fn blend_toward_contrast(bg: Hsla, ink: Hsla, target: f32) -> Hsla {
    let bg = bg.alpha(1.0);
    let ink = ink.alpha(1.0);
    if theme_color::contrast_ratio(ink, bg) <= target {
        return ink;
    }
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        let mixed = bg.blend(ink.alpha(mid)).alpha(1.0);
        if theme_color::contrast_ratio(mixed, bg) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bg.blend(ink.alpha(hi)).alpha(1.0)
}

/// Text ink for `bg` aimed at `target` contrast.
///
/// Prefer the palette foreground when it can reach the target, so a theme hue
/// survives. Otherwise step to black or white, whichever contrasts more, so a
/// light and a dark palette can hit the same ratio.
fn tone_for_contrast(bg: Hsla, preferred: Hsla, target: f32) -> Hsla {
    let preferred = preferred.alpha(1.0);
    let extreme = if prefers_light_ink(bg) {
        Hsla::white()
    } else {
        Hsla::black()
    };
    let ink = if theme_color::contrast_ratio(preferred, bg) + 0.05 >= target {
        preferred
    } else {
        extreme
    };
    blend_toward_contrast(bg, ink, target)
}

impl ChromeTokens {
    pub fn from_palette(p: &TerminalPalette, window_active: bool) -> Self {
        let content_bg = p.background.alpha(1.0);
        // Same target ratios on every palette. Light and dark then land within
        // 1.0 of each other because both are solved to the same number, not
        // because one is the other with lightness flipped.
        let surface_ink = if prefers_light_ink(content_bg) {
            Hsla::white()
        } else {
            Hsla::black()
        };
        let surface = blend_toward_contrast(content_bg, surface_ink, 1.18);
        let hover = blend_toward_contrast(content_bg, surface_ink, 1.32);
        let border = blend_toward_contrast(content_bg, surface_ink, 1.55);
        let fg = tone_for_contrast(content_bg, p.foreground, 7.0);
        let fg_muted = tone_for_contrast(surface, p.foreground, 4.6);
        let fg_disabled = tone_for_contrast(surface, p.foreground, 3.2);

        let mut tokens = Self {
            content_bg,
            surface,
            hover,
            border,
            fg,
            fg_muted,
            fg_disabled,
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

/// WCAG contrast ratio between two colors (assumes opaque presentation).
pub fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    theme_color::contrast_ratio(a, b)
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
            ThemeName::NocturneViolet,
            ThemeName::MonokaiPro,
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

    /// Light chrome is paired to dark by contrast ratio, not by flipping lightness.
    #[test]
    fn light_tokens_match_dark_contrast() {
        let pairs = [
            (ThemeName::Mocha, ThemeName::Latte),
            (ThemeName::GithubDark, ThemeName::GithubLight),
        ];
        for (dark_name, light_name) in pairs {
            let dark =
                ChromeTokens::from_palette(&palette_for_theme(dark_name, Appearance::Dark), true);
            let light =
                ChromeTokens::from_palette(&palette_for_theme(light_name, Appearance::Light), true);
            for (name, dark_fg, dark_bg, light_fg, light_bg) in [
                ("fg", dark.fg, dark.content_bg, light.fg, light.content_bg),
                (
                    "fg_muted",
                    dark.fg_muted,
                    dark.surface,
                    light.fg_muted,
                    light.surface,
                ),
                (
                    "fg_disabled",
                    dark.fg_disabled,
                    dark.surface,
                    light.fg_disabled,
                    light.surface,
                ),
            ] {
                let dark_ratio = contrast_ratio(dark_fg, dark_bg);
                let light_ratio = contrast_ratio(light_fg, light_bg);
                assert!(
                    (dark_ratio - light_ratio).abs() < 1.0,
                    "{dark_name:?}/{light_name:?} {name}: dark {dark_ratio:.2}:1 vs light {light_ratio:.2}:1"
                );
            }
        }
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
