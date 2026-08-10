//! Built-in terminal palettes (Catppuccin-inspired) plus a few extras, and an
//! `Auto` theme that follows the system light/dark Appearance (ADR-0002).

use gpui::{Hsla, Rgba, rgb};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    /// Follows the system Appearance: dark → Mocha, light → Latte.
    Auto,
    #[default]
    Mocha,
    Macchiato,
    Frappe,
    Latte,
    TokyoNight,
    Nord,
    GruvboxDark,
    SolarizedLight,
}

/// System light/dark appearance, used to resolve the `Auto` theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    Light,
    #[default]
    Dark,
}

/// Catppuccin-style ANSI palette used for terminal cell colors.
#[derive(Clone, Debug)]
pub struct TerminalPalette {
    pub name: ThemeName,
    pub background: Hsla,
    pub foreground: Hsla,
    pub bright_foreground: Hsla,
    pub cursor: Hsla,
    pub selection: Hsla,
    pub ansi: [Hsla; 16],
    pub dim: [Hsla; 8],
}

fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

/// Resolve a theme name to a concrete palette. `Auto` picks a dark/light pair
/// from the supplied system `appearance`.
pub fn palette_for_theme(name: ThemeName, appearance: Appearance) -> TerminalPalette {
    match name {
        ThemeName::Auto => match appearance {
            Appearance::Dark => mocha(),
            Appearance::Light => latte(),
        },
        ThemeName::Mocha => mocha(),
        ThemeName::Macchiato => macchiato(),
        ThemeName::Frappe => frappe(),
        ThemeName::Latte => latte(),
        ThemeName::TokyoNight => tokyo_night(),
        ThemeName::Nord => nord(),
        ThemeName::GruvboxDark => gruvbox_dark(),
        ThemeName::SolarizedLight => solarized_light(),
    }
}

fn mocha() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::Mocha,
        background: hex(0x1e1e2e),
        foreground: hex(0xcdd6f4),
        bright_foreground: hex(0xcdd6f4),
        cursor: hex(0xf5e0dc),
        selection: hex(0x585b70),
        ansi: [
            hex(0x45475a),
            hex(0xf38ba8),
            hex(0xa6e3a1),
            hex(0xf9e2af),
            hex(0x89b4fa),
            hex(0xf5c2e7),
            hex(0x94e2d5),
            hex(0xbac2de),
            hex(0x585b70),
            hex(0xf38ba8),
            hex(0xa6e3a1),
            hex(0xf9e2af),
            hex(0x89b4fa),
            hex(0xf5c2e7),
            hex(0x94e2d5),
            hex(0xa6adc8),
        ],
        dim: [
            hex(0x45475a),
            hex(0xf38ba8),
            hex(0xa6e3a1),
            hex(0xf9e2af),
            hex(0x89b4fa),
            hex(0xf5c2e7),
            hex(0x94e2d5),
            hex(0xbac2de),
        ],
    }
}

fn macchiato() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::Macchiato,
        background: hex(0x24273a),
        foreground: hex(0xcad3f5),
        bright_foreground: hex(0xcad3f5),
        cursor: hex(0xf4dbd6),
        selection: hex(0x5b6078),
        ansi: [
            hex(0x494d64),
            hex(0xed8796),
            hex(0xa6da95),
            hex(0xeed49f),
            hex(0x8aadf4),
            hex(0xf5bde6),
            hex(0x8bd5ca),
            hex(0xb8c0e0),
            hex(0x5b6078),
            hex(0xed8796),
            hex(0xa6da95),
            hex(0xeed49f),
            hex(0x8aadf4),
            hex(0xf5bde6),
            hex(0x8bd5ca),
            hex(0xa5adcb),
        ],
        dim: [
            hex(0x494d64),
            hex(0xed8796),
            hex(0xa6da95),
            hex(0xeed49f),
            hex(0x8aadf4),
            hex(0xf5bde6),
            hex(0x8bd5ca),
            hex(0xb8c0e0),
        ],
    }
}

fn frappe() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::Frappe,
        background: hex(0x303446),
        foreground: hex(0xc6d0f5),
        bright_foreground: hex(0xc6d0f5),
        cursor: hex(0xf2d5cf),
        selection: hex(0x626880),
        ansi: [
            hex(0x51576d),
            hex(0xe78284),
            hex(0xa6d189),
            hex(0xe5c890),
            hex(0x8caaee),
            hex(0xf4b8e4),
            hex(0x81c8be),
            hex(0xb5bfe2),
            hex(0x626880),
            hex(0xe78284),
            hex(0xa6d189),
            hex(0xe5c890),
            hex(0x8caaee),
            hex(0xf4b8e4),
            hex(0x81c8be),
            hex(0xa5adce),
        ],
        dim: [
            hex(0x51576d),
            hex(0xe78284),
            hex(0xa6d189),
            hex(0xe5c890),
            hex(0x8caaee),
            hex(0xf4b8e4),
            hex(0x81c8be),
            hex(0xb5bfe2),
        ],
    }
}

fn latte() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::Latte,
        background: hex(0xeff1f5),
        foreground: hex(0x4c4f69),
        bright_foreground: hex(0x4c4f69),
        cursor: hex(0xdc8a78),
        selection: hex(0xacb0be),
        ansi: [
            hex(0x5c5f77),
            hex(0xd20f39),
            hex(0x40a02b),
            hex(0xdf8e1d),
            hex(0x1e66f5),
            hex(0xea76cb),
            hex(0x179299),
            hex(0xacb0be),
            hex(0x6c6f85),
            hex(0xd20f39),
            hex(0x40a02b),
            hex(0xdf8e1d),
            hex(0x1e66f5),
            hex(0xea76cb),
            hex(0x179299),
            hex(0xbcc0cc),
        ],
        dim: [
            hex(0x5c5f77),
            hex(0xd20f39),
            hex(0x40a02b),
            hex(0xdf8e1d),
            hex(0x1e66f5),
            hex(0xea76cb),
            hex(0x179299),
            hex(0xacb0be),
        ],
    }
}

fn tokyo_night() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::TokyoNight,
        background: hex(0x1a1b26),
        foreground: hex(0xc0caf5),
        bright_foreground: hex(0xc0caf5),
        cursor: hex(0xc0caf5),
        selection: hex(0x33467c),
        ansi: [
            hex(0x15161e),
            hex(0xf7768e),
            hex(0x9ece6a),
            hex(0xe0af68),
            hex(0x7aa2f7),
            hex(0xbb9af7),
            hex(0x7dcfff),
            hex(0xa9b1d6),
            hex(0x414868),
            hex(0xf7768e),
            hex(0x9ece6a),
            hex(0xe0af68),
            hex(0x7aa2f7),
            hex(0xbb9af7),
            hex(0x7dcfff),
            hex(0xc0caf5),
        ],
        dim: [
            hex(0x15161e),
            hex(0xf7768e),
            hex(0x9ece6a),
            hex(0xe0af68),
            hex(0x7aa2f7),
            hex(0xbb9af7),
            hex(0x7dcfff),
            hex(0xa9b1d6),
        ],
    }
}

fn nord() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::Nord,
        background: hex(0x2e3440),
        foreground: hex(0xd8dee9),
        bright_foreground: hex(0xeceff4),
        cursor: hex(0xd8dee9),
        selection: hex(0x434c5e),
        ansi: [
            hex(0x3b4252),
            hex(0xbf616a),
            hex(0xa3be8c),
            hex(0xebcb8b),
            hex(0x81a1c1),
            hex(0xb48ead),
            hex(0x88c0d0),
            hex(0xe5e9f0),
            hex(0x4c566a),
            hex(0xbf616a),
            hex(0xa3be8c),
            hex(0xebcb8b),
            hex(0x81a1c1),
            hex(0xb48ead),
            hex(0x8fbcbb),
            hex(0xeceff4),
        ],
        dim: [
            hex(0x3b4252),
            hex(0xbf616a),
            hex(0xa3be8c),
            hex(0xebcb8b),
            hex(0x81a1c1),
            hex(0xb48ead),
            hex(0x88c0d0),
            hex(0xe5e9f0),
        ],
    }
}

fn gruvbox_dark() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::GruvboxDark,
        background: hex(0x282828),
        foreground: hex(0xebdbb2),
        bright_foreground: hex(0xfbf1c7),
        cursor: hex(0xebdbb2),
        selection: hex(0x504945),
        ansi: [
            hex(0x282828),
            hex(0xcc241d),
            hex(0x98971a),
            hex(0xd79921),
            hex(0x458588),
            hex(0xb16286),
            hex(0x689d6a),
            hex(0xa89984),
            hex(0x928374),
            hex(0xfb4934),
            hex(0xb8bb26),
            hex(0xfabd2f),
            hex(0x83a598),
            hex(0xd3869b),
            hex(0x8ec07c),
            hex(0xebdbb2),
        ],
        dim: [
            hex(0x282828),
            hex(0xcc241d),
            hex(0x98971a),
            hex(0xd79921),
            hex(0x458588),
            hex(0xb16286),
            hex(0x689d6a),
            hex(0xa89984),
        ],
    }
}

fn solarized_light() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::SolarizedLight,
        background: hex(0xfdf6e3),
        foreground: hex(0x657b83),
        bright_foreground: hex(0x586e75),
        cursor: hex(0x657b83),
        selection: hex(0xeee8d5),
        ansi: [
            hex(0x073642),
            hex(0xdc322f),
            hex(0x859900),
            hex(0xb58900),
            hex(0x268bd2),
            hex(0xd33682),
            hex(0x2aa198),
            hex(0xeee8d5),
            hex(0x002b36),
            hex(0xcb4b16),
            hex(0x586e75),
            hex(0x657b83),
            hex(0x839496),
            hex(0x6c71c4),
            hex(0x93a1a1),
            hex(0xfdf6e3),
        ],
        dim: [
            hex(0x073642),
            hex(0xdc322f),
            hex(0x859900),
            hex(0xb58900),
            hex(0x268bd2),
            hex(0xd33682),
            hex(0x2aa198),
            hex(0xeee8d5),
        ],
    }
}

/// Convert an 8-bit ANSI color index to HSLA (alacritty-compatible indices).
pub fn get_color_at_index(index: usize, palette: &TerminalPalette) -> Hsla {
    match index {
        0..=15 => palette.ansi[index],
        16..=231 => {
            let (r, g, b) = rgb_for_index(index as u8);
            rgba_color(
                if r == 0 { 0 } else { r * 40 + 55 },
                if g == 0 { 0 } else { g * 40 + 55 },
                if b == 0 { 0 } else { b * 40 + 55 },
            )
        }
        232..=255 => {
            let i = index as u8 - 232;
            let value = i * 10 + 8;
            rgba_color(value, value, value)
        }
        256 => palette.foreground,
        257 => palette.background,
        258 => palette.cursor,
        259..=266 => palette.dim[(index - 259).min(7)],
        267 => palette.bright_foreground,
        268 => palette.ansi[0],
        _ => Hsla::black(),
    }
}

fn rgb_for_index(i: u8) -> (u8, u8, u8) {
    debug_assert!((16..=231).contains(&i));
    let i = i - 16;
    let r = (i - (i % 36)) / 36;
    let g = ((i % 36) - (i % 6)) / 6;
    let b = (i % 36) % 6;
    (r, g, b)
}

fn rgba_color(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: 1.,
    }
    .into()
}
