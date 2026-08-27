//! Single-window multi-tab shell for sleipnir (HIG-aligned chrome).

/// Maps `CommandId` to canonical shell actions. A child module so it can reach
/// `AppShell`'s private methods without widening them to the whole crate.
mod command_dispatch;
mod layout;
mod panels;
mod settings;
mod update;

use gpui::{
    App, AppContext as _, BorrowAppContext, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions, actions, deferred, div,
    prelude::FluentBuilder as _, px, relative, size,
};
use run_ledger::{PaneKey, RunEvent};
use sleipnir_settings::{Appearance, ConfirmClose, TerminalPalette, TerminalSettings};
use std::path::PathBuf;

use crate::chrome::{ChromeGeometry, ChromeTokens, active_after_close};
use crate::command_palette::{
    CommandId, CommandItem, commands as palette_commands, filter_commands,
};
use crate::pane_tree::{
    CloseOutcome, Direction, MIN_RATIO, PaneId, PaneNode, PaneRect, SplitAxis, SplitPath, neighbor,
};
use crate::run_ledger_global::RunLedgerGlobal;
use crate::session::{
    SessionAxis, SessionFile, SessionNode, SessionTab, load_session, resolve_cwd, restore_pane_key,
    sanitize_session, save_session, session_path,
};
pub(crate) use crate::tab_convert::Tab;
use crate::tab_convert::{extract_pane, merge_tab};
use crate::ui_mode::{OverlayKind, PANE_FACTS_MAX_AGE, PaneFactsState, UiMode};
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
}

impl SettingsSection {
    const ALL: &'static [SettingsSection] = &[SettingsSection::Theme, SettingsSection::General];

    fn id(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
            SettingsSection::General => "general",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
            SettingsSection::General => "general",
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
    find_query: String,
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
    /// Tab ids currently flashing for visual bell (M12).
    pub(crate) bell_flash_tabs: std::collections::HashSet<u64>,
    /// Fan-out keystrokes to all panes in the active tab (M13).
    broadcast: bool,
    /// Focused-pane facts: async collection state machine. Carries its own
    /// snapshot timestamp and in-flight flag.
    facts: PaneFactsState,
    tombstone_gate: crate::chrome::tombstone::TombstoneGate,
    history_query: String,
    history_selected: usize,
    /// Git diff inspector (ADR-0012). Not a Pane.
    pub(crate) diff_view: Option<crate::diff::DiffView>,
    diff_gen: u64,
    /// Debounced session save task.
    _session_save_task: Option<Task<()>>,
    /// Keep the app-quit subscription alive for the window lifetime.
    _quit_subscription: Option<gpui::Subscription>,
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
            palette_items: palette_commands(),
            find_query: String::new(),
            find_match_count: 0,
            find_active_index: 0,
            find_regex: false,
            find_match_case: false,
            font_size_override: None,
            close_confirm: None,
            bell_flash_tabs: std::collections::HashSet::new(),
            broadcast: false,
            tombstone_gate: crate::chrome::tombstone::TombstoneGate::default(),
            history_query: String::new(),
            history_selected: 0,
            diff_view: None,
            diff_gen: 0,
            facts: PaneFactsState::default(),
            _session_save_task: None,
            _quit_subscription: None,
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
        let override_size = self.font_size_override;
        let view = cx.new(|cx| {
            let mut v = TermView::new_local_with_cwd(cwd, window, cx);
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
                    crate::TermViewEvent::UserTyped => {
                        if let Some(pane) = this.pane_key_for_view(view) {
                            this.tombstone_gate.dismiss(pane);
                            cx.notify();
                        }
                    }
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
        cx.update_global(|g: &mut RunLedgerGlobal, cx| {
            let at_ms = g.now_ms();
            match &mut event {
                RunEvent::Started { at_ms: slot, .. }
                | RunEvent::Finished { at_ms: slot, .. }
                | RunEvent::PaneClosed { at_ms: slot, .. } => {
                    *slot = at_ms;
                }
            }
            g.apply(event, cx);
        });
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

    /// Re-read settings from disk, dropping any in-session font override.
    fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.font_size_override = None;
        self.apply_font_override_to_all_panes(cx);
        TerminalSettings::reload(cx);
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

    // ── session persistence (M8) ────────────────────────────────────────────

    fn try_restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let path = session_path();
        let Some(raw) = load_session(&path) else {
            return false;
        };
        let Some(session) = sanitize_session(raw) else {
            return false;
        };
        self.restore_from_session(session, window, cx);
        !self.tabs.is_empty()
    }

    fn restore_from_session(
        &mut self,
        session: SessionFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.clear();
        let mut max_pane = 0u64;
        let mut max_tab = 0u64;
        for (i, stab) in session.tabs.into_iter().enumerate() {
            max_pane = max_pane.max(stab.tree.max_pane_id());
            let tab_id = (i as u64) + 1;
            max_tab = max_tab.max(tab_id);
            let tree = self.materialize_tree(&stab.tree, window, cx);
            let active_pane = if tree_contains(&tree, stab.active_pane) {
                stab.active_pane
            } else {
                tree.first_leaf_id()
            };
            self.tabs.push(Tab {
                id: tab_id,
                tree,
                active_pane,
                custom_title: stab
                    .custom_title
                    .filter(|s| !s.is_empty())
                    .map(SharedString::from),
                zoomed_pane: None,
            });
        }
        self.next_id = max_tab + 1;
        self.next_pane_id = max_pane + 1;
        self.active = session.active_tab.min(self.tabs.len().saturating_sub(1));
        self.focus_active(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        cx.notify();
        log::info!(
            "restored session: {} tab(s), active={}",
            self.tabs.len(),
            self.active
        );
    }

    fn materialize_tree(
        &mut self,
        node: &SessionNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneNode {
        match node {
            SessionNode::Leaf { id, cwd, pane_key } => {
                let view = self.spawn_term_view_with_cwd(resolve_cwd(cwd.as_deref()), window, cx);
                PaneNode::leaf_with_key(*id, restore_pane_key(*pane_key), view)
            }
            SessionNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.materialize_tree(first, window, cx);
                let second = self.materialize_tree(second, window, cx);
                PaneNode::Split {
                    axis: match axis {
                        SessionAxis::Horizontal => SplitAxis::Horizontal,
                        SessionAxis::Vertical => SplitAxis::Vertical,
                    },
                    ratio: (*ratio).clamp(MIN_RATIO, 1.0 - MIN_RATIO),
                    first: Box::new(first),
                    second: Box::new(second),
                }
            }
        }
    }

    fn snapshot_session(&self, cx: &App) -> SessionFile {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| SessionTab {
                custom_title: tab.custom_title.as_ref().map(|s| s.to_string()),
                active_pane: tab.active_pane,
                tree: snapshot_tree(&tab.tree, cx),
            })
            .collect();
        SessionFile {
            version: crate::session::SESSION_VERSION,
            active_tab: self.active,
            tabs,
        }
    }

    fn persist_session_now(&self, cx: &App) {
        if !TerminalSettings::get_global(cx).restore_session {
            return;
        }
        let session = self.snapshot_session(cx);
        if session.tabs.is_empty() {
            return;
        }
        let path = session_path();
        if let Err(err) = save_session(&path, &session) {
            log::warn!("failed to save session to {}: {err}", path.display());
        } else {
            log::debug!(
                "session saved: {} tab(s) → {}",
                session.tabs.len(),
                path.display()
            );
        }
    }

    /// Mark layout dirty and write session after a short debounce so rapid
    /// tab switches don't thrash the disk.
    pub(crate) fn schedule_session_save(&mut self, cx: &mut Context<Self>) {
        // Cancel any pending save timer and start a fresh debounce.
        // Dropping the previous task cancels the prior write, so only the most
        // recent structural change is persisted.
        self._session_save_task = Some(cx.spawn(async move |this, cx| {
            // Yield a few times via background executor to defer the write
            // past any rapid burst of sequential calls.
            for _ in 0..3 {
                cx.background_spawn(std::future::ready(())).await;
            }
            this.update(cx, |this, cx| {
                this.persist_session_now(cx);
            })
            .ok();
        }));
    }

    /// The active pane's `TermView`, if any.
    fn active_view(&self, _cx: &App) -> Option<Entity<TermView>> {
        let tab = self.tabs.get(self.active)?;
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        leaves
            .iter()
            .find(|(id, _)| *id == tab.active_pane)
            .map(|(_, v)| (*v).clone())
            .or_else(|| leaves.first().map(|(_, v)| (*v).clone()))
    }

    /// The active pane's working directory, when its PTY reports one. New tabs
    /// and splits inherit this so they open where you are instead of in `$HOME`.
    fn active_working_directory(&self, cx: &App) -> Option<std::path::PathBuf> {
        self.active_view(cx)
            .and_then(|view| view.read(cx).working_directory(cx))
    }

    pub(crate) fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self
            .active_working_directory(cx)
            .map(|cwd| crate::chrome::workspace::spawn_cwd(&cwd));
        self.add_tab_at(cwd, window, cx);
    }

    pub(crate) fn add_tab_at(
        &mut self,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let view = self.spawn_term_view_with_cwd(cwd, window, cx);
        self.tabs.push(Tab {
            id,
            tree: PaneNode::leaf(pane_id, view),
            active_pane: pane_id,
            custom_title: None,
            zoomed_pane: None,
        });
        self.active = self.tabs.len() - 1;
        self.commit_workspace(window, cx);
    }

    /// Begin an inline rename for the given tab, seeding the editable buffer
    /// with the text currently shown on the chip.
    pub(crate) fn begin_rename(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let buffer = tab.path_label(cx).to_string();
        self.rename = Some(RenameState { tab_id, buffer });
        cx.notify();
    }

    /// Commit the in-progress rename to the target tab. An empty buffer clears
    /// the custom title so the tab falls back to the pane title (side) or cwd
    /// path (top).
    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.rename.take() {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == state.tab_id) {
                let trimmed = state.buffer.trim();
                tab.custom_title = if trimmed.is_empty() {
                    None
                } else {
                    Some(SharedString::from(trimmed.to_string()))
                };
            }
            self.sync_window_title(window, cx);
            self.schedule_session_save(cx);
            cx.notify();
        }
    }

    /// Abandon the in-progress rename without changing the tab title.
    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            cx.notify();
        }
    }

    /// Handle a keystroke while an inline rename is active. Returns true if the
    /// keystroke was consumed by the rename editor.
    fn rename_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.rename.is_none() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                self.commit_rename(window, cx);
                true
            }
            "escape" => {
                self.cancel_rename(cx);
                true
            }
            "backspace" => {
                if let Some(state) = self.rename.as_mut() {
                    state.buffer.pop();
                    cx.notify();
                }
                true
            }
            _ => {
                // Append any typed printable character to the buffer.
                if let Some(ch) = event.keystroke.key_char.as_ref() {
                    if !ch.is_empty() && !ch.chars().any(|c| c.is_control()) {
                        if let Some(state) = self.rename.as_mut() {
                            state.buffer.push_str(ch);
                            cx.notify();
                        }
                    }
                }
                // While renaming, swallow every other key too so shortcuts
                // (e.g. ⌘W, ⌘T) and stray terminal input don't fire mid-edit.
                true
            }
        }
    }

    fn close_tab_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        // Drop any inline rename targeting the tab being removed.
        if let Some(state) = self.rename.as_ref() {
            if self.tabs[index].id == state.tab_id {
                self.rename = None;
            }
        }
        let closed_keys = self.tabs[index].tree.all_pane_keys();
        for pane in closed_keys {
            self.apply_pane_closed(pane, cx);
        }
        match active_after_close(self.active, index, self.tabs.len()) {
            None => {
                self.tabs.remove(index);
                // Always keep at least one tab.
                self.add_tab(window, cx);
            }
            Some(new_active) => {
                self.tabs.remove(index);
                self.active = new_active;
                self.commit_workspace(window, cx);
            }
        }
    }

    fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = self.active.min(self.tabs.len() - 1);
        self.close_tab_at(idx, window, cx);
    }

    pub(crate) fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active = index;
            self.commit_workspace(window, cx);
        }
    }

    /// Move the dragged tab so it sits immediately before `target_id` (tab drag
    /// reorder). `active` follows the previously-active tab to its new index.
    pub(crate) fn reorder_tab(
        &mut self,
        dragged_id: u64,
        target_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.tabs.iter().position(|t| t.id == dragged_id) else {
            return;
        };
        let Some(target) = self.tabs.iter().position(|t| t.id == target_id) else {
            return;
        };
        if dragged_id == target_id {
            return;
        }
        let active_id = self.tabs.get(self.active).map(|t| t.id);
        let tab = self.tabs.remove(from);
        // Insertion index: dropping on a target to the right shifts it left by
        // one once the dragged tab is removed.
        let to = reorder_insert_index(from, target);
        self.tabs.insert(to, tab);
        if let Some(active_id) = active_id {
            self.active = self
                .tabs
                .iter()
                .position(|t| t.id == active_id)
                .unwrap_or(0);
        }
        self.commit_workspace(window, cx);
    }

    /// Move a tab out of this window into a fresh window (drag tab to the
    /// content area). Keeps ≥1 tab here; the detached tab's panes keep running.
    fn detach_tab_to_new_window(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.len() <= 1 {
            return;
        }
        let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let Some(new_active) = active_after_close(self.active, idx, self.tabs.len()) else {
            return;
        };
        let tab = self.tabs.remove(idx);
        self.active = new_active;
        self.commit_workspace(window, cx);
        open_sleipnir_window_with_tab(tab, cx);
    }

    /// Tab dropped on the visible pane area: a *different* tab merges in as a
    /// pane; dropping the visible tab itself still detaches it to a new window.
    fn on_tab_dropped_on_pane_area(
        &mut self,
        dragged_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(visible_id) = self.tabs.get(self.active).map(|t| t.id) else {
            return;
        };
        if dragged_id == visible_id {
            self.detach_tab_to_new_window(dragged_id, window, cx);
        } else {
            self.merge_tab_into_visible(dragged_id, window, cx);
        }
    }

    fn merge_tab_into_visible(
        &mut self,
        source_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dest_id) = self.tabs.get(self.active).map(|t| t.id) else {
            return;
        };
        let Ok(dest_idx) = merge_tab(&mut self.tabs, source_id, dest_id) else {
            return;
        };
        // A successful merge always leaves the destination tab behind.
        self.active = dest_idx.min(self.tabs.len() - 1);
        self.commit_workspace(window, cx);
    }

    /// Drop a pane onto the tab list at `insert_at` (clamped).
    pub(crate) fn extract_pane_to_tab(
        &mut self,
        pane_id: PaneId,
        insert_at: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_id = self.next_id;
        let Ok(idx) = extract_pane(&mut self.tabs, pane_id, insert_at, new_id) else {
            return;
        };
        self.next_id += 1;
        // A successful extract only ever adds a tab.
        self.active = idx.min(self.tabs.len() - 1);
        self.commit_workspace(window, cx);
    }

    /// Replace this window's placeholder tab with a detached `tab` and re-wire
    /// each pane's observers to route events here.
    fn adopt_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        // Drop the placeholder tab `new` created (its shell exits).
        self.tabs.clear();
        let mut tab = tab;
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        let max_pane = leaves.iter().map(|(id, _)| *id).max().unwrap_or(0);
        let adopted = rebase_detached_tab(max_pane);
        tab.id = adopted.tab_id;
        self.next_id = adopted.next_id;
        self.next_pane_id = adopted.next_pane_id;
        let views: Vec<Entity<TermView>> = leaves.into_iter().map(|(_, v)| v.clone()).collect();
        self.tabs.push(tab);
        self.active = 0;
        for view in &views {
            self.wire_term_view(view, window, cx);
        }
        self.commit_workspace(window, cx);
    }

    pub(crate) fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let next = (self.active + 1) % self.tabs.len();
        self.activate(next, window, cx);
    }

    pub(crate) fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let prev = if self.active == 0 {
            self.tabs.len() - 1
        } else {
            self.active - 1
        };
        self.activate(prev, window, cx);
    }

    pub(crate) fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_view(cx) {
            let handle = view.focus_handle(cx);
            window.focus(&handle, cx);
        }
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
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let target = tab.active_pane;
        let closed_key = tab.tree.pane_key_for_id(target);
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

    fn on_send_git_diff(&mut self, _: &SendGitDiff, _window: &mut Window, cx: &mut Context<Self>) {
        self.send_git_diff_to_pty(cx);
    }

    fn on_toggle_diff(&mut self, _: &ToggleDiff, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_diff(window, cx);
    }

    pub(crate) fn toggle_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.is(OverlayKind::Diff) {
            self.close_diff(window, cx);
            return;
        }
        self.mode.open(OverlayKind::Diff);
        self.refresh_diff(false, window, cx);
        cx.notify();
    }

    pub(crate) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.mode.close(OverlayKind::Diff) {
            return;
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(crate) fn refresh_diff(
        &mut self,
        force: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = self.active_working_directory(cx).unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
        let root = crate::chrome::workspace::git_root(&cwd).unwrap_or_else(|| cwd.clone());
        if !force {
            if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_ref() {
                if session.still_fresh(&root) {
                    self.mode.open(OverlayKind::Diff);
                    cx.notify();
                    return;
                }
            }
        }
        let mode = match &self.diff_view {
            Some(crate::diff::DiffView::Ready(session)) => session.mode,
            _ => crate::diff::ViewMode::default(),
        };
        let minimap_visible = match &self.diff_view {
            Some(crate::diff::DiffView::Ready(session)) => session.minimap_visible,
            _ => true,
        };
        self.diff_gen = self.diff_gen.wrapping_add(1);
        let generation = self.diff_gen;
        let title = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        self.diff_view = Some(crate::diff::DiffView::Loading { title, generation });
        self.mode.open(OverlayKind::Diff);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { crate::git_service::fetch_worktree_patch(&cwd) })
                .await;
            this.update(cx, |this, cx| {
                if this.diff_gen != generation {
                    return;
                }
                this.diff_view = Some(match outcome {
                    crate::git_service::PatchOutcome::Ready(ready) => {
                        let root = ready.root.clone();
                        let jobs: Vec<crate::diff::upgrade::UpgradeJob> = ready
                            .parsed
                            .files
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| {
                                f.status != diff_core::FileStatus::Binary && !f.hunks.is_empty()
                            })
                            .map(|(ix, f)| crate::diff::upgrade::UpgradeJob {
                                file_ix: ix,
                                old_path: f.old_path.clone(),
                                new_path: f.new_path.clone(),
                                status: f.status,
                            })
                            .collect();
                        let mut session = crate::diff::DiffSession::from_ready(ready, mode);
                        session.minimap_visible = minimap_visible;
                        if !jobs.is_empty() {
                            this.spawn_diff_upgrade(generation, root, jobs, cx);
                        }
                        crate::diff::DiffView::Ready(session)
                    }
                    crate::git_service::PatchOutcome::Clean { title } => {
                        crate::diff::DiffView::Message {
                            title,
                            body: "Working tree clean".into(),
                        }
                    }
                    crate::git_service::PatchOutcome::Failed { title, message } => {
                        crate::diff::DiffView::Message {
                            title,
                            body: message,
                        }
                    }
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn spawn_diff_upgrade(
        &mut self,
        generation: u64,
        root: std::path::PathBuf,
        jobs: Vec<crate::diff::upgrade::UpgradeJob>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async move { crate::diff::upgrade::run_upgrade(&root, jobs) })
                .await;
            this.update(cx, |this, cx| {
                if this.diff_gen != generation {
                    return;
                }
                if let Some(crate::diff::DiffView::Ready(session)) = this.diff_view.as_mut() {
                    session.apply_upgrades(files);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn expand_diff_gap(
        &mut self,
        file_ix: usize,
        gap_ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() else {
            return;
        };
        let Some((gap_row, inserted)) = session.expand_gap(file_ix, gap_ix) else {
            return;
        };
        if session.cursor > gap_row {
            session.cursor += inserted;
        }
        cx.notify();
    }

    pub(crate) fn toggle_diff_minimap(&mut self, cx: &mut Context<Self>) {
        if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
            session.minimap_visible = !session.minimap_visible;
            cx.notify();
        }
    }

    pub(crate) fn toggle_diff_mode(&mut self, cx: &mut Context<Self>) {
        if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
            let next = session.mode.toggle();
            session.set_mode(next);
            cx.notify();
        }
    }

    pub(crate) fn jump_diff_file(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
            session.jump_to_row(row);
            cx.notify();
        }
    }

    pub(crate) fn send_open_diff_to_pty(&mut self, cx: &mut Context<Self>) {
        let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_ref() else {
            self.send_git_diff_to_pty(cx);
            return;
        };
        let Some(payload) = crate::chrome::send_context::git_diff_payload(&session.patch) else {
            return;
        };
        let Some(view) = self.active_view(cx) else {
            return;
        };
        view.update(cx, |v, cx| v.input_bytes(payload.into_bytes(), cx));
        cx.notify();
    }

    fn handle_diff_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.mode.is(OverlayKind::Diff) {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_diff(window, cx);
                true
            }
            "n" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.hunk_rows.clone();
                    session.jump_next(&targets);
                }
                cx.notify();
                true
            }
            "p" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.hunk_rows.clone();
                    session.jump_prev(&targets);
                }
                cx.notify();
                true
            }
            "]" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.file_rows.clone();
                    session.jump_next(&targets);
                }
                cx.notify();
                true
            }
            "[" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    let targets = session.file_rows.clone();
                    session.jump_prev(&targets);
                }
                cx.notify();
                true
            }
            "v" => {
                self.toggle_diff_mode(cx);
                true
            }
            "m" => {
                self.toggle_diff_minimap(cx);
                true
            }
            "r" => {
                self.refresh_diff(true, window, cx);
                true
            }
            "home" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    session.jump_home();
                }
                cx.notify();
                true
            }
            "end" => {
                if let Some(crate::diff::DiffView::Ready(session)) = self.diff_view.as_mut() {
                    session.jump_end();
                }
                cx.notify();
                true
            }
            _ => false,
        }
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

    /// Paste the work-tree patch into the focused PTY.
    ///
    /// `git diff` can take seconds on a large repository, so it runs on the
    /// background executor. This path only needs the raw text, so it uses the
    /// cheap `fetch_patch_text` layer rather than paying for a full patch parse.
    fn send_git_diff_to_pty(&mut self, cx: &mut Context<Self>) {
        let cwd = self
            .active_working_directory(cx)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let Some(view) = self.active_view(cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { crate::git_service::fetch_patch_text(&cwd) })
                .await;
            let crate::git_service::PatchOutcome::Ready(text) = outcome else {
                return;
            };
            let Some(payload) = crate::chrome::send_context::git_diff_payload(&text.patch) else {
                return;
            };
            this.update(cx, |this, cx| {
                // Focus may have moved while git was running; pasting a patch
                // into whichever pane is focused now would be wrong.
                if this.active_view(cx).as_ref() != Some(&view) {
                    return;
                }
                view.update(cx, |v, cx| v.input_bytes(payload.into_bytes(), cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    #[cfg_attr(not(unix), allow(dead_code))]
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

    fn on_toggle_pane_facts(
        &mut self,
        _: &TogglePaneFacts,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_pane_facts(cx);
    }

    fn toggle_pane_facts(&mut self, cx: &mut Context<Self>) {
        if self.mode.toggle(OverlayKind::PaneFacts) {
            self.refresh_pane_facts(cx);
        } else {
            self.discard_pane_facts();
        }
        cx.notify();
    }

    /// Close the facts panel and drop its cached snapshot. Any in-flight
    /// collection lands as stale, because it checks both the overlay and the
    /// pane it was started for before storing anything.
    fn close_pane_facts(&mut self, cx: &mut Context<Self>) {
        self.mode.close(OverlayKind::PaneFacts);
        self.discard_pane_facts();
        cx.notify();
    }

    fn discard_pane_facts(&mut self) {
        self.facts = PaneFactsState::Idle;
    }

    /// Kick off an off-thread facts collection for the focused pane.
    ///
    /// `sysinfo` process-tree walks and `lsof` are far too slow to run on the
    /// UI thread, so the collection happens on the background executor and the
    /// result lands back through `PaneFactsState`. Results tagged with a stale
    /// pane, or arriving after the panel closed, are dropped.
    fn refresh_pane_facts(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane_key() else {
            self.facts = PaneFactsState::Idle;
            return;
        };
        let view = self.active_view(cx);
        let cwd = view.as_ref().and_then(|v| v.read(cx).working_directory(cx));
        let foreground = view
            .as_ref()
            .and_then(|v| v.read(cx).foreground_process_command_name(cx));
        let root = view.as_ref().and_then(|v| v.read(cx).shell_pid(cx));

        self.facts.begin_collection(pane);

        cx.spawn(async move |this, cx| {
            let facts = cx
                .background_spawn(async move {
                    crate::chrome::pane_facts::collect_live_facts(cwd, foreground, root)
                })
                .await;
            this.update(cx, |this, cx| {
                // Focus may have moved, or the panel closed, while we were off
                // thread. Either way this snapshot is no longer what is shown.
                if !this.mode.is(OverlayKind::PaneFacts) || this.active_pane_key() != Some(pane) {
                    return;
                }
                this.facts.finish_collection(pane, facts);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_pane_facts_if_stale(&mut self, cx: &mut Context<Self>) {
        if !self.mode.is(OverlayKind::PaneFacts) {
            return;
        }
        let Some(pane) = self.active_pane_key() else {
            return;
        };
        // Render calls this every frame; never stack a second collection for a
        // pane that already has one in flight.
        if self.facts.is_collecting_for(pane) {
            return;
        }
        if self.facts.needs_refresh_for(pane, PANE_FACTS_MAX_AGE) {
            self.refresh_pane_facts(cx);
        }
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

    fn open_palette(&mut self, cx: &mut Context<Self>) {
        self.mode.open(OverlayKind::Palette);
        self.palette_query.clear();
        self.palette_selected = 0;
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.close(OverlayKind::Palette) {
            self.palette_query.clear();
            self.palette_selected = 0;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn filtered_palette_indices(&self) -> Vec<usize> {
        filter_commands(&self.palette_items, &self.palette_query)
    }

    /// Palette entry point: close the palette, then run the command through
    /// the single canonical dispatcher in `command_dispatch`.
    fn run_command(&mut self, id: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        self.dispatch_command(id, window, cx);
    }

    fn palette_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.mode.is(OverlayKind::Palette) {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_palette(window, cx);
                true
            }
            "enter" => {
                let hits = self.filtered_palette_indices();
                if let Some(&idx) = hits.get(self.palette_selected) {
                    let id = self.palette_items[idx].id;
                    self.run_command(id, window, cx);
                }
                true
            }
            "up" | "arrowup" => {
                let hits = self.filtered_palette_indices();
                if !hits.is_empty() {
                    self.palette_selected = if self.palette_selected == 0 {
                        hits.len() - 1
                    } else {
                        self.palette_selected - 1
                    };
                    cx.notify();
                }
                true
            }
            "down" | "arrowdown" => {
                let hits = self.filtered_palette_indices();
                if !hits.is_empty() {
                    self.palette_selected = (self.palette_selected + 1) % hits.len();
                    cx.notify();
                }
                true
            }
            "backspace" => {
                self.palette_query.pop();
                self.palette_selected = 0;
                cx.notify();
                true
            }
            _ => {
                if let Some(ch) = event.keystroke.key_char.as_ref() {
                    if !ch.is_empty() && !ch.chars().any(|c| c.is_control()) {
                        self.palette_query.push_str(ch);
                        self.palette_selected = 0;
                        cx.notify();
                    }
                }
                true
            }
        }
    }

    // ── find in scrollback (M10) ────────────────────────────────────────────

    fn on_find(&mut self, _: &Find, _window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(cx);
    }

    pub(crate) fn open_find(&mut self, cx: &mut Context<Self>) {
        self.mode.open_find();
        cx.notify();
        // Re-run search if query already present.
        if !self.find_query.is_empty() {
            self.run_find(cx);
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.find_open {
            self.mode.close_find();
            self.clear_find_matches(cx);
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn clear_find_matches(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self.active_view(cx) {
            if let Some(term) = view.read(cx).terminal_entity().cloned() {
                term.update(cx, |t, _| t.matches.clear());
            }
        }
        self.find_match_count = 0;
        self.find_active_index = 0;
    }

    /// Resolve the raw query into the regex handed to alacritty's search.
    ///
    /// Literal mode escapes regex metacharacters; regex mode passes the query
    /// through unchanged. Case is controlled with the `(?i)` / `(?-i)` inline
    /// flags, which alacritty's regex engine honours regardless of its
    /// smart-case default.
    fn find_pattern(&self, query: &str) -> String {
        let pattern = if self.find_regex {
            query.to_owned()
        } else {
            regex_escape_literal(query)
        };
        if self.find_match_case {
            format!("(?-i){pattern}")
        } else {
            format!("(?i){pattern}")
        }
    }

    fn run_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query.clone();
        if query.is_empty() {
            self.clear_find_matches(cx);
            cx.notify();
            return;
        }
        let Some(search) = terminal::Search::new(&self.find_pattern(&query)) else {
            self.clear_find_matches(cx);
            cx.notify();
            return;
        };
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let Some(term) = view.read(cx).terminal_entity().cloned() else {
            return;
        };
        let task = term.update(cx, |t, cx| t.find_matches(search, cx));
        cx.spawn(async move |this, cx| {
            let matches = task.await;
            this.update(cx, |this, cx| {
                let count = matches.len();
                if let Some(view) = this.active_view(cx) {
                    if let Some(term) = view.read(cx).terminal_entity().cloned() {
                        term.update(cx, |t, _| {
                            t.matches = matches;
                            if count > 0 {
                                t.activate_match(0);
                            }
                        });
                    }
                }
                this.find_match_count = count;
                this.find_active_index = 0;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn on_find_next(&mut self, _: &FindNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_find(1, cx);
    }

    fn on_find_prev(&mut self, _: &FindPrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_find(-1, cx);
    }

    fn step_find(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.find_match_count == 0 {
            if self.mode.find_open && !self.find_query.is_empty() {
                self.run_find(cx);
            }
            return;
        }
        let n = self.find_match_count as i32;
        let next = (self.find_active_index as i32 + delta).rem_euclid(n) as usize;
        self.find_active_index = next;
        if let Some(view) = self.active_view(cx) {
            if let Some(term) = view.read(cx).terminal_entity().cloned() {
                term.update(cx, |t, _| t.activate_match(next));
            }
        }
        cx.notify();
    }

    fn find_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.mode.find_open {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_find(window, cx);
                true
            }
            "enter" => {
                if event.keystroke.modifiers.shift {
                    self.step_find(-1, cx);
                } else {
                    // First Enter runs search; subsequent Enter = next match.
                    if self.find_match_count == 0 || event.keystroke.modifiers.platform {
                        self.run_find(cx);
                    } else {
                        self.step_find(1, cx);
                    }
                }
                true
            }
            "backspace" => {
                self.find_query.pop();
                self.run_find(cx);
                true
            }
            // ⌥⌘C toggles match-case; ⌥⌘R toggles regex (macOS find-bar convention).
            "c" if event.keystroke.modifiers.alt && event.keystroke.modifiers.platform => {
                self.find_match_case = !self.find_match_case;
                self.run_find(cx);
                true
            }
            "r" if event.keystroke.modifiers.alt && event.keystroke.modifiers.platform => {
                self.find_regex = !self.find_regex;
                self.run_find(cx);
                true
            }
            _ => {
                if let Some(ch) = event.keystroke.key_char.as_ref() {
                    if !ch.is_empty() && !ch.chars().any(|c| c.is_control()) {
                        self.find_query.push_str(ch);
                        self.run_find(cx);
                    }
                }
                // Swallow non-platform keys so they don't go to the PTY.
                !event.keystroke.modifiers.platform
            }
        }
    }

    fn render_command_palette(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hits = self.filtered_palette_indices();
        let selected = self.palette_selected.min(hits.len().saturating_sub(1));
        let query: SharedString = if self.palette_query.is_empty() {
            "Type a command…".into()
        } else {
            format!("{}|", self.palette_query).into()
        };
        let query_color = if self.palette_query.is_empty() {
            tokens.fg_muted
        } else {
            tokens.fg
        };

        let mut list = div()
            .id("palette-list")
            .flex()
            .flex_col()
            .w_full()
            .max_h(px(320.0))
            .overflow_y_scroll()
            .py_1();

        if hits.is_empty() {
            list = list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_color(tokens.fg_muted)
                    .text_sm()
                    .child("No matching commands"),
            );
        } else {
            for (row_i, &item_i) in hits.iter().enumerate() {
                let item = &self.palette_items[item_i];
                let id = item.id;
                let title = item.title.clone();
                let shortcut = item.shortcut.clone();
                let is_sel = row_i == selected;
                list = list.child(
                    div()
                        .id(("palette-row", row_i as u64))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .px_3()
                        .py_1p5()
                        .cursor_pointer()
                        .when(is_sel, |el| el.bg(tokens.hover))
                        .hover(|el| el.bg(tokens.hover))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.run_command(id, window, cx);
                        }))
                        .child(div().text_sm().text_color(tokens.fg).child(title))
                        .child(div().text_xs().text_color(tokens.fg_muted).child(shortcut)),
                );
            }
        }

        deferred(
            div()
                .id("palette-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // BlockMouse: otherwise TermElement under the overlay still
                // sees should_handle_scroll() and the terminal scrolls too.
                .occlude()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(80.0))
                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.35))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.close_palette(window, cx);
                    }),
                )
                .child(
                    div()
                        .id("palette-panel")
                        .w(px(480.0))
                        .max_w(relative(0.9))
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(tokens.border)
                        .bg(tokens.content_bg)
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .px_3()
                                .py_2p5()
                                .border_b_1()
                                .border_color(tokens.border)
                                .text_sm()
                                .text_color(query_color)
                                .child(query),
                        )
                        .child(list),
                ),
        )
    }

    fn render_find_bar(&self, tokens: &ChromeTokens, cx: &mut Context<Self>) -> impl IntoElement {
        let query_display: SharedString = if self.find_query.is_empty() {
            "Find in scrollback…".into()
        } else {
            format!("{}|", self.find_query).into()
        };
        let query_color = if self.find_query.is_empty() {
            tokens.fg_muted
        } else {
            tokens.fg
        };
        let count: SharedString = if self.find_query.is_empty() {
            "".into()
        } else if self.find_match_count == 0 {
            "0 matches".into()
        } else {
            format!("{}/{}", self.find_active_index + 1, self.find_match_count).into()
        };
        // Legible on-accent foreground for the active toggle buttons.
        let on_accent = if tokens.accent.l < 0.5 {
            Hsla::white()
        } else {
            Hsla::black()
        };

        div()
            .id("find-bar")
            .w_full()
            .h(px(36.0))
            .px_3()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .bg(tokens.content_bg)
            .border_b_1()
            .border_color(tokens.border)
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from("Find")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(tokens.hover)
                    .text_sm()
                    .text_color(query_color)
                    .child(query_display),
            )
            .child(
                div()
                    .id("find-match-case")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .when(self.find_match_case, |el| el.bg(tokens.accent))
                    .text_xs()
                    .text_color(if self.find_match_case {
                        on_accent
                    } else {
                        tokens.fg_muted
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find_match_case = !this.find_match_case;
                        this.run_find(cx);
                    }))
                    .child(SharedString::from("Aa")),
            )
            .child(
                div()
                    .id("find-regex")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .when(self.find_regex, |el| el.bg(tokens.accent))
                    .text_xs()
                    .text_color(if self.find_regex {
                        on_accent
                    } else {
                        tokens.fg_muted
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find_regex = !this.find_regex;
                        this.run_find(cx);
                    }))
                    .child(SharedString::from(".*")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .min_w(px(72.0))
                    .child(count),
            )
            .child(
                div()
                    .id("find-prev")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_sm()
                    .text_color(tokens.fg)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.step_find(-1, cx);
                    }))
                    .child(SharedString::from("↑")),
            )
            .child(
                div()
                    .id("find-next")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_sm()
                    .text_color(tokens.fg)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.step_find(1, cx);
                    }))
                    .child(SharedString::from("↓")),
            )
            .child(
                div()
                    .id("find-close")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_sm()
                    .text_color(tokens.fg_muted)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.close_find(window, cx);
                    }))
                    .child(SharedString::from("✕")),
            )
    }
}

/// Escape a literal string for use inside a regex (alacritty search is regex-based).
fn regex_escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn tree_contains(tree: &PaneNode, id: PaneId) -> bool {
    let mut leaves = Vec::new();
    tree.leaves(&mut leaves);
    leaves.iter().any(|(leaf, _)| *leaf == id)
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
}

fn snapshot_tree(node: &PaneNode, cx: &App) -> SessionNode {
    match node {
        PaneNode::Leaf { id, pane_key, view } => SessionNode::Leaf {
            id: *id,
            cwd: view
                .read(cx)
                .working_directory(cx)
                .map(|p| p.to_string_lossy().into_owned()),
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
        let palette = TerminalPalette::get_global(cx);
        let window_active = window.is_window_active();
        let tokens = ChromeTokens::from_palette(&palette, window_active);
        let fullscreen = window.is_fullscreen();
        let geo = ChromeGeometry::for_window(cfg!(not(target_os = "macos")), fullscreen);
        let leading = geo.leading_pad;
        let chrome_h = geo.height;
        let banner_top = chrome_h;

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
                let tab_scroll = self.render_tab_strip(&tokens, &geo, window, cx);
                let chrome_band = div()
                    .id("chrome-band")
                    .h(chrome_h)
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .bg(tokens.content_bg)
                    .child(leading_drag)
                    .child(tab_scroll)
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_2()
                            .child(self.render_diff_chrome_button(&tokens, &palette, cx)),
                    )
                    .child(trailing_drag)
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
            .when(self.mode.is(OverlayKind::History), |el| {
                el.child(self.render_history_search(&tokens, cx))
            })
            .when(self.mode.is(OverlayKind::Diff), |el| {
                el.child(self.render_diff_overlay(&tokens, &palette, window, cx))
            })
            .when_some(self.active_tombstone(cx), |el, stone| {
                el.child(self.render_tombstone(&tokens, stone))
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
