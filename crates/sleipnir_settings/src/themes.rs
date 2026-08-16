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
    /// Primer-based GitHub Dark (canvas `#0d1117`).
    GithubDark,
    /// Primer-based GitHub Light (canvas `#ffffff`).
    GithubLight,
    /// Dracula (canvas `#282a36`).
    Dracula,
    /// Atom One Dark (canvas `#282c34`).
    OneDark,
    /// A user-defined palette from `custom_theme` (not cycled).
    Custom,
}

impl ThemeName {
    /// Stable order for theme pickers and cycle.
    pub const ALL: &'static [ThemeName] = &[
        ThemeName::Auto,
        ThemeName::Mocha,
        ThemeName::Macchiato,
        ThemeName::Frappe,
        ThemeName::Latte,
        ThemeName::TokyoNight,
        ThemeName::Nord,
        ThemeName::GruvboxDark,
        ThemeName::SolarizedLight,
        ThemeName::GithubDark,
        ThemeName::GithubLight,
        ThemeName::Dracula,
        ThemeName::OneDark,
    ];

    /// Snake_case settings key (`"mocha"`, `"tokyo_night"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeName::Auto => "auto",
            ThemeName::Mocha => "mocha",
            ThemeName::Macchiato => "macchiato",
            ThemeName::Frappe => "frappe",
            ThemeName::Latte => "latte",
            ThemeName::TokyoNight => "tokyo_night",
            ThemeName::Nord => "nord",
            ThemeName::GruvboxDark => "gruvbox_dark",
            ThemeName::SolarizedLight => "solarized_light",
            ThemeName::GithubDark => "github_dark",
            ThemeName::GithubLight => "github_light",
            ThemeName::Dracula => "dracula",
            ThemeName::OneDark => "one_dark",
            ThemeName::Custom => "custom",
        }
    }

    /// Human-readable label for the settings UI.
    pub fn display_name(self) -> &'static str {
        match self {
            ThemeName::Auto => "Auto (System)",
            ThemeName::Mocha => "Catppuccin Mocha",
            ThemeName::Macchiato => "Catppuccin Macchiato",
            ThemeName::Frappe => "Catppuccin Frappé",
            ThemeName::Latte => "Catppuccin Latte",
            ThemeName::TokyoNight => "Tokyo Night",
            ThemeName::Nord => "Nord",
            ThemeName::GruvboxDark => "Gruvbox Dark",
            ThemeName::SolarizedLight => "Solarized Light",
            ThemeName::GithubDark => "GitHub Dark",
            ThemeName::GithubLight => "GitHub Light",
            ThemeName::Dracula => "Dracula",
            ThemeName::OneDark => "One Dark",
            ThemeName::Custom => "Custom",
        }
    }

    /// Next theme in [`Self::ALL`] (wraps around). Used by cycle shortcuts.
    pub fn next(self) -> ThemeName {
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    /// Parse a settings key back into a built-in name (`"mocha"`, `"auto"`, …).
    pub fn from_str(s: &str) -> Option<ThemeName> {
        Self::ALL.iter().copied().find(|t| t.as_str() == s)
    }
}

/// A theme reference: a built-in name or a user/imported theme name.
///
/// Serializes as a plain string (the built-in key, or a free-form name for
/// themes from the user `themes.json` catalog), so `"theme": "mocha"` and
/// `"theme": "kanagawa"` both work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeSetting {
    Builtin(ThemeName),
    Custom(String),
}

impl ThemeSetting {
    /// Settings key / JSON value.
    pub fn as_str(&self) -> String {
        match self {
            ThemeSetting::Builtin(name) => name.as_str().to_string(),
            ThemeSetting::Custom(name) => name.clone(),
        }
    }

    /// Human-readable label.
    pub fn display_name(&self) -> String {
        match self {
            ThemeSetting::Builtin(name) => name.display_name().to_string(),
            ThemeSetting::Custom(name) => name.clone(),
        }
    }

    /// Next theme for cycle shortcuts: steps through built-ins; a custom theme
    /// wraps back to `Auto`.
    pub fn next(&self) -> ThemeSetting {
        match self {
            ThemeSetting::Custom(_) => ThemeSetting::Builtin(ThemeName::Auto),
            ThemeSetting::Builtin(name) => ThemeSetting::Builtin(name.next()),
        }
    }
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
        ThemeName::GithubDark => github_dark(),
        ThemeName::GithubLight => github_light(),
        ThemeName::Dracula => dracula(),
        ThemeName::OneDark => one_dark(),
        // Custom palettes are resolved by `resolve_palette` before this; the
        // fallback keeps the match total and gives a sane palette if misused.
        ThemeName::Custom => mocha(),
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

/// Primer GitHub Dark — matches github.com dark default / github-vscode-theme.
fn github_dark() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::GithubDark,
        background: hex(0x0d1117),
        foreground: hex(0xe6edf3),
        bright_foreground: hex(0xffffff),
        cursor: hex(0xe6edf3),
        selection: hex(0x264f78),
        ansi: [
            hex(0x484f58), // black
            hex(0xff7b72), // red
            hex(0x3fb950), // green
            hex(0xd29922), // yellow
            hex(0x58a6ff), // blue
            hex(0xbc8cff), // magenta
            hex(0x39c5cf), // cyan
            hex(0xb1bac4), // white
            hex(0x6e7681), // bright black
            hex(0xffa198), // bright red
            hex(0x56d364), // bright green
            hex(0xe3b341), // bright yellow
            hex(0x79c0ff), // bright blue
            hex(0xd2a8ff), // bright magenta
            hex(0x56d4dd), // bright cyan
            hex(0xffffff), // bright white
        ],
        dim: [
            hex(0x484f58),
            hex(0xff7b72),
            hex(0x3fb950),
            hex(0xd29922),
            hex(0x58a6ff),
            hex(0xbc8cff),
            hex(0x39c5cf),
            hex(0xb1bac4),
        ],
    }
}

/// Primer GitHub Light — matches github.com light default / github-vscode-theme.
fn github_light() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::GithubLight,
        background: hex(0xffffff),
        foreground: hex(0x1f2328),
        bright_foreground: hex(0x1f2328),
        cursor: hex(0x1f2328),
        selection: hex(0xb6e3ff),
        ansi: [
            hex(0x24292f), // black
            hex(0xcf222e), // red
            hex(0x116329), // green
            hex(0x4d2d00), // yellow
            hex(0x0969da), // blue
            hex(0x8250df), // magenta
            hex(0x1b7c83), // cyan
            hex(0x6e7781), // white
            hex(0x57606a), // bright black
            hex(0xa40e26), // bright red
            hex(0x1a7f37), // bright green
            hex(0x633c01), // bright yellow
            hex(0x218bff), // bright blue
            hex(0xa475f9), // bright magenta
            hex(0x3192aa), // bright cyan
            hex(0x8c959f), // bright white
        ],
        dim: [
            hex(0x24292f),
            hex(0xcf222e),
            hex(0x116329),
            hex(0x4d2d00),
            hex(0x0969da),
            hex(0x8250df),
            hex(0x1b7c83),
            hex(0x6e7781),
        ],
    }
}

/// Dracula — the popular dark theme (canvas `#282a36`).
fn dracula() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::Dracula,
        background: hex(0x282a36),
        foreground: hex(0xf8f8f2),
        bright_foreground: hex(0xffffff),
        cursor: hex(0xf8f8f2),
        selection: hex(0x44475a),
        ansi: [
            hex(0x21222c), // black
            hex(0xff5555), // red
            hex(0x50fa7b), // green
            hex(0xf1fa8c), // yellow
            hex(0xbd93f9), // blue
            hex(0xff79c6), // magenta
            hex(0x8be9fd), // cyan
            hex(0xf8f8f2), // white
            hex(0x6272a4), // bright black
            hex(0xff6e6e), // bright red
            hex(0x69ff94), // bright green
            hex(0xffffa5), // bright yellow
            hex(0xd6acff), // bright blue
            hex(0xff92df), // bright magenta
            hex(0xa4ffff), // bright cyan
            hex(0xffffff), // bright white
        ],
        dim: [
            hex(0x21222c),
            hex(0xff5555),
            hex(0x50fa7b),
            hex(0xf1fa8c),
            hex(0xbd93f9),
            hex(0xff79c6),
            hex(0x8be9fd),
            hex(0xf8f8f2),
        ],
    }
}

/// Atom One Dark (canvas `#282c34`).
fn one_dark() -> TerminalPalette {
    TerminalPalette {
        name: ThemeName::OneDark,
        background: hex(0x282c34),
        foreground: hex(0xabb2bf),
        bright_foreground: hex(0xffffff),
        cursor: hex(0x528bff),
        selection: hex(0x3e4451),
        ansi: [
            hex(0x282c34), // black
            hex(0xe06c75), // red
            hex(0x98c379), // green
            hex(0xe5c07b), // yellow
            hex(0x61afef), // blue
            hex(0xc678dd), // magenta
            hex(0x56b6c2), // cyan
            hex(0xabb2bf), // white
            hex(0x5c6370), // bright black
            hex(0xe06c75), // bright red
            hex(0x98c379), // bright green
            hex(0xe5c07b), // bright yellow
            hex(0x61afef), // bright blue
            hex(0xc678dd), // bright magenta
            hex(0x56b6c2), // bright cyan
            hex(0xffffff), // bright white
        ],
        dim: [
            hex(0x282c34),
            hex(0xe06c75),
            hex(0x98c379),
            hex(0xe5c07b),
            hex(0x61afef),
            hex(0xc678dd),
            hex(0x56b6c2),
            hex(0xabb2bf),
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

/// Parse a hex color (`#rrggbb`, `#rgb`, or bare `rrggbb`) into opaque HSLA.
pub fn parse_hex_color(s: &str) -> Option<Hsla> {
    let s = s.trim().trim_start_matches('#');
    let (r, g, b) = match s.len() {
        3 => {
            let chars: Vec<char> = s.chars().collect();
            let d = |c: char| c.to_digit(16).map(|v| (v * 17) as u8);
            (d(chars[0])?, d(chars[1])?, d(chars[2])?)
        }
        6 => (
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
        ),
        _ => return None,
    };
    Some(rgba_color(r, g, b))
}

/// A user-supplied palette (hex colors) from `custom_theme` in settings.json.
///
/// Missing colors fall back to Mocha so a partial definition still works.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CustomPalette {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bright_foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    /// 16 ANSI colors: normal 0–7, then bright 8–15.
    #[serde(default)]
    pub ansi: Vec<String>,
}

impl CustomPalette {
    /// Resolve into a full palette, borrowing Mocha for any missing color.
    pub fn to_palette(&self) -> TerminalPalette {
        let base = mocha();
        let background = self
            .background
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(base.background);
        let foreground = self
            .foreground
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(base.foreground);
        let bright_foreground = self
            .bright_foreground
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(foreground);
        let cursor = self
            .cursor
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(foreground);
        let selection = self
            .selection
            .as_deref()
            .and_then(parse_hex_color)
            .unwrap_or(base.selection);

        let mut ansi = base.ansi;
        for (i, hex) in self.ansi.iter().enumerate().take(16) {
            if let Some(color) = parse_hex_color(hex) {
                ansi[i] = color;
            }
        }
        let mut dim = base.dim;
        dim.copy_from_slice(&ansi[..8]);

        TerminalPalette {
            name: ThemeName::Custom,
            background,
            foreground,
            bright_foreground,
            cursor,
            selection,
            ansi,
            dim,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_hash_rgb_and_bare() {
        assert_eq!(parse_hex_color("#ff0000").map(|c| c.to_rgb().r), Some(1.0));
        assert_eq!(parse_hex_color("00ff00").map(|c| c.to_rgb().g), Some(1.0));
        // #abc expands each nibble.
        assert_eq!(parse_hex_color("#abc").map(|c| c.to_rgb().b), Some(0.8));
        assert!(parse_hex_color("#xyz").is_none());
        assert!(parse_hex_color("#12345").is_none());
    }

    #[test]
    fn custom_palette_fills_missing_from_mocha_and_parses_ansi() {
        let custom = CustomPalette {
            background: Some("#0d1117".into()),
            foreground: Some("e6edf3".into()),
            bright_foreground: None,
            cursor: None,
            selection: None,
            ansi: vec!["ff0000".into(), "00ff00".into(), "0000ff".into()],
        };
        let p = custom.to_palette();
        assert_eq!(p.name, ThemeName::Custom);
        assert_eq!(p.background.to_rgb(), parse_hex_color("#0d1117").unwrap().to_rgb());
        assert_eq!(p.foreground.to_rgb(), parse_hex_color("#e6edf3").unwrap().to_rgb());
        // Red, green, blue overwrote ansi[0..3]; the rest inherit Mocha.
        assert_eq!(p.ansi[0].to_rgb(), parse_hex_color("#ff0000").unwrap().to_rgb());
        assert_eq!(p.ansi[1].to_rgb(), parse_hex_color("#00ff00").unwrap().to_rgb());
        assert_eq!(p.ansi[2].to_rgb(), parse_hex_color("#0000ff").unwrap().to_rgb());
        assert_eq!(p.ansi[3].to_rgb(), mocha().ansi[3].to_rgb());
        assert_eq!(p.dim[0].to_rgb(), p.ansi[0].to_rgb());
    }
}
