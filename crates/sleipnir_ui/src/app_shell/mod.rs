//! Single-window multi-tab shell for sleipnir (HIG-aligned chrome).

/// Maps `CommandId` to canonical shell actions. A child module so it can reach
/// `AppShell`'s private methods without widening them to the whole crate.
mod command_dispatch;
mod diff;
mod find;
mod layout;
mod palette;
mod panels;
mod persist;
mod settings;
mod tabs;
mod update;

use gpui::{
    App, AppContext as _, BorrowAppContext, Bounds, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels,
    Render, ScrollHandle, SharedString, Styled as _, Task, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions, actions, div,
    prelude::FluentBuilder as _, px, size,
};
use run_ledger::{PaneKey, RunEvent};
use sleipnir_settings::{Appearance, ConfirmClose, TerminalPalette, TerminalSettings};
use std::path::PathBuf;

use crate::chrome::{ChromeGeometry, ChromeTokens};
use crate::command_palette::{CommandId, CommandItem, commands as palette_commands};
use crate::pane_tree::{
    CloseOutcome, Direction, PaneId, PaneNode, PaneRect, SplitAxis, SplitPath, neighbor,
};
use crate::run_ledger_global::RunLedgerGlobal;
use crate::session::{SessionAxis, SessionNode};
pub(crate) use crate::tab_convert::Tab;
use crate::ui_mode::{OverlayKind, PaneFactsState, UiMode};
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
        /// Clear the Run Ledger (memory + runs.json). Palette / menu only.
        ClearRunLedger,
        /// Toggle the Run Ledger panel (P1). Name is registered for key_bindings.
        ToggleRunLedger,
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
            .rounded(px(6.0))
            .bg(tokens.hover)
            .border_1()
            .border_color(tokens.border)
            .shadow_lg()
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
        if let Some((_, _, crate::LeafContent::Panel { plugin_id })) = active {
            return plugin_id.clone().into();
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

/// In-progress inline tab rename triggered by a right-click on a tab.
#[derive(Clone)]
pub(crate) struct RenameState {
    pub(crate) tab_id: u64,
    pub(crate) buffer: String,
}

/// Top-level section inside the settings panel (WezTerm-style tabs).
/// Add variants here as new setting pages land.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SettingsSection {
    #[default]
    Theme,
    /// Session restore, ligatures, and other app/terminal toggles.
    General,
    /// Read-only reference for the shortcuts shipped on the current platform.
    Shortcuts,
}

impl SettingsSection {
    const ALL: &'static [SettingsSection] = &[
        SettingsSection::Theme,
        SettingsSection::General,
        SettingsSection::Shortcuts,
    ];

    fn id(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
            SettingsSection::General => "general",
            SettingsSection::Shortcuts => "shortcuts",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
            SettingsSection::General => "general",
            SettingsSection::Shortcuts => "shortcuts",
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
    /// Pane rects from the last render, for keyboard neighbor navigation.
    pane_rects: Vec<PaneRect>,
    /// Content area bounds captured last frame (origin + size), for analytic
    /// pane layout and divider hit-testing.
    content_bounds: Option<Bounds<Pixels>>,
    /// Active divider drag, if any.
    drag: Option<DragState>,
    /// In-progress inline tab rename, if any.
    pub(crate) rename: Option<RenameState>,
    /// Which modal overlay is showing, plus the transient find / quick-select
    /// modes. Replaces the old one-bool-per-overlay matrix, so illegal
    /// combinations are unrepresentable.
    pub(crate) mode: UiMode,
    /// Active section tab inside the settings panel.
    settings_section: SettingsSection,
    /// Type-to-filter query for the theme picker (empty = all).
    theme_query: String,
    palette_query: String,
    palette_selected: usize,
    palette_items: Vec<CommandItem>,
    plugin_commands: Vec<plugin_host::LoadedPluginCommand>,
    find_query: String,
    /// Monotonic request id used to discard stale asynchronous search results.
    find_gen: u64,
    find_match_count: usize,
    find_active_index: usize,
    /// Regex mode: treat the query as a raw regex instead of a literal (⌥⌘R).
    find_regex: bool,
    /// Match case (⌥⌘C). Off = case-insensitive; on = case-sensitive.
    find_match_case: bool,
    /// Window-scoped font size override (M12 zoom); not written to settings.
    pub(crate) font_size_override: Option<Pixels>,
    /// Close-confirm dialog pending (M12).
    close_confirm: Option<CloseConfirmState>,
    /// Consent prompt pending. Copied off `plugin_grants::check`; the overlay
    /// never borrows the grants file. Approve writes a grant; Deny writes nothing.
    plugin_consent: Option<PluginConsentPending>,
    /// Tab ids currently flashing for visual bell (M12).
    pub(crate) bell_flash_tabs: std::collections::HashSet<u64>,
    /// Fan-out keystrokes to all panes in the active tab (M13).
    broadcast: bool,
    /// Focused-pane facts: async collection state machine. Carries its own
    /// snapshot timestamp and in-flight flag.
    facts: PaneFactsState,
    history_query: String,
    history_selected: usize,
    /// Git diff inspector (ADR-0012). Not a Pane.
    pub(crate) diff_view: Option<crate::diff::DiffView>,
    diff_gen: u64,
    /// Debounced session save task.
    _session_save_task: Option<Task<()>>,
    /// Keep the app-quit subscription alive for the window lifetime.
    _quit_subscription: Option<gpui::Subscription>,
    /// Last-seen pane facts for plugin events. Polled, never per-frame.
    plugin_watch: crate::plugin_event_watch::PluginEventWatch,
    /// Host-owned plugin Panel surfaces (ADR-0017). Keyed by pane_key.
    plugin_panels: crate::plugin_panel::PanelRegistry,
    /// Per-plugin HostCall rate limiter (ADR-0016 §3). UI-side so a resident
    /// plugin cannot spam Notify / OpenPane; drops are shown on the Monitor.
    plugin_calls: crate::plugin_host_calls::HostCallLimiter,
    /// Chrome contributions (ADR-0017 status mount).
    plugin_chrome: crate::plugin_chrome::ChromeRegistry,
}

/// What the shared confirm dialog is asking about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmKind {
    ClosePane,
    #[cfg(target_os = "linux")]
    CloseWindow,
    ClearRunLedger,
}

/// Pending confirmation dialog (close pane, or clear the Run Ledger).
struct CloseConfirmState {
    /// Human-readable what will happen.
    message: SharedString,
    kind: ConfirmKind,
}

/// Enough to finish an invoke after the user approves. The dialog itself
/// renders [`crate::plugin_monitor_panel::ConsentPrompt`] only.
struct PluginConsentPending {
    prompt: crate::plugin_monitor_panel::ConsentPrompt,
    plugin: plugin_host::LoadedPluginCommand,
    hash: String,
    request: Vec<plugin_protocol::v2::Capability>,
}

impl Focusable for AppShell {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for AppShell {}

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

/// Open a new window and move `tab` into it (detach tab to a new window).
/// The tab's panes keep their live PTYs; observers are re-wired to the new
/// window's `AppShell`.
fn open_sleipnir_window_with_tab(tab: Tab, cx: &mut App) {
    open_shell_window(
        move |window, cx| {
            cx.new(|cx| {
                let mut shell = AppShell::new(window, cx);
                shell.adopt_tab(tab, window, cx);
                shell
            })
        },
        cx,
    );
}

impl AppShell {
    fn construct(window: &mut Window, cx: &mut Context<Self>) -> Self {
        UpdateModel::init(cx);
        let has_update_outcome = matches!(
            cx.global::<UpdateModel>().state,
            UpdateUiState::Updated { .. }
                | UpdateUiState::RolledBack { .. }
                | UpdateUiState::ManualInstallRequired { .. }
                | UpdateUiState::RecoveryRequired { .. }
        );
        crate::plugin_runtime::PluginRuntime::init(cx);
        let plugin_commands = crate::plugin_runtime::PluginRuntime::commands(cx);
        let mut palette_items = palette_commands();
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
            pane_rects: Vec::new(),
            content_bounds: None,
            drag: None,
            rename: None,
            mode: UiMode {
                overlay: if has_update_outcome {
                    OverlayKind::Update
                } else {
                    OverlayKind::None
                },
                ..UiMode::default()
            },
            settings_section: SettingsSection::Theme,
            theme_query: String::new(),
            palette_query: String::new(),
            palette_selected: 0,
            palette_items,
            plugin_commands,
            find_query: String::new(),
            find_gen: 0,
            find_match_count: 0,
            find_active_index: 0,
            find_regex: false,
            find_match_case: false,
            font_size_override: None,
            close_confirm: None,
            plugin_consent: None,
            bell_flash_tabs: std::collections::HashSet::new(),
            broadcast: false,
            history_query: String::new(),
            history_selected: 0,
            diff_view: None,
            diff_gen: 0,
            facts: PaneFactsState::default(),
            _session_save_task: None,
            _quit_subscription: None,
            plugin_watch: crate::plugin_event_watch::PluginEventWatch::default(),
            plugin_panels: crate::plugin_panel::PanelRegistry::new(),
            plugin_calls: crate::plugin_host_calls::HostCallLimiter::new(),
            plugin_chrome: crate::plugin_chrome::ChromeRegistry::new(),
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

        // Persist session on quit (M8) and flush the Run Ledger.
        shell._quit_subscription = Some(cx.on_app_quit(|this, cx| {
            this.emit_all_panes_closed(cx);
            this.persist_session_now(cx);
            RunLedgerGlobal::flush_now_in(cx);
            async {}
        }));
        shell
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut shell = Self::construct(window, cx);
        let restore = TerminalSettings::get_global(cx).restore_session;
        let restored = if restore {
            shell.try_restore_session(window, cx)
        } else {
            false
        };
        if !restored {
            shell.add_tab(window, cx);
        } else {
            shell.sync_ledger_focus(window, cx);
        }
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
    /// detached into another window (re-wired there via `adopt_tab`).
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
                    crate::TermViewEvent::RequestNewTab => {
                        this.add_tab(window, cx);
                    }
                    crate::TermViewEvent::RequestNextTab => {
                        this.next_tab(window, cx);
                    }
                    crate::TermViewEvent::RequestPrevTab => {
                        this.prev_tab(window, cx);
                    }
                    crate::TermViewEvent::RequestReloadSettings => {
                        // Reload clears window font zoom override (plan risk mitigation).
                        this.font_size_override = None;
                        this.apply_font_override_to_all_panes(cx);
                        TerminalSettings::reload(cx);
                        RunLedgerGlobal::reload_settings_in(cx);
                        crate::control_surface::reload(cx);
                        crate::attention_chrome::refresh(cx);
                        cx.notify();
                    }
                    crate::TermViewEvent::RequestCycleTheme => {
                        let next = TerminalSettings::get_global(cx).theme.next();
                        TerminalSettings::set_theme(next, cx);
                        cx.notify();
                    }
                    crate::TermViewEvent::RequestOpenSettings => {
                        this.toggle_settings(window, cx);
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
                    crate::TermViewEvent::GutterClicked { line } => {
                        if let Some(pane) = this.pane_key_for_view(view) {
                            this.jump_to_gutter(pane, *line, window, cx);
                        }
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
        let host_event = cx.update_global(|g: &mut RunLedgerGlobal, cx| {
            let at_ms = g.now_ms();
            match &mut event {
                RunEvent::Started { at_ms: slot, .. }
                | RunEvent::Finished { at_ms: slot, .. }
                | RunEvent::PaneClosed { at_ms: slot, .. } => {
                    *slot = at_ms;
                }
            }
            let kind = event.clone();
            g.apply(event, cx);
            run_event_to_host(&kind, &g.snapshot())
        });
        if let Some(ev) = host_event {
            crate::plugin_runtime::broadcast_event(ev, cx);
        }
        crate::attention_chrome::refresh(cx);
    }

    fn poll_plugin_events(&mut self, cx: &mut Context<Self>) {
        use crate::plugin_event_watch::PaneUiFacts;
        if !self
            .plugin_watch
            .due(std::time::Instant::now(), std::time::Duration::from_secs(1))
        {
            return;
        }
        let focus = self.active_pane_key();
        let mut facts = Vec::new();
        let mut port_jobs = Vec::new();
        for tab in &self.tabs {
            let mut leaves = Vec::new();
            tab.tree.leaves_with_keys(&mut leaves);
            for (pane, view) in leaves {
                let cwd = view
                    .read(cx)
                    .working_directory(cx)
                    .map(|p| p.to_string_lossy().into_owned());
                let fg = view.read(cx).foreground_process_command_name(cx);
                let agent = fg
                    .as_deref()
                    .and_then(crate::chrome::agent::identify)
                    .map(|kind| kind.id.to_string());
                port_jobs.push((pane, view.read(cx).shell_pid(cx)));
                facts.push(PaneUiFacts { pane, cwd, agent });
            }
        }
        for ev in self.plugin_watch.ingest_ui(focus, &facts) {
            crate::plugin_runtime::broadcast_event(ev, cx);
        }
        if self.plugin_watch.ports_inflight {
            return;
        }
        self.plugin_watch.ports_inflight = true;
        cx.spawn(async move |this, cx| {
            let mut found = Vec::new();
            for (pane, pid) in port_jobs {
                let facts = cx
                    .background_spawn(async move {
                        crate::chrome::pane_facts::collect_live_facts(None, None, pid)
                    })
                    .await;
                found.push((pane, facts.ports));
            }
            this.update(cx, |this, cx| {
                this.plugin_watch.ports_inflight = false;
                for (pane, ports) in found {
                    for ev in this.plugin_watch.ingest_ports(pane, &ports) {
                        crate::plugin_runtime::broadcast_event(ev, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn poll_plugin_inbound(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use plugin_host::resident::{ConnectionState, Inbound};
        use plugin_protocol::v2::RenderTarget;
        let inbound = crate::plugin_runtime::drain_all_inbound(cx);
        for (plugin_id, msg) in inbound {
            match msg {
                Inbound::Render {
                    target: RenderTarget::Panel { pane },
                    tree,
                    ..
                } => self.apply_panel_render(&plugin_id, pane, tree, window, cx),
                Inbound::Render {
                    target: RenderTarget::Status,
                    tree,
                    ..
                } => self.apply_chrome_status(&plugin_id, tree, cx),
                Inbound::Render { .. } => {}
                Inbound::Call { id, call } => {
                    self.handle_host_call(&plugin_id, id, call, window, cx)
                }
            }
        }
        let snapshots = crate::plugin_runtime::snapshots(cx);
        let mut live = std::collections::BTreeSet::new();
        for snap in snapshots {
            if snap.state == ConnectionState::Live {
                live.insert(snap.plugin_id);
            } else {
                self.plugin_panels.mark_plugin_stale(&snap.plugin_id);
            }
        }
        self.plugin_panels.mark_missing_stale(&live);
        if self.plugin_chrome.sync_live(&live) {
            self.rebuild_palette_items();
        }
    }

    fn apply_panel_render(
        &mut self,
        plugin_id: &str,
        pane: PaneKey,
        tree: plugin_protocol::v2::Widget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_panel::{ApplyPanel, render_panel_granted};
        use plugin_protocol::v2::Capability;
        // Same source as the event bus: Hello.granted, not "the plugin asked".
        let granted = crate::plugin_runtime::has_grant(plugin_id, Capability::RenderPanel, cx);
        let granted = render_panel_granted(if granted {
            &[Capability::RenderPanel]
        } else {
            &[]
        });
        let mut terminals = std::collections::BTreeSet::new();
        for tab in &self.tabs {
            let mut out = Vec::new();
            tab.tree.leaves_with_keys(&mut out);
            for (key, _) in out {
                terminals.insert(key);
            }
        }
        match self
            .plugin_panels
            .apply_render(plugin_id, pane, tree, granted, &terminals)
        {
            ApplyPanel::Create { pane_key } => {
                if !self.insert_panel_leaf(pane_key, plugin_id, window, cx) {
                    self.plugin_panels.remove(pane_key);
                }
            }
            ApplyPanel::Replace { .. } => cx.notify(),
            ApplyPanel::DeniedGrant => {
                log::warn!("plugin {plugin_id} RenderPanel denied (no grant)");
            }
            ApplyPanel::DeniedTerminal => {
                log::warn!("plugin {plugin_id} tried to draw into a terminal pane");
            }
            ApplyPanel::DeniedOccupied => {
                log::warn!("plugin {plugin_id} tried to take another plugin's panel");
            }
        }
    }

    fn apply_chrome_status(
        &mut self,
        plugin_id: &str,
        tree: plugin_protocol::v2::Widget,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_chrome::{ApplyChrome, render_status_granted};
        use plugin_protocol::v2::Capability;
        let granted = crate::plugin_runtime::has_grant(plugin_id, Capability::RenderStatus, cx);
        let granted = render_status_granted(if granted {
            &[Capability::RenderStatus]
        } else {
            &[]
        });
        let hint = self.active_pane_key();
        match self
            .plugin_chrome
            .apply_status(plugin_id, tree, granted, hint)
        {
            ApplyChrome::Applied => {
                self.rebuild_palette_items();
                cx.notify();
            }
            ApplyChrome::DeniedGrant => {
                log::warn!("plugin {plugin_id} RenderStatus denied (no grant)");
            }
        }
    }

    fn insert_panel_leaf(
        &mut self,
        pane_key: PaneKey,
        plugin_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let target = tab.active_pane;
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let content = crate::LeafContent::Panel {
            plugin_id: plugin_id.to_string(),
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        if !tab
            .tree
            .split_content(target, SplitAxis::Horizontal, new_id, pane_key, content)
        {
            return false;
        }
        tab.active_pane = new_id;
        self.commit_workspace(window, cx);
        true
    }

    fn handle_host_call(
        &mut self,
        plugin_id: &str,
        id: plugin_protocol::v2::MessageId,
        call: plugin_protocol::v2::HostCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_host_calls::{
            CallPlan, cap_screen, error_result, filter_listed_panes, plan_call, read_screen_access,
        };
        use plugin_protocol::v2::{Capability, HostCallResult, PaneInfo};
        let granted: Vec<Capability> = [
            Capability::HostCallNotify,
            Capability::HostCallReadScreen,
            Capability::HostCallListPanes,
            Capability::HostCallOpenPane,
        ]
        .into_iter()
        .filter(|cap| crate::plugin_runtime::has_grant(plugin_id, *cap, cx))
        .collect();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let plan = plan_call(plugin_id, &call, &granted, &mut self.plugin_calls, now_ms);
        let result = match plan {
            CallPlan::Reply(result) => result,
            CallPlan::Notify { title, body } => {
                crate::notify_message(&title, &body);
                HostCallResult::Ok
            }
            CallPlan::ListPanes => {
                let live = crate::control_surface::live_terminal_panes(cx);
                let mut terminals = std::collections::BTreeSet::new();
                let mut infos = Vec::new();
                for (pane, view) in &live {
                    terminals.insert(*pane);
                    infos.push(PaneInfo {
                        pane: *pane,
                        cwd: view
                            .read(cx)
                            .working_directory(cx)
                            .map(|p| p.to_string_lossy().into_owned()),
                        title: Some(view.read(cx).title().to_string()),
                        busy: view.read(cx).looks_busy(cx),
                    });
                }
                HostCallResult::Panes {
                    panes: filter_listed_panes(infos, &terminals),
                }
            }
            CallPlan::ReadScreen { pane } => {
                let live = crate::control_surface::live_terminal_panes(cx);
                let mut terminals = std::collections::BTreeSet::new();
                for (key, _) in &live {
                    terminals.insert(*key);
                }
                let (_, panels) = self.terminal_and_panel_keys();
                match read_screen_access(pane, &terminals, &panels) {
                    Err(message) => error_result(message),
                    Ok(()) => match live.into_iter().find(|(key, _)| *key == pane) {
                        Some((_, view)) => HostCallResult::Screen {
                            text: cap_screen(view.read(cx).visible_screen_text(cx)),
                        },
                        None => error_result(format!("pane {pane} not found")),
                    },
                }
            }
            CallPlan::OpenPane { cwd, command } => self.execute_open_pane(cwd, command, window, cx),
        };
        if !crate::plugin_runtime::reply_host_call(plugin_id, id, result, cx) {
            log::debug!("plugin {plugin_id} Call {id} reply dropped (session gone)");
        }
    }

    fn terminal_and_panel_keys(
        &self,
    ) -> (
        std::collections::BTreeSet<PaneKey>,
        std::collections::BTreeSet<PaneKey>,
    ) {
        let mut terminals = std::collections::BTreeSet::new();
        let mut panels = std::collections::BTreeSet::new();
        for tab in &self.tabs {
            let mut all = Vec::new();
            tab.tree.walk_leaves(&mut all);
            for (_, key, content) in all {
                if content.is_terminal() {
                    terminals.insert(key);
                } else {
                    panels.insert(key);
                }
            }
        }
        (terminals, panels)
    }

    fn execute_open_pane(
        &mut self,
        cwd: Option<String>,
        command: Option<crate::plugin_host_calls::OpenCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> plugin_protocol::v2::HostCallResult {
        use crate::plugin_host_calls::error_result;
        use crate::session::resolve_cwd;
        use plugin_protocol::v2::HostCallResult;
        let cwd = match cwd.as_deref() {
            None => None,
            Some(raw) => {
                let resolved = resolve_cwd(Some(raw));
                if resolved.is_none() {
                    return error_result(format!("cwd not found: {raw}"));
                }
                resolved
            }
        };
        let argv = command.map(|c| (c.program, c.args));
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let tab_id = self.next_id;
        self.next_id += 1;
        let view = self.spawn_term_view(cwd, argv, window, cx);
        let pane_key = {
            let tab = crate::tab_convert::Tab {
                id: tab_id,
                tree: crate::pane_tree::PaneNode::leaf(pane_id, view),
                active_pane: pane_id,
                custom_title: None,
                zoomed_pane: None,
            };
            let key = tab.tree.pane_key_for_id(pane_id).expect("fresh leaf");
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            key
        };
        self.commit_workspace(window, cx);
        HostCallResult::Pane { pane: pane_key }
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

    fn refresh_plugin_commands(&mut self, cx: &mut Context<Self>) {
        crate::plugin_runtime::PluginRuntime::reload(cx);
        self.plugin_commands = crate::plugin_runtime::PluginRuntime::commands(cx);
        self.rebuild_palette_items();
        self.palette_selected = 0;
    }

    pub(crate) fn plugin_badges_for_tab(
        &self,
        tab_panes: &[crate::pane_tree::PaneKey],
        tab_is_active: bool,
    ) -> Vec<crate::plugin_chrome::PluginTabBadge> {
        self.plugin_chrome.badges_for_tab(tab_panes, tab_is_active)
    }

    fn rebuild_palette_items(&mut self) {
        self.palette_items = palette_commands();
        self.palette_items
            .extend(crate::command_palette::plugin_items(&self.plugin_commands));
        self.palette_items
            .extend(crate::command_palette::contribution_items(
                self.plugin_chrome.palette_entries(),
            ));
    }

    fn run_plugin_contribution(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.plugin_chrome.palette_entries().get(index).cloned() else {
            return;
        };
        crate::plugin_runtime::push_action(
            &entry.plugin_id,
            entry.surface_id,
            entry.action,
            entry.arg,
            cx,
        );
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
            }
            Err(err) => log::error!("export scrollback failed: {err:#}"),
        }
    }

    /// The active pane's `TermView`, if any.
    fn active_view(&self, _cx: &App) -> Option<Entity<TermView>> {
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
        if let Some(next) = neighbor(&self.pane_rects, tab.active_pane, direction) {
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
        let target = tab.active_pane;
        let closed_key = tab.tree.pane_key_for_id(target);
        let closing_terminal = tab.tree.is_terminal_leaf(target);
        let terminals_after = tab
            .tree
            .terminal_count()
            .saturating_sub(usize::from(closing_terminal));
        if crate::plugin_panel::tab_close_policy(terminals_after)
            == crate::plugin_panel::TabClosePolicy::CloseTab
        {
            // Last shell is going away. Panels cannot keep the tab alive:
            // a tab of only plugin UI would look like a workspace with no
            // PTY (ADR-0001). Close the tab, dropping guest panels.
            let mut all = Vec::new();
            tab.tree.walk_leaves(&mut all);
            let panel_keys: Vec<_> = all
                .into_iter()
                .filter(|(_, _, c)| c.is_panel())
                .map(|(_, k, _)| k)
                .collect();
            self.plugin_panels.remove_all(panel_keys);
            if let Some(key) = closed_key {
                self.plugin_panels.remove(key);
            }
            self.close_active_tab(window, cx);
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let outcome = tab.tree.close(target);
        match outcome {
            CloseOutcome::TreeEmpty => {
                self.close_active_tab(window, cx);
            }
            CloseOutcome::NotFound => {
                // Stale active_pane id: recover focus instead of nuking the tab.
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = tab.tree.first_leaf_id();
                }
                self.focus_active(window, cx);
                self.sync_ledger_focus(window, cx);
                cx.notify();
            }
            CloseOutcome::Closed => {
                if let Some(pane) = closed_key {
                    self.plugin_panels.remove(pane);
                    self.apply_pane_closed(pane, cx);
                }
                // Surviving subtree: focus its first leaf (the collapsed sibling
                // when the closed pane was a direct child of a split).
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.active_pane = tab.tree.first_leaf_id();
                }
                self.commit_workspace(window, cx);
            }
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

    /// Gate close on `confirm_close` setting; may open a modal instead of closing.
    fn request_close_active_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_confirm.is_some() {
            return;
        }
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
            self.close_confirm = Some(CloseConfirmState {
                message: message.into(),
                kind: ConfirmKind::ClosePane,
            });
            cx.notify();
        } else {
            self.close_active_pane(window, cx);
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn request_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_confirm.is_some() {
            return;
        }
        let policy = TerminalSettings::get_global(cx).confirm_close;
        let needs_confirm = match policy {
            ConfirmClose::Never => false,
            ConfirmClose::Always => true,
            ConfirmClose::Dirty => self.any_pane_is_dirty(cx),
        };
        if needs_confirm {
            self.close_confirm = Some(CloseConfirmState {
                message: "A process is still running. Close this window anyway?".into(),
                kind: ConfirmKind::CloseWindow,
            });
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
        self.persist_session_now(cx);
        RunLedgerGlobal::flush_now_in(cx);
        window.remove_window();
    }

    fn request_clear_run_ledger(&mut self, cx: &mut Context<Self>) {
        if self.close_confirm.is_some() {
            return;
        }
        self.close_confirm = Some(CloseConfirmState {
            message: "This deletes the recorded command history from this machine. The terminal is not affected.".into(),
            kind: ConfirmKind::ClearRunLedger,
        });
        cx.notify();
    }

    fn confirm_close_proceed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.close_confirm.take().map(|s| s.kind);
        match kind {
            Some(ConfirmKind::ClearRunLedger) => self.clear_run_ledger(cx),
            #[cfg(target_os = "linux")]
            Some(ConfirmKind::CloseWindow) => self.finish_window_close(window, cx),
            _ => self.close_active_pane(window, cx),
        }
    }

    fn clear_run_ledger(&mut self, cx: &mut Context<Self>) {
        RunLedgerGlobal::clear_in(cx);
        cx.notify();
    }

    fn on_clear_run_ledger(
        &mut self,
        _: &ClearRunLedger,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_clear_run_ledger(cx);
    }

    fn on_toggle_run_ledger(
        &mut self,
        _: &ToggleRunLedger,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command(CommandId::ToggleRunLedger, window, cx);
    }

    /// Toggle the Run Ledger panel.
    fn toggle_run_ledger(&mut self, cx: &mut Context<Self>) {
        self.mode.toggle(OverlayKind::RunLedger);
        cx.notify();
    }

    fn on_toggle_plugin_monitor(
        &mut self,
        _: &TogglePluginMonitor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command(CommandId::TogglePluginMonitor, window, cx);
    }

    fn toggle_plugin_monitor(&mut self, cx: &mut Context<Self>) {
        self.mode.toggle(OverlayKind::PluginMonitor);
        cx.notify();
    }

    fn close_plugin_monitor(&mut self, cx: &mut Context<Self>) {
        self.mode.close(OverlayKind::PluginMonitor);
        cx.notify();
    }

    fn deny_plugin_consent(&mut self, cx: &mut Context<Self>) {
        // Deny writes nothing: a dismissed prompt must not become a grant.
        self.plugin_consent = None;
        self.mode.close(OverlayKind::PluginConsent);
        cx.notify();
    }

    fn approve_plugin_consent(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.plugin_consent.take() else {
            return;
        };
        self.mode.close(OverlayKind::PluginConsent);
        crate::plugin_runtime::save_grant(
            &pending.plugin.plugin_id,
            &pending.request,
            &pending.hash,
            pending.prompt.tier,
        );
        self.invoke_plugin_command(pending.plugin, cx);
        cx.notify();
    }

    fn kill_plugin(&mut self, plugin_id: String, cx: &mut Context<Self>) {
        crate::plugin_runtime::kill_plugin(&plugin_id, cx);
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
    fn toggle_history_search(&mut self, cx: &mut Context<Self>) {
        if !self.mode.toggle(OverlayKind::History) {
            self.history_query.clear();
            self.history_selected = 0;
        }
        cx.notify();
    }

    fn run_plugin_command(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(plugin) = self.plugin_commands.get(index).cloned() else {
            return;
        };
        self.start_plugin_command(plugin, cx);
    }

    fn start_plugin_command(
        &mut self,
        plugin: plugin_host::LoadedPluginCommand,
        cx: &mut Context<Self>,
    ) {
        let request = crate::plugin_runtime::requested_capabilities(&plugin);
        let hash = crate::plugin_runtime::plugin_binary_hash(&plugin).unwrap_or_default();
        let grants = crate::plugin_runtime::grants();
        let record = grants.grants.get(&plugin.plugin_id);
        match plugin_grants::check(&request, record, &hash) {
            plugin_grants::Decision::Allowed => self.invoke_plugin_command(plugin, cx),
            plugin_grants::Decision::NeedsConsent { reason, missing } => {
                let previously: Vec<_> = record
                    .map(|r| r.granted.iter().copied().collect())
                    .unwrap_or_default();
                let tier = record.map(|r| r.tier).unwrap_or(plugin_grants::Tier::Local);
                self.plugin_consent = Some(PluginConsentPending {
                    prompt: crate::plugin_monitor_panel::consent_prompt(
                        &plugin.plugin_id,
                        &plugin.plugin_name,
                        tier,
                        reason,
                        &missing,
                        &previously,
                    ),
                    plugin,
                    hash,
                    request,
                });
                self.mode.open(OverlayKind::PluginConsent);
                cx.notify();
            }
            plugin_grants::Decision::Denied(message) => {
                log::warn!("plugin {} denied: {message}", plugin.qualified_id());
            }
        }
    }

    fn invoke_plugin_command(
        &mut self,
        plugin: plugin_host::LoadedPluginCommand,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let context = crate::plugin_runtime::build_context(&plugin, &view, cx);
        let allowed = crate::plugin_runtime::allowed_permissions(cx);
        let qualified_id = plugin.qualified_id();
        log::info!(
            "plugin: invoking {qualified_id} (selection={} bytes, cwd={:?})",
            context.selection.as_deref().map(str::len).unwrap_or(0),
            context.cwd,
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { plugin_host::run_command(&plugin, &context, &allowed) },
                )
                .await;
            this.update(cx, |_this, cx| match result {
                Ok(output) => crate::plugin_runtime::apply_output(output, &view, cx),
                Err(err) => log::warn!("plugin {qualified_id} failed: {err}"),
            })
            .ok();
        })
        .detach();
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

    fn jump_to_gutter(
        &mut self,
        pane: run_ledger::PaneKey,
        line: i32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode.open(OverlayKind::RunLedger);
        let run_id = if cx.has_global::<RunLedgerGlobal>() {
            let snapshot = cx.global::<RunLedgerGlobal>().snapshot();
            run_id_for_gutter(&snapshot, pane, line)
        } else {
            None
        };
        if let Some(id) = run_id {
            if cx.has_global::<RunLedgerGlobal>() {
                cx.update_global(|g: &mut RunLedgerGlobal, _| {
                    g.mark_run_seen(id);
                });
            }
        }
        self.jump_to_ledger_row(pane, run_id, window, cx);
        if run_id.is_none() {
            if let Some(view) = self.view_for_pane(pane) {
                view.update(cx, |v, cx| v.scroll_to_anchor(line, 0, cx));
            }
        }
        cx.notify();
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
        self.close_confirm = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(window, cx);
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
        // Settings reload clears window-scoped font zoom.
        self.font_size_override = None;
        self.apply_font_override_to_all_panes(cx);
        TerminalSettings::reload(cx);
        self.refresh_plugin_commands(cx);
        RunLedgerGlobal::reload_settings_in(cx);
        crate::control_surface::reload(cx);
        crate::attention_chrome::refresh(cx);
        cx.notify();
    }

    fn on_cycle_theme(&mut self, _: &CycleTheme, _window: &mut Window, cx: &mut Context<Self>) {
        let next = TerminalSettings::get_global(cx).theme.next();
        TerminalSettings::set_theme(next, cx);
        cx.notify();
    }

    // ── command palette (M9) ────────────────────────────────────────────────

    fn on_toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode.is(OverlayKind::Palette) {
            self.close_palette(window, cx);
        } else {
            self.open_palette(cx);
        }
    }

    // ── find in scrollback (M10) ────────────────────────────────────────────
}

fn run_event_to_host(
    event: &RunEvent,
    snapshot: &[run_ledger::Run],
) -> Option<plugin_protocol::v2::HostEvent> {
    use plugin_protocol::v2::HostEvent;
    match event {
        RunEvent::Started { pane, cwd, .. } => {
            let run = snapshot
                .iter()
                .rev()
                .find(|r| r.pane == *pane && r.state == run_ledger::RunState::Running)?;
            Some(HostEvent::RunStarted {
                run_id: run.id,
                pane: run.pane,
                command: run.command.clone(),
                cwd: cwd.clone().or_else(|| run.cwd.clone()),
            })
        }
        RunEvent::Finished { pane, .. } => {
            let run = snapshot.iter().rev().find(|r| {
                r.pane == *pane
                    && r.state.is_finished()
                    && r.state != run_ledger::RunState::Abandoned
            })?;
            Some(HostEvent::RunFinished {
                run_id: run.id,
                pane: run.pane,
                exit_code: run.exit_code,
                duration_ms: run.duration.as_millis() as u64,
            })
        }
        RunEvent::PaneClosed { .. } => None,
    }
}

fn tree_contains(tree: &PaneNode, id: PaneId) -> bool {
    tree.contains_leaf(id)
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
        run_event_to_host, traffic_light_position_for,
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
        for required in [
            "emit_all_panes_closed(cx)",
            "persist_session_now(cx)",
            "RunLedgerGlobal::flush_now_in(cx)",
            "window.remove_window()",
        ] {
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

    #[test]
    fn run_started_host_event_uses_ledger_redacted_command() {
        let mut ledger = run_ledger::Ledger::new(run_ledger::LaunchId::nil());
        let pane = run_ledger::PaneKey::from_u128(1);
        let event = run_ledger::RunEvent::started(
            pane,
            "AWS_SECRET_ACCESS_KEY=supersecret aws s3 ls",
            None,
            10,
        );
        ledger.apply(event.clone());
        let host = run_event_to_host(&event, &ledger.snapshot()).expect("mapped");
        let plugin_protocol::v2::HostEvent::RunStarted { command, .. } = host else {
            panic!("expected RunStarted");
        };
        assert!(
            !command.contains("supersecret"),
            "plugins must never see the raw command line: {command}"
        );
    }
}

fn snapshot_tree(node: &PaneNode, cx: &App) -> SessionNode {
    match node {
        PaneNode::Leaf {
            id,
            pane_key,
            content: crate::LeafContent::Terminal(view),
        } => SessionNode::Leaf {
            id: *id,
            cwd: view
                .read(cx)
                .working_directory(cx)
                .map(|p| p.to_string_lossy().into_owned()),
            pane_key: Some(*pane_key),
        },
        PaneNode::Leaf { id, pane_key, .. } => SessionNode::Panel {
            id: *id,
            pane_key: Some(*pane_key),
        },
        PaneNode::Split {
            axis,
            ratio,
            first,
            second,
        } => SessionNode::Split {
            axis: match axis {
                SplitAxis::Horizontal => SessionAxis::Horizontal,
                SplitAxis::Vertical => SessionAxis::Vertical,
            },
            ratio: *ratio,
            first: Box::new(snapshot_tree(first, cx)),
            second: Box::new(snapshot_tree(second, cx)),
        },
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_ledger_focus(window, cx);
        self.refresh_pane_facts_if_stale(cx);
        self.poll_plugin_events(cx);
        self.poll_plugin_inbound(window, cx);
        // Navigating away from the consent overlay is a deny: never grant
        // because a different surface took the keyboard.
        if !self.mode.is(OverlayKind::PluginConsent) {
            self.plugin_consent = None;
        }
        let palette = TerminalPalette::get_global(cx);
        let window_active = window.is_window_active();
        let tokens = ChromeTokens::from_palette(&palette, window_active);
        let fullscreen = window.is_fullscreen();
        let geo = ChromeGeometry::for_window(cfg!(not(target_os = "macos")), fullscreen);
        let leading = geo.leading_pad;
        let chrome_h = geo.height;
        let banner_top = chrome_h;
        let show_tab_strip = self.tabs.len() > 1;

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
                if this.close_confirm.is_some() {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.confirm_close_cancel(window, cx);
                            cx.stop_propagation();
                        }
                        "enter" => {
                            this.confirm_close_proceed(window, cx);
                            cx.stop_propagation();
                        }
                        _ => {
                            if !event.keystroke.modifiers.platform {
                                cx.stop_propagation();
                            }
                        }
                    }
                    return;
                }
                if this.mode.is(OverlayKind::PluginConsent) {
                    // Enter must not grant — Approve is an explicit click only.
                    match event.keystroke.key.as_str() {
                        "escape" | "enter" => {
                            this.deny_plugin_consent(cx);
                            cx.stop_propagation();
                        }
                        _ => {
                            if !event.keystroke.modifiers.platform {
                                cx.stop_propagation();
                            }
                        }
                    }
                    return;
                }
                if this.mode.is(OverlayKind::Update) {
                    if event.keystroke.key.as_str() == "escape" {
                        this.close_update(cx);
                        cx.stop_propagation();
                    }
                    if !event.keystroke.modifiers.platform {
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.mode.is(OverlayKind::PaneFacts) && event.keystroke.key.as_str() == "escape"
                {
                    this.close_pane_facts(cx);
                    cx.stop_propagation();
                    return;
                }
                if this.mode.is(OverlayKind::PluginMonitor)
                    && event.keystroke.key.as_str() == "escape"
                {
                    this.close_plugin_monitor(cx);
                    cx.stop_propagation();
                    return;
                }
                if this.mode.is(OverlayKind::Palette) {
                    if this.palette_key_down(event, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.mode.find_open {
                    if this.find_key_down(event, window, cx) {
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.mode.is(OverlayKind::Settings) {
                    // Type-to-filter the theme picker when that section is
                    // active; escape clears the filter before closing.
                    if this.settings_section == SettingsSection::Theme {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                if !this.theme_query.is_empty() {
                                    this.theme_query.clear();
                                    cx.notify();
                                } else {
                                    this.close_settings(window, cx);
                                }
                                cx.stop_propagation();
                                return;
                            }
                            "backspace" => {
                                this.theme_query.pop();
                                cx.notify();
                                cx.stop_propagation();
                                return;
                            }
                            _ => {
                                if !event.keystroke.modifiers.platform
                                    && let Some(ch) = event.keystroke.key_char.as_ref()
                                    && !ch.is_empty()
                                    && !ch.chars().any(|c| c.is_control())
                                {
                                    this.theme_query.push_str(ch);
                                    cx.notify();
                                }
                            }
                        }
                    }
                    if event.keystroke.key.as_str() == "escape" {
                        this.close_settings(window, cx);
                        cx.stop_propagation();
                    }
                    // Swallow other keys while the settings panel is open
                    // so they don't reach the terminal underneath.
                    // ⌘, (OpenSettings) still fires via on_action.
                    if !event.keystroke.modifiers.platform {
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.mode.is(OverlayKind::Diff) {
                    if this.handle_diff_key(event, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if !event.keystroke.modifiers.platform {
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.rename_key_down(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            // Clicking anywhere else (terminal, another tab) commits the
            // in-progress rename.
            .capture_any_mouse_down(cx.listener(
                |this, event: &gpui::MouseDownEvent, window, cx| {
                    if this.rename.is_some() && event.button == MouseButton::Left {
                        this.commit_rename(window, cx);
                    }
                },
            ))
            .on_action(cx.listener(Self::on_new_tab))
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
            .on_action(cx.listener(Self::on_clear_run_ledger))
            .on_action(cx.listener(Self::on_toggle_run_ledger))
            .on_action(cx.listener(Self::on_toggle_plugin_monitor))
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
                    .when(self.mode.find_open, |el| {
                        el.child(self.render_find_bar(&tokens, cx))
                    })
                    .child(self.render_content(&tokens, window, cx))
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
                                .rounded(px(6.0))
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
                                .rounded(px(6.0))
                                .bg(tokens.surface)
                                .border_1()
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
            .when(self.mode.is(OverlayKind::Settings), |el| {
                el.child(self.render_settings_overlay(&tokens, window, cx))
            })
            .when(self.mode.is(OverlayKind::Update), |el| {
                el.child(self.render_update_overlay(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::Palette), |el| {
                el.child(self.render_command_palette(&tokens, cx))
            })
            .when(self.close_confirm.is_some(), |el| {
                el.child(self.render_close_confirm(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::PaneFacts), |el| {
                el.child(self.render_pane_facts(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::RunLedger), |el| {
                el.child(self.render_run_ledger(&tokens, window, cx))
            })
            .when(self.mode.is(OverlayKind::PluginMonitor), |el| {
                el.child(self.render_plugin_monitor(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::PluginConsent), |el| {
                el.child(self.render_plugin_consent(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::History), |el| {
                el.child(self.render_history_search(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::Diff), |el| {
                el.child(self.render_diff_overlay(&tokens, &palette, window, cx))
            })
    }
}

fn run_id_for_gutter(
    snapshot: &[run_ledger::Run],
    pane: PaneKey,
    line: i32,
) -> Option<run_ledger::RunId> {
    if let Some(run) = snapshot
        .iter()
        .rev()
        .find(|run| run.pane == pane && run.anchor.is_some_and(|anchor| anchor.line == line))
    {
        return Some(run.id);
    }
    snapshot
        .iter()
        .rev()
        .filter(|run| run.pane == pane)
        .filter_map(|run| run.anchor.map(|anchor| (run.id, anchor.line)))
        .filter(|(_, start)| *start <= line)
        .max_by_key(|(_, start)| *start)
        .map(|(id, _)| id)
}

#[cfg(test)]
mod gutter_jump_tests {
    use super::run_id_for_gutter;
    use run_ledger::{Anchor, LaunchId, Ledger, PaneKey, RunEvent};

    #[test]
    fn prefers_exact_start_line_then_nearest_preceding() {
        let pane = PaneKey::new_v4();
        let mut ledger = Ledger::new(LaunchId::new_v4());
        ledger.set_redact(false);
        ledger.apply(RunEvent::started_at(
            pane,
            "first",
            None,
            0,
            false,
            Some(Anchor {
                line: 10,
                column: 0,
            }),
        ));
        ledger.apply(RunEvent::finished(pane, Some(0), 5));
        ledger.apply(RunEvent::started_at(
            pane,
            "second",
            None,
            10,
            false,
            Some(Anchor {
                line: 20,
                column: 0,
            }),
        ));
        let snap = ledger.snapshot();
        let first = snap.iter().find(|r| r.command == "first").unwrap().id;
        let second = snap.iter().find(|r| r.command == "second").unwrap().id;
        assert_eq!(run_id_for_gutter(&snap, pane, 10), Some(first));
        assert_eq!(run_id_for_gutter(&snap, pane, 20), Some(second));
        assert_eq!(run_id_for_gutter(&snap, pane, 24), Some(second));
        assert_eq!(run_id_for_gutter(&snap, pane, 15), Some(first));
    }
}
