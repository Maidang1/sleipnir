//! Built-in terminal palettes (Catppuccin-inspired).

use gpui::{Hsla, Rgba, rgb};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemeName {
    #[default]
    Mocha,
    Macchiato,
    Frappe,
    Latte,
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

pub fn palette_for_theme(name: ThemeName) -> TerminalPalette {
    match name {
        ThemeName::Mocha => mocha(),
        ThemeName::Macchiato => macchiato(),
        ThemeName::Frappe => frappe(),
        ThemeName::Latte => latte(),
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
