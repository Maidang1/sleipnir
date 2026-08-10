//! Thin settings for harbor.
//!
//! JSON keys align with Zed's `terminal` segment where practical.
//! Path: `~/.config/harbor/settings.json` (not Zed's config path).

mod themes;

pub use themes::{
    Appearance, TerminalPalette, ThemeName, get_color_at_index, palette_for_theme,
};

use collections::HashMap;
use gpui::{App, FontFallbacks, FontFeatures, FontWeight, Global, Pixels, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use util::shell::Shell;

// ── enums (schema-compatible) ───────────────────────────────────────────────

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

/// Line height: bare number (Zed also accepts objects; we accept `f32` or `{"custom": n}`).
#[derive(Copy, Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum TerminalLineHeight {
    Custom(f32),
    Named { custom: f32 },
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
            Self::Named { custom } => *custom,
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

// ── runtime settings ────────────────────────────────────────────────────────

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
    /// Active color theme name (harbor extension; also top-level `theme` key).
    pub theme: ThemeName,
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
            scroll_multiplier: 3.0,
            toolbar: Toolbar::default(),
            minimum_contrast: 45.0,
            path_hyperlink_regexes: Vec::new(),
            path_hyperlink_timeout_ms: 50,
            bell: TerminalBell::Off,
            theme: ThemeName::Mocha,
        }
    }
}

struct TerminalSettingsGlobal(TerminalSettings);
impl Global for TerminalSettingsGlobal {}

struct TerminalPaletteGlobal(Arc<TerminalPalette>);
impl Global for TerminalPaletteGlobal {}

/// Last-known system appearance, used to resolve the `Auto` theme.
struct AppearanceGlobal(Appearance);
impl Global for AppearanceGlobal {}

impl TerminalSettings {
    pub fn get_global(cx: &App) -> &TerminalSettings {
        &cx.global::<TerminalSettingsGlobal>().0
    }

    pub fn init(cx: &mut App) {
        apply_loaded(load_or_default(), cx);
    }

    /// Re-read `settings.json` and refresh globals.
    pub fn reload(cx: &mut App) {
        let settings = load_or_default();
        log::info!(
            "reloaded settings: theme={:?} font={:?} size={:?}",
            settings.theme,
            settings.font_family,
            settings.font_size
        );
        apply_loaded(settings, cx);
    }

    /// Apply an in-memory settings snapshot (e.g. session theme cycle).
    pub fn apply(settings: TerminalSettings, cx: &mut App) {
        apply_loaded(settings, cx);
    }

    /// Record a new system appearance and re-resolve the palette (for `Auto`).
    pub fn set_appearance(appearance: Appearance, cx: &mut App) {
        cx.set_global(AppearanceGlobal(appearance));
        let settings = Self::get_global(cx).clone();
        let palette = Arc::new(palette_for_theme(settings.theme, appearance));
        cx.set_global(TerminalPaletteGlobal(palette));
    }
}

impl TerminalPalette {
    pub fn get_global(cx: &App) -> Arc<TerminalPalette> {
        cx.global::<TerminalPaletteGlobal>().0.clone()
    }
}

fn current_appearance(cx: &App) -> Appearance {
    cx.try_global::<AppearanceGlobal>()
        .map(|g| g.0)
        .unwrap_or_default()
}

fn apply_loaded(settings: TerminalSettings, cx: &mut App) {
    let appearance = current_appearance(cx);
    let palette = Arc::new(palette_for_theme(settings.theme, appearance));
    cx.set_global(TerminalPaletteGlobal(palette));
    cx.set_global(TerminalSettingsGlobal(settings));
}

// ── file schema ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
struct SettingsFile {
    /// Theme name: mocha | macchiato | frappe | latte
    #[serde(default)]
    theme: Option<ThemeName>,
    #[serde(default)]
    terminal: TerminalSettingsFile,
}

/// Zed-compatible terminal block (subset).
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
struct TerminalSettingsFile {
    font_size: Option<f32>,
    font_family: Option<String>,
    font_fallbacks: Option<Vec<String>>,
    /// Font weight 100–900 (optional).
    font_weight: Option<f32>,
    line_height: Option<TerminalLineHeight>,
    option_as_meta: Option<bool>,
    copy_on_select: Option<bool>,
    keep_selection_on_copy: Option<bool>,
    max_scroll_history_lines: Option<usize>,
    scroll_multiplier: Option<f32>,
    minimum_contrast: Option<f32>,
    cursor_shape: Option<CursorShape>,
    blinking: Option<TerminalBlink>,
    alternate_scroll: Option<AlternateScroll>,
    bell: Option<TerminalBell>,
    env: Option<HashMap<String, String>>,
    /// Optional nested theme override.
    theme: Option<ThemeName>,
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/harbor/settings.json")
}

fn load_or_default() -> TerminalSettings {
    let mut settings = TerminalSettings::default();
    let path = config_path();
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<SettingsFile>(&bytes) {
            Ok(file) => merge_file(&mut settings, file),
            Err(err) => log::warn!("failed to parse {}: {err}", path.display()),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("failed to read {}: {err}", path.display()),
    }
    settings
}

fn merge_file(settings: &mut TerminalSettings, file: SettingsFile) {
    if let Some(theme) = file.theme.or(file.terminal.theme) {
        settings.theme = theme;
    }
    let t = file.terminal;
    if let Some(size) = t.font_size {
        settings.font_size = Some(px(size));
    }
    if let Some(family) = t.font_family {
        settings.font_family = Some(family);
    }
    if let Some(fallbacks) = t.font_fallbacks {
        settings.font_fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
    }
    if let Some(w) = t.font_weight {
        settings.font_weight = Some(FontWeight(w));
    }
    if let Some(lh) = t.line_height {
        settings.line_height = lh;
    }
    if let Some(v) = t.option_as_meta {
        settings.option_as_meta = v;
    }
    if let Some(v) = t.copy_on_select {
        settings.copy_on_select = v;
    }
    if let Some(v) = t.keep_selection_on_copy {
        settings.keep_selection_on_copy = v;
    }
    if let Some(v) = t.max_scroll_history_lines {
        settings.max_scroll_history_lines = Some(v);
    }
    if let Some(v) = t.scroll_multiplier {
        settings.scroll_multiplier = v;
    }
    if let Some(v) = t.minimum_contrast {
        settings.minimum_contrast = v;
    }
    if let Some(v) = t.cursor_shape {
        settings.cursor_shape = v;
    }
    if let Some(v) = t.blinking {
        settings.blinking = v;
    }
    if let Some(v) = t.alternate_scroll {
        settings.alternate_scroll = v;
    }
    if let Some(v) = t.bell {
        settings.bell = v;
    }
    if let Some(env) = t.env {
        settings.env = env;
    }
}

/// Write a default settings file if missing.
pub fn ensure_default_config_file() -> anyhow::Result<()> {
    let path = config_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let default = SettingsFile {
        theme: Some(ThemeName::Mocha),
        terminal: TerminalSettingsFile {
            font_size: Some(14.0),
            font_family: Some("Menlo".into()),
            line_height: Some(TerminalLineHeight::Custom(1.3)),
            option_as_meta: Some(true),
            copy_on_select: Some(false),
            max_scroll_history_lines: Some(10_000),
            scroll_multiplier: Some(3.0),
            minimum_contrast: Some(45.0),
            cursor_shape: Some(CursorShape::Block),
            ..Default::default()
        },
    };
    let json = serde_json::to_string_pretty(&default)?;
    std::fs::write(&path, format!("{json}\n"))?;
    log::info!("wrote default settings to {}", path.display());
    Ok(())
}

pub fn init(cx: &mut App) {
    let _ = ensure_default_config_file();
    TerminalSettings::init(cx);
}
