//! Thin settings for sleipnir.
//!
//! JSON keys align with Zed's `terminal` segment where practical.
//! Path: `config_path()` — `~/.config/sleipnir/settings.json` on macOS/Unix,
//! `%APPDATA%\sleipnir\settings.json` on Windows (not Zed's config path).

mod themes;

pub use themes::{
    Appearance, CustomPalette, TerminalPalette, ThemeName, ThemeSetting, get_color_at_index,
    palette_for_theme,
};

use collections::HashMap;
use gpui::{App, FontFallbacks, FontFeatures, FontWeight, Global, Pixels, px};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap as StdHashMap;
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
    /// Flash the tab chrome briefly (no audio).
    Visual,
}

/// When to prompt before closing a pane, tab, or window (M12).
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmClose {
    /// Confirm only when a non-shell foreground job is running.
    #[default]
    Dirty,
    /// Always confirm before close.
    Always,
    /// Never confirm (previous behavior).
    Never,
}

/// When to notify that a long-running foreground job finished (M14 → matrix).
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotifyOnCommandFinish {
    /// Never show a command-finish notification.
    Never,
    /// Only when the window is not focused (default).
    #[default]
    Unfocused,
    /// Always show a command-finish notification, even when focused.
    Always,
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
    /// Active theme: a built-in name, or a user theme from `themes.json`.
    pub theme: ThemeSetting,
    /// User-defined inline palette; when set, it overrides `theme`.
    pub custom_theme: Option<CustomPalette>,
    /// Restore tabs/splits/cwd from the last session on launch (M8).
    pub restore_session: bool,
    /// Enable OpenType ligatures (`calt`) when the font supports them (M10).
    pub font_ligatures: bool,
    /// Optional key binding overrides loaded from settings (M9).
    pub key_bindings: Vec<KeyBindingSpec>,
    /// When to confirm before closing a pane/tab (M12). Default: dirty.
    pub confirm_close: ConfirmClose,
    /// Open path-like navigation targets (cmd-click) in the default app (M12).
    pub path_links: bool,
    /// Window content opacity 0.15..=1.0 (M15). Default 1.0 (fully opaque).
    pub background_opacity: f32,
    /// Notify when a long-running foreground job finishes while unfocused (M14).
    /// Seconds; `0` disables. Default 5.
    pub notify_on_command_finish_secs: u64,
    /// When to fire the command-finish notification: never | unfocused | always.
    pub notify_on_command_finish_mode: NotifyOnCommandFinish,
    /// Opt-in: source OSC 133 A/B/C/D into newly spawned zsh/bash/fish.
    /// Off (default) keeps detect-only behavior.
    pub inject_osc133: bool,
}

/// One user-defined key binding: GPUI keystroke string + action name.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct KeyBindingSpec {
    /// GPUI keystroke, e.g. `"cmd-t"`, `"ctrl-shift-f"`.
    pub key: String,
    /// Action id: `new_tab`, `close_tab`, `find`, `toggle_command_palette`, …
    pub action: String,
    /// Optional GPUI context: `AppShell`, `Terminal`, or omit for global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Default monospace family for this OS.
pub fn default_font_family() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        // Ubuntu Mono ships with Ubuntu; DejaVu Sans Mono is the universal
        // fallback (fontconfig), so this always resolves on Ubuntu desktops.
        "Ubuntu Mono"
    }
    #[cfg(not(target_os = "linux"))]
    {
        default_font_family_for(cfg!(windows))
    }
}

/// Default monospace family. `windows = true` selects Cascadia Mono.
pub fn default_font_family_for(windows: bool) -> &'static str {
    if windows {
        "Cascadia Mono"
    } else {
        "Menlo"
    }
}

/// Fallback families for this OS (Linux prefers system monospace).
pub fn default_font_fallbacks() -> Option<FontFallbacks> {
    #[cfg(target_os = "linux")]
    {
        Some(FontFallbacks::from_fonts(vec![
            "DejaVu Sans Mono".into(),
            "Liberation Mono".into(),
        ]))
    }
    #[cfg(not(target_os = "linux"))]
    {
        default_font_fallbacks_for(cfg!(windows))
    }
}

/// Fallback families so a missing Cascadia Mono still renders a grid.
pub fn default_font_fallbacks_for(windows: bool) -> Option<FontFallbacks> {
    if windows {
        Some(FontFallbacks::from_fonts(vec![
            "Consolas".into(),
            "Courier New".into(),
        ]))
    } else {
        None
    }
}

/// Alt-as-meta default. Off on Windows so Alt can reach the menu bar.
pub fn option_as_meta_default_for(windows: bool) -> bool {
    !windows
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            shell: Shell::System,
            working_directory: WorkingDirectory::AlwaysHome,
            font_size: Some(px(14.)),
            font_family: Some(default_font_family().into()),
            font_fallbacks: default_font_fallbacks(),
            font_features: Some(FontFeatures::disable_ligatures()),
            font_weight: None,
            line_height: TerminalLineHeight::default(),
            env: HashMap::default(),
            cursor_shape: CursorShape::Block,
            blinking: TerminalBlink::TerminalControlled,
            alternate_scroll: AlternateScroll::On,
            option_as_meta: option_as_meta_default_for(cfg!(windows)),
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
            theme: ThemeSetting::Builtin(ThemeName::Mocha),
            custom_theme: None,
            restore_session: true,
            font_ligatures: false,
            key_bindings: Vec::new(),
            confirm_close: ConfirmClose::Dirty,
            path_links: true,
            background_opacity: 1.0,
            notify_on_command_finish_secs: 5,
            notify_on_command_finish_mode: NotifyOnCommandFinish::Unfocused,
            inject_osc133: false,
        }
    }
}

struct TerminalSettingsGlobal(TerminalSettings);
impl Global for TerminalSettingsGlobal {}

struct TerminalPaletteGlobal(Arc<TerminalPalette>);
impl Global for TerminalPaletteGlobal {}

/// User theme catalog from `themes.json`: `{ "name": {palette}, … }`.
struct UserThemesGlobal(Arc<StdHashMap<String, CustomPalette>>);
impl Global for UserThemesGlobal {}

/// Last-known system appearance, used to resolve the `Auto` theme.
struct AppearanceGlobal(Appearance);
impl Global for AppearanceGlobal {}

impl TerminalSettings {
    pub fn get_global(cx: &App) -> &TerminalSettings {
        &cx.global::<TerminalSettingsGlobal>().0
    }

    /// The user theme catalog (`~/.config/sleipnir/themes.json`), for custom
    /// `"theme": "<name>"` values.
    pub fn user_themes(cx: &App) -> Arc<StdHashMap<String, CustomPalette>> {
        cx.global::<UserThemesGlobal>().0.clone()
    }

    pub fn init(cx: &mut App) {
        cx.set_global(UserThemesGlobal(Arc::new(load_user_themes())));
        apply_loaded(load_or_default(), cx);
    }

    /// Re-read `settings.json` (and the theme catalog) and refresh globals.
    pub fn reload(cx: &mut App) {
        cx.set_global(UserThemesGlobal(Arc::new(load_user_themes())));
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

    /// Set the active theme, refresh the palette, and persist to settings.json.
    pub fn set_theme(theme: ThemeSetting, cx: &mut App) {
        let mut settings = Self::get_global(cx).clone();
        settings.theme = theme.clone();
        settings.custom_theme = None;
        apply_loaded(settings, cx);
        if let Err(err) = persist_theme(&theme) {
            log::warn!("failed to persist theme={:?}: {err}", theme.as_str());
        } else {
            log::info!("theme -> {} (persisted)", theme.as_str());
        }
    }

    /// Toggle session restore and persist to settings.json.
    pub fn set_restore_session(enabled: bool, cx: &mut App) {
        let mut settings = Self::get_global(cx).clone();
        settings.restore_session = enabled;
        apply_loaded(settings, cx);
        if let Err(err) = persist_bool_key("restore_session", enabled) {
            log::warn!("failed to persist restore_session={enabled}: {err}");
        } else {
            log::info!("restore_session -> {enabled} (persisted)");
        }
    }

    /// Toggle font ligatures and persist under `terminal.font_ligatures`.
    pub fn set_font_ligatures(enabled: bool, cx: &mut App) {
        let mut settings = Self::get_global(cx).clone();
        settings.font_ligatures = enabled;
        settings.font_features = Some(if enabled {
            FontFeatures::default()
        } else {
            FontFeatures::disable_ligatures()
        });
        apply_loaded(settings, cx);
        if let Err(err) = persist_terminal_bool("font_ligatures", enabled) {
            log::warn!("failed to persist font_ligatures={enabled}: {err}");
        } else {
            log::info!("font_ligatures -> {enabled} (persisted)");
        }
    }

    /// Toggle copy-on-select and persist under `terminal.copy_on_select`.
    pub fn set_copy_on_select(enabled: bool, cx: &mut App) {
        let mut settings = Self::get_global(cx).clone();
        settings.copy_on_select = enabled;
        apply_loaded(settings, cx);
        if let Err(err) = persist_terminal_bool("copy_on_select", enabled) {
            log::warn!("failed to persist copy_on_select={enabled}: {err}");
        } else {
            log::info!("copy_on_select -> {enabled} (persisted)");
        }
    }

    /// Record a new system appearance and re-resolve the palette (for `Auto`).
    pub fn set_appearance(appearance: Appearance, cx: &mut App) {
        cx.set_global(AppearanceGlobal(appearance));
        let settings = Self::get_global(cx).clone();
        let palette = Arc::new(resolve_palette(&settings, appearance, cx));
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
    let palette = Arc::new(resolve_palette(&settings, appearance, cx));
    cx.set_global(TerminalPaletteGlobal(palette));
    cx.set_global(TerminalSettingsGlobal(settings));
}

/// Resolve the effective palette: a user `custom_theme` wins, else the built-in
/// theme name (resolving `Auto` against the system appearance), else a theme
/// from the user catalog by name.
fn resolve_palette(settings: &TerminalSettings, appearance: Appearance, cx: &App) -> TerminalPalette {
    if let Some(custom) = &settings.custom_theme {
        return custom.to_palette();
    }
    match &settings.theme {
        ThemeSetting::Builtin(name) => palette_for_theme(*name, appearance),
        ThemeSetting::Custom(name) => {
            let catalog = TerminalSettings::user_themes(cx);
            catalog
                .get(name)
                .map(CustomPalette::to_palette)
                .unwrap_or_else(|| palette_for_theme(ThemeName::Mocha, appearance))
        }
    }
}

/// Load the theme catalog: the bundled 600+ scheme library (from
/// `resources/themes.json`, converted from iterm2-color-schemes), overlaid
/// with any user themes from `themes.json` in the config dir.
fn load_user_themes() -> StdHashMap<String, CustomPalette> {
    let mut themes: StdHashMap<String, CustomPalette> =
        serde_json::from_str(include_str!("../../../resources/themes.json")).unwrap_or_default();
    let path = config_dir().join("themes.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<StdHashMap<String, CustomPalette>>(&raw) {
            Ok(user) => {
                themes.extend(user);
            }
            Err(err) => log::warn!("failed to parse themes.json: {err}"),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("failed to read themes.json: {err}"),
    }
    themes
}

// ── file schema ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
struct SettingsFile {
    /// Theme name: auto | mocha | … or any user theme name from `themes.json`.
    #[serde(default)]
    theme: Option<String>,
    /// User-defined palette (hex colors); overrides `theme` when present.
    #[serde(default)]
    custom_theme: Option<CustomPalette>,
    /// Restore last session (tabs/splits/cwd) on launch. Default true.
    #[serde(default)]
    restore_session: Option<bool>,
    /// Extra key bindings layered on top of the built-in map.
    #[serde(default)]
    key_bindings: Option<Vec<KeyBindingSpec>>,
    /// Confirm close policy: dirty | always | never (M12).
    #[serde(default)]
    confirm_close: Option<ConfirmClose>,
    /// Open path-like targets on cmd-click (M12). Default true.
    #[serde(default)]
    path_links: Option<bool>,
    /// Content background opacity 0.15–1.0 (M15). Default 1.0.
    #[serde(default)]
    background_opacity: Option<f32>,
    /// Notify after unfocused commands longer than N seconds (M14). 0 = off.
    #[serde(default)]
    notify_on_command_finish_secs: Option<u64>,
    /// When to fire the finish notification (never | unfocused | always).
    #[serde(default)]
    notify_on_command_finish_mode: Option<NotifyOnCommandFinish>,
    /// Opt-in OSC 133 inject (also accepted at the top level).
    #[serde(default)]
    inject_osc133: Option<bool>,
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
    /// Enable font ligatures (`calt`). Default false.
    font_ligatures: Option<bool>,
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
    /// Opt-in: inject OSC 133 A/B/C/D into zsh/bash/fish (default false).
    inject_osc133: Option<bool>,
}

/// Directory that holds `settings.json` and `session.json`.
pub fn config_dir() -> PathBuf {
    config_dir_for(cfg!(windows))
}

/// Settings/session directory for a given OS family.
pub fn config_dir_for(windows: bool) -> PathBuf {
    if windows {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir")
    }
}

pub fn config_path() -> PathBuf {
    config_path_for(cfg!(windows))
}

pub fn config_path_for(windows: bool) -> PathBuf {
    config_dir_for(windows).join("settings.json")
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
    if let Some(name) = file.theme {
        settings.theme = ThemeName::from_str(&name)
            .map(ThemeSetting::Builtin)
            .unwrap_or(ThemeSetting::Custom(name));
    } else if let Some(name) = file.terminal.theme {
        settings.theme = ThemeSetting::Builtin(name);
    }
    if let Some(custom) = file.custom_theme {
        settings.custom_theme = Some(custom);
    }
    if let Some(v) = file.restore_session {
        settings.restore_session = v;
    }
    if let Some(bindings) = file.key_bindings {
        settings.key_bindings = bindings;
    }
    if let Some(v) = file.confirm_close {
        settings.confirm_close = v;
    }
    if let Some(v) = file.path_links {
        settings.path_links = v;
    }
    if let Some(v) = file.background_opacity {
        settings.background_opacity = v.clamp(0.15, 1.0);
    }
    if let Some(v) = file.notify_on_command_finish_secs {
        settings.notify_on_command_finish_secs = v;
    }
    if let Some(v) = file.notify_on_command_finish_mode {
        settings.notify_on_command_finish_mode = v;
    }
    if let Some(v) = file.inject_osc133 {
        settings.inject_osc133 = v;
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
    if let Some(v) = t.font_ligatures {
        settings.font_ligatures = v;
        settings.font_features = Some(if v {
            FontFeatures::default()
        } else {
            FontFeatures::disable_ligatures()
        });
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
    if let Some(v) = t.inject_osc133 {
        settings.inject_osc133 = v;
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
        theme: Some("mocha".into()),
        custom_theme: None,
        restore_session: Some(true),
        key_bindings: None,
        confirm_close: Some(ConfirmClose::Dirty),
        path_links: Some(true),
        background_opacity: Some(1.0),
        notify_on_command_finish_secs: Some(5),
        notify_on_command_finish_mode: Some(NotifyOnCommandFinish::Unfocused),
        inject_osc133: Some(false),
        terminal: TerminalSettingsFile {
            font_size: Some(14.0),
            font_family: Some(default_font_family().into()),
            line_height: Some(TerminalLineHeight::Custom(1.3)),
            option_as_meta: Some(option_as_meta_default_for(cfg!(windows))),
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

/// Parse settings JSON (or empty object), apply `patch`, return pretty JSON + newline.
pub fn merge_settings_json(
    raw: Option<&str>,
    patch: impl FnOnce(&mut serde_json::Value),
) -> String {
    let mut value: serde_json::Value = raw
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !value.is_object() {
        value = serde_json::json!({});
    }
    patch(&mut value);
    let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into());
    format!("{pretty}\n")
}

/// Merge `theme` into an existing settings JSON document, preserving other keys.
///
/// Returns pretty-printed JSON with a trailing newline. On empty/invalid input,
/// starts from an empty object so only `"theme"` is written.
pub fn merge_theme_into_json(raw: Option<&str>, theme: &ThemeSetting) -> String {
    merge_settings_json(raw, |value| {
        value["theme"] = serde_json::Value::String(theme.as_str());
        // Prefer top-level theme; drop nested terminal.theme if present so the two
        // cannot disagree after a picker write.
        if let Some(terminal) = value.get_mut("terminal") {
            if let Some(obj) = terminal.as_object_mut() {
                obj.remove("theme");
            }
        }
    })
}

fn read_settings_raw() -> anyhow::Result<Option<String>> {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn write_settings_json(json: &str) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, json)?;
    Ok(())
}

fn persist_theme(theme: &ThemeSetting) -> anyhow::Result<()> {
    let raw = read_settings_raw()?;
    let json = merge_theme_into_json(raw.as_deref(), theme);
    write_settings_json(&json)
}

fn persist_bool_key(key: &str, value: bool) -> anyhow::Result<()> {
    let raw = read_settings_raw()?;
    let json = merge_settings_json(raw.as_deref(), |doc| {
        doc[key] = serde_json::Value::Bool(value);
    });
    write_settings_json(&json)
}

fn persist_terminal_bool(key: &str, value: bool) -> anyhow::Result<()> {
    let raw = read_settings_raw()?;
    let json = merge_settings_json(raw.as_deref(), |doc| {
        if !doc.get("terminal").map(|t| t.is_object()).unwrap_or(false) {
            doc["terminal"] = serde_json::json!({});
        }
        if let Some(terminal) = doc.get_mut("terminal").and_then(|t| t.as_object_mut()) {
            terminal.insert(key.to_string(), serde_json::Value::Bool(value));
        }
    });
    write_settings_json(&json)
}

pub fn init(cx: &mut App) {
    let _ = ensure_default_config_file();
    TerminalSettings::init(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_theme_sets_top_level_on_empty() {
        let out = merge_theme_into_json(None, &ThemeSetting::Builtin(ThemeName::Nord));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "nord");
    }

    #[test]
    fn merge_theme_preserves_other_keys() {
        let raw = r#"{
  "theme": "mocha",
  "terminal": {
    "font_size": 14,
    "font_family": "Menlo"
  }
}"#;
        let out = merge_theme_into_json(Some(raw), &ThemeSetting::Builtin(ThemeName::Latte));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "latte");
        assert_eq!(v["terminal"]["font_size"], 14);
        assert_eq!(v["terminal"]["font_family"], "Menlo");
        assert!(v["terminal"].get("theme").is_none());
    }

    #[test]
    fn merge_theme_removes_nested_terminal_theme() {
        let raw = r#"{"theme":"mocha","terminal":{"theme":"latte","font_size":12}}"#;
        let out = merge_theme_into_json(Some(raw), &ThemeSetting::Builtin(ThemeName::TokyoNight));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "tokyo_night");
        assert!(v["terminal"].get("theme").is_none());
        assert_eq!(v["terminal"]["font_size"], 12);
    }

    #[test]
    fn theme_next_wraps_all() {
        let mut t = ThemeName::Auto;
        for _ in 0..ThemeName::ALL.len() {
            t = t.next();
        }
        assert_eq!(t, ThemeName::Auto);
    }

    #[test]
    fn theme_as_str_matches_serde() {
        for &name in ThemeName::ALL {
            let json = serde_json::to_string(&name).unwrap();
            assert_eq!(json, format!("\"{}\"", name.as_str()));
        }
    }

    #[test]
    fn theme_name_from_str_roundtrips_all() {
        for &name in ThemeName::ALL {
            assert_eq!(ThemeName::from_str(name.as_str()), Some(name));
        }
        assert_eq!(ThemeName::from_str("definitely_not_a_theme"), None);
    }

    #[test]
    fn theme_setting_parses_builtin_key() {
        let raw = r#"{ "theme": "dracula" }"#;
        let file: SettingsFile = serde_json::from_str(raw).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.theme, ThemeSetting::Builtin(ThemeName::Dracula));
    }

    #[test]
    fn theme_setting_accepts_custom_names() {
        let raw = r#"{ "theme": "kanagawa" }"#;
        let file: SettingsFile = serde_json::from_str(raw).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.theme, ThemeSetting::Custom("kanagawa".into()));
        assert_eq!(settings.theme.as_str(), "kanagawa");
    }

    #[test]
    fn bundled_theme_catalog_parses_and_is_large() {
        let catalog: StdHashMap<String, CustomPalette> =
            serde_json::from_str(include_str!("../../../resources/themes.json")).unwrap();
        assert!(
            catalog.len() >= 600,
            "bundled catalog has {} themes",
            catalog.len()
        );
        for name in ["Dracula", "Nord", "Catppuccin Mocha"] {
            assert!(catalog.contains_key(name), "missing {name}");
        }
    }

    #[test]
    fn merge_settings_preserves_and_sets_bools() {
        let raw = r#"{ "theme": "mocha", "terminal": { "font_size": 14 } }"#;
        let out = merge_settings_json(Some(raw), |v| {
            v["restore_session"] = serde_json::Value::Bool(false);
            if !v["terminal"].is_object() {
                v["terminal"] = serde_json::json!({});
            }
            v["terminal"]["font_ligatures"] = serde_json::Value::Bool(true);
        });
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "mocha");
        assert_eq!(v["restore_session"], false);
        assert_eq!(v["terminal"]["font_size"], 14);
        assert_eq!(v["terminal"]["font_ligatures"], true);
    }

    #[test]
    fn m12_keys_default_when_absent() {
        let settings = TerminalSettings::default();
        assert_eq!(settings.confirm_close, ConfirmClose::Dirty);
        assert!(settings.path_links);
        assert_eq!(settings.bell, TerminalBell::Off);
        assert!(!settings.copy_on_select);
    }

    #[test]
    fn inject_osc133_defaults_off() {
        assert!(
            !TerminalSettings::default().inject_osc133,
            "inject is opt-in so detect-only remains the default"
        );
    }

    #[test]
    fn inject_osc133_parses_from_terminal_block() {
        let raw = r#"{"terminal":{"inject_osc133":true}}"#;
        let file: SettingsFile = serde_json::from_str(raw).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert!(settings.inject_osc133);
    }

    #[test]
    fn default_font_is_os_specific() {
        assert_eq!(default_font_family_for(false), "Menlo");
        assert_eq!(default_font_family_for(true), "Cascadia Mono");
        assert_eq!(
            TerminalSettings::default().font_family.as_deref(),
            Some(default_font_family())
        );
        let win_fallbacks = default_font_fallbacks_for(true).expect("windows fallbacks");
        assert!(win_fallbacks.0.iter().any(|f| f == "Consolas"));
        assert!(win_fallbacks.0.iter().any(|f| f == "Courier New"));
        assert!(default_font_fallbacks_for(false).is_none());
    }

    #[test]
    fn option_as_meta_defaults_off_on_windows() {
        assert!(option_as_meta_default_for(false));
        assert!(!option_as_meta_default_for(true));
        assert_eq!(
            TerminalSettings::default().option_as_meta,
            option_as_meta_default_for(cfg!(windows))
        );
    }

    #[test]
    fn config_path_uses_os_config_dir_on_windows() {
        let unix = config_path_for(false);
        assert!(
            unix.ends_with(".config/sleipnir/settings.json"),
            "unix path was {}",
            unix.display()
        );

        let win = config_path_for(true);
        assert_eq!(win.file_name().and_then(|s| s.to_str()), Some("settings.json"));
        assert_eq!(
            win.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()),
            Some("sleipnir")
        );
        let win_dir = config_dir_for(true);
        assert_eq!(win.parent(), Some(win_dir.as_path()));
        if let Some(config) = dirs::config_dir() {
            assert_eq!(win_dir, config.join("sleipnir"));
        }
    }

    #[test]
    fn m12_keys_parse_from_settings_json() {
        let raw = r#"{
  "confirm_close": "always",
  "path_links": false,
  "terminal": {
    "bell": "visual",
    "copy_on_select": true
  }
}"#;
        let file: SettingsFile = serde_json::from_str(raw).expect("parse settings");
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.confirm_close, ConfirmClose::Always);
        assert!(!settings.path_links);
        assert_eq!(settings.bell, TerminalBell::Visual);
        assert!(settings.copy_on_select);
    }

    #[test]
    fn m12_confirm_close_variants_roundtrip() {
        for (json, expected) in [
            ("dirty", ConfirmClose::Dirty),
            ("always", ConfirmClose::Always),
            ("never", ConfirmClose::Never),
        ] {
            let raw = format!(r#"{{"confirm_close":"{json}"}}"#);
            let file: SettingsFile = serde_json::from_str(&raw).unwrap();
            let mut settings = TerminalSettings::default();
            merge_file(&mut settings, file);
            assert_eq!(settings.confirm_close, expected);
        }
    }

    #[test]
    fn m12_bell_visual_parses() {
        let raw = r#"{"terminal":{"bell":"visual"}}"#;
        let file: SettingsFile = serde_json::from_str(raw).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.bell, TerminalBell::Visual);
        assert_eq!(
            serde_json::to_string(&TerminalBell::Visual).unwrap(),
            "\"visual\""
        );
    }
}
