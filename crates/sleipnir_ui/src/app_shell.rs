//! Single-window multi-tab shell for sleipnir (HIG-aligned chrome).

use gpui::{
    App, AppContext as _, Bounds, ClickEvent, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, actions, canvas, deferred, div, point,
    prelude::FluentBuilder as _, px,
};
use sleipnir_settings::{
    Appearance, TerminalPalette, TerminalSettings, ThemeName, palette_for_theme,
};

use crate::TermView;
use crate::chrome::{ChromeGeometry, ChromeTokens, active_after_close};
use crate::pane_tree::{
    Branch, CloseOutcome, Direction, MIN_RATIO, PaneId, PaneNode, PaneRect, SplitAxis, SplitPath,
    neighbor,
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
}

impl SettingsSection {
    const ALL: &'static [SettingsSection] = &[SettingsSection::Theme];

    fn id(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SettingsSection::Theme => "theme",
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
    /// Auto-update lifecycle state (drives the update notification bar).
    update_state: UpdateUiState,
    /// Verified update zip path, ready to install on restart.
    staged_zip: Option<std::path::PathBuf>,
}

impl Focusable for AppShell {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<()> for AppShell {}

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
            staged_zip: None,
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
        shell.add_tab(window, cx);
        // Silently check for updates on launch when enabled; only the update
        // notification bar surfaces a result (errors stay in the log).
        if TerminalSettings::get_global(cx).auto_update {
            shell.spawn_update_check(true, window, cx);
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
    /// (repaint on change, window-title sync on title change).
    fn spawn_term_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TermView> {
        let shell = cx.weak_entity();
        let view = cx.new(|cx| TermView::new_local_in_shell(Some(shell), window, cx));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        cx.subscribe_in(
            &view,
            window,
            |this, _view, event: &crate::TermViewEvent, window, cx| match event {
                crate::TermViewEvent::TitleChanged => {
                    this.sync_window_title(window, cx);
                    cx.notify();
                }
            },
        )
        .detach();
        view
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
        });
        self.active = self.tabs.len() - 1;
        self.focus_active(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        cx.notify();
    }

    pub(crate) fn add_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(window, cx);
    }

    pub(crate) fn next_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.next_tab(window, cx);
    }

    pub(crate) fn prev_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prev_tab(window, cx);
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
                cx.notify();
            }
        }
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(window, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_active_pane(window, cx);
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
        TerminalSettings::reload(cx);
        cx.notify();
    }

    fn on_cycle_theme(&mut self, _: &CycleTheme, _window: &mut Window, cx: &mut Context<Self>) {
        let next = TerminalSettings::get_global(cx).theme.next();
        TerminalSettings::set_theme(next, cx);
        cx.notify();
    }

    // ── auto-update ─────────────────────────────────────────────────────────

    fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Manual check: surface "up to date" and errors in the bar.
        self.spawn_update_check(false, window, cx);
    }

    /// Query GitHub for a newer release. When `silent`, a no-update result or
    /// error clears the bar instead of showing status.
    fn spawn_update_check(&mut self, silent: bool, window: &mut Window, cx: &mut Context<Self>) {
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
            let result = updater::fetch_latest(&current).await;
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
                        this.update_state = if silent {
                            UpdateUiState::Idle
                        } else {
                            UpdateUiState::UpToDate
                        };
                    }
                    Err(err) => {
                        log::warn!("update check failed: {err:#}");
                        this.update_state = if silent {
                            UpdateUiState::Idle
                        } else {
                            UpdateUiState::Failed(format!("Update check failed: {err}"))
                        };
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
            let result = updater::download_and_verify(&info, &dest).await;
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

    /// Dismiss the current update notification bar.
    fn dismiss_update(&mut self, cx: &mut Context<Self>) {
        self.update_state = UpdateUiState::Idle;
        cx.notify();
    }

    /// A small pill button for the update bar.
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
            (tokens.surface, tokens.fg)
        };
        div()
            .id(id)
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .cursor_pointer()
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }

    /// Render the update notification bar for the current [`UpdateUiState`].
    /// Returns `None` when idle so nothing is inserted into the layout.
    fn render_update_bar(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let (message, buttons): (SharedString, Vec<gpui::AnyElement>) = match &self.update_state {
            UpdateUiState::Idle => return None,
            UpdateUiState::Checking => ("Checking for updates…".into(), Vec::new()),
            UpdateUiState::UpToDate => (
                "You’re on the latest version.".into(),
                vec![
                    self.update_button("upd-dismiss", "Dismiss", tokens, false, cx, |this, _, cx| {
                        this.dismiss_update(cx);
                    })
                    .into_any_element(),
                ],
            ),
            UpdateUiState::Available(u) => (
                format!("Sleipnir {} is available.", u.version).into(),
                vec![
                    self.update_button(
                        "upd-install",
                        "Download & Install",
                        tokens,
                        true,
                        cx,
                        |this, window, cx| this.start_download(window, cx),
                    )
                    .into_any_element(),
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
                        this.dismiss_update(cx);
                    })
                    .into_any_element(),
                ],
            ),
            UpdateUiState::Downloading(u) => (
                format!("Downloading Sleipnir {}…", u.version).into(),
                Vec::new(),
            ),
            UpdateUiState::ReadyToRestart(u) => (
                format!("Sleipnir {} is ready.", u.version).into(),
                vec![
                    self.update_button(
                        "upd-restart",
                        "Restart & Update",
                        tokens,
                        true,
                        cx,
                        |this, _, cx| this.install_and_restart(cx),
                    )
                    .into_any_element(),
                    self.update_button("upd-later2", "Later", tokens, false, cx, |this, _, cx| {
                        this.dismiss_update(cx);
                    })
                    .into_any_element(),
                ],
            ),
            UpdateUiState::Failed(msg) => (
                msg.clone().into(),
                vec![
                    self.update_button("upd-retry", "Dismiss", tokens, false, cx, |this, _, cx| {
                        this.dismiss_update(cx);
                    })
                    .into_any_element(),
                ],
            ),
        };

        Some(
            div()
                .id("update-bar")
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .bg(tokens.surface)
                .border_b_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(tokens.fg)
                        .child(message),
                )
                .children(buttons),
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

    pub(crate) fn open_settings_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

        // Gather leaves (id -> view) in tree order.
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        let leaves: Vec<(PaneId, Entity<TermView>)> =
            leaves.into_iter().map(|(id, v)| (id, v.clone())).collect();

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
            let mut pane = div()
                .absolute()
                .left(b.origin.x)
                .top(b.origin.y)
                .w(b.size.width)
                .h(b.size.height)
                .overflow_hidden()
                .child(view.clone().into_any_element());
            if !is_active {
                // Subtle inactive treatment: hairline border; active pane gets accent.
                pane = pane.border_1().border_color(tokens.border);
            } else {
                pane = pane.border_1().border_color(tokens.accent);
            }
            let pane_id = *id;
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
            SettingsSection::Theme => self.render_settings_theme_section(tokens, window, cx),
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
            .min_w(px(8.0));

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

                let bg = if is_active {
                    tokens.active_tab_bg()
                } else if is_hovered {
                    tokens.hover
                } else {
                    // Transparent over the chrome band
                    gpui::hsla(0.0, 0.0, 0.0, 0.0)
                };
                let fg = if is_active {
                    tokens.fg
                } else if is_hovered {
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
            // Intercept keys during an inline tab rename / settings before the
            // focused terminal sees them (capture phase runs top-down).
            .capture_key_down(cx.listener(
                |this, event: &gpui::KeyDownEvent, window, cx| {
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
            .child(chrome_band)
            .children(self.render_update_bar(&tokens, cx))
            .child(self.render_content(&tokens, window, cx))
            .when(self.settings_open, |el| {
                el.child(self.render_settings_overlay(&tokens, window, cx))
            })
    }
}
