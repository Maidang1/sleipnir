//! Thin settings for jiajia-term.
//!
//! JSON keys are intentionally aligned with Zed's `terminal` / font schema where possible.
//! Storage path: `~/.config/jiajia-term/settings.json` (not Zed's path).

use collections::HashMap;
use gpui::{App, FontFallbacks, FontFeatures, FontWeight, Global, Hsla, Pixels, Rgba, px, rgb};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use util::shell::Shell;

/// Re-export for terminal crate compatibility.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlternateScroll {
    #[default]
    On,
    Off,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalBlink {
    #[default]
    TerminalControlled,
    On,
    Off,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalBell {
    #[default]
    Off,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkingDirectory {
    #[default]
    CurrentProjectDirectory,
    FirstProjectDirectory,
    AlwaysHome,
    Always { directory: String },
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum TerminalLineHeight {
    Custom(f32),
}

impl Default for TerminalLineHeight {
    fn default() -> Self {
        Self::Custom(1.3)
    }
}

impl TerminalLineHeight {
    pub fn value(&self) -> f32 {
        match self {
            Self::Custom(v) => *v,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
    Hollow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct Toolbar {
    pub breadcrumbs: bool,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self { breadcrumbs: true }
    }
}

/// Runtime terminal settings (schema-compatible subset).
#[derive(Clone, Debug)]
pub struct TerminalSettings {
    pub shell: Shell,
    pub working_directory: WorkingDirectory,
    pub font_size: Option<Pixels>,
    pub font_family: Option<String>,
    pub font_fallbacks: Option<FontFallbacks>,
    pub font_features: Option<FontFeatures>,
    pub font_weight: Option<FontWeight>,
    pub line_height: TerminalLineHeight,
    pub env: HashMap<String, String>,
    pub cursor_shape: CursorShape,
    pub blinking: TerminalBlink,
    pub alternate_scroll: AlternateScroll,
    pub option_as_meta: bool,
    pub copy_on_select: bool,
    pub keep_selection_on_copy: bool,
    pub open_links_in_mouse_mode: bool,
    pub max_scroll_history_lines: Option<usize>,
    pub scroll_multiplier: f32,
    pub toolbar: Toolbar,
    pub minimum_contrast: f32,
    pub path_hyperlink_regexes: Vec<String>,
    pub path_hyperlink_timeout_ms: u64,
    pub bell: TerminalBell,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: Shell::System,
            working_directory: WorkingDirectory::AlwaysHome,
            font_size: Some(px(14.)),
            font_family: Some("Menlo".into()),
            font_fallbacks: None,
            font_features: Some(FontFeatures::disable_ligatures()),
            font_weight: None,
            line_height: TerminalLineHeight::default(),
            env: HashMap::default(),
            cursor_shape: CursorShape::Block,
            blinking: TerminalBlink::TerminalControlled,
            alternate_scroll: AlternateScroll::On,
            option_as_meta: true,
            copy_on_select: false,
            keep_selection_on_copy: true,
            open_links_in_mouse_mode: true,
            max_scroll_history_lines: Some(10_000),
            scroll_multiplier: 1.0,
            toolbar: Toolbar::default(),
            minimum_contrast: 0.0,
            path_hyperlink_regexes: Vec::new(),
            path_hyperlink_timeout_ms: 50,
            bell: TerminalBell::Off,
        }
    }
}

struct TerminalSettingsGlobal(TerminalSettings);

impl Global for TerminalSettingsGlobal {}

impl TerminalSettings {
    pub fn get_global(cx: &App) -> &TerminalSettings {
        &cx.global::<TerminalSettingsGlobal>().0
    }

    pub fn init(cx: &mut App) {
        let settings = load_or_default();
        cx.set_global(TerminalSettingsGlobal(settings));
    }

    pub fn reload(cx: &mut App) {
        let settings = load_or_default();
        cx.set_global(TerminalSettingsGlobal(settings));
    }
}

/// Catppuccin-ish dark ANSI palette used for terminal cell colors.
#[derive(Clone, Debug)]
pub struct TerminalPalette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub bright_foreground: Hsla,
    pub cursor: Hsla,
    pub ansi: [Hsla; 16],
    pub dim: [Hsla; 8],
}

impl Default for TerminalPalette {
    fn default() -> Self {
        // Catppuccin Mocha-inspired
        fn hex(c: u32) -> Hsla {
            rgb(c).into()
        }
        Self {
            background: hex(0x1e1e2e),
            foreground: hex(0xcdd6f4),
            bright_foreground: hex(0xcdd6f4),
            cursor: hex(0xf5e0dc),
            ansi: [
                hex(0x45475a), // black
                hex(0xf38ba8), // red
                hex(0xa6e3a1), // green
                hex(0xf9e2af), // yellow
                hex(0x89b4fa), // blue
                hex(0xf5c2e7), // magenta
                hex(0x94e2d5), // cyan
                hex(0xbac2de), // white
                hex(0x585b70), // bright black
                hex(0xf38ba8),
                hex(0xa6e3a1),
                hex(0xf9e2af),
                hex(0x89b4fa),
                hex(0xf5c2e7),
                hex(0x94e2d5),
                hex(0xa6adc8), // bright white
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
}

struct TerminalPaletteGlobal(Arc<TerminalPalette>);
impl Global for TerminalPaletteGlobal {}

impl TerminalPalette {
    pub fn get_global(cx: &App) -> Arc<TerminalPalette> {
        cx.global::<TerminalPaletteGlobal>().0.clone()
    }

    pub fn init(cx: &mut App) {
        cx.set_global(TerminalPaletteGlobal(Arc::new(TerminalPalette::default())));
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

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
struct SettingsFile {
    /// Font size in pixels (Zed-compatible top-level convenience).
    #[serde(default)]
    terminal: TerminalSettingsFile,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
struct TerminalSettingsFile {
    font_size: Option<f32>,
    font_family: Option<String>,
    line_height: Option<f32>,
    option_as_meta: Option<bool>,
    copy_on_select: Option<bool>,
    max_scroll_history_lines: Option<usize>,
    scroll_multiplier: Option<f32>,
    minimum_contrast: Option<f32>,
    cursor_shape: Option<CursorShape>,
}

fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/jiajia-term/settings.json")
}

fn load_or_default() -> TerminalSettings {
    let mut settings = TerminalSettings::default();
    let path = config_path();
    if let Ok(bytes) = std::fs::read(&path) {
        match serde_json::from_slice::<SettingsFile>(&bytes) {
            Ok(file) => {
                if let Some(size) = file.terminal.font_size {
                    settings.font_size = Some(px(size));
                }
                if let Some(family) = file.terminal.font_family {
                    settings.font_family = Some(family);
                }
                if let Some(lh) = file.terminal.line_height {
                    settings.line_height = TerminalLineHeight::Custom(lh);
                }
                if let Some(v) = file.terminal.option_as_meta {
                    settings.option_as_meta = v;
                }
                if let Some(v) = file.terminal.copy_on_select {
                    settings.copy_on_select = v;
                }
                if let Some(v) = file.terminal.max_scroll_history_lines {
                    settings.max_scroll_history_lines = Some(v);
                }
                if let Some(v) = file.terminal.scroll_multiplier {
                    settings.scroll_multiplier = v;
                }
                if let Some(v) = file.terminal.minimum_contrast {
                    settings.minimum_contrast = v;
                }
                if let Some(v) = file.terminal.cursor_shape {
                    settings.cursor_shape = v;
                }
            }
            Err(err) => log::warn!("failed to parse {}: {err}", path.display()),
        }
    }
    settings
}

/// Ensure config directory exists and write a default file if missing.
pub fn ensure_default_config_file() -> anyhow::Result<()> {
    let path = config_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let default = SettingsFile {
        terminal: TerminalSettingsFile {
            font_size: Some(14.0),
            font_family: Some("Menlo".into()),
            line_height: Some(1.3),
            ..Default::default()
        },
    };
    let json = serde_json::to_string_pretty(&default)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn init(cx: &mut App) {
    let _ = ensure_default_config_file();
    TerminalSettings::init(cx);
    TerminalPalette::init(cx);
}
