//! Single-window multi-tab shell for sleipnir (HIG-aligned chrome).

use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString, size,
    StatefulInteractiveElement as _, Styled as _, Task, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, canvas, deferred, div, point,
    prelude::FluentBuilder as _, px, relative,
};
use sleipnir_settings::{
    Appearance, ConfirmClose, TerminalPalette, TerminalSettings, ThemeName, palette_for_theme,
};

use crate::TermView;
use crate::chrome::{ChromeGeometry, ChromeTokens, active_after_close};
use crate::command_palette::{CommandId, CommandItem, commands as palette_commands, filter_commands};
use crate::pane_tree::{
    Branch, CloseOutcome, Direction, MIN_RATIO, PaneId, PaneNode, PaneRect, SplitAxis, SplitPath,
    neighbor,
};
use crate::session::{
    SessionAxis, SessionFile, SessionNode, SessionTab, load_session, resolve_cwd, sanitize_session,
    save_session, session_path,
};

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
    ]
);

/// Activate the tab at the given 1-based index (⌘1..⌘9).
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = sleipnir, no_json)]
pub struct ActivateTab(pub usize);

struct Tab {
    id: u64,
    /// Recursive pane layout; a fresh tab is a single leaf.
    tree: PaneNode,
    /// The pane that currently holds focus within this tab.
    active_pane: PaneId,
    /// User-assigned title (via right-click rename). When set, it overrides the
    /// active pane's title on the tab chip.
    custom_title: Option<SharedString>,
    /// When set, only this pane is shown full-content (M13 pane zoom).
    zoomed_pane: Option<PaneId>,
}

impl Tab {
    /// Title shown on the tab chip: the user-assigned title if set, otherwise
    /// the active pane's title.
    fn title(&self, cx: &App) -> SharedString {
        if let Some(custom) = self.custom_title.as_ref() {
            if !custom.is_empty() {
                return custom.clone();
            }
        }
        self.pane_title(cx)
    }

    /// The active pane's own title (ignores any custom override).
    fn pane_title(&self, cx: &App) -> SharedString {
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
struct RenameState {
    tab_id: u64,
    buffer: String,
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

/// A divider's hit rectangle plus the split it controls, produced by layout.
#[derive(Clone)]
struct DividerRect {
    path: SplitPath,
    axis: SplitAxis,
    /// The split container these children live in (for ratio math on drag).
    container: Bounds<Pixels>,
    /// The thin hit strip to render.
    hit: Bounds<Pixels>,
}

/// Summary of an available release, kept UI-side (decoupled from `updater`).
#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    pub version: String,
    pub tag: String,
    pub notes: String,
    zip_url: String,
    sha256_url: String,
}

/// Auto-update lifecycle, surfaced through the notification bar.
#[derive(Clone, Debug, Default)]
pub enum UpdateUiState {
    /// No update activity to show.
    #[default]
    Idle,
    /// A background/manual check is running.
    Checking,
    /// Running build is current (only shown after a manual check).
    UpToDate,
    /// A newer release is available to download.
    Available(AvailableUpdate),
    /// Downloading + verifying the release artifact.
    Downloading(AvailableUpdate),
    /// Verified and staged; a restart will apply it.
    ReadyToRestart(AvailableUpdate),
    /// Something went wrong (message shown to the user).
    Failed(String),
}

/// Window root: unified chrome band + active terminal.
pub struct AppShell {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    /// Monotonic id source for panes across all tabs.
    next_pane_id: PaneId,
    focus_handle: FocusHandle,
    /// Empty-region drag: true after mouse-down on a drag strip until move/up.
    should_move: bool,
    tab_scroll_handle: ScrollHandle,
    /// Tab id currently under the pointer (for hover close / hover fill).
    hovered_tab: Option<u64>,
    /// Pane rects from the last render, for keyboard neighbor navigation.
    pane_rects: Vec<PaneRect>,
    /// Content area bounds captured last frame (origin + size), for analytic
    /// pane layout and divider hit-testing.
    content_bounds: Option<Bounds<Pixels>>,
    /// Active divider drag, if any.
    drag: Option<DragState>,
    /// In-progress inline tab rename, if any.
    rename: Option<RenameState>,
    /// Whether the settings overlay is visible.
    settings_open: bool,
    /// Active section tab inside the settings panel.
    settings_section: SettingsSection,
    /// Auto-update lifecycle state (drives the update dialog).
    update_state: UpdateUiState,
    /// Whether the update dialog is visible.
    update_open: bool,
    /// Verified update zip path, ready to install on restart.
    staged_zip: Option<std::path::PathBuf>,
    /// Command palette open state (M9).
    palette_open: bool,
    palette_query: String,
    palette_selected: usize,
    palette_items: Vec<CommandItem>,
    /// Find-in-scrollback bar (M10).
    find_open: bool,
    find_query: String,
    find_match_count: usize,
    find_active_index: usize,
    /// Window-scoped font size override (M12 zoom); not written to settings.
    font_size_override: Option<Pixels>,
    /// Close-confirm dialog pending (M12).
    close_confirm: Option<CloseConfirmState>,
    /// Tab ids currently flashing for visual bell (M12).
    bell_flash_tabs: std::collections::HashSet<u64>,
    /// Fan-out keystrokes to all panes in the active tab (M13).
    broadcast: bool,
    /// Quick Select overlay active (M15).
    quick_select_open: bool,
    /// Debounced session save task.
    _session_save_task: Option<Task<()>>,
    /// Keep the app-quit subscription alive for the window lifetime.
    _quit_subscription: Option<gpui::Subscription>,
}

/// Pending close confirmation dialog.
struct CloseConfirmState {
    /// Human-readable what will close.
    message: SharedString,
}

impl Focusable for AppShell {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for AppShell {}

/// Open a new independent Sleipnir window (startup + ⌘N).
pub fn open_sleipnir_window(cx: &mut App) {
    let geo = ChromeGeometry::standard();
    let bounds = Bounds::centered(None, size(px(1024.0), px(680.0)), cx);
    if let Err(err) = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Sleipnir".into()),
                appears_transparent: true,
                traffic_light_position: if cfg!(windows) {
                    None
                } else {
                    Some(geo.traffic_light_position)
                },
            }),
            app_owns_titlebar_drag: true,
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(360.0), px(240.0))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| AppShell::new(window, cx)),
    ) {
        log::error!("failed to open window: {err:#}");
    }
}

impl AppShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            settings_open: false,
            settings_section: SettingsSection::Theme,
            update_state: UpdateUiState::Idle,
            update_open: false,
            staged_zip: None,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_items: palette_commands(),
            find_open: false,
            find_query: String::new(),
            find_match_count: 0,
            find_active_index: 0,
            font_size_override: None,
            close_confirm: None,
            bell_flash_tabs: std::collections::HashSet::new(),
            broadcast: false,
            quick_select_open: false,
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

        // Persist session on quit (M8).
        shell._quit_subscription = Some(cx.on_app_quit(|this, cx| {
            this.persist_session_now(cx);
            async {}
        }));

        let restore = TerminalSettings::get_global(cx).restore_session;
        let restored = if restore {
            shell.try_restore_session(window, cx)
        } else {
            false
        };
        if !restored {
            shell.add_tab(window, cx);
        }
        shell
    }

    fn sync_window_title(&self, window: &mut Window, cx: &App) {
        let title = self
            .tabs
            .get(self.active)
            .map(|t| t.title(cx).to_string())
            .unwrap_or_else(|| "Sleipnir".to_string());
        window.set_window_title(&title);
    }

    /// Create a fresh `TermView` and wire the observers a pane needs
    /// (repaint on change, window-title sync on title change, event routing).
    fn spawn_term_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TermView> {
        self.spawn_term_view_with_cwd(None, window, cx)
    }

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
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(
            &view,
            window,
            |this, view, event: &crate::TermViewEvent, window, cx| match event {
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
            },
        )
        .detach();
        view
    }

    /// Flash the tab that owns `view` when visual bell is enabled.
    fn on_term_bell(&mut self, view: &Entity<TermView>, cx: &mut Context<Self>) {
        use sleipnir_settings::TerminalBell;
        if !matches!(
            TerminalSettings::get_global(cx).bell,
            TerminalBell::Visual
        ) {
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

    fn apply_font_override_to_all_panes(&self, cx: &mut Context<Self>) {
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
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.zoomed_pane.is_some() {
            tab.zoomed_pane = None;
        } else {
            tab.zoomed_pane = Some(tab.active_pane);
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    fn on_toggle_broadcast(
        &mut self,
        _: &ToggleBroadcast,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.quick_select_open = !self.quick_select_open;
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
            SessionNode::Leaf { id, cwd } => {
                let view = self.spawn_term_view_with_cwd(resolve_cwd(cwd.as_deref()), window, cx);
                PaneNode::leaf(*id, view)
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
    fn schedule_session_save(&mut self, cx: &mut Context<Self>) {
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
            }).ok();
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

    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let view = self.spawn_term_view(window, cx);
        self.tabs.push(Tab {
            id,
            tree: PaneNode::leaf(pane_id, view),
            active_pane: pane_id,
            custom_title: None,
            zoomed_pane: None,
        });
        self.active = self.tabs.len() - 1;
        self.focus_active(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        self.schedule_session_save(cx);
        cx.notify();
    }


    /// Begin an inline rename for the given tab, seeding the editable buffer
    /// with its current title.
    fn begin_rename(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let buffer = tab.title(cx).to_string();
        self.rename = Some(RenameState { tab_id, buffer });
        cx.notify();
    }

    /// Commit the in-progress rename to the target tab. An empty buffer clears
    /// the custom title so the tab falls back to the pane's own title.
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
        match active_after_close(self.active, index, self.tabs.len()) {
            None => {
                self.tabs.remove(index);
                // Always keep at least one tab.
                self.add_tab(window, cx);
            }
            Some(new_active) => {
                self.tabs.remove(index);
                self.active = new_active;
                self.focus_active(window, cx);
                self.sync_window_title(window, cx);
                self.tab_scroll_handle.scroll_to_item(self.active);
                self.schedule_session_save(cx);
                cx.notify();
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

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active = index;
            self.focus_active(window, cx);
            self.sync_window_title(window, cx);
            self.tab_scroll_handle.scroll_to_item(self.active);
            self.schedule_session_save(cx);
            cx.notify();
        }
    }

    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let next = (self.active + 1) % self.tabs.len();
        self.activate(next, window, cx);
    }

    fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_view(cx) {
            let handle = view.focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    /// Split the active pane along `axis`, placing a new pane on the far side
    /// and focusing it.
    fn split_active(
        &mut self,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.tabs.get(self.active).map(|t| t.active_pane) else {
            return;
        };
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let view = self.spawn_term_view(window, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.tree.split(target, axis, new_id, view) {
                tab.active_pane = new_id;
            }
        }
        self.focus_active(window, cx);
        self.schedule_session_save(cx);
        cx.notify();
    }

    /// Move focus to the neighboring pane in `direction`, if one exists.
    fn focus_pane(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if let Some(next) = neighbor(&self.pane_rects, tab.active_pane, direction) {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.active_pane = next;
            }
            self.focus_active(window, cx);
            self.schedule_session_save(cx);
            cx.notify();
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
        match tab.tree.close(target) {
            CloseOutcome::TreeEmpty => {
                self.close_active_tab(window, cx);
            }
            CloseOutcome::NotFound => {
                // Stale active_pane id: recover focus instead of nuking the tab.
                tab.active_pane = tab.tree.first_leaf_id();
                self.focus_active(window, cx);
                cx.notify();
            }
            CloseOutcome::Closed => {
                // Surviving subtree: focus its first leaf (the collapsed sibling
                // when the closed pane was a direct child of a split).
                tab.active_pane = tab.tree.first_leaf_id();
                self.sync_window_title(window, cx);
                self.focus_active(window, cx);
                self.schedule_session_save(cx);
                cx.notify();
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
        leaves
            .iter()
            .any(|(_, view)| view.read(cx).looks_busy(cx))
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
            self.close_confirm = Some(CloseConfirmState {
                message: "A process is still running. Close this pane anyway?".into(),
            });
            cx.notify();
        } else {
            self.close_active_pane(window, cx);
        }
    }

    fn confirm_close_proceed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_confirm = None;
        self.close_active_pane(window, cx);
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

    fn on_activate_tab(&mut self, action: &ActivateTab, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.palette_open {
            self.close_palette(window, cx);
        } else {
            self.open_palette(cx);
        }
    }

    fn open_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.settings_open = false;
        self.find_open = false;
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette_open {
            self.palette_open = false;
            self.palette_query.clear();
            self.palette_selected = 0;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn filtered_palette_indices(&self) -> Vec<usize> {
        filter_commands(&self.palette_items, &self.palette_query)
    }

    fn run_command(&mut self, id: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        match id {
            CommandId::NewTab => self.add_tab(window, cx),
            CommandId::ClosePane => self.request_close_active_pane(window, cx),
            CommandId::NextTab => self.next_tab(window, cx),
            CommandId::PrevTab => self.prev_tab(window, cx),
            CommandId::SplitRight => self.split_active(SplitAxis::Horizontal, window, cx),
            CommandId::SplitDown => self.split_active(SplitAxis::Vertical, window, cx),
            CommandId::OpenSettings => {
                self.settings_open = true;
                self.settings_section = SettingsSection::Theme;
                cx.notify();
            }
            CommandId::ReloadSettings => {
                self.font_size_override = None;
                self.apply_font_override_to_all_panes(cx);
                TerminalSettings::reload(cx);
                cx.notify();
            }
            CommandId::CycleTheme => {
                let next = TerminalSettings::get_global(cx).theme.next();
                TerminalSettings::set_theme(next, cx);
                cx.notify();
            }
            CommandId::CheckForUpdates => {
                self.update_open = true;
                self.spawn_update_check(window, cx);
            }
            CommandId::Find => self.open_find(cx),
            CommandId::ToggleCommandPalette => self.open_palette(cx),
            CommandId::IncreaseFontSize => {
                use crate::FONT_SIZE_STEP;
                self.step_font_size(FONT_SIZE_STEP, cx);
            }
            CommandId::DecreaseFontSize => {
                use crate::FONT_SIZE_STEP;
                self.step_font_size(-FONT_SIZE_STEP, cx);
            }
            CommandId::ResetFontSize => self.reset_font_size(cx),
            CommandId::NewWindow => open_sleipnir_window(cx),
            CommandId::TogglePaneZoom => self.on_toggle_pane_zoom(&TogglePaneZoom, window, cx),
            CommandId::ToggleBroadcast => self.on_toggle_broadcast(&ToggleBroadcast, window, cx),
            CommandId::JumpPrevPrompt => self.on_jump_prev_prompt(&JumpPrevPrompt, window, cx),
            CommandId::JumpNextPrompt => self.on_jump_next_prompt(&JumpNextPrompt, window, cx),
            CommandId::ToggleQuickSelect => {
                self.on_toggle_quick_select(&ToggleQuickSelect, window, cx)
            }
            CommandId::OpenQuickTerminal => {
                self.on_open_quick_terminal(&OpenQuickTerminal, window, cx)
            }
        }
    }

    fn palette_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.palette_open {
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

    fn open_find(&mut self, cx: &mut Context<Self>) {
        self.find_open = true;
        self.palette_open = false;
        self.settings_open = false;
        cx.notify();
        // Re-run search if query already present.
        if !self.find_query.is_empty() {
            self.run_find(cx);
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_open {
            self.find_open = false;
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

    fn run_find(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query.clone();
        if query.is_empty() {
            self.clear_find_matches(cx);
            cx.notify();
            return;
        }
        let Some(search) = terminal::Search::new(&regex_escape_literal(&query)) else {
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
            if self.find_open && !self.find_query.is_empty() {
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
        if !self.find_open {
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

    // ── auto-update ─────────────────────────────────────────────────────────

    fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Open the update dialog and start a check.
        self.update_open = true;
        self.spawn_update_check(window, cx);
    }

    fn close_update(&mut self, cx: &mut Context<Self>) {
        self.update_open = false;
        cx.notify();
    }

    /// Query GitHub for a newer release; result is shown in the update dialog.
    fn spawn_update_check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            UpdateUiState::Checking | UpdateUiState::Downloading(_)
        ) {
            return;
        }
        self.update_state = UpdateUiState::Checking;
        cx.notify();

        let current = release_channel::AppVersion::global(cx).to_string();
        cx.spawn_in(window, async move |this, cx| {
            // ureq is blocking — run it on the background executor so we never
            // touch a (nonexistent) Tokio reactor on the main thread.
            let result = cx
                .background_spawn(async move { updater::fetch_latest(&current) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(updater::UpdateStatus::Available(info)) => {
                        this.update_state = UpdateUiState::Available(AvailableUpdate {
                            version: info.version.to_string(),
                            tag: info.tag,
                            notes: info.notes,
                            zip_url: info.zip_url,
                            sha256_url: info.sha256_url,
                        });
                    }
                    Ok(updater::UpdateStatus::UpToDate) => {
                        this.update_state = UpdateUiState::UpToDate;
                    }
                    Err(err) => {
                        log::warn!("update check failed: {err:#}");
                        this.update_state =
                            UpdateUiState::Failed(format!("Update check failed: {err}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Download + verify the available release, then stage it for restart.
    fn start_download(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let update = match &self.update_state {
            UpdateUiState::Available(u) => u.clone(),
            _ => return,
        };
        self.update_state = UpdateUiState::Downloading(update.clone());
        cx.notify();

        let info = updater::ReleaseInfo {
            version: match updater::parse_tag(&update.tag) {
                Ok(v) => v,
                Err(err) => {
                    self.update_state = UpdateUiState::Failed(format!("{err}"));
                    cx.notify();
                    return;
                }
            },
            tag: update.tag.clone(),
            notes: update.notes.clone(),
            zip_url: update.zip_url.clone(),
            sha256_url: update.sha256_url.clone(),
        };
        let dest = std::env::temp_dir().join(format!("sleipnir-update-{}", std::process::id()));

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { updater::download_and_verify(&info, &dest) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(zip_path) => {
                        // Remember where the verified zip landed so a restart
                        // can install it.
                        this.staged_zip = Some(zip_path);
                        this.update_state = UpdateUiState::ReadyToRestart(update.clone());
                    }
                    Err(err) => {
                        log::warn!("update download failed: {err:#}");
                        this.update_state =
                            UpdateUiState::Failed(format!("Download failed: {err}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Install the staged update and relaunch, or fall back to the releases page.
    fn install_and_restart(&mut self, cx: &mut Context<Self>) {
        let Some(zip) = self.staged_zip.clone() else {
            return;
        };
        match updater::current_app_bundle_path() {
            Some(app) => match updater::install_and_relaunch(&zip, &app) {
                Ok(()) => cx.quit(),
                Err(err) => {
                    log::warn!("install failed: {err:#}");
                    self.update_state =
                        UpdateUiState::Failed(format!("Install failed: {err}"));
                    cx.open_url(updater::RELEASES_PAGE);
                    cx.notify();
                }
            },
            None => {
                // Dev build or non-bundle launch: manual install.
                cx.open_url(updater::RELEASES_PAGE);
            }
        }
    }

    /// A pill button for the update dialog.
    fn update_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        tokens: &ChromeTokens,
        primary: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let (bg, fg) = if primary {
            (tokens.accent, tokens.content_bg)
        } else {
            (tokens.hover, tokens.fg)
        };
        div()
            .id(id)
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(bg)
            .text_color(fg)
            .text_size(px(13.0))
            .cursor_pointer()
            .hover(|el| el.opacity(0.9))
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    /// Modal update dialog reflecting the current [`UpdateUiState`].
    fn render_update_overlay(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = release_channel::AppVersion::global(cx).to_string();

        // Headline + detail + action buttons per state.
        let (headline, detail, buttons): (SharedString, SharedString, Vec<gpui::AnyElement>) =
            match &self.update_state {
                UpdateUiState::Idle | UpdateUiState::Checking => (
                    "Checking for updates…".into(),
                    format!("Current version {current}").into(),
                    Vec::new(),
                ),
                UpdateUiState::UpToDate => (
                    "You’re up to date".into(),
                    format!("Sleipnir {current} is the latest version.").into(),
                    vec![
                        self.update_button("upd-close", "Close", tokens, true, cx, |this, _, cx| {
                            this.close_update(cx);
                        })
                        .into_any_element(),
                    ],
                ),
                UpdateUiState::Available(u) => (
                    "Update available".into(),
                    format!("Sleipnir {} is available (you have {current}).", u.version).into(),
                    vec![
                        self.update_button(
                            "upd-notes",
                            "Release Notes",
                            tokens,
                            false,
                            cx,
                            |_, _, cx| cx.open_url(updater::RELEASES_PAGE),
                        )
                        .into_any_element(),
                        self.update_button("upd-later", "Later", tokens, false, cx, |this, _, cx| {
                            this.close_update(cx);
                        })
                        .into_any_element(),
                        self.update_button(
                            "upd-install",
                            "Download & Install",
                            tokens,
                            true,
                            cx,
                            |this, window, cx| this.start_download(window, cx),
                        )
                        .into_any_element(),
                    ],
                ),
                UpdateUiState::Downloading(u) => (
                    "Downloading update…".into(),
                    format!("Fetching and verifying Sleipnir {}…", u.version).into(),
                    Vec::new(),
                ),
                UpdateUiState::ReadyToRestart(u) => (
                    "Ready to install".into(),
                    format!("Sleipnir {} is verified. Restart to finish updating.", u.version)
                        .into(),
                    vec![
                        self.update_button("upd-later2", "Later", tokens, false, cx, |this, _, cx| {
                            this.close_update(cx);
                        })
                        .into_any_element(),
                        self.update_button(
                            "upd-restart",
                            "Restart & Update",
                            tokens,
                            true,
                            cx,
                            |this, _, cx| this.install_and_restart(cx),
                        )
                        .into_any_element(),
                    ],
                ),
                UpdateUiState::Failed(msg) => (
                    "Update failed".into(),
                    msg.clone().into(),
                    vec![
                        self.update_button("upd-close2", "Close", tokens, true, cx, |this, _, cx| {
                            this.close_update(cx);
                        })
                        .into_any_element(),
                    ],
                ),
            };

        let panel = div()
            .id("update-panel")
            .w(px(420.0))
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(12.0))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .text_color(tokens.fg)
            .px_5()
            .py_4()
            // Keep clicks inside the panel from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(headline),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(tokens.fg_muted)
                    .child(detail),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .children(buttons),
            );

        deferred(
            div()
                .id("update-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // BlockMouse: otherwise TermElement under the overlay still
                // sees should_handle_scroll() and the terminal scrolls too.
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("update-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _window, cx| {
                                this.close_update(cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }

    fn on_open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_settings(window, cx);
    }


    fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        if self.settings_open {
            // Always land on Theme when reopening; future sections can restore.
            self.settings_section = SettingsSection::Theme;
        } else {
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            self.settings_open = false;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn select_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.settings_section != section {
            self.settings_section = section;
            cx.notify();
        }
    }

    fn select_theme(&mut self, theme: ThemeName, cx: &mut Context<Self>) {
        TerminalSettings::set_theme(theme, cx);
        cx.notify();
    }

    fn attach_empty_drag(
        &self,
        id: impl Into<ElementId>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .h_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = false;
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.should_move = false;
            }))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.should_move {
                    this.should_move = false;
                    window.start_window_move();
                }
            }))
            .on_click(cx.listener(|_, event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            }))
    }

    /// Width/height of a divider's draggable hit strip.
    const DIVIDER_HIT: f32 = 8.0;

    /// Walk the active tab's tree over `area`, producing every leaf's rect and
    /// every divider's hit strip. Purely analytic — mirrors the flex layout.
    fn compute_layout(
        tree: &PaneNode,
        area: Bounds<Pixels>,
        path: SplitPath,
        panes: &mut Vec<PaneRect>,
        dividers: &mut Vec<DividerRect>,
    ) {
        match tree {
            PaneNode::Leaf { id, .. } => {
                panes.push(PaneRect { id: *id, bounds: area });
            }
            PaneNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let hit = px(Self::DIVIDER_HIT);
                match axis {
                    SplitAxis::Horizontal => {
                        let w = f32::from(area.size.width);
                        let first_w = (w * *ratio).max(0.0);
                        let first_area = Bounds::new(
                            area.origin,
                            gpui::size(px(first_w), area.size.height),
                        );
                        let second_area = Bounds::new(
                            point(px(f32::from(area.origin.x) + first_w), area.origin.y),
                            gpui::size(px(w - first_w), area.size.height),
                        );
                        dividers.push(DividerRect {
                            path: path.clone(),
                            axis: *axis,
                            container: area,
                            hit: Bounds::new(
                                point(
                                    px(f32::from(area.origin.x) + first_w - f32::from(hit) / 2.0),
                                    area.origin.y,
                                ),
                                gpui::size(hit, area.size.height),
                            ),
                        });
                        Self::compute_layout(first, first_area, path.child(Branch::First), panes, dividers);
                        Self::compute_layout(second, second_area, path.child(Branch::Second), panes, dividers);
                    }
                    SplitAxis::Vertical => {
                        let h = f32::from(area.size.height);
                        let first_h = (h * *ratio).max(0.0);
                        let first_area = Bounds::new(
                            area.origin,
                            gpui::size(area.size.width, px(first_h)),
                        );
                        let second_area = Bounds::new(
                            point(area.origin.x, px(f32::from(area.origin.y) + first_h)),
                            gpui::size(area.size.width, px(h - first_h)),
                        );
                        dividers.push(DividerRect {
                            path: path.clone(),
                            axis: *axis,
                            container: area,
                            hit: Bounds::new(
                                point(
                                    area.origin.x,
                                    px(f32::from(area.origin.y) + first_h - f32::from(hit) / 2.0),
                                ),
                                gpui::size(area.size.width, hit),
                            ),
                        });
                        Self::compute_layout(first, first_area, path.child(Branch::First), panes, dividers);
                        Self::compute_layout(second, second_area, path.child(Branch::Second), panes, dividers);
                    }
                }
            }
        }
    }

    fn render_content(
        &mut self,
        tokens: &ChromeTokens,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(tab) = self.tabs.get(self.active) else {
            return div().flex_1().size_full().min_h_0().into_any_element();
        };
        let active_pane = tab.active_pane;
        let tab_id = tab.id;
        let zoomed = tab.zoomed_pane;

        // Gather leaves (id -> view) in tree order.
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        let leaves: Vec<(PaneId, Entity<TermView>)> =
            leaves.into_iter().map(|(id, v)| (id, v.clone())).collect();

        // Pane zoom (M13): only the zoomed leaf is shown full-size.
        if let Some(zid) = zoomed {
            if let Some((_, view)) = leaves.iter().find(|(id, _)| *id == zid) {
                return div()
                    .id("pane-area-zoomed")
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .child(view.clone().into_any_element()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(6.0))
                            .right(px(8.0))
                            .px_2()
                            .py_0p5()
                            .rounded(px(4.0))
                            .bg(tokens.accent.opacity(0.85))
                            .text_size(px(11.0))
                            .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                            .child(format!(
                                "Zoomed · {} to restore",
                                crate::display_shortcut("toggle_pane_zoom")
                            )),
                    )
                    .into_any_element();
            }
        }

        // Analytic layout over last frame's content bounds (if known and non-zero).
        // A 0×0 measure (collapsed canvas) must not drive absolute pane layout.
        let mut pane_rects = Vec::new();
        let mut dividers = Vec::new();
        let usable_bounds = self.content_bounds.filter(|area| {
            f32::from(area.size.width) > 1.0 && f32::from(area.size.height) > 1.0
        });
        if let Some(area) = usable_bounds {
            // Lay out relative to a zero origin; absolute children are positioned
            // relative to the content container, not the window.
            let local = Bounds::new(point(px(0.0), px(0.0)), area.size);
            Self::compute_layout(&tab.tree, local, SplitPath::new(), &mut pane_rects, &mut dividers);
        }
        // Record rects (with true screen origin) for neighbor navigation.
        self.pane_rects = if let Some(area) = usable_bounds {
            pane_rects
                .iter()
                .map(|r| PaneRect {
                    id: r.id,
                    bounds: Bounds::new(
                        point(
                            px(f32::from(area.origin.x) + f32::from(r.bounds.origin.x)),
                            px(f32::from(area.origin.y) + f32::from(r.bounds.origin.y)),
                        ),
                        r.bounds.size,
                    ),
                })
                .collect()
        } else {
            Vec::new()
        };

        // Single pane: render the view directly, still capturing bounds.
        let single = leaves.len() == 1;

        // Measure the content area with a full-size absolute canvas (Zed pattern).
        // Without size_full the canvas collapses to 0×0, which makes multi-pane
        // absolute layout produce empty rects and a blank terminal area.
        let mut container = div()
            .id("pane-area")
            .flex_1()
            .size_full()
            .min_h_0()
            .relative()
            .child(
                canvas(
                    {
                        let shell = cx.weak_entity();
                        move |bounds, _window, cx| {
                            let _ = shell.update(cx, |this, cx| {
                                if this.content_bounds != Some(bounds) {
                                    this.content_bounds = Some(bounds);
                                    // Re-render so multi-pane absolute layout
                                    // picks up the measured size.
                                    cx.notify();
                                }
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        if single {
            let (_, view) = &leaves[0];
            container = container.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(view.clone().into_any_element()),
            );
            return container.into_any_element();
        }

        // Multi-pane: absolutely position each leaf by its computed rect.
        // If we have no measured layout yet (first frame after open/split),
        // fall back to equal flex so panes never disappear entirely.
        if pane_rects.is_empty() {
            let mut row = div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_row()
                .min_h_0();
            for (id, view) in &leaves {
                let is_active = *id == active_pane;
                let pane_id = *id;
                let mut pane = div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(view.clone().into_any_element());
                if !is_active {
                    pane = pane.border_1().border_color(tokens.border);
                } else {
                    pane = pane.border_1().border_color(tokens.accent);
                }
                pane = pane.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            if tab.active_pane != pane_id {
                                tab.active_pane = pane_id;
                                this.focus_active(window, cx);
                                cx.notify();
                            }
                        }
                    }),
                );
                row = row.child(pane);
            }
            container = container.child(row);
            return container.into_any_element();
        }

        for (id, view) in &leaves {
            let rect = pane_rects.iter().find(|r| r.id == *id);
            let Some(rect) = rect else { continue };
            let is_active = *id == active_pane;
            let b = rect.bounds;
            let pane_id = *id;
            let mut pane = div()
                .absolute()
                .left(b.origin.x)
                .top(b.origin.y)
                .w(b.size.width)
                .h(b.size.height)
                .overflow_hidden()
                .child(view.clone().into_any_element())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        if let Some(tab) = this.tabs.get_mut(this.active) {
                            if tab.active_pane != pane_id {
                                tab.active_pane = pane_id;
                                this.focus_active(window, cx);
                                cx.notify();
                            }
                        }
                    }),
                );
            if !is_active {
                // Unfocused split dim (M13): dark overlay ~20% + muted border.
                // Overlay also receives clicks so focusing still works.
                pane = pane.border_1().border_color(tokens.border).child(
                    div()
                        .id(("pane-dim", pane_id))
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.22))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                if let Some(tab) = this.tabs.get_mut(this.active) {
                                    if tab.active_pane != pane_id {
                                        tab.active_pane = pane_id;
                                        this.focus_active(window, cx);
                                        cx.notify();
                                    }
                                }
                            }),
                        ),
                );
            } else {
                pane = pane.border_1().border_color(tokens.accent);
            }
            container = container.child(pane);
        }

        // Divider hit strips.
        for divider in &dividers {
            let h = divider.hit;
            let is_h = matches!(divider.axis, SplitAxis::Horizontal);
            let path = divider.path.clone();
            let axis = divider.axis;
            let container_bounds = divider.container;
            let strip = div()
                .absolute()
                .left(h.origin.x)
                .top(h.origin.y)
                .w(h.size.width)
                .h(h.size.height)
                .when(is_h, |s| s.cursor_col_resize())
                .when(!is_h, |s| s.cursor_row_resize())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        // Recover this divider's live container in screen space.
                        let screen_container = this
                            .content_bounds
                            .map(|area| Bounds::new(
                                point(
                                    px(f32::from(area.origin.x) + f32::from(container_bounds.origin.x)),
                                    px(f32::from(area.origin.y) + f32::from(container_bounds.origin.y)),
                                ),
                                container_bounds.size,
                            ))
                            .unwrap_or(container_bounds);
                        this.drag = Some(DragState {
                            tab_id,
                            path: path.clone(),
                            axis,
                            container: screen_container,
                        });
                        cx.notify();
                    }),
                );
            container = container.child(strip);
        }

        // While dragging, an overlay captures move/up across the whole area.
        if self.drag.is_some() {
            let overlay = div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                    this.update_drag(ev.position, cx);
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.drag = None;
                        this.schedule_session_save(cx);
                        cx.notify();
                    }),
                );
            container = container.child(deferred(overlay));
        }

        container.into_any_element()
    }

    /// Update the dragged split's ratio from the pointer position.
    fn update_drag(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.clone() else { return };
        let Some(idx) = self.tabs.iter().position(|t| t.id == drag.tab_id) else {
            return;
        };
        let c = drag.container;
        let ratio = match drag.axis {
            SplitAxis::Horizontal => {
                let w = f32::from(c.size.width).max(1.0);
                (f32::from(position.x) - f32::from(c.origin.x)) / w
            }
            SplitAxis::Vertical => {
                let h = f32::from(c.size.height).max(1.0);
                (f32::from(position.y) - f32::from(c.origin.y)) / h
            }
        };
        let ratio = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        self.tabs[idx].tree.set_ratio(&drag.path, ratio);
        cx.notify();
    }

    /// Settings overlay: WezTerm-style panel with section tabs + content body.
    fn render_settings_overlay(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let section = self.settings_section;

        // ── Section tab strip ────────────────────────────────────────────
        let mut section_tabs = div()
            .id("settings-section-tabs")
            .flex()
            .flex_row()
            .items_end()
            .gap_4()
            .w_full()
            .px_4()
            .pt_1();

        for &s in SettingsSection::ALL {
            let active = s == section;
            let tab_id: ElementId = format!("settings-section-{}", s.id()).into();
            let label: SharedString = s.label().into();
            section_tabs = section_tabs.child(
                div()
                    .id(tab_id)
                    .cursor_pointer()
                    .pb_1p5()
                    .text_size(px(13.0))
                    .when(active, |el| {
                        el.text_color(tokens.accent)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .border_b_2()
                            .border_color(tokens.accent)
                    })
                    .when(!active, |el| {
                        el.text_color(tokens.fg_muted)
                            .hover(|el| el.text_color(tokens.fg))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.select_settings_section(s, cx);
                    }))
                    .child(label),
            );
        }

        // ── Body for the active section ──────────────────────────────────
        let body = match section {
            SettingsSection::Theme => {
                self.render_settings_theme_section(tokens, window, cx)
                    .into_any_element()
            }
            SettingsSection::General => {
                self.render_settings_general_section(tokens, cx)
                    .into_any_element()
            }
        };

        // ── Footer: key hints (reference: WezTerm settings) ──────────────
        let footer = div()
            .id("settings-footer")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(tokens.border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(
                                div()
                                    .px_1()
                                    .rounded(px(3.0))
                                    .bg(tokens.hover)
                                    .text_color(tokens.fg)
                                    .child("click"),
                            )
                            .child("apply"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(
                                div()
                                    .px_1()
                                    .rounded(px(3.0))
                                    .bg(tokens.hover)
                                    .text_color(tokens.fg)
                                    .child("esc"),
                            )
                            .child("close"),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child("~/.config/sleipnir/settings.json"),
            );

        let panel = div()
            .id("settings-panel")
            .w(px(560.0))
            .max_w(px(720.0))
            .h(px(480.0))
            .max_h(px(560.0))
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .text_color(tokens.fg)
            .overflow_hidden()
            // Keep clicks inside the panel from reaching the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Title
            .child(
                div()
                    .px_4()
                    .pt_3()
                    .pb_1()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg)
                    .child("settings"),
            )
            // Section tabs
            .child(section_tabs)
            // Divider under tabs
            .child(
                div()
                    .w_full()
                    .h(px(1.0))
                    .bg(tokens.border),
            )
            // Scrollable body
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .px_4()
                    .py_3()
                    .child(body),
            )
            // Footer
            .child(footer);

        deferred(
            div()
                .id("settings-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // BlockMouse: otherwise TermElement under the overlay still
                // sees should_handle_scroll() and the terminal scrolls too.
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("settings-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.close_settings(window, cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }

    /// General section: session restore, ligatures, and pointers for advanced config.
    fn render_settings_general_section(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings = TerminalSettings::get_global(cx);
        let restore = settings.restore_session;
        let ligatures = settings.font_ligatures;

        div()
            .id("settings-general")
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child("Application"),
            )
            .child(self.settings_toggle_row(
                "restore-session",
                "Restore session on launch",
                "Reopen tabs, splits, and working directories from the last quit",
                restore,
                tokens,
                cx,
                |this, cx| {
                    let next = !TerminalSettings::get_global(cx).restore_session;
                    TerminalSettings::set_restore_session(next, cx);
                    // Drop session file writes immediately when turned off so a
                    // later quit does not overwrite with a stale layout.
                    if next {
                        this.schedule_session_save(cx);
                    }
                    cx.notify();
                },
            ))
            .child(
                div()
                    .mt_2()
                    .text_size(px(11.0))
                    .text_color(tokens.fg_muted)
                    .child("Terminal"),
            )
            .child(self.settings_toggle_row(
                "font-ligatures",
                "Font ligatures",
                "Enable OpenType ligatures when the font supports them (e.g. JetBrains Mono)",
                ligatures,
                tokens,
                cx,
                |_this, cx| {
                    let next = !TerminalSettings::get_global(cx).font_ligatures;
                    TerminalSettings::set_font_ligatures(next, cx);
                    cx.notify();
                },
            ))
            .child(self.settings_toggle_row(
                "copy-on-select",
                "Copy on select",
                "Copy selected text when you release the mouse; shows a brief “copied to clipboard” toast",
                TerminalSettings::get_global(cx).copy_on_select,
                tokens,
                cx,
                |_this, cx| {
                    let next = !TerminalSettings::get_global(cx).copy_on_select;
                    TerminalSettings::set_copy_on_select(next, cx);
                    cx.notify();
                },
            ))
            .child(
                div()
                    .mt_3()
                    .p_3()
                    .rounded(px(6.0))
                    .bg(tokens.hover)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(tokens.fg)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("Advanced"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(tokens.fg_muted)
                            .child(
                                format!(
                                    "Custom key bindings, font family/size, and shell options live in settings.json. Open the file and reload with {} (key bindings apply on next launch).",
                                    crate::display_shortcut("reload_settings")
                                ),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .text_color(tokens.accent)
                            .child(sleipnir_settings::config_path().display().to_string()),
                    ),
            )
    }

    /// One toggle row: title + description + on/off marker. `on_toggle` runs on click.
    fn settings_toggle_row(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        enabled: bool,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
        on_toggle: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let check: SharedString = if enabled { "●".into() } else { "○".into() };
        let state: SharedString = if enabled { "On".into() } else { "Off".into() };
        let state_color = if enabled { tokens.accent } else { tokens.fg_muted };

        div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .w_full()
            .px_2()
            .py_2()
            .rounded(px(6.0))
            .cursor_pointer()
            .hover(|el| el.bg(tokens.hover))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                on_toggle(this, cx);
            }))
            .child(
                div()
                    .w(px(18.0))
                    .text_size(px(14.0))
                    .text_color(state_color)
                    .child(check),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(tokens.fg)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(tokens.fg_muted)
                            .child(SharedString::from(description)),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(state_color)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(state),
            )
    }

    /// Theme section body: selectable list with ANSI swatches.
    fn render_settings_theme_section(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = TerminalSettings::get_global(cx).theme;
        let appearance = appearance_of(window.appearance());

        let mut list = div()
            .id("settings-theme-list")
            .flex()
            .flex_col()
            .gap_0p5()
            .w_full();

        for &theme in ThemeName::ALL {
            let selected = theme == current;
            let preview = palette_for_theme(theme, appearance);
            let label: SharedString = theme.display_name().into();
            let row_id: ElementId = format!("theme-row-{}", theme.as_str()).into();

            let mut swatches = div().flex().flex_row().items_center().gap_1();
            let swatch_colors = [
                preview.background,
                preview.ansi[1],
                preview.ansi[2],
                preview.ansi[3],
                preview.ansi[4],
                preview.ansi[5],
                preview.ansi[6],
            ];
            for (i, color) in swatch_colors.into_iter().enumerate() {
                swatches = swatches.child(
                    div()
                        .id(format!("swatch-{}-{}", theme.as_str(), i))
                        .w(px(11.0))
                        .h(px(11.0))
                        .rounded(px(2.0))
                        .bg(color)
                        .border_1()
                        .border_color(tokens.border),
                );
            }

            // Marker: ▸ + check for active (WezTerm-like), empty space otherwise.
            let marker: SharedString = if selected { "▸".into() } else { " ".into() };
            let check: SharedString = if selected { "✓".into() } else { "".into() };

            let row = div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .px_2()
                .py_1p5()
                .rounded(px(4.0))
                .cursor_pointer()
                .when(selected, |el| el.bg(tokens.hover))
                .hover(|el| el.bg(tokens.hover))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.select_theme(theme, cx);
                }))
                .child(
                    div()
                        .w(px(14.0))
                        .text_color(tokens.accent)
                        .child(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.0))
                        .text_color(tokens.fg)
                        .child(label),
                )
                .child(
                    div()
                        .w(px(16.0))
                        .text_color(tokens.accent)
                        .child(check),
                )
                .child(swatches);

            list = list.child(row);
        }

        list
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
                        .child(
                            div()
                                .text_sm()
                                .text_color(tokens.fg)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.fg_muted)
                                .child(shortcut),
                        ),
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

    fn render_find_bar(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            format!(
                "{}/{}",
                self.find_active_index + 1,
                self.find_match_count
            )
            .into()
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

fn snapshot_tree(node: &PaneNode, cx: &App) -> SessionNode {
    match node {
        PaneNode::Leaf { id, view } => SessionNode::Leaf {
            id: *id,
            cwd: view
                .read(cx)
                .working_directory(cx)
                .map(|p| p.to_string_lossy().into_owned()),
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
        let palette = TerminalPalette::get_global(cx);
        let window_active = window.is_window_active();
        let tokens = ChromeTokens::from_palette(&palette, window_active);
        let geo = ChromeGeometry::standard();
        let active = self.active;
        let hovered = self.hovered_tab;
        let fullscreen = window.is_fullscreen();
        let leading = if fullscreen {
            ChromeGeometry::fullscreen_leading_pad()
        } else {
            geo.leading_pad
        };

        let leading_drag = self
            .attach_empty_drag("chrome-drag-leading", cx)
            .w(leading);

        let trailing_drag = self
            .attach_empty_drag("chrome-drag-trailing", cx)
            .flex_1()
            .min_w(geo.trailing_pad);

        let tab_scroll = div()
            .id("tab-scroller")
            .flex()
            .flex_row()
            .items_center()
            .gap(geo.tab_gap)
            .h_full()
            .min_w_0()
            .flex_shrink(1.)
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll_handle)
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let title: SharedString = tab.title(cx);
                let is_active = ix == active;
                let is_hovered = hovered == Some(tab.id);
                let tab_id = tab.id;
                let rename_buffer = self
                    .rename
                    .as_ref()
                    .filter(|s| s.tab_id == tab_id)
                    .map(|s| s.buffer.clone());
                let is_renaming = rename_buffer.is_some();

                let is_bell = self.bell_flash_tabs.contains(&tab_id);
                let bg = if is_bell {
                    tokens.accent.opacity(0.35)
                } else if is_active {
                    tokens.active_tab_bg()
                } else if is_hovered {
                    tokens.hover
                } else {
                    // Transparent over the chrome band
                    gpui::hsla(0.0, 0.0, 0.0, 0.0)
                };
                let fg = if is_active || is_bell || is_hovered {
                    tokens.fg
                } else {
                    tokens.fg_muted
                };

                div()
                    .id(("tab", tab_id))
                    .h(geo.tab_height)
                    .min_w(geo.tab_min_width)
                    .max_w(geo.tab_max_width)
                    .px(geo.tab_px)
                    .rounded(geo.tab_radius)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .bg(bg)
                    .text_color(fg)
                    .text_sm()
                    .cursor_pointer()
                    .overflow_hidden()
                    .when(is_renaming, |el| {
                        el.border_1().border_color(tokens.accent)
                    })
                    .when(is_bell, |el| el.border_1().border_color(tokens.accent))
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered {
                            this.hovered_tab = Some(tab_id);
                        } else if this.hovered_tab == Some(tab_id) {
                            this.hovered_tab = None;
                        }
                        cx.notify();
                    }))
                    // Right-click a tab to rename it inline.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.begin_rename(tab_id, cx);
                        }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // While renaming, a click shouldn't switch tabs.
                        if this.rename.as_ref().is_some_and(|s| s.tab_id == tab_id) {
                            return;
                        }
                        this.activate(ix, window, cx);
                    }))
                    .child(if let Some(buffer) = rename_buffer {
                        let text: SharedString = format!("{buffer}|").into();
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_color(tokens.fg)
                            .child(text)
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(title)
                    })
            }));

        let chrome_band = div()
            .id("chrome-band")
            .h(geo.height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .bg(tokens.content_bg)
            .child(leading_drag)
            .child(tab_scroll)
            .child(trailing_drag);

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
            .capture_key_down(cx.listener(
                |this, event: &gpui::KeyDownEvent, window, cx| {
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
                    if this.update_open {
                        if event.keystroke.key.as_str() == "escape" {
                            this.close_update(cx);
                            cx.stop_propagation();
                        }
                        if !event.keystroke.modifiers.platform {
                            cx.stop_propagation();
                        }
                        return;
                    }
                    if this.palette_open {
                        if this.palette_key_down(event, window, cx) {
                            cx.stop_propagation();
                        }
                        return;
                    }
                    if this.find_open {
                        if this.find_key_down(event, window, cx) {
                            cx.stop_propagation();
                        }
                        return;
                    }
                    if this.settings_open {
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
                    if this.rename_key_down(event, window, cx) {
                        cx.stop_propagation();
                    }
                },
            ))
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
            .child(chrome_band)
            .when(self.broadcast, |el| {
                el.child(
                    div()
                        .id("broadcast-banner")
                        .absolute()
                        .top(px(36.0))
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
            .when(self.quick_select_open, |el| {
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
            .when(self.find_open, |el| {
                el.child(self.render_find_bar(&tokens, cx))
            })
            .child(self.render_content(&tokens, window, cx))
            .when(self.settings_open, |el| {
                el.child(self.render_settings_overlay(&tokens, window, cx))
            })
            .when(self.update_open, |el| {
                el.child(self.render_update_overlay(&tokens, cx))
            })
            .when(self.palette_open, |el| {
                el.child(self.render_command_palette(&tokens, cx))
            })
            .when(self.close_confirm.is_some(), |el| {
                el.child(self.render_close_confirm(&tokens, cx))
            })
    }
}

impl AppShell {
    fn render_close_confirm(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let message = self
            .close_confirm
            .as_ref()
            .map(|s| s.message.clone())
            .unwrap_or_else(|| "Close this pane?".into());

        let panel = div()
            .id("close-confirm-panel")
            .w(px(360.0))
            .rounded(px(10.0))
            .bg(tokens.content_bg)
            .border_1()
            .border_color(tokens.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .overflow_hidden()
            // Keep clicks inside the panel from reaching the backdrop.
            // Without this, mouse_down on Close/Cancel hits the full-size
            // backdrop first and cancel wins — the pane never closes.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg)
                    .child("Close pane?"),
            )
            .child(
                div()
                    .px_4()
                    .pb_4()
                    .text_size(px(13.0))
                    .text_color(tokens.fg_muted)
                    .child(message),
            )
            .child(
                div()
                    .px_4()
                    .pb_4()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("close-confirm-cancel")
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|el| el.bg(tokens.hover))
                            .text_size(px(13.0))
                            .text_color(tokens.fg)
                            .child("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_close_cancel(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("close-confirm-ok")
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(tokens.accent)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                            .child("Close")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_close_proceed(window, cx);
                            })),
                    ),
            );

        deferred(
            div()
                .id("close-confirm-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // BlockMouse: otherwise TermElement under the overlay still
                // sees should_handle_scroll() and the terminal scrolls too.
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("close-confirm-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.confirm_close_cancel(window, cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }
}
