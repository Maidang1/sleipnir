//! Thin settings for sleipnir.
//!
//! JSON keys align with Zed's `terminal` segment where practical.
//! Path: `config_path()` — `~/.config/sleipnir/settings.json` on macOS/Unix,
//! `%APPDATA%\sleipnir\settings.json` on Windows.

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

/// Where tab chips live in the window chrome.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabPlacement {
    /// Vertical rail on the left, grouped by workspace.
    Side,
    /// Horizontal strip across the top. Default. Same tab features as [`Self::Side`].
    #[default]
    Top,
}

impl TabPlacement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Side => "side",
            Self::Top => "top",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Side => Self::Top,
            Self::Top => Self::Side,
        }
    }
}

/// Optional keymap overlay. Extra `key_bindings` still win.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeybindingPreset {
    #[default]
    Default,
    /// Prefix-style pane/tab chords: `ctrl-b` then `c` / `%` / `"` / arrows.
    Tmux,
}

pub const SIDEBAR_WIDTH_MIN: f32 = 160.0;
pub const SIDEBAR_WIDTH_MAX: f32 = 320.0;
pub const SIDEBAR_WIDTH_DEFAULT: f32 = 200.0;

/// Clamp a user `sidebar_width` to the supported range.
pub fn clamp_sidebar_width(width: f32) -> f32 {
    width.clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX)
}

/// Where the Run Ledger keeps its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunLedgerMode {
    /// 不采集、无 UI、不读写 runs.json（文件保留）。
    Off,
    /// 采集并显示，但不读写 runs.json（磁盘文件原样保留）。
    Memory,
    #[default]
    /// 采集、显示、读写 runs.json。
    Persist,
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

// ── runtime settings ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TerminalSettings {
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
    /// Source OSC 133 A/B/C/D into newly spawned zsh/bash/fish.
    /// Default true so the Run Ledger gets real command boundaries.
    /// Set false to restore detect-only behavior.
    pub inject_osc133: bool,
    /// Whether the Run Ledger collects, shows, and persists runs.
    pub run_ledger: RunLedgerMode,
    /// Retention window for persisted runs, in days.
    pub run_ledger_retention_days: u64,
    /// Cap on persisted runs (oldest dropped first).
    pub run_ledger_max_runs: usize,
    /// Redact command lines at capture time (heuristic, not a guarantee).
    pub run_ledger_redact: bool,
    /// Side rail (default) or the top tab strip. Same tab features either way.
    pub tab_placement: TabPlacement,
    /// Left rail width in logical pixels (clamped 160–320).
    pub sidebar_width: f32,
    /// Draw agent monograms on tab chips.
    pub agent_icons: bool,
    /// Default-off external control surface (ADR-0011).
    pub control_surface: bool,
    /// Menu-bar attention item. Default true.
    pub show_tray_icon: bool,
    /// User command to receive the current selection. Empty = disabled.
    pub pipe_selection_command: Option<String>,
    /// Built-in keymap overlay.
    pub keybinding_preset: KeybindingPreset,
    /// Chrome banner after session restore when a pane has Run history.
    pub show_tombstone: bool,
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
    default_font_family_for(cfg!(windows))
}

/// Default monospace family. `windows = true` selects Cascadia Mono.
pub fn default_font_family_for(windows: bool) -> &'static str {
    if windows {
        "Cascadia Mono"
    } else {
        "Menlo"
    }
}

/// Fallback families so a missing Cascadia Mono still renders a grid.
pub fn default_font_fallbacks() -> Option<FontFallbacks> {
    default_font_fallbacks_for(cfg!(windows))
}

/// Fallback families. `windows = true` adds Consolas / Courier New.
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
pub fn option_as_meta_default() -> bool {
    option_as_meta_default_for(cfg!(windows))
}

/// Alt-as-meta default. `windows = true` is off.
pub fn option_as_meta_default_for(windows: bool) -> bool {
    !windows
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
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
            option_as_meta: option_as_meta_default(),
            copy_on_select: false,
            keep_selection_on_copy: true,
            open_links_in_mouse_mode: true,
            max_scroll_history_lines: Some(10_000),
            scroll_multiplier: 3.0,
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
            inject_osc133: true,
            run_ledger: RunLedgerMode::Persist,
            run_ledger_retention_days: 7,
            run_ledger_max_runs: 500,
            run_ledger_redact: true,
            tab_placement: TabPlacement::Top,
            sidebar_width: SIDEBAR_WIDTH_DEFAULT,
            agent_icons: true,
            control_surface: false,
            show_tray_icon: true,
            pipe_selection_command: None,
            keybinding_preset: KeybindingPreset::Default,
            show_tombstone: true,
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

    /// Switch tab chrome between the side rail and the top strip.
    pub fn set_tab_placement(placement: TabPlacement, cx: &mut App) {
        let mut settings = Self::get_global(cx).clone();
        settings.tab_placement = placement;
        apply_loaded(settings, cx);
        if let Err(err) = persist_string_key("tab_placement", placement.as_str()) {
            log::warn!(
                "failed to persist tab_placement={}: {err}",
                placement.as_str()
            );
        } else {
            log::info!("tab_placement -> {} (persisted)", placement.as_str());
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
fn resolve_palette(
    settings: &TerminalSettings,
    appearance: Appearance,
    cx: &App,
) -> TerminalPalette {
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

/// Load extra themes from `themes.json` in the config dir.
fn load_user_themes() -> StdHashMap<String, CustomPalette> {
    let path = config_dir().join("themes.json");
    match std::fs::read_to_string(&path) {
        Ok(raw) => match serde_json::from_str::<StdHashMap<String, CustomPalette>>(&raw) {
            Ok(user) => user,
            Err(err) => {
                log::warn!("failed to parse themes.json: {err}");
                StdHashMap::new()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => StdHashMap::new(),
        Err(err) => {
            log::warn!("failed to read themes.json: {err}");
            StdHashMap::new()
        }
    }
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
    /// OSC 133 inject (also accepted at the top level). Default true.
    #[serde(default)]
    inject_osc133: Option<bool>,
    /// Run Ledger mode: off | memory | persist (default persist).
    #[serde(default, deserialize_with = "lenient_opt_run_ledger_mode")]
    run_ledger: Option<RunLedgerMode>,
    #[serde(default)]
    run_ledger_retention_days: Option<u64>,
    #[serde(default)]
    run_ledger_max_runs: Option<usize>,
    #[serde(default)]
    run_ledger_redact: Option<bool>,
    /// side | top. Default side.
    #[serde(default)]
    tab_placement: Option<TabPlacement>,
    /// Left rail width. Clamped to 160–320.
    #[serde(default)]
    sidebar_width: Option<f32>,
    /// Draw agent monograms on tabs. Default true.
    #[serde(default)]
    agent_icons: Option<bool>,
    #[serde(default)]
    control_surface: Option<bool>,
    #[serde(default)]
    show_tray_icon: Option<bool>,
    #[serde(default)]
    pipe_selection_command: Option<String>,
    #[serde(default)]
    keybinding_preset: Option<KeybindingPreset>,
    #[serde(default)]
    show_tombstone: Option<bool>,
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
    /// Inject OSC 133 A/B/C/D into zsh/bash/fish (default true).
    inject_osc133: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt_run_ledger_mode")]
    run_ledger: Option<RunLedgerMode>,
    run_ledger_retention_days: Option<u64>,
    run_ledger_max_runs: Option<usize>,
    run_ledger_redact: Option<bool>,
}

/// Unknown `run_ledger` values become `None` so a typo cannot reject the file.
fn lenient_opt_run_ledger_mode<'de, D>(deserializer: D) -> Result<Option<RunLedgerMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
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
    if let Some(v) = file.run_ledger {
        settings.run_ledger = v;
    }
    if let Some(v) = file.run_ledger_retention_days {
        settings.run_ledger_retention_days = v;
    }
    if let Some(v) = file.run_ledger_max_runs {
        settings.run_ledger_max_runs = v;
    }
    if let Some(v) = file.run_ledger_redact {
        settings.run_ledger_redact = v;
    }
    if let Some(v) = file.tab_placement {
        settings.tab_placement = v;
    }
    if let Some(v) = file.sidebar_width {
        settings.sidebar_width = clamp_sidebar_width(v);
    }
    if let Some(v) = file.agent_icons {
        settings.agent_icons = v;
    }
    if let Some(v) = file.control_surface {
        settings.control_surface = v;
    }
    if let Some(v) = file.show_tray_icon {
        settings.show_tray_icon = v;
    }
    if file.pipe_selection_command.is_some() {
        settings.pipe_selection_command = file.pipe_selection_command.filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = file.keybinding_preset {
        settings.keybinding_preset = v;
    }
    if let Some(v) = file.show_tombstone {
        settings.show_tombstone = v;
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
    if let Some(v) = t.run_ledger {
        settings.run_ledger = v;
    }
    if let Some(v) = t.run_ledger_retention_days {
        settings.run_ledger_retention_days = v;
    }
    if let Some(v) = t.run_ledger_max_runs {
        settings.run_ledger_max_runs = v;
    }
    if let Some(v) = t.run_ledger_redact {
        settings.run_ledger_redact = v;
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
        inject_osc133: Some(true),
        run_ledger: Some(RunLedgerMode::Persist),
        run_ledger_retention_days: Some(7),
        run_ledger_max_runs: Some(500),
        run_ledger_redact: Some(true),
        tab_placement: Some(TabPlacement::Top),
        sidebar_width: Some(SIDEBAR_WIDTH_DEFAULT),
        agent_icons: Some(true),
        control_surface: Some(false),
        show_tray_icon: Some(true),
        pipe_selection_command: None,
        keybinding_preset: Some(KeybindingPreset::Default),
        show_tombstone: Some(true),
        terminal: TerminalSettingsFile {
            font_size: Some(14.0),
            font_family: Some(default_font_family().into()),
            line_height: Some(TerminalLineHeight::Custom(1.3)),
            option_as_meta: Some(option_as_meta_default()),
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
/// Drops `custom_theme` and nested `terminal.theme` so a picker write cannot
/// leave a higher-priority override that would resurrect on reload.
///
/// Returns pretty-printed JSON with a trailing newline. On empty/invalid input,
/// starts from an empty object so only `"theme"` is written.
pub fn merge_theme_into_json(raw: Option<&str>, theme: &ThemeSetting) -> String {
    merge_settings_json(raw, |value| {
        value["theme"] = serde_json::Value::String(theme.as_str());
        if let Some(obj) = value.as_object_mut() {
            obj.remove("custom_theme");
        }
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

fn persist_string_key(key: &str, value: &str) -> anyhow::Result<()> {
    let raw = read_settings_raw()?;
    let json = merge_settings_json(raw.as_deref(), |doc| {
        doc[key] = serde_json::Value::String(value.to_string());
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
    fn merge_theme_clears_inline_custom_theme() {
        // custom_theme wins over theme on load; a picker write must drop it or
        // the chosen theme comes back as the inline palette after reload.
        let raw = r##"{
  "theme": "mocha",
  "custom_theme": {
    "background": "#0d1117",
    "foreground": "#e6edf3"
  },
  "restore_session": true
}"##;
        let out = merge_theme_into_json(Some(raw), &ThemeSetting::Builtin(ThemeName::Nord));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "nord");
        assert!(v.get("custom_theme").is_none());
        assert_eq!(v["restore_session"], serde_json::Value::Bool(true));
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
    fn inject_osc133_defaults_on() {
        assert!(TerminalSettings::default().inject_osc133);
    }

    #[test]
    fn run_ledger_defaults_to_persist() {
        let s = TerminalSettings::default();
        assert_eq!(s.run_ledger, RunLedgerMode::Persist);
        assert_eq!(s.run_ledger_retention_days, 7);
        assert_eq!(s.run_ledger_max_runs, 500);
        assert!(s.run_ledger_redact);
    }

    #[test]
    fn run_ledger_mode_parses_from_file() {
        let file: SettingsFile = serde_json::from_str(r#"{"run_ledger":"memory"}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.run_ledger, RunLedgerMode::Memory);

        let file: SettingsFile = serde_json::from_str(r#"{"run_ledger":"off"}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.run_ledger, RunLedgerMode::Off);

        // Illegal value must not panic and must keep the default.
        let file: SettingsFile = serde_json::from_str(r#"{"run_ledger":"nope"}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.run_ledger, RunLedgerMode::Persist);
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
        assert!(default_font_fallbacks_for(true).is_some());
        assert!(default_font_fallbacks_for(false).is_none());
    }

    #[test]
    fn option_as_meta_defaults_off_on_windows() {
        assert!(option_as_meta_default_for(false));
        assert!(!option_as_meta_default_for(true));
        assert_eq!(
            TerminalSettings::default().option_as_meta,
            option_as_meta_default()
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
        assert_eq!(
            win.file_name().and_then(|s| s.to_str()),
            Some("settings.json")
        );
        assert_eq!(
            win.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str()),
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

    #[test]
    fn tab_rail_keys_default_when_absent() {
        let s = TerminalSettings::default();
        assert_eq!(s.tab_placement, TabPlacement::Top);
        assert_eq!(s.sidebar_width, SIDEBAR_WIDTH_DEFAULT);
        assert!(s.agent_icons);
        let file: SettingsFile = serde_json::from_str("{}").unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.tab_placement, TabPlacement::Top);
        assert_eq!(settings.sidebar_width, SIDEBAR_WIDTH_DEFAULT);
        assert!(settings.agent_icons);
    }

    #[test]
    fn tab_placement_parses_top() {
        let file: SettingsFile = serde_json::from_str(r#"{"tab_placement":"top"}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.tab_placement, TabPlacement::Top);
    }

    #[test]
    fn tab_placement_toggle_and_merge() {
        assert_eq!(TabPlacement::Side.toggle(), TabPlacement::Top);
        assert_eq!(TabPlacement::Top.toggle(), TabPlacement::Side);
        assert_eq!(TabPlacement::Side.as_str(), "side");
        assert_eq!(TabPlacement::Top.as_str(), "top");
        let out = merge_settings_json(Some(r#"{ "theme": "mocha" }"#), |v| {
            v["tab_placement"] = serde_json::Value::String(TabPlacement::Top.as_str().into());
        });
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "mocha");
        assert_eq!(v["tab_placement"], "top");
    }

    #[test]
    fn sidebar_width_is_clamped() {
        let file: SettingsFile = serde_json::from_str(r#"{"sidebar_width":80}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.sidebar_width, SIDEBAR_WIDTH_MIN);

        let file: SettingsFile = serde_json::from_str(r#"{"sidebar_width":900}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert_eq!(settings.sidebar_width, SIDEBAR_WIDTH_MAX);
    }

    #[test]
    fn agent_icons_can_be_disabled() {
        let file: SettingsFile = serde_json::from_str(r#"{"agent_icons":false}"#).unwrap();
        let mut settings = TerminalSettings::default();
        merge_file(&mut settings, file);
        assert!(!settings.agent_icons);
    }
}
