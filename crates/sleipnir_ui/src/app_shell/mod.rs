//! Single-window multi-tab shell for sleipnir (HIG-aligned chrome).

/// Maps `CommandId` to canonical shell actions. A child module so it can reach
/// `AppShell`'s private methods without widening them to the whole crate.
mod agent_hud;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod browser_panel;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod browser_panel_stub;
mod command_dispatch;
mod diff;
mod find;
mod layout;
mod palette;
mod panels;
pub(crate) mod plugin_paint;
mod plugins;
pub(crate) mod query;
mod settings;
mod tabs;
mod terminal_menu;
mod update;

pub(crate) use plugins::PluginConsentPending;

use gpui::{
    App, AppContext as _, BorrowAppContext, Bounds, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels,
    Render, ScrollHandle, SharedString, Styled as _, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions, actions, div,
    prelude::FluentBuilder as _, px, size,
};
use run_ledger::{PaneKey, RunEvent};
use sleipnir_settings::{Appearance, ConfirmClose, Language, TerminalPalette, TerminalSettings};
use std::path::PathBuf;

use crate::chrome::pixel;
use crate::chrome::{ChromeGeometry, ChromeTokens};
use crate::command_palette::{CommandId, CommandItem, commands_for as palette_commands_for};
use crate::pane_tree::{CloseOutcome, Direction, PaneId, PaneRect, SplitAxis, SplitPath, neighbor};
use crate::run_ledger_global::RunLedgerGlobal;
pub(crate) use crate::tab_convert::Tab;
use crate::ui_mode::{InputMode, InputOwner, OverlayKind, PaneFactsState, UiMode};
use crate::{TermView, UpdateModel, UpdateUiState};

/// Map a GPUI window appearance to our light/dark `Appearance`.
fn appearance_of(a: gpui::WindowAppearance) -> Appearance {
    match a {
        gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => Appearance::Light,
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

actions!(
    sleipnir,
    [
        /// Open a new terminal tab.
        NewTab,
        /// Reopen the most recently closed terminal tab.
        ReopenClosedTab,
        /// Close the active pane (or the tab, if it is the last pane).
        CloseTab,
        /// Activate the next tab.
        NextTab,
        /// Activate the previous tab.
        PrevTab,
        /// Reload `~/.config/sleipnir/settings.json`.
        ReloadSettings,
        /// Cycle built-in theme (persists to settings.json).
        CycleTheme,
        /// Toggle the settings panel (⌘,).
        OpenSettings,
        /// Split the active pane left|right (new pane on the right). ⌘D.
        SplitRight,
        /// Split the active pane top/bottom (new pane below). ⌘⇧D.
        SplitDown,
        /// Move focus to the pane left of the active one. ⌘⌥←.
        FocusPaneLeft,
        /// Move focus to the pane right of the active one. ⌘⌥→.
        FocusPaneRight,
        /// Move focus to the pane above the active one. ⌘⌥↑.
        FocusPaneUp,
        /// Move focus to the pane below the active one. ⌘⌥↓.
        FocusPaneDown,
        /// Check GitHub Releases for a newer version.
        CheckForUpdates,
        /// Toggle the command palette (⌘⇧K).
        ToggleCommandPalette,
        /// Open find-in-scrollback (⌘F).
        Find,
        /// Jump to the next search match (⌘G).
        FindNext,
        /// Jump to the previous search match (⌘⇧G).
        FindPrev,
        /// Increase window font size (⌘+ / ⌘=).
        IncreaseFontSize,
        /// Decrease window font size (⌘-).
        DecreaseFontSize,
        /// Reset window font size to settings (⌘0).
        ResetFontSize,
        /// Open a new independent OS window (⌘N).
        NewWindow,
        /// Toggle pane zoom (maximize active pane) — M13.
        TogglePaneZoom,
        /// Toggle broadcast input to all panes in the tab — M13.
        ToggleBroadcast,
        /// Jump to previous OSC 133 prompt — M14.
        JumpPrevPrompt,
        /// Jump to next OSC 133 prompt — M14.
        JumpNextPrompt,
        /// Toggle Quick Select overlay labels — M15.
        ToggleQuickSelect,
        /// Open Quick Terminal window — M15.
        OpenQuickTerminal,
        /// Export the active pane's scrollback to a temp file and open it.
        ExportScrollback,
        /// Clear Attention on every pane in the active tab. Does not delete Runs.
        MarkTabSeen,
        /// Toggle the focused-pane facts overlay (cwd / process tree / ports).
        TogglePaneFacts,
        /// Paste the terminal selection into the focused pane.
        SendSelection,
        /// Pipe the selection through `pipe_selection_command`.
        PipeSelection,
        /// Paste `git diff` wrapped as a review prompt.
        SendGitDiff,
        /// Fuzzy search shell history in chrome (does not take the PTY line).
        ToggleHistorySearch,
        /// Toggle the git diff inspector overlay.
        ToggleDiff,
        /// Toggle the Plugin Monitor overlay (ADR-0016 §7).
        TogglePluginMonitor,
        /// Toggle the native right-side browser (macOS / Windows).
        ToggleBrowser,
    ]
);

/// Activate the tab at the given 1-based index (⌘1..⌘9).
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = sleipnir, no_json)]
pub struct ActivateTab(pub usize);

/// Ghost chip rendered under the pointer while dragging a tab to reorder it.
pub(crate) struct TabDragPreview {
    pub(crate) title: SharedString,
}

/// Drag payload for pulling a pane out onto the tab list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaneDrag {
    pub pane_id: PaneId,
}

impl Render for TabDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let tokens = ChromeTokens::from_palette(&palette, window.is_window_active());
        div()
            .px_3()
            .py_1()
            .rounded(px(0.0))
            .bg(tokens.hover)
            .border(pixel::PIXEL_BORDER)
            .border_color(tokens.border)
            .shadow(pixel::hard_shadow())
            .text_sm()
            .text_color(tokens.fg)
            .child(self.title.clone())
    }
}

impl Tab {
    /// Title shown on the side-rail chip and the window: user rename, else the
    /// active pane's process title.
    pub(crate) fn title(&self, cx: &App) -> SharedString {
        if let Some(custom) = self.custom_title.as_ref() {
            if !custom.is_empty() {
                return custom.clone();
            }
        }
        self.pane_title(cx)
    }

    /// Top-strip chip label: user rename, else the last two cwd components.
    pub(crate) fn path_label(&self, cx: &App) -> SharedString {
        if let Some(custom) = self.custom_title.as_ref() {
            if !custom.is_empty() {
                return custom.clone();
            }
        }
        crate::chrome::workspace::tab_path_label(self.workspace_cwd(cx).as_deref()).into()
    }

    /// The focused leaf in this tab (falls back to the first leaf).
    pub(crate) fn active_pane_id(&self) -> PaneId {
        self.active_pane
    }

    /// Working directory of the active pane, when the PTY reports one.
    pub(crate) fn workspace_cwd(&self, cx: &App) -> Option<std::path::PathBuf> {
        let mut leaves = Vec::new();
        self.tree.leaves(&mut leaves);
        let view = leaves
            .iter()
            .find(|(id, _)| *id == self.active_pane)
            .or_else(|| leaves.first())
            .map(|(_, view)| *view)?;
        view.read(cx).working_directory(cx)
    }

    /// The active pane's own title (ignores any custom override).
    pub(crate) fn pane_title(&self, cx: &App) -> SharedString {
        let mut all = Vec::new();
        self.tree.walk_leaves(&mut all);
        let active = all.iter().find(|(id, _, _)| *id == self.active_pane);
        if let Some((_, _, crate::LeafContent::Panel(view))) = active {
            return view.plugin_id().to_string().into();
        }
        let mut leaves = Vec::new();
        self.tree.leaves(&mut leaves);
        let view = leaves
            .iter()
            .find(|(id, _)| *id == self.active_pane)
            .map(|(_, v)| *v)
            .or_else(|| leaves.first().map(|(_, v)| *v));
        view.map(|v| v.read(cx).title().to_string())
            .unwrap_or_else(|| "shell".to_string())
            .into()
    }
}

/// In-progress inline tab rename triggered from a tab's context menu.
#[derive(Clone)]
pub(crate) struct RenameState {
    pub(crate) tab_id: u64,
    pub(crate) query: crate::app_shell::query::QueryBox,
}

#[derive(Clone, Copy)]
pub(crate) struct TabMenuState {
    pub(crate) tab_id: u64,
    pub(crate) position: gpui::Point<gpui::Pixels>,
    /// Keyboard-highlighted row (↑/↓ in the capture key handler).
    pub(crate) selected: usize,
}

/// Right-click menu state for a terminal pane (normal mouse mode).
pub(crate) struct TerminalMenuState {
    pub(crate) position: gpui::Point<gpui::Pixels>,
    pub(crate) link: Option<terminal::MaybeNavigationTarget>,
    pub(crate) selected: usize,
}

pub(crate) struct ClosedTab {
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) title: Option<SharedString>,
}

/// Top-level section inside the settings panel (WezTerm-style tabs).
/// Add variants here as new setting pages land.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SettingsSection {
    #[default]
    Theme,
    /// Session restore, ligatures, and other app/terminal toggles.
    General,
    /// Agent panel and built-in agents toggles.
    Agents,
    /// Read-only reference for the shortcuts shipped on the current platform.
    Shortcuts,
}

impl SettingsSection {
    const ALL: &'static [SettingsSection] = &[
        SettingsSection::Theme,
        SettingsSection::General,
        SettingsSection::Agents,
        SettingsSection::Shortcuts,
    ];

    fn id(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
            SettingsSection::General => "general",
            SettingsSection::Agents => "agents",
            SettingsSection::Shortcuts => "shortcuts",
        }
    }

    fn label(self, language: Language) -> &'static str {
        match self {
            SettingsSection::Theme => language.text("settings.section.theme"),
            SettingsSection::General => language.text("settings.section.general"),
            SettingsSection::Agents => language.text("settings.section.agents"),
            SettingsSection::Shortcuts => language.text("settings.section.shortcuts"),
        }
    }
}

/// In-progress divider drag: which tab, which split, and its axis.
#[derive(Clone)]
struct DragState {
    tab_id: u64,
    path: SplitPath,
    axis: SplitAxis,
    /// Screen bounds of the split's container at drag start.
    container: Bounds<Pixels>,
}

/// Window root: unified chrome band + active terminal.
struct SettingsState {
    section: SettingsSection,
    themes: crate::app_shell::query::QueryBox,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            section: SettingsSection::Theme,
            themes: crate::app_shell::query::QueryBox::default(),
        }
    }
}

struct PaletteState {
    input: crate::app_shell::query::QueryBox,
    recents: Vec<CommandId>,
    items: Vec<CommandItem>,
    plugin_commands: Vec<plugin_host::LoadedPluginCommand>,
}

impl PaletteState {
    fn new(
        items: Vec<CommandItem>,
        plugin_commands: Vec<plugin_host::LoadedPluginCommand>,
    ) -> Self {
        Self {
            input: crate::app_shell::query::QueryBox::default(),
            recents: Vec::new(),
            items,
            plugin_commands,
        }
    }
}

#[derive(Default)]
struct HistoryState {
    input: crate::app_shell::query::QueryBox,
    /// Hits loaded once when the overlay opens. Reading `$HISTFILE` per
    /// keystroke or per render frame was measurable on large histories.
    hits: Vec<crate::chrome::history_search::HistoryHit>,
}

impl HistoryState {
    /// Load the shell history once for this overlay session.
    fn load_hits(&mut self) {
        self.hits = crate::chrome::history_search::load_history_hits();
    }

    /// The hits matching `query`, capped for display and selection.
    fn shown(&self) -> Vec<&crate::chrome::history_search::HistoryHit> {
        crate::chrome::history_search::filter_history(&self.hits, &self.input.text, 20)
    }
}

pub struct AppShell {
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    next_id: u64,
    /// Monotonic id source for panes across all tabs.
    next_pane_id: PaneId,
    focus_handle: FocusHandle,
    /// Empty-region drag: true after mouse-down on a drag strip until move/up.
    should_move: bool,
    pub(crate) tab_scroll_handle: ScrollHandle,
    /// Tab id currently under the pointer (for hover close / hover fill).
    pub(crate) hovered_tab: Option<u64>,
    /// Tab id a drag is hovering over, for the drop-target insertion bar.
    pub(crate) tab_drop_target: Option<u64>,
    /// Pane rects from the last render, for keyboard neighbor navigation.
    pane_rects: Vec<PaneRect>,
    /// Content area bounds captured last frame (origin + size), for analytic
    /// pane layout and divider hit-testing.
    content_bounds: Option<Bounds<Pixels>>,
    /// Active divider drag, if any.
    drag: Option<DragState>,
    /// Recently closed tabs, oldest first and capped at ten entries.
    pub(crate) closed_tabs: Vec<ClosedTab>,
    /// Owned keyboard owner. Confirm, consent, menus, rename, find, and
    /// modal overlays are arms of this enum, so they cannot coexist.
    pub(crate) input: InputMode,
    /// Quick-select banner. Independent of [`InputMode`] because it is
    /// designed to coexist with terminal content.
    pub(crate) mode: UiMode,
    settings: SettingsState,
    palette: PaletteState,
    /// Find-in-scrollback state domain (query, IME marks, match cursor, modes).
    find: find::FindState,
    /// Window-scoped font size override (M12 zoom); not written to settings.
    pub(crate) font_size_override: Option<Pixels>,
    /// Tab ids currently flashing for visual bell (M12).
    pub(crate) bell_flash_tabs: std::collections::HashSet<u64>,
    /// Fan-out keystrokes to all panes in the active tab (M13).
    broadcast: bool,
    /// Focused-pane facts: async collection state machine. Carries its own
    /// snapshot timestamp and in-flight flag.
    facts: PaneFactsState,
    history: HistoryState,
    /// Git diff inspector (ADR-0012). Not a Pane.
    pub(crate) diff_view: Option<crate::diff::DiffView>,
    diff_gen: u64,
    /// Keep the app-quit subscription alive for the window lifetime.
    _quit_subscription: Option<gpui::Subscription>,
    /// Per-window housekeeping timer: ledger focus sync + pane-facts refresh.
    /// Runs on a fixed interval so these side-effects are decoupled from Render.
    _housekeeping: gpui::Task<()>,
    /// Per-window plugin surfaces: chrome contributions and the polled
    /// pane-fact watch. `PluginRuntime` (supervisor / catalog / pump) stays a
    /// process `Global`; these two registries are the window-scoped half.
    /// Panel surfaces are owned by the pane tree (`LeafContent::Panel`).
    plugin_chrome: crate::plugin_chrome::ChromeRegistry,
    plugin_watch: crate::plugin_event_watch::PluginEventWatch,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    browser: Option<Entity<crate::browser::BrowserView>>,
    /// Read through `browser_panel_is_open()` so the render path stays
    /// platform-independent.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    browser_open: bool,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    _browser_bridge: Option<crate::browser_control::WindowBridge>,
}

/// What the shared confirm dialog is asking about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmKind {
    ClosePane(PaneKey),
    CloseTab(u64),
    #[cfg(target_os = "linux")]
    CloseWindow,
}

impl ConfirmKind {
    /// Resolve a captured identity against current ownership, not current focus.
    fn pane_target(self, tabs: &[Tab]) -> Option<(usize, PaneId)> {
        let Self::ClosePane(key) = self else {
            return None;
        };
        tabs.iter()
            .enumerate()
            .find_map(|(index, tab)| tab.tree.pane_id_for_key(key).map(|id| (index, id)))
    }
}

/// Pending confirmation dialog (close pane / tab / window).
pub(crate) struct CloseConfirmState {
    /// Human-readable what will happen.
    pub(crate) message: SharedString,
    pub(crate) kind: ConfirmKind,
}

impl Focusable for AppShell {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for AppShell {}

impl AppShell {
    /// Replace the keyboard owner, running teardown for the outgoing mode.
    /// This and the `take_*` / `dismiss_*` methods below are the only writers of
    /// `self.input`: each runs `teardown_input` for the mode it replaces, so
    /// Find highlights, IME state, menu selection, and theme query are cleaned
    /// up no matter which owner is being dropped. Nothing (paint click-away
    /// included) may assign `self.input` or call `InputMode::dismiss_*` directly.
    pub(crate) fn set_input(&mut self, next: InputMode, cx: &mut Context<Self>) {
        let old = self.input.owner();
        self.input.replace(next);
        self.teardown_input(old, cx);
        self.sync_browser_presentation(cx);
    }

    pub(crate) fn open_overlay(&mut self, kind: OverlayKind, cx: &mut Context<Self>) {
        if self.input.is_overlay(kind) {
            return;
        }
        self.set_input(InputMode::Overlay(kind), cx);
    }

    pub(crate) fn toggle_overlay(&mut self, kind: OverlayKind, cx: &mut Context<Self>) -> bool {
        if self.input.is_overlay(kind) {
            self.set_input(InputMode::Terminal, cx);
            false
        } else {
            self.set_input(InputMode::Overlay(kind), cx);
            true
        }
    }

    pub(crate) fn close_overlay(&mut self, kind: OverlayKind, cx: &mut Context<Self>) -> bool {
        if !self.input.is_overlay(kind) {
            return false;
        }
        self.set_input(InputMode::Terminal, cx);
        true
    }

    pub(crate) fn dismiss_tab_menu(&mut self, cx: &mut Context<Self>) {
        if self.input.dismiss_tab_menu() {
            self.teardown_input(InputOwner::TabMenu, cx);
        }
    }

    pub(crate) fn dismiss_terminal_menu(&mut self, cx: &mut Context<Self>) {
        if self.input.dismiss_terminal_menu() {
            self.teardown_input(InputOwner::TerminalMenu, cx);
        }
    }

    pub(crate) fn dismiss_consent_input(&mut self, cx: &mut Context<Self>) {
        if self.input.dismiss_consent() {
            self.teardown_input(InputOwner::Consent, cx);
        }
    }

    pub(crate) fn take_confirm_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<CloseConfirmState> {
        let r = self.input.take_confirm()?;
        self.teardown_input(InputOwner::Confirm, cx);
        Some(r)
    }

    pub(crate) fn take_consent_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<PluginConsentPending> {
        let r = self.input.take_consent()?;
        self.teardown_input(InputOwner::Consent, cx);
        Some(r)
    }

    pub(crate) fn take_tab_menu_input(&mut self, cx: &mut Context<Self>) -> Option<TabMenuState> {
        let r = self.input.take_tab_menu()?;
        self.teardown_input(InputOwner::TabMenu, cx);
        Some(r)
    }

    pub(crate) fn take_terminal_menu_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<TerminalMenuState> {
        let r = self.input.take_terminal_menu()?;
        self.teardown_input(InputOwner::TerminalMenu, cx);
        Some(r)
    }

    pub(crate) fn take_rename_input(&mut self, cx: &mut Context<Self>) -> Option<RenameState> {
        let r = self.input.take_rename()?;
        self.teardown_input(InputOwner::Rename, cx);
        Some(r)
    }

    fn teardown_input(&mut self, old: InputOwner, cx: &mut Context<Self>) {
        match old {
            InputOwner::Find => {
                self.find.teardown();
                self.clear_find_matches(cx);
            }
            InputOwner::Rename => {}
            InputOwner::TabMenu | InputOwner::TerminalMenu => {}
            InputOwner::Overlay(OverlayKind::Settings) => {
                self.settings.themes.reset();
            }
            InputOwner::Overlay(OverlayKind::PaneFacts) => {
                self.discard_pane_facts();
            }
            _ => {}
        }
    }

    /// Swallow a key during a modal owner unless it carries the platform
    /// modifier (⌘ on macOS). Global bindings are Cmd-based and fire via
    /// `on_action`, so letting them through keeps ⌘Q / ⌘W / ⌘, live while a
    /// dialog, menu, or overlay is open.
    fn swallow_unless_platform(event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        if !event.keystroke.modifiers.platform {
            cx.stop_propagation();
        }
    }

    fn handle_capture_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.input.owner() {
            InputOwner::Confirm => match event.keystroke.key.as_str() {
                "escape" => {
                    self.confirm_close_cancel(window, cx);
                    cx.stop_propagation();
                }
                "enter" => {
                    self.confirm_close_proceed(window, cx);
                    cx.stop_propagation();
                }
                // Platform-modified keys (⌘Q / ⌘W) still fire via on_action.
                _ => Self::swallow_unless_platform(event, cx),
            },
            InputOwner::Consent => match event.keystroke.key.as_str() {
                "escape" | "enter" => {
                    self.deny_plugin_consent(cx);
                    cx.stop_propagation();
                }
                _ => Self::swallow_unless_platform(event, cx),
            },
            InputOwner::TabMenu => {
                let key = event.keystroke.key.as_str();
                match key {
                    "escape" => {
                        self.dismiss_tab_menu(cx);
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "down" | "up" if !event.keystroke.modifiers.platform => {
                        let selected = self.input.tab_menu().map(|m| m.selected).unwrap_or(0);
                        let count = AppShell::TAB_MENU_ITEM_COUNT;
                        let next = if key == "down" {
                            (selected + 1) % count
                        } else {
                            (selected + count - 1) % count
                        };
                        if let Some(menu) = self.input.tab_menu_mut() {
                            menu.selected = next;
                        }
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "enter" => {
                        let selected = self.input.tab_menu().map(|m| m.selected).unwrap_or(0);
                        self.run_tab_menu_item(selected, window, cx);
                        cx.stop_propagation();
                    }
                    _ => Self::swallow_unless_platform(event, cx),
                }
            }
            InputOwner::TerminalMenu => {
                let key = event.keystroke.key.as_str();
                match key {
                    "escape" => {
                        self.dismiss_terminal_menu(cx);
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "down" | "up" if !event.keystroke.modifiers.platform => {
                        let count = self.terminal_menu_items().len();
                        let selected = self.input.terminal_menu().map(|m| m.selected).unwrap_or(0);
                        let next = if key == "down" {
                            (selected + 1) % count.max(1)
                        } else {
                            (selected + count.max(1) - 1) % count.max(1)
                        };
                        if let Some(menu) = self.input.terminal_menu_mut() {
                            menu.selected = next;
                        }
                        cx.notify();
                        cx.stop_propagation();
                    }
                    "enter" => {
                        let selected = self.input.terminal_menu().map(|m| m.selected).unwrap_or(0);
                        if let Some(item) = self.terminal_menu_items().get(selected).copied() {
                            self.run_terminal_menu_item(item, window, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => Self::swallow_unless_platform(event, cx),
                }
            }
            InputOwner::Overlay(OverlayKind::Update) => {
                if event.keystroke.key.as_str() == "escape" {
                    self.close_update(cx);
                    cx.stop_propagation();
                } else {
                    Self::swallow_unless_platform(event, cx);
                }
            }
            InputOwner::Overlay(OverlayKind::PaneFacts) => {
                // Old behavior: only Escape is intercepted; every other key
                // (including plain typing) falls through to the focused terminal.
                if event.keystroke.key.as_str() == "escape" {
                    self.close_pane_facts(cx);
                    cx.stop_propagation();
                }
            }
            InputOwner::Overlay(OverlayKind::PluginMonitor) => {
                // Same as PaneFacts: only Escape is swallowed; the terminal keeps
                // receiving keys while the monitor is open.
                if event.keystroke.key.as_str() == "escape" {
                    self.close_plugin_monitor(cx);
                    cx.stop_propagation();
                }
            }
            InputOwner::Overlay(OverlayKind::Palette) => {
                if self.palette_key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }
            InputOwner::Overlay(OverlayKind::History) => {
                self.history_key_down(event, window, cx);
                Self::swallow_unless_platform(event, cx);
            }
            InputOwner::Find => {
                if self.find_key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }
            InputOwner::Overlay(OverlayKind::Settings) => {
                if self.settings.section == SettingsSection::Theme {
                    match event.keystroke.key.as_str() {
                        "up" | "arrowup" | "down" | "arrowdown" | "enter" => {
                            self.settings_theme_key_down(event.keystroke.key.as_str(), cx);
                            cx.stop_propagation();
                            return;
                        }
                        "escape" => {
                            if !self.settings.themes.text.is_empty() {
                                self.settings.themes.text.clear();
                                cx.notify();
                            } else {
                                self.close_settings(window, cx);
                            }
                            cx.stop_propagation();
                            return;
                        }
                        _ => {
                            let items = self.theme_item_count(cx);
                            let changed =
                                self.settings.themes.edit(&event.keystroke, items, &mut || {
                                    cx.read_from_clipboard().and_then(|item| item.text())
                                });
                            if changed {
                                self.settings.themes.scroll.scroll_to_item(0);
                                cx.notify();
                            }
                        }
                    }
                }
                if event.keystroke.key.as_str() == "escape" {
                    self.close_settings(window, cx);
                    cx.stop_propagation();
                    return;
                }
                // Swallow other keys while the settings panel is open so they
                // don't reach the terminal underneath. ⌘, (OpenSettings) and the
                // other global bindings still fire via on_action.
                Self::swallow_unless_platform(event, cx);
            }
            InputOwner::Overlay(OverlayKind::Diff) => {
                if self.handle_diff_key(event, window, cx) {
                    cx.stop_propagation();
                    return;
                }
                Self::swallow_unless_platform(event, cx);
            }
            InputOwner::Rename => {
                if self.rename_key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }
            InputOwner::Terminal => {}
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn linux_window_open_diagnostic(source: &str) -> String {
    format!(
        "{source}\nLinux window creation failed. Check WAYLAND_DISPLAY or DISPLAY, \
         install libvulkan1 and mesa-vulkan-drivers, or install the vendor \
         Vulkan driver for your GPU."
    )
}

fn traffic_light_position_for(
    macos: bool,
    position: gpui::Point<Pixels>,
) -> Option<gpui::Point<Pixels>> {
    macos.then_some(position)
}

fn log_window_open_error(err: &impl std::fmt::Display) {
    #[cfg(target_os = "linux")]
    log::error!("{}", linux_window_open_diagnostic(&format!("{err:#}")));

    #[cfg(not(target_os = "linux"))]
    log::error!("failed to open window: {err:#}");
}

fn terminal_window_options(cx: &App) -> WindowOptions {
    let geo = ChromeGeometry::standard();
    let bounds = Bounds::centered(None, size(px(1024.0), px(680.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Sleipnir".into()),
            appears_transparent: true,
            traffic_light_position: traffic_light_position_for(
                cfg!(target_os = "macos"),
                geo.traffic_light_position,
            ),
        }),
        app_owns_titlebar_drag: true,
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(360.0), px(240.0))),
        ..Default::default()
    }
}

fn open_shell_window(
    build: impl FnOnce(&mut Window, &mut App) -> Entity<AppShell>,
    cx: &mut App,
) -> Option<WindowHandle<AppShell>> {
    match cx.open_window(terminal_window_options(cx), build) {
        Ok(handle) => Some(handle),
        Err(err) => {
            log_window_open_error(&err);
            None
        }
    }
}

/// Open a new independent Sleipnir window (startup + ⌘N).
pub fn open_sleipnir_window(cx: &mut App) {
    let _ = try_open_sleipnir_window(cx);
}

/// Open the startup window and report whether GPUI created it successfully.
/// Candidate update health is reported only after this returns `Some`.
pub fn try_open_sleipnir_window(cx: &mut App) -> Option<WindowHandle<AppShell>> {
    open_shell_window(|window, cx| cx.new(|cx| AppShell::new(window, cx)), cx)
}

/// Open a window whose first tab starts in `cwd`. Does not restore a session.
#[cfg(target_os = "macos")]
pub fn open_sleipnir_window_at_cwd(cwd: PathBuf, cx: &mut App) -> Option<WindowHandle<AppShell>> {
    open_shell_window(
        |window, cx| cx.new(|cx| AppShell::new_at_cwd(cwd, window, cx)),
        cx,
    )
}

impl AppShell {
    fn construct(window: &mut Window, cx: &mut Context<Self>) -> Self {
        UpdateModel::init(cx);
        // The outcome is already captured in the model, so terminal
        // transactions are cleared immediately instead of depending on the
        // user pressing the dialog's Close button.
        if matches!(
            cx.global::<UpdateModel>().state,
            UpdateUiState::Updated { .. } | UpdateUiState::RolledBack { .. }
        ) {
            let _ = updater::install::acknowledge_active_outcome();
        }
        let has_update_outcome = matches!(
            cx.global::<UpdateModel>().state,
            UpdateUiState::Updated { .. }
                | UpdateUiState::RolledBack { .. }
                | UpdateUiState::ManualInstallRequired { .. }
                | UpdateUiState::RecoveryRequired { .. }
                | UpdateUiState::Failed(_)
        );
        crate::plugin_runtime::PluginRuntime::init(cx);
        let plugin_commands = crate::plugin_runtime::PluginRuntime::commands(cx);
        let mut palette_items = palette_commands_for(TerminalSettings::get_global(cx).language);
        palette_items.extend(crate::command_palette::plugin_items(&plugin_commands));
        let mut shell = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            next_pane_id: 1,
            focus_handle: cx.focus_handle(),
            should_move: false,
            tab_scroll_handle: ScrollHandle::new(),
            hovered_tab: None,
            tab_drop_target: None,
            pane_rects: Vec::new(),
            content_bounds: None,
            drag: None,
            closed_tabs: Vec::new(),
            input: if has_update_outcome {
                InputMode::Overlay(OverlayKind::Update)
            } else {
                InputMode::Terminal
            },
            mode: UiMode::default(),
            settings: SettingsState::default(),
            palette: PaletteState::new(palette_items, plugin_commands),
            find: find::FindState::default(),
            font_size_override: None,
            bell_flash_tabs: std::collections::HashSet::new(),
            broadcast: false,
            history: HistoryState::default(),
            diff_view: None,
            diff_gen: 0,
            facts: PaneFactsState::default(),
            _quit_subscription: None,
            _housekeeping: gpui::Task::ready(()),
            plugin_chrome: crate::plugin_chrome::ChromeRegistry::default(),
            plugin_watch: crate::plugin_event_watch::PluginEventWatch::default(),
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            browser: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            browser_open: false,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            _browser_bridge: crate::browser_control::start(window, cx),
        };
        // Seed the current system appearance and follow future changes so the
        // `Auto` theme tracks light/dark (ADR-0002).
        TerminalSettings::set_appearance(appearance_of(window.appearance()), cx);
        window
            .observe_window_appearance(|window, cx| {
                TerminalSettings::set_appearance(appearance_of(window.appearance()), cx);
                cx.refresh_windows();
            })
            .detach();

        RunLedgerGlobal::init(cx);
        crate::control_surface::init(cx);
        crate::attention_chrome::refresh(cx);

        // Flush pane-close events on quit.
        shell._quit_subscription = Some(cx.on_app_quit(|this, cx| {
            this.emit_all_panes_closed(cx);
            // Quitting from the update dialog also acknowledges the outcome.
            let _ = updater::install::acknowledge_active_outcome();
            async {}
        }));
        // Resident plugins need a live session to receive events. This is
        // where first-run consent is asked; a grant is never implied.
        shell.start_resident_plugins(cx);

        // Per-window housekeeping: ledger focus + pane-facts refresh run on a
        // fixed 200 ms timer so Render stays paint-only. The task self-terminates
        // once the window/entity is gone (`update_in` starts failing), so it does
        // not outlive a closed window.
        shell._housekeeping = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.sync_ledger_focus(window, cx);
                        this.poll_browser(window, cx);
                        this.refresh_pane_facts_if_stale(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        shell
    }

    /// Every window starts fresh with a single empty tab; sessions are not
    /// persisted, so a window's tab strip is always its own.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut shell = Self::construct(window, cx);
        shell.add_tab(window, cx);
        shell
    }

    /// Fresh window with one tab at `cwd`. Used by Finder "New Window Here".
    #[cfg(target_os = "macos")]
    pub fn new_at_cwd(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut shell = Self::construct(window, cx);
        shell.add_tab_at(Some(cwd), window, cx);
        shell
    }

    pub(crate) fn sync_window_title(&self, window: &mut Window, cx: &App) {
        let title = self
            .tabs
            .get(self.active)
            .map(|t| t.title(cx).to_string())
            .unwrap_or_else(|| "Sleipnir".to_string());
        window.set_window_title(&title);
    }

    /// Create a fresh `TermView` and wire the observers a pane needs
    /// (repaint on change, window-title sync on title change, event routing).
    fn spawn_term_view_with_cwd(
        &mut self,
        cwd: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TermView> {
        self.spawn_term_view(cwd, None, window, cx)
    }

    fn spawn_term_view(
        &mut self,
        cwd: Option<std::path::PathBuf>,
        command: Option<(String, Vec<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TermView> {
        let override_size = self.font_size_override;
        let view = cx.new(|cx| {
            let mut v = TermView::new_local_with(cwd, command, window, cx);
            if override_size.is_some() {
                v.set_font_size_override(override_size, cx);
            }
            v
        });
        self.wire_term_view(&view, window, cx);
        view
    }

    /// Observe a pane's `TermView` so its events route to this AppShell. The
    /// ownership guard makes stale subscriptions harmless once a pane is
    /// detached into another window and re-wired there.
    fn wire_term_view(
        &mut self,
        view: &Entity<TermView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.observe(view, |this, view, cx| {
            // Only *visible* panes may drive a window repaint. A pane in a
            // background tab (or hidden behind pane zoom) is not part of the
            // element tree at all, so notifying for it repaints the whole
            // window for content nobody can see. That matters a lot when a
            // long-running agent streams output in a background tab: without
            // this guard it steals ~250 repaints/second (the PTY event loop
            // coalesces into 4 ms batches) from the pane the user is typing in.
            if this.is_pane_visible(&view) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe_in(
            view,
            window,
            |this, view, event: &crate::TermViewEvent, window, cx| {
                // Events from panes we no longer own (detached) are stale.
                if this.tab_id_for_view(view, cx).is_none() {
                    return;
                }
                match event {
                    crate::TermViewEvent::TitleChanged => {
                        this.sync_window_title(window, cx);
                        cx.notify();
                    }
                    crate::TermViewEvent::Bell => {
                        this.on_term_bell(view, cx);
                    }
                    crate::TermViewEvent::RunStarted {
                        command,
                        cwd,
                        inferred,
                        line,
                        column,
                    } => {
                        if let Some(pane) = this.pane_key_for_view(view) {
                            let cwd = cwd.as_ref().map(|p| p.to_string_lossy().into_owned());
                            let anchor = line.map(|line| run_ledger::Anchor {
                                line,
                                column: column.unwrap_or(0),
                            });
                            this.apply_run_event(
                                RunEvent::Started {
                                    pane,
                                    command: command.clone(),
                                    cwd,
                                    at_ms: 0, // stamped in apply_run_event
                                    inferred: *inferred,
                                    anchor,
                                },
                                cx,
                            );
                        }
                        cx.notify();
                    }
                    crate::TermViewEvent::RunFinished { exit_code } => {
                        if let Some(pane) = this.pane_key_for_view(view) {
                            this.apply_run_event(RunEvent::finished(pane, *exit_code, 0), cx);
                        }
                        cx.notify();
                    }
                    crate::TermViewEvent::ContextMenu { position, link } => {
                        this.set_input(
                            InputMode::TerminalMenu(TerminalMenuState {
                                position: *position,
                                link: link.clone(),
                                selected: 0,
                            }),
                            cx,
                        );
                        cx.notify();
                    }
                    crate::TermViewEvent::UserTyped => {}
                }
            },
        )
        .detach();
    }

    /// Flash the tab that owns `view` when visual bell is enabled.
    fn on_term_bell(&mut self, view: &Entity<TermView>, cx: &mut Context<Self>) {
        use sleipnir_settings::TerminalBell;
        if !matches!(TerminalSettings::get_global(cx).bell, TerminalBell::Visual) {
            return;
        }
        let Some(tab_id) = self.tab_id_for_view(view, cx) else {
            return;
        };
        self.bell_flash_tabs.insert(tab_id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            this.update(cx, |this, cx| {
                this.bell_flash_tabs.remove(&tab_id);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Whether this pane is currently on screen: it must live in the active
    /// tab, and — if that tab has a zoomed pane — be the zoomed one. Mirrors
    /// exactly what [`Self::render_content`] puts into the element tree.
    ///
    /// Used to suppress repaints driven by off-screen panes. Anything the
    /// chrome shows *about* a background pane (tab title, visual bell) arrives
    /// through low-frequency `TermViewEvent`s instead, not through this path.
    fn is_pane_visible(&self, view: &Entity<TermView>) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        leaves
            .iter()
            .find(|(_, leaf)| *leaf == view)
            .is_some_and(|(id, _)| pane_is_on_screen(tab.zoomed_pane, *id))
    }

    fn tab_id_for_view(&self, view: &Entity<TermView>, _cx: &App) -> Option<u64> {
        for tab in &self.tabs {
            let mut leaves = Vec::new();
            tab.tree.leaves(&mut leaves);
            for (_, leaf) in leaves {
                if leaf == view {
                    return Some(tab.id);
                }
            }
        }
        None
    }

    fn pane_key_for_view(&self, view: &Entity<TermView>) -> Option<PaneKey> {
        self.tabs
            .iter()
            .find_map(|tab| tab.tree.pane_key_for_view(view))
    }

    fn active_pane_key(&self) -> Option<PaneKey> {
        let tab = self.tabs.get(self.active)?;
        tab.tree.pane_key_for_id(tab.active_pane)
    }

    fn apply_run_event(&self, mut event: RunEvent, cx: &mut App) {
        if !cx.has_global::<RunLedgerGlobal>() {
            return;
        }
        let host_event = cx.update_global(|g: &mut RunLedgerGlobal, _cx| {
            let at_ms = g.now_ms();
            match &mut event {
                RunEvent::Started { at_ms: slot, .. }
                | RunEvent::Finished { at_ms: slot, .. }
                | RunEvent::PaneClosed { at_ms: slot, .. } => {
                    *slot = at_ms;
                }
            }
            let kind = event.clone();
            g.apply(event);
            plugins::run_event_to_host(&kind, &g.snapshot())
        });
        if let Some(ev) = host_event {
            crate::plugin_runtime::broadcast_event(ev, cx);
        }
        crate::attention_chrome::refresh(cx);
    }

    fn apply_pane_closed(&self, pane: PaneKey, cx: &mut App) {
        self.apply_run_event(RunEvent::PaneClosed { pane, at_ms: 0 }, cx);
    }

    fn emit_all_panes_closed(&self, cx: &mut App) {
        let keys: Vec<PaneKey> = self
            .tabs
            .iter()
            .flat_map(|tab| tab.tree.all_pane_keys())
            .collect();
        for pane in keys {
            self.apply_pane_closed(pane, cx);
        }
    }

    pub(crate) fn sync_ledger_focus(&self, window: &Window, cx: &mut App) {
        if !cx.has_global::<RunLedgerGlobal>() {
            return;
        }
        let pane = self.active_pane_key();
        let active = window.is_window_active();
        cx.update_global(|g: &mut RunLedgerGlobal, _cx| {
            g.set_focus(pane, active);
            if active {
                if let Some(pane) = pane {
                    g.mark_pane_seen(pane);
                }
            }
        });
    }

    pub(crate) fn apply_font_override_to_all_panes(&self, cx: &mut Context<Self>) {
        let size = self.font_size_override;
        for tab in &self.tabs {
            let mut leaves = Vec::new();
            tab.tree.leaves(&mut leaves);
            for (_, view) in leaves {
                view.update(cx, |v, cx| {
                    v.set_font_size_override(size, cx);
                });
            }
        }
    }

    fn step_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        use crate::{FONT_SIZE_MAX, FONT_SIZE_MIN, effective_font_size};
        let current = f32::from(effective_font_size(self.font_size_override, cx));
        let next = (current + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        self.font_size_override = Some(px(next));
        self.apply_font_override_to_all_panes(cx);
        cx.notify();
    }

    fn reset_font_size(&mut self, cx: &mut Context<Self>) {
        self.font_size_override = None;
        self.apply_font_override_to_all_panes(cx);
        cx.notify();
    }

    pub(crate) fn plugin_badges_for_tab(
        &self,
        tab_panes: &[crate::pane_tree::PaneKey],
        tab_is_active: bool,
    ) -> Vec<crate::plugin_chrome::PluginTabBadge> {
        self.plugin_chrome.badges_for_tab(tab_panes, tab_is_active)
    }

    fn rebuild_palette_items(&mut self, cx: &gpui::App) {
        self.palette.items = palette_commands_for(TerminalSettings::get_global(cx).language);
        self.palette
            .items
            .extend(crate::command_palette::plugin_items(
                &self.palette.plugin_commands,
            ));
        self.palette
            .items
            .extend(crate::command_palette::contribution_items(
                self.plugin_chrome.palette_entries(),
            ));
    }

    /// Re-read settings from disk, dropping any in-session font override.
    fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.font_size_override = None;
        self.apply_font_override_to_all_panes(cx);
        TerminalSettings::reload(cx);
        self.refresh_plugin_commands(cx);
        crate::run_ledger_global::RunLedgerGlobal::reload_settings_in(cx);
        crate::control_surface::reload(cx);
        crate::attention_chrome::refresh(cx);
        cx.notify();
    }

    /// Advance to the next built-in theme.
    fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        let next = TerminalSettings::get_global(cx).theme.next();
        TerminalSettings::set_theme(next, cx);
        cx.notify();
    }

    fn on_increase_font_size(
        &mut self,
        _: &IncreaseFontSize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::FONT_SIZE_STEP;
        self.step_font_size(FONT_SIZE_STEP, cx);
    }

    fn on_decrease_font_size(
        &mut self,
        _: &DecreaseFontSize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::FONT_SIZE_STEP;
        self.step_font_size(-FONT_SIZE_STEP, cx);
    }

    fn on_reset_font_size(
        &mut self,
        _: &ResetFontSize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_font_size(cx);
    }

    fn on_new_window(&mut self, _: &NewWindow, _window: &mut Window, cx: &mut Context<Self>) {
        open_sleipnir_window(cx);
    }

    fn on_toggle_pane_zoom(
        &mut self,
        _: &TogglePaneZoom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_pane_zoom(window, cx);
    }

    /// Zoom the active pane to fill the tab, or restore the split layout.
    fn toggle_pane_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        tab.zoomed_pane = match tab.zoomed_pane {
            Some(_) => None,
            None => Some(tab.active_pane),
        };
        self.focus_active(window, cx);
        cx.notify();
    }

    fn on_toggle_broadcast(
        &mut self,
        _: &ToggleBroadcast,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_broadcast(cx);
    }

    /// Fan out keystrokes to every pane in the active tab.
    fn toggle_broadcast(&mut self, cx: &mut Context<Self>) {
        self.broadcast = !self.broadcast;
        cx.notify();
    }

    fn on_jump_prev_prompt(
        &mut self,
        _: &JumpPrevPrompt,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_prompt(-1, cx);
    }

    fn on_jump_next_prompt(
        &mut self,
        _: &JumpNextPrompt,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_prompt(1, cx);
    }

    fn jump_prompt(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        if let Some(term) = view.read(cx).terminal_entity().cloned() {
            let jumped = term.update(cx, |t, _| t.jump_prompt(delta));
            if jumped {
                cx.notify();
            }
        }
    }

    fn on_toggle_quick_select(
        &mut self,
        _: &ToggleQuickSelect,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_quick_select(cx);
    }

    fn toggle_quick_select(&mut self, cx: &mut Context<Self>) {
        self.mode.toggle_quick_select();
        cx.notify();
    }

    fn on_open_quick_terminal(
        &mut self,
        _: &OpenQuickTerminal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Dedicated lightweight window (M15). Same shell stack; user can assign
        // a global hotkey via system settings / `key_bindings`.
        open_sleipnir_window(cx);
    }

    fn on_export_scrollback(
        &mut self,
        _: &ExportScrollback,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_scrollback(cx);
    }

    /// Dump the active pane's scrollback to a temp file and open it.
    fn export_scrollback(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let Some(term) = view.read(cx).terminal_entity().cloned() else {
            return;
        };
        let text = term.read(cx).scrollback_text();

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("sleipnir-scrollback-{stamp}.txt"));
        match std::fs::write(&path, text) {
            Ok(()) => {
                log::info!("exported scrollback to {}", path.display());
                crate::open_existing_path(&path);
                view.update(cx, |v, cx| v.show_toast("scrollback exported", cx));
            }
            Err(err) => {
                log::error!("export scrollback failed: {err:#}");
                view.update(cx, |v, cx| v.show_toast("export scrollback failed", cx));
            }
        }
    }

    /// The active pane's `TermView`, if any.
    pub(crate) fn active_view(&self, _cx: &App) -> Option<Entity<TermView>> {
        self.active_terminal(_cx)
            .or_else(|| self.first_terminal(_cx))
    }

    /// The focused leaf, only if it is a terminal. None when a Panel is focused.
    fn active_terminal(&self, _cx: &App) -> Option<Entity<TermView>> {
        let tab = self.tabs.get(self.active)?;
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        leaves
            .iter()
            .find(|(id, _)| *id == tab.active_pane)
            .map(|(_, v)| (*v).clone())
    }

    fn first_terminal(&self, _cx: &App) -> Option<Entity<TermView>> {
        let tab = self.tabs.get(self.active)?;
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        leaves.first().map(|(_, v)| (*v).clone())
    }

    /// The active pane's terminal entity, for menu actions (copy/paste/clear).
    pub(crate) fn active_terminal_entity(&self, cx: &App) -> Option<Entity<terminal::Terminal>> {
        let view = self.active_view(cx)?;
        view.read(cx).terminal_entity().cloned()
    }

    /// The active pane's working directory, when its PTY reports one. New tabs
    /// and splits inherit this so they open where you are instead of in `$HOME`.
    fn active_working_directory(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.active_view(cx)
            .and_then(|view| view.read(cx).working_directory(cx))
    }

    /// Split the active pane along `axis`, placing a new pane on the far side
    /// and focusing it.
    fn split_active(&mut self, axis: SplitAxis, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.tabs.get(self.active).map(|t| t.active_pane) else {
            return;
        };
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let cwd = self.active_working_directory(cx);
        let view = self.spawn_term_view_with_cwd(cwd, window, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.tree.split(target, axis, new_id, view) {
                tab.active_pane = new_id;
            }
        }
        self.commit_workspace(window, cx);
    }

    /// Move focus to the neighboring pane in `direction`, if one exists.
    fn focus_pane(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(area) = self.content_bounds else {
            return;
        };
        let rects = Self::navigation_rects(&tab.tree, area);
        if let Some(next) = neighbor(&rects, tab.active_pane, direction) {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.active_pane = next;
            }
            self.commit_workspace(window, cx);
        }
    }

    /// Close the active pane. If it is the last pane in the tab, close the tab.
    ///
    /// ⌘W / Shell → Close (handled only on AppShell — never from TermView, which
    /// must not drop itself mid-action):
    /// - multi-pane tab → drop the focused pane, focus a survivor
    /// - single-pane tab → close the tab (shell always keeps ≥1 tab open)
    fn close_active_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        self.close_pane_at(self.active, tab.active_pane, window, cx);
    }

    fn close_pane_at(
        &mut self,
        index: usize,
        target: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let Some(closed_key) = tab.tree.pane_key_for_id(target) else {
            return;
        };
        let closing_terminal = tab.tree.is_terminal_leaf(target);
        let terminals_after = tab
            .tree
            .terminal_count()
            .saturating_sub(usize::from(closing_terminal));
        if crate::plugin_panel::tab_close_policy(terminals_after)
            == crate::plugin_panel::TabClosePolicy::CloseTab
        {
            self.close_tab_at(index, window, cx);
            return;
        }
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        // Only replace focus when closing the focused pane. A confirmation can
        // outlive a focus change or a move to another tab.
        let was_active = tab.active_pane == target;
        let successor = tab.tree.close_successor_id(target);
        let outcome = tab.tree.close(target);
        match outcome {
            CloseOutcome::TreeEmpty => {
                self.close_tab_at(index, window, cx);
            }
            CloseOutcome::NotFound => {}
            CloseOutcome::Closed => {
                self.apply_pane_closed(closed_key, cx);
                if was_active {
                    if let Some(tab) = self.tabs.get_mut(index) {
                        tab.active_pane = successor
                            .filter(|id| tab.tree.contains_leaf(*id))
                            .unwrap_or_else(|| tab.tree.first_leaf_id());
                    }
                }
                self.commit_workspace(window, cx);
            }
        }
    }

    /// Close one plugin Panel leaf from its host-drawn close control (ADR-0017).
    ///
    /// The host owns the surface, so this always works even when the plugin is
    /// wedged or offers no exit of its own. A Panel is never a PTY and never the
    /// last workspace pane (it always shares a tab with a terminal), so closing
    /// it collapses its split and keeps the tab; it never triggers tab close.
    fn close_panel_pane(
        &mut self,
        pane_id: PaneId,
        _pane_key: PaneKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        match tab.tree.close(pane_id) {
            CloseOutcome::Closed => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = tab.tree.first_leaf_id();
                }
                self.commit_workspace(window, cx);
            }
            CloseOutcome::TreeEmpty => {
                self.close_active_tab(window, cx);
            }
            CloseOutcome::NotFound => {}
        }
    }

    /// Whether the active pane (or any pane in the active tab when closing the
    /// last pane) looks dirty for close-confirm.
    fn active_pane_is_dirty(&self, cx: &App) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        // Prefer the focused pane; if it is the only leaf we still check it.
        if let Some((_, view)) = leaves.iter().find(|(id, _)| *id == tab.active_pane) {
            return view.read(cx).looks_busy(cx);
        }
        leaves.iter().any(|(_, view)| view.read(cx).looks_busy(cx))
    }

    /// Foreground process name of the pane that would be closed, when it is busy.
    fn active_busy_process_name(&self, cx: &App) -> Option<String> {
        let tab = self.tabs.get(self.active)?;
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        let view = leaves
            .iter()
            .find(|(id, _)| *id == tab.active_pane)
            .or_else(|| leaves.first())
            .map(|(_, view)| *view)?;
        if !view.read(cx).looks_busy(cx) {
            return None;
        }
        view.read(cx).foreground_process_command_name(cx)
    }

    fn mark_active_tab_seen(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let keys = tab.tree.all_pane_keys();
        if !cx.has_global::<RunLedgerGlobal>() {
            return;
        }
        cx.update_global(|g: &mut RunLedgerGlobal, _cx| {
            for pane in keys {
                g.mark_pane_seen(pane);
            }
        });
        crate::attention_chrome::refresh(cx);
        cx.notify();
    }

    /// Request close of a specific terminal pane via the same user-policy path
    /// as ⌘W. Activates the pane so `confirm_close` / busy-process copy apply
    /// to it. Does not force-close. `Ok` means the request was accepted; the
    /// pane may still be open if a confirm modal is showing.
    pub(crate) fn request_close_terminal_pane(
        &mut self,
        pane: PaneKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.input.confirm().is_some() {
            return Err("a close confirmation is already pending".into());
        }
        let found = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(ix, tab)| tab.tree.pane_id_for_key(pane).map(|id| (ix, id)));
        let Some((ix, id)) = found else {
            return Err(format!("pane {pane} not found"));
        };
        self.activate(ix, window, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.active_pane = id;
        }
        self.commit_workspace(window, cx);
        self.request_close_active_pane(window, cx);
        Ok(())
    }

    /// Gate close on `confirm_close` setting; may open a modal instead of closing.
    fn request_close_active_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.confirm().is_some() {
            return;
        }
        let Some(target) = self.active_pane_key() else {
            return;
        };
        let policy = TerminalSettings::get_global(cx).confirm_close;
        let needs_confirm = match policy {
            ConfirmClose::Never => false,
            ConfirmClose::Always => true,
            ConfirmClose::Dirty => self.active_pane_is_dirty(cx),
        };
        if needs_confirm {
            let name = self.active_busy_process_name(cx);
            let message = if policy == ConfirmClose::Dirty || name.is_some() {
                crate::chrome::close_copy::close_confirm_message(name.as_deref())
            } else {
                "Close this pane anyway?".into()
            };
            self.set_input(
                InputMode::Confirm(CloseConfirmState {
                    message: message.into(),
                    kind: ConfirmKind::ClosePane(target),
                }),
                cx,
            );
            cx.notify();
        } else {
            self.close_active_pane(window, cx);
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn request_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.confirm().is_some() {
            return;
        }
        let policy = TerminalSettings::get_global(cx).confirm_close;
        let needs_confirm = match policy {
            ConfirmClose::Never => false,
            ConfirmClose::Always => true,
            ConfirmClose::Dirty => self.any_pane_is_dirty(cx),
        };
        if needs_confirm {
            self.set_input(
                InputMode::Confirm(CloseConfirmState {
                    message: "A process is still running. Close this window anyway?".into(),
                    kind: ConfirmKind::CloseWindow,
                }),
                cx,
            );
            cx.notify();
        } else {
            self.finish_window_close(window, cx);
        }
    }

    #[cfg(target_os = "linux")]
    fn any_pane_is_dirty(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| {
            let mut leaves = Vec::new();
            tab.tree.leaves(&mut leaves);
            leaves.iter().any(|(_, view)| view.read(cx).looks_busy(cx))
        })
    }

    #[cfg(target_os = "linux")]
    fn finish_window_close(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.emit_all_panes_closed(cx);
        window.remove_window();
    }

    fn confirm_close_proceed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.take_confirm_input(cx).map(|s| s.kind);
        match kind {
            Some(ConfirmKind::CloseTab(tab_id)) => {
                if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
                    self.close_tab_at(index, window, cx);
                }
            }
            #[cfg(target_os = "linux")]
            Some(ConfirmKind::CloseWindow) => self.finish_window_close(window, cx),
            Some(kind @ ConfirmKind::ClosePane(_)) => {
                if let Some((index, target)) = kind.pane_target(&self.tabs) {
                    self.close_pane_at(index, target, window, cx);
                }
            }
            None => {}
        }
        cx.notify();
    }

    fn on_send_selection(
        &mut self,
        _: &SendSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_selection_to_pty(cx);
    }

    fn on_pipe_selection(
        &mut self,
        _: &PipeSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pipe_selection(cx);
    }

    fn on_toggle_history_search(
        &mut self,
        _: &ToggleHistorySearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command(CommandId::ToggleHistorySearch, window, cx);
    }

    /// Toggle the history overlay, resetting the query when it closes.
    fn toggle_history_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.toggle_overlay(OverlayKind::History, cx) {
            self.history.input.reset();
            self.focus_active(window, cx);
        } else {
            self.history.load_hits();
            // Focus the shell so the history query box's IME input handler
            // activates and keystrokes stop leaking to the PTY underneath.
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    /// Send the selected history hit to the active pane and close the overlay.
    pub(crate) fn run_history_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.history.shown();
        let Some(hit) = shown.get(
            self.history
                .input
                .selected
                .min(shown.len().saturating_sub(1)),
        ) else {
            return;
        };
        let cmd = hit.command.clone();
        if let Some(view) = self.active_view(cx) {
            view.update(cx, |v, cx| v.input_bytes(cmd.into_bytes(), cx));
        }
        self.toggle_history_search(window, cx);
    }

    /// One keystroke while the history overlay is open. Always consumes the
    /// key (non-platform) so nothing leaks to the PTY underneath.
    pub(super) fn history_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                self.toggle_history_search(window, cx);
            }
            "enter" => {
                self.run_history_selection(window, cx);
            }
            _ => {
                let shown = self.history.shown().len();
                let changed = self.history.input.edit(&event.keystroke, shown, &mut || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
                if changed {
                    cx.notify();
                }
            }
        }
    }

    fn send_selection_to_pty(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let Some(text) = crate::chrome::send_context::selection_payload(
            &view.read(cx).selection_text(cx).unwrap_or_default(),
        ) else {
            return;
        };
        view.update(cx, |v, cx| v.input_bytes(text.into_bytes(), cx));
        cx.notify();
    }

    fn pipe_selection(&mut self, cx: &mut Context<Self>) {
        let settings = TerminalSettings::get_global(cx);
        let Some(template) = settings.pipe_selection_command.clone() else {
            return;
        };
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let Some(payload) = crate::chrome::send_context::selection_payload(
            &view.read(cx).selection_text(cx).unwrap_or_default(),
        ) else {
            return;
        };
        let Ok(argv) = crate::chrome::send_context::format_pipe_command(&template, &payload) else {
            return;
        };
        if argv.is_empty() {
            return;
        }
        let program = argv[0].clone();
        let args: Vec<String> = argv.into_iter().skip(1).collect();
        let _ = std::process::Command::new(program).args(args).spawn();
    }

    fn jump_to_ledger_row(
        &mut self,
        pane: run_ledger::PaneKey,
        run_id: Option<run_ledger::RunId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = self
            .tabs
            .iter()
            .enumerate()
            .find_map(|(ix, tab)| tab.tree.pane_id_for_key(pane).map(|id| (ix, id)));
        let Some((ix, id)) = found else {
            return;
        };
        self.activate(ix, window, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.active_pane = id;
        }
        self.focus_active(window, cx);
        let anchor = run_id.and_then(|id| {
            if !cx.has_global::<RunLedgerGlobal>() {
                return None;
            }
            cx.global::<RunLedgerGlobal>()
                .snapshot()
                .into_iter()
                .find(|run| run.id == id)
                .and_then(|run| run.anchor)
        });
        if let Some(anchor) = anchor {
            if let Some(view) = self.view_for_pane(pane) {
                view.update(cx, |v, cx| {
                    v.scroll_to_anchor(anchor.line, anchor.column, cx)
                });
            }
        }
        crate::attention_chrome::refresh(cx);
    }

    fn view_for_pane(&self, pane: run_ledger::PaneKey) -> Option<Entity<TermView>> {
        for tab in &self.tabs {
            let mut out = Vec::new();
            tab.tree.leaves_with_keys(&mut out);
            if let Some((_, view)) = out.into_iter().find(|(key, _)| *key == pane) {
                return Some(view);
            }
        }
        None
    }

    pub(crate) fn all_live_panes(&self) -> Vec<(PaneKey, Entity<TermView>)> {
        let mut out = Vec::new();
        for tab in &self.tabs {
            tab.tree.leaves_with_keys(&mut out);
        }
        out
    }

    fn on_mark_tab_seen(&mut self, _: &MarkTabSeen, _window: &mut Window, cx: &mut Context<Self>) {
        self.mark_active_tab_seen(cx);
    }

    fn confirm_close_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.take_confirm_input(cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(window, cx);
    }

    fn on_reopen_closed_tab(
        &mut self,
        _: &ReopenClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reopen_closed_tab(window, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close_active_pane(window, cx);
    }

    fn on_split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.split_active(SplitAxis::Horizontal, window, cx);
    }

    fn on_split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.split_active(SplitAxis::Vertical, window, cx);
    }

    fn on_focus_pane_left(
        &mut self,
        _: &FocusPaneLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(Direction::Left, window, cx);
    }

    fn on_focus_pane_right(
        &mut self,
        _: &FocusPaneRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(Direction::Right, window, cx);
    }

    fn on_focus_pane_up(&mut self, _: &FocusPaneUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pane(Direction::Up, window, cx);
    }

    fn on_focus_pane_down(
        &mut self,
        _: &FocusPaneDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(Direction::Down, window, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.next_tab(window, cx);
    }

    fn on_prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.prev_tab(window, cx);
    }

    fn on_activate_tab(
        &mut self,
        action: &ActivateTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 1-based index from ⌘1..⌘9; ignore out-of-range.
        if let Some(index) = action.0.checked_sub(1) {
            if index < self.tabs.len() {
                self.activate(index, window, cx);
            }
        }
    }

    fn on_reload_settings(
        &mut self,
        _: &ReloadSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reload_settings(cx);
    }

    fn on_cycle_theme(&mut self, _: &CycleTheme, _window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_theme(cx);
    }

    // ── command palette (M9) ────────────────────────────────────────────────

    fn on_toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.input.is_overlay(OverlayKind::Palette) {
            self.close_palette(window, cx);
        } else {
            self.open_palette(window, cx);
        }
    }

    // ── find in scrollback (M10) ────────────────────────────────────────────
}

/// Whether a pane of the *active* tab is actually painted, given the tab's
/// zoom state. Pane zoom shows exactly one leaf, so every other leaf is
/// off-screen even though it is still in the tree and still draining its PTY.
///
/// This is the render contract [`AppShell::render_content`] implements; keep
/// the two in sync, because [`AppShell::is_pane_visible`] uses this to decide
/// whether a pane's change may request a window repaint.
fn pane_is_on_screen(zoomed: Option<PaneId>, pane: PaneId) -> bool {
    match zoomed {
        Some(zoomed_pane) => zoomed_pane == pane,
        None => true,
    }
}

/// Insertion index for a tab-drag reorder: after removing the tab at `from`,
/// dropping it on `target` places it immediately before the target. When the
/// dragged tab sits left of the target, the target shifts left by one.
fn reorder_insert_index(from: usize, target: usize) -> usize {
    if from < target { target - 1 } else { target }
}

/// Identity assigned to a tab that just landed in a fresh window.
///
/// Source-window tab ids keep growing. A new `AppShell` starts `next_id` at 1,
/// so keeping the old id lets a later `add_tab` collide. Rebase the tab to 1
/// and advance both counters past the adopted tree. Pane ids stay as-is
/// (they remain unique inside the tree).
struct AdoptedTabIds {
    tab_id: u64,
    next_id: u64,
    next_pane_id: PaneId,
}

fn rebase_detached_tab(max_pane_id: PaneId) -> AdoptedTabIds {
    AdoptedTabIds {
        tab_id: 1,
        next_id: 2,
        next_pane_id: max_pane_id.saturating_add(1),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        linux_window_open_diagnostic, pane_is_on_screen, rebase_detached_tab, reorder_insert_index,
        traffic_light_position_for,
    };
    use gpui::{point, px};

    #[test]
    fn safe_window_close_finishes_runtime_before_removal() {
        let src = include_str!("mod.rs");
        let needle = ["fn finish_window_", "close("].concat();
        let method = src
            .split(&needle)
            .nth(1)
            .expect("shared window-close finalizer");
        let body = method.split("\n    }").next().expect("finalizer body");
        for required in ["emit_all_panes_closed(cx)", "window.remove_window()"] {
            assert!(
                body.contains(required),
                "close finalizer missing {required}"
            );
        }
    }

    #[test]
    fn traffic_lights_are_only_positioned_on_macos() {
        let position = point(px(12.0), px(12.0));
        assert_eq!(traffic_light_position_for(true, position), Some(position));
        assert_eq!(traffic_light_position_for(false, position), None);
    }

    #[test]
    fn linux_window_open_diagnostic_keeps_source_and_actionable_hints() {
        let message = linux_window_open_diagnostic("Vulkan adapter unavailable");
        assert!(message.contains("Vulkan adapter unavailable"));
        assert!(message.contains("WAYLAND_DISPLAY"));
        assert!(message.contains("DISPLAY"));
        assert!(message.contains("libvulkan1"));
        assert!(message.contains("mesa-vulkan-drivers"));
        assert!(message.contains("vendor Vulkan driver"));
    }

    #[test]
    fn every_leaf_is_on_screen_without_zoom() {
        assert!(pane_is_on_screen(None, 1));
        assert!(pane_is_on_screen(None, 2));
    }

    #[test]
    fn zoom_hides_every_pane_but_the_zoomed_one() {
        // Pane 2 is zoomed: 1 and 3 keep draining their PTYs but are not
        // painted, so their output must not request a window repaint.
        assert!(pane_is_on_screen(Some(2), 2));
        assert!(!pane_is_on_screen(Some(2), 1));
        assert!(!pane_is_on_screen(Some(2), 3));
    }

    #[test]
    fn reorder_insert_index_places_before_target() {
        // [A, B, C, D]: drag A onto C → insert at 1 → [B, A, C, D].
        assert_eq!(reorder_insert_index(0, 2), 1);
        // [A, B, C, D]: drag D onto B → insert at 1 → [A, D, B, C].
        assert_eq!(reorder_insert_index(3, 1), 1);
        // Drag onto the immediate right neighbour: stays in place.
        assert_eq!(reorder_insert_index(0, 1), 0);
        // Drag onto the immediate left neighbour: lands just before it.
        assert_eq!(reorder_insert_index(1, 0), 0);
    }

    #[test]
    fn rebase_detached_tab_restarts_ids_in_the_new_window() {
        // A high source-window id (tab 5, panes up to 7) must not leak into
        // the destination window's counters — next add_tab / split would collide.
        let ids = rebase_detached_tab(7);
        assert_eq!(ids.tab_id, 1);
        assert_eq!(ids.next_id, 2);
        assert_eq!(ids.next_pane_id, 8);
    }

    #[test]
    fn rebase_detached_tab_advances_pane_counter_past_a_single_leaf() {
        let ids = rebase_detached_tab(1);
        assert_eq!(ids.tab_id, 1);
        assert_eq!(ids.next_id, 2);
        assert_eq!(ids.next_pane_id, 2);
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let window_active = window.is_window_active();
        let tokens = ChromeTokens::from_palette(&palette, window_active);
        let fullscreen = window.is_fullscreen();
        let geo = ChromeGeometry::for_window(cfg!(not(target_os = "macos")), fullscreen);
        let leading = geo.leading_pad;
        let chrome_h = geo.height;
        let banner_top = chrome_h;
        let show_tab_strip = self.tabs.len() > 1;
        self.sync_browser_presentation(cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(tokens.content_bg)
            // Clip content to the macOS window corner radius so the opaque
            // terminal background follows the window's rounded corners
            // (no clipping in fullscreen, where the window has square corners).
            .when(!fullscreen, |el| {
                el.rounded(geo.window_radius).overflow_hidden()
            })
            .track_focus(&self.focus_handle)
            .key_context("AppShell")
            // Intercept keys during overlays / rename before the focused terminal
            // sees them (capture phase runs top-down).
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                this.handle_capture_key(event, window, cx);
            }))
            // Clicking anywhere else (terminal, another tab) commits the
            // in-progress rename.
            .capture_any_mouse_down(cx.listener(
                |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.reclaim_browser_focus(cx);
                    if matches!(this.input, InputMode::Rename(_))
                        && event.button == MouseButton::Left
                    {
                        this.commit_rename(window, cx);
                    }
                },
            ))
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_reopen_closed_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_activate_tab))
            .on_action(cx.listener(Self::on_reload_settings))
            .on_action(cx.listener(Self::on_cycle_theme))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_split_right))
            .on_action(cx.listener(Self::on_split_down))
            .on_action(cx.listener(Self::on_focus_pane_left))
            .on_action(cx.listener(Self::on_focus_pane_right))
            .on_action(cx.listener(Self::on_focus_pane_up))
            .on_action(cx.listener(Self::on_focus_pane_down))
            .on_action(cx.listener(Self::on_check_for_updates))
            .on_action(cx.listener(Self::on_toggle_command_palette))
            .on_action(cx.listener(Self::on_find))
            .on_action(cx.listener(Self::on_find_next))
            .on_action(cx.listener(Self::on_find_prev))
            .on_action(cx.listener(Self::on_increase_font_size))
            .on_action(cx.listener(Self::on_decrease_font_size))
            .on_action(cx.listener(Self::on_reset_font_size))
            .on_action(cx.listener(Self::on_new_window))
            .on_action(cx.listener(Self::on_toggle_pane_zoom))
            .on_action(cx.listener(Self::on_toggle_broadcast))
            .on_action(cx.listener(Self::on_jump_prev_prompt))
            .on_action(cx.listener(Self::on_jump_next_prompt))
            .on_action(cx.listener(Self::on_toggle_quick_select))
            .on_action(cx.listener(Self::on_open_quick_terminal))
            .on_action(cx.listener(Self::on_export_scrollback))
            .on_action(cx.listener(Self::on_toggle_plugin_monitor))
            .on_action(cx.listener(Self::on_toggle_browser))
            .on_action(cx.listener(Self::on_mark_tab_seen))
            .on_action(cx.listener(Self::on_toggle_pane_facts))
            .on_action(cx.listener(Self::on_send_selection))
            .on_action(cx.listener(Self::on_pipe_selection))
            .on_action(cx.listener(Self::on_send_git_diff))
            .on_action(cx.listener(Self::on_toggle_diff))
            .on_action(cx.listener(Self::on_toggle_history_search))
            .child({
                let leading_drag = self
                    .attach_empty_drag("chrome-drag-leading", cx)
                    .h_full()
                    .w(leading);
                let trailing_drag = self
                    .attach_empty_drag("chrome-drag-trailing", cx)
                    .h_full()
                    .flex_1()
                    .min_w(geo.trailing_pad);
                let tab_scroll =
                    show_tab_strip.then(|| self.render_tab_strip(&tokens, &geo, window, cx));
                let chrome_band = div()
                    .id("chrome-band")
                    .h(chrome_h)
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .bg(tokens.content_bg)
                    .child(leading_drag)
                    .children(tab_scroll)
                    // The chip lives at the trailing end of the chrome band, not
                    // beside the traffic lights: with a single tab `tab_scroll`
                    // is `None`, so anything placed here collapses left into the
                    // macOS window-control and drag region.
                    .child(trailing_drag)
                    .when(
                        cfg!(any(target_os = "macos", target_os = "windows")),
                        |el| el.child(self.render_browser_toggle(&tokens, cx)),
                    )
                    .child(self.render_plugin_status_chip(&tokens, cx))
                    .child(self.render_plugin_chrome_status(&tokens, window, cx))
                    .when(!fullscreen, |el| {
                        el.child(self.render_desktop_titlebar_end(&tokens, window, cx))
                    });
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(chrome_band)
                    .when(self.input.is_find(), |el| {
                        el.child(self.render_find_bar(&tokens, cx))
                    })
                    .child({
                        let agent_panel_enabled =
                            sleipnir_settings::TerminalSettings::get_global(cx)
                                .plugins
                                .agent_panel;
                        let has_agents = agent_panel_enabled && !self.agent_hud_rows(cx).is_empty();
                        let content = self.render_content(&tokens, window, cx);
                        if has_agents || self.browser_panel_is_open() {
                            div()
                                .flex_1()
                                .min_h_0()
                                .flex()
                                .flex_row()
                                .child(div().flex_1().min_w_0().size_full().child(content))
                                .when(has_agents, |el| {
                                    el.child(self.render_agent_panel(&tokens, cx))
                                })
                                .when(self.browser_panel_is_open(), |el| {
                                    el.child(self.render_browser_panel(&tokens, window, cx))
                                })
                                .into_any_element()
                        } else {
                            content
                        }
                    })
            })
            .when(self.broadcast, |el| {
                el.child(
                    div()
                        .id("broadcast-banner")
                        .absolute()
                        .top(banner_top)
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded(px(0.0))
                                .bg(tokens.accent.opacity(0.9))
                                .text_size(px(12.0))
                                .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                                .child("Broadcast on · input goes to all panes"),
                        ),
                )
            })
            .when(self.mode.quick_select_open, |el| {
                el.child(
                    div()
                        .id("quick-select-banner")
                        .absolute()
                        .bottom(px(12.0))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_3()
                                .py_1()
                                .rounded(px(0.0))
                                .bg(tokens.surface)
                                .border(pixel::PIXEL_BORDER)
                                .border_color(tokens.accent)
                                .text_size(px(12.0))
                                .text_color(tokens.fg)
                                .child(format!(
                                    "Quick Select · Esc to close · click links with {}",
                                    crate::display_shortcut("secondary_click")
                                )),
                        ),
                )
            })
            .when(self.input.tab_menu().is_some(), |el| {
                el.child(self.render_tab_menu(&tokens, window, cx))
            })
            .when(self.input.terminal_menu().is_some(), |el| {
                el.child(self.render_terminal_menu(&tokens, window, cx))
            })
            .when(self.input.is_overlay(OverlayKind::Settings), |el| {
                el.child(self.render_settings_overlay(&tokens, window, cx))
            })
            .when(self.input.is_overlay(OverlayKind::Update), |el| {
                el.child(self.render_update_overlay(&tokens, cx))
            })
            .when(self.input.is_overlay(OverlayKind::Palette), |el| {
                el.child(self.render_command_palette(&tokens, cx))
            })
            .when(self.input.confirm().is_some(), |el| {
                el.child(self.render_close_confirm(&tokens, cx))
            })
            .when(self.input.is_overlay(OverlayKind::PaneFacts), |el| {
                el.child(self.render_pane_facts(&tokens, cx))
            })
            .when(self.input.is_overlay(OverlayKind::PluginMonitor), |el| {
                el.child(self.render_plugin_monitor(&tokens, cx))
            })
            .when(self.input.consent().is_some(), |el| {
                el.child(self.render_plugin_consent(&tokens, cx))
            })
            .when(self.input.is_overlay(OverlayKind::History), |el| {
                el.child(self.render_history_search(&tokens, cx))
            })
            .when(self.input.is_overlay(OverlayKind::Diff), |el| {
                el.child(self.render_diff_overlay(&tokens, &palette, window, cx))
            })
    }
}

#[cfg(test)]
mod workspace_regression_tests {
    use super::{ConfirmKind, PaneKey, Tab};
    use crate::pane_tree::{CloseOutcome, PaneNode, SplitAxis};

    fn demo_surface() -> crate::plugin_panel::PanelSurface {
        crate::plugin_panel::PanelSurface {
            plugin_id: "test".into(),
            owner_instance_id: uuid::Uuid::nil(),
            pane_key: PaneKey::new_v4(),
            surface_id: uuid::Uuid::new_v4(),
            tree: plugin_protocol::v2::Widget::Text {
                s: "test".into(),
                fg: plugin_protocol::v2::Tone::Fg,
                bold: false,
            },
            stale: false,
        }
    }

    fn tab(id: u64, pane_id: u64, key: PaneKey) -> Tab {
        Tab {
            id,
            tree: PaneNode::panel_leaf(pane_id, key, demo_surface()),
            active_pane: pane_id,
            custom_title: None,
            zoomed_pane: None,
        }
    }

    #[test]
    fn workspace_regression_pending_close_keeps_original_target_after_focus_changes() {
        let first = PaneKey::new_v4();
        let second = PaneKey::new_v4();
        let mut tabs = vec![tab(1, 10, first)];
        tabs[0].tree = PaneNode::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(tabs[0].tree.clone()),
            second: Box::new(PaneNode::panel_leaf(20, second, demo_surface())),
        };
        let pending = ConfirmKind::ClosePane(first);
        // A second request changes focus while the first dialog is pending.
        tabs[0].active_pane = 20;
        let (owner, target) = pending.pane_target(&tabs).unwrap();
        assert_eq!(target, 10);
        assert_eq!(tabs[owner].tree.close(target), CloseOutcome::Closed);
        assert_eq!(tabs[0].tree.pane_id_for_key(first), None);
        assert_eq!(tabs[0].tree.pane_id_for_key(second), Some(20));
    }

    #[test]
    fn workspace_regression_pending_close_resolves_current_owner_and_local_id() {
        let key = PaneKey::new_v4();
        let pending = ConfirmKind::ClosePane(key);
        let mut tabs = vec![tab(1, 10, key), tab(2, 20, PaneKey::new_v4())];
        assert_eq!(pending.pane_target(&tabs), Some((0, 10)));
        // A transfer can change both the owning tab and its local pane id.
        tabs[0] = tab(1, 10, PaneKey::new_v4());
        tabs[1] = tab(2, 30, key);
        assert_eq!(pending.pane_target(&tabs), Some((1, 30)));
        tabs.swap(0, 1);
        assert_eq!(pending.pane_target(&tabs), Some((0, 30)));
    }

    #[test]
    fn workspace_regression_zoom_navigation_uses_current_tree_after_split() {
        let mut tab = tab(1, 10, PaneKey::new_v4());
        tab.zoomed_pane = Some(10);
        tab.tree = PaneNode::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(tab.tree),
            second: Box::new(PaneNode::panel_leaf(20, PaneKey::new_v4(), demo_surface())),
        };
        tab.active_pane = 20;
        tab.reconcile_pane_focus();
        let area = gpui::Bounds::new(
            gpui::point(gpui::px(0.0), gpui::px(0.0)),
            gpui::size(gpui::px(800.0), gpui::px(600.0)),
        );
        let rects = super::AppShell::navigation_rects(&tab.tree, area);
        let next =
            crate::pane_tree::neighbor(&rects, tab.active_pane, crate::pane_tree::Direction::Left)
                .unwrap();
        assert_eq!(next, 10);
        tab.active_pane = next;
        tab.reconcile_pane_focus();
        assert_eq!(tab.zoomed_pane, Some(10));
    }

    #[test]
    fn workspace_regression_missing_close_target_never_falls_back_to_active() {
        let key = PaneKey::new_v4();
        let pending = ConfirmKind::ClosePane(key);
        let tabs = vec![tab(1, 10, PaneKey::new_v4())];
        assert_eq!(pending.pane_target(&tabs), None);
        assert_eq!(ConfirmKind::CloseTab(1).pane_target(&tabs), None);
        assert_eq!(tabs[0].active_pane, 10);
    }
}
