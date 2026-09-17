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
    /// Nocturne Violet (canvas `#151020`).
    NocturneViolet,
    /// Monokai Pro (canvas `#2d2a2e`).
    MonokaiPro,
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
        ThemeName::NocturneViolet,
        ThemeName::MonokaiPro,
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
            ThemeName::NocturneViolet => "nocturne_violet",
            ThemeName::MonokaiPro => "monokai_pro",
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
            ThemeName::NocturneViolet => "Nocturne Violet",
            ThemeName::MonokaiPro => "Monokai Pro",
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
    // Auto resolves against the system appearance. Custom palettes are
    // resolved by `resolve_palette` before this; the Mocha fallback keeps the
    // function total and gives a sane palette if misused.
    let resolved = match name {
        ThemeName::Auto => match appearance {
            Appearance::Dark => ThemeName::Mocha,
            Appearance::Light => ThemeName::Latte,
        },
        ThemeName::Custom => ThemeName::Mocha,
        concrete => concrete,
    };
    spec_for(resolved).palette()
}

/// One built-in theme as plain data: hex colors for the fixed palette slots.
/// `ansi` holds normal 0–7 then bright 8–15; `dim` is the subdued set.
struct ThemeSpec {
    name: ThemeName,
    background: u32,
    foreground: u32,
    bright_foreground: u32,
    cursor: u32,
    selection: u32,
    ansi: [u32; 16],
    dim: [u32; 8],
}

impl ThemeSpec {
    fn palette(&self) -> TerminalPalette {
        TerminalPalette {
            name: self.name,
            background: hex(self.background),
            foreground: hex(self.foreground),
            bright_foreground: hex(self.bright_foreground),
            cursor: hex(self.cursor),
            selection: hex(self.selection),
            ansi: self.ansi.map(hex),
            dim: self.dim.map(hex),
        }
    }
}

// Keep Mocha first: it is the fallback for unknown names and the base palette
// that partial `CustomPalette` definitions borrow from.
const THEME_SPECS: &[ThemeSpec] = &[
    ThemeSpec {
        name: ThemeName::Mocha,
        background: 0x1e1e2e,
        foreground: 0xcdd6f4,
        bright_foreground: 0xcdd6f4,
        cursor: 0xf5e0dc,
        selection: 0x585b70,
        ansi: [
            0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
            0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
        ],
        dim: [
            0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
        ],
    },
    ThemeSpec {
        name: ThemeName::Macchiato,
        background: 0x24273a,
        foreground: 0xcad3f5,
        bright_foreground: 0xcad3f5,
        cursor: 0xf4dbd6,
        selection: 0x5b6078,
        ansi: [
            0x494d64, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xb8c0e0,
            0x5b6078, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xa5adcb,
        ],
        dim: [
            0x494d64, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xb8c0e0,
        ],
    },
    ThemeSpec {
        name: ThemeName::Frappe,
        background: 0x303446,
        foreground: 0xc6d0f5,
        bright_foreground: 0xc6d0f5,
        cursor: 0xf2d5cf,
        selection: 0x626880,
        ansi: [
            0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2,
            0x626880, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xa5adce,
        ],
        dim: [
            0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2,
        ],
    },
    ThemeSpec {
        name: ThemeName::Latte,
        background: 0xeff1f5,
        foreground: 0x4c4f69,
        bright_foreground: 0x4c4f69,
        cursor: 0xdc8a78,
        selection: 0xacb0be,
        ansi: [
            0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
            0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
        ],
        dim: [
            0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
        ],
    },
    ThemeSpec {
        name: ThemeName::TokyoNight,
        background: 0x1a1b26,
        foreground: 0xc0caf5,
        bright_foreground: 0xc0caf5,
        cursor: 0xc0caf5,
        selection: 0x33467c,
        ansi: [
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ],
        dim: [
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
        ],
    },
    ThemeSpec {
        name: ThemeName::Nord,
        background: 0x2e3440,
        foreground: 0xd8dee9,
        bright_foreground: 0xeceff4,
        cursor: 0xd8dee9,
        selection: 0x434c5e,
        ansi: [
            0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
            0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
        ],
        dim: [
            0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
        ],
    },
    ThemeSpec {
        name: ThemeName::GruvboxDark,
        background: 0x282828,
        foreground: 0xebdbb2,
        bright_foreground: 0xfbf1c7,
        cursor: 0xebdbb2,
        selection: 0x504945,
        ansi: [
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
            0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ],
        dim: [
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
        ],
    },
    ThemeSpec {
        name: ThemeName::SolarizedLight,
        background: 0xfdf6e3,
        foreground: 0x657b83,
        bright_foreground: 0x586e75,
        cursor: 0x657b83,
        selection: 0xeee8d5,
        ansi: [
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
            0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
        ],
        dim: [
            0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
        ],
    },
    // Primer GitHub Dark — matches github.com dark default / github-vscode-theme.
    ThemeSpec {
        name: ThemeName::GithubDark,
        background: 0x0d1117,
        foreground: 0xe6edf3,
        bright_foreground: 0xffffff,
        cursor: 0xe6edf3,
        selection: 0x264f78,
        ansi: [
            0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4,
            0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
        ],
        dim: [
            0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4,
        ],
    },
    // Primer GitHub Light — matches github.com light default / github-vscode-theme.
    ThemeSpec {
        name: ThemeName::GithubLight,
        background: 0xffffff,
        foreground: 0x1f2328,
        bright_foreground: 0x1f2328,
        cursor: 0x1f2328,
        selection: 0xb6e3ff,
        ansi: [
            0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
            0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
        ],
        dim: [
            0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
        ],
    },
    // Dracula — the popular dark theme (canvas `#282a36`).
    ThemeSpec {
        name: ThemeName::Dracula,
        background: 0x282a36,
        foreground: 0xf8f8f2,
        bright_foreground: 0xffffff,
        cursor: 0xf8f8f2,
        selection: 0x44475a,
        ansi: [
            0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
            0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
        ],
        dim: [
            0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
        ],
    },
    // Atom One Dark (canvas `#282c34`).
    ThemeSpec {
        name: ThemeName::OneDark,
        background: 0x282c34,
        foreground: 0xabb2bf,
        bright_foreground: 0xffffff,
        cursor: 0x528bff,
        selection: 0x3e4451,
        ansi: [
            0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
            0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
        ],
        dim: [
            0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
        ],
    },
    // Nocturne Violet — a purple-forward dark theme (canvas `#151020`).
    ThemeSpec {
        name: ThemeName::NocturneViolet,
        background: 0x151020,
        foreground: 0xd9d2e8,
        bright_foreground: 0xf4f0fb,
        cursor: 0xb98df7,
        selection: 0x3b2d5e,
        ansi: [
            0x221832, 0xe06c92, 0xa0cfa8, 0xe0b76e, 0x93a4f5, 0xcf9bf0, 0x8ad4cc, 0xcdc4e0,
            0x75649a, 0xf58bab, 0xb2e3ba, 0xf0ce8e, 0xaab8f9, 0xdcb6f7, 0xa2e8df, 0xf4f0fb,
        ],
        dim: [
            0x221832, 0xe06c92, 0xa0cfa8, 0xe0b76e, 0x93a4f5, 0xcf9bf0, 0x8ad4cc, 0xcdc4e0,
        ],
    },
    // Monokai Pro — the official terminal palette from the VS Code extension
    // (canvas `#2d2a2e`; note the signature orange occupies the blue slot).
    ThemeSpec {
        name: ThemeName::MonokaiPro,
        background: 0x2d2a2e,
        foreground: 0xfcfcfa,
        bright_foreground: 0xfcfcfa,
        cursor: 0xfcfcfa,
        selection: 0x4c494c,
        ansi: [
            0x403e41, 0xff6188, 0xa9dc76, 0xffd866, 0xfc9867, 0xab9df2, 0x78dce8, 0xfcfcfa,
            0x727072, 0xff6188, 0xa9dc76, 0xffd866, 0xfc9867, 0xab9df2, 0x78dce8, 0xfcfcfa,
        ],
        dim: [
            0x403e41, 0xff6188, 0xa9dc76, 0xffd866, 0xfc9867, 0xab9df2, 0x78dce8, 0xfcfcfa,
        ],
    },
];

fn spec_for(name: ThemeName) -> &'static ThemeSpec {
    THEME_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .unwrap_or(&THEME_SPECS[0])
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
        let base = spec_for(ThemeName::Mocha).palette();
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
    fn theme_spec_table_covers_every_builtin() {
        for name in ThemeName::ALL {
            if matches!(name, ThemeName::Auto) {
                continue;
            }
            assert!(
                THEME_SPECS.iter().any(|spec| spec.name == *name),
                "missing ThemeSpec for {name:?}"
            );
        }
        // Mocha stays first: spec_for falls back to index 0.
        assert_eq!(THEME_SPECS[0].name, ThemeName::Mocha);
    }

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
        assert_eq!(
            p.background.to_rgb(),
            parse_hex_color("#0d1117").unwrap().to_rgb()
        );
        assert_eq!(
            p.foreground.to_rgb(),
            parse_hex_color("#e6edf3").unwrap().to_rgb()
        );
        // Red, green, blue overwrote ansi[0..3]; the rest inherit Mocha.
        assert_eq!(
            p.ansi[0].to_rgb(),
            parse_hex_color("#ff0000").unwrap().to_rgb()
        );
        assert_eq!(
            p.ansi[1].to_rgb(),
            parse_hex_color("#00ff00").unwrap().to_rgb()
        );
        assert_eq!(
            p.ansi[2].to_rgb(),
            parse_hex_color("#0000ff").unwrap().to_rgb()
        );
        assert_eq!(
            p.ansi[3].to_rgb(),
            spec_for(ThemeName::Mocha).palette().ansi[3].to_rgb()
        );
        assert_eq!(p.dim[0].to_rgb(), p.ansi[0].to_rgb());
    }
}
