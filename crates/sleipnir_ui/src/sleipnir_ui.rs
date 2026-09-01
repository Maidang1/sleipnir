//! Terminal UI for sleipnir (M2 PTY input, M3 tabs + URL open, HIG chrome).

mod app_shell;
mod assets;
mod attention_chrome;
mod blink;
mod chrome;
mod command_palette;
mod control_surface;
mod diff;
mod finder_service;
mod git_service;
mod keymap;
mod pane_tree;
mod plugin_block;
mod plugin_chrome;
mod plugin_event_watch;
mod plugin_host_calls;
mod plugin_monitor_panel;
mod plugin_panel;
mod plugin_runtime;
mod panel_scene_paint;
mod run_ledger_global;
mod run_ledger_panel;
mod session;
mod tab_convert;
mod term_element;
mod ui_mode;
mod update_model;
mod workspace_commit;

pub use assets::AgentAssets;
pub use blink::{BLINK_HALF_PERIOD, cursor_blink_alpha};

pub use app_shell::{
    ActivateTab, AppShell, CheckForUpdates, ClearRunLedger, CloseTab, CycleTheme, DecreaseFontSize,
    ExportScrollback, Find, FindNext, FindPrev, FocusPaneDown, FocusPaneLeft, FocusPaneRight,
    FocusPaneUp, IncreaseFontSize, JumpNextPrompt, JumpPrevPrompt, MarkTabSeen, NewTab, NewWindow,
    NextTab, OpenQuickTerminal, OpenSettings, PipeSelection, PrevTab, ReloadSettings,
    ResetFontSize, SendGitDiff, SendSelection, SplitDown, SplitRight, ToggleBroadcast,
    ToggleCommandPalette, ToggleDiff, ToggleHistorySearch, TogglePaneFacts, TogglePaneZoom,
    TogglePluginMonitor, ToggleQuickSelect, ToggleRunLedger, open_sleipnir_window,
    try_open_sleipnir_window,
};
pub use chrome::{ChromeGeometry, ChromeTokens, active_after_close, contrast_ratio};
pub use command_palette::{CommandId, CommandItem, commands as palette_commands};
pub use finder_service::install_finder_services;
pub use keymap::{
    BindingContext, BuiltinAction, BuiltinBinding, builtin_bindings, display_shortcut,
    font_zoom_key_bindings, last_window_close_quits, tmux_preset_bindings,
};
pub use pane_tree::{
    Branch, CloseOutcome, Direction, LeafContent, MIN_RATIO, PaneId, PaneNode, PaneRect, SplitAxis,
    SplitPath, neighbor,
};
pub use run_ledger_global::RunLedgerGlobal;
pub use session::{SessionFile, SessionNode, SessionTab, load_session, save_session, session_path};
pub use term_element::TermElement;
pub use update_model::{AvailableUpdate, UpdateModel, UpdateUiState};

use collections::HashMap;
use gpui::Pixels;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, ClipboardEntry, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div, rgb,
};
use sleipnir_settings::{
    NotifyOnCommandFinish, TerminalBell, TerminalBlink, TerminalPalette, TerminalSettings,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use terminal::{
    Clear, Copy, Event, MaybeNavigationTarget, Modes, Paste, PasteText, ScrollLineDown,
    ScrollLineUp, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, SelectAll,
    SendKeystroke, SendText, ShowCharacterPalette, Terminal, TerminalBuilder, ToggleViMode,
};
use util::paths::PathStyle;
use util::shell::Shell;

/// Bubbled from a tab so the shell can refresh titles.
#[derive(Clone, Debug)]
pub enum TermViewEvent {
    TitleChanged,
    /// Request the shell open a new tab (avoids TermView holding a WeakEntity<AppShell>).
    RequestNewTab,
    /// Request switching to the next tab.
    RequestNextTab,
    /// Request switching to the previous tab.
    RequestPrevTab,
    /// Request reload of settings.
    RequestReloadSettings,
    /// Request cycling the theme.
    RequestCycleTheme,
    /// Request opening the settings panel.
    RequestOpenSettings,
    /// Terminal BEL — shell may flash tab chrome (visual bell).
    Bell,
    /// A command started in this pane (Run Ledger).
    RunStarted {
        command: String,
        cwd: Option<PathBuf>,
        inferred: bool,
        line: Option<i32>,
        column: Option<usize>,
    },
    /// The current command in this pane finished.
    RunFinished {
        exit_code: Option<i32>,
    },
    /// Overlay triangle on a command start/end line was clicked.
    GutterClicked {
        line: i32,
    },
    /// The user sent input to this pane (keystroke, paste, IME).
    UserTyped,
}

impl EventEmitter<TermViewEvent> for TermView {}

enum TerminalSlot {
    Loading,
    Ready(Entity<Terminal>),
    Failed(SharedString),
}

/// Host view: owns a local-PTY `Terminal` (or loading/error state).
pub struct TermView {
    terminal: TerminalSlot,
    focus_handle: FocusHandle,
    title: SharedString,
    /// Window-scoped font size override from AppShell zoom (not persisted).
    font_size_override: Option<Pixels>,
    /// App-reported cursor blink (DECSCUSR); used with settings for M11 fade.
    terminal_wants_blink: bool,
    /// Last keystroke / input time — cursor stays solid briefly after typing.
    last_input_at: Instant,
    /// Bottom toast after copy / copy-on-select.
    copy_toast: Option<CopyToast>,
    /// Last time `render` ran, i.e. the last time this pane was actually in the
    /// window's element tree. `AppShell::render_content` only emits the active
    /// tab's panes (and, under pane zoom, only the zoomed one), so a stale value
    /// means "off screen" — see [`Self::offscreen_repaint_is_throttled`].
    last_render_at: Option<Instant>,
    /// Last repaint we requested while off screen (throttle bookkeeping).
    last_offscreen_notify_at: Option<Instant>,
    /// Host-owned Block surfaces for this pane (ADR-0018). Process-local.
    blocks: crate::plugin_block::BlockRegistry,
    last_block_history: usize,
    _spawn: Task<()>,
}

/// A pane that rendered within this window is treated as on screen. Must stay
/// comfortably above one frame interval (8.3 ms at 120 Hz) so a visible pane is
/// never mistaken for a hidden one.
const ONSCREEN_WINDOW: Duration = Duration::from_millis(250);

/// While off screen, request at most one repaint per this interval. Throttling
/// rather than suppressing keeps the path self-healing: if this heuristic is ever
/// wrong about visibility, the pane still refreshes within this bound instead of
/// freezing.
const OFFSCREEN_REPAINT_INTERVAL: Duration = Duration::from_millis(250);

struct CopyToast {
    /// Hide task; dropping it cancels a previous toast.
    _hide: Task<()>,
}

impl Focusable for TermView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TermView {
    /// Spawn a local interactive shell PTY and attach UI when ready.
    pub fn new_local(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_local_with_cwd(None, window, cx)
    }

    pub(crate) fn new_local_with_cwd(
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_local_with(cwd, None, window, cx)
    }

    /// Spawn a pane. `command` is program + argv, never a shell line
    /// (ADR-0013 / HostCall::OpenPane).
    pub(crate) fn new_local_with(
        cwd: Option<PathBuf>,
        command: Option<(String, Vec<String>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = TerminalSettings::get_global(cx).clone();
        let window_id = window.window_handle().window_id().as_u64();
        let cwd = cwd.or_else(dirs::home_dir);
        let mut env: HashMap<String, String> = std::env::vars().collect();
        for (k, v) in &settings.env {
            env.insert(k.clone(), v.clone());
        }
        let shell = match command {
            Some((program, args)) => Shell::WithArguments {
                program,
                args,
                title_override: None,
            },
            None => Shell::System,
        };
        let shell = terminal::apply_inject_to_shell(shell, &mut env, settings.inject_osc133);

        let builder_task = TerminalBuilder::new(
            cwd,
            shell,
            env,
            settings.cursor_shape,
            settings.alternate_scroll,
            settings.max_scroll_history_lines,
            settings.path_hyperlink_regexes.clone(),
            settings.path_hyperlink_timeout_ms,
            window_id,
            cx,
            PathStyle::local(),
        );

        let spawn = cx.spawn_in(window, async move |this, cx| {
            let result = builder_task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(builder) => {
                    let terminal = cx.new(|cx| builder.subscribe(cx));
                    this.attach_terminal(terminal, window, cx);
                }
                Err(err) => {
                    this.terminal =
                        TerminalSlot::Failed(format!("failed to open PTY: {err:#}").into());
                    cx.notify();
                }
            })
            .ok();
        });

        Self {
            terminal: TerminalSlot::Loading,
            focus_handle: cx.focus_handle(),
            title: "Sleipnir".into(),
            font_size_override: None,
            terminal_wants_blink: true,
            last_input_at: Instant::now(),
            copy_toast: None,
            last_render_at: None,
            last_offscreen_notify_at: None,
            blocks: crate::plugin_block::BlockRegistry::new(),
            last_block_history: 0,
            _spawn: spawn,
        }
    }

    /// Whether this pane looks like it is on screen, judged by whether `render`
    /// ran recently. `AppShell::render_content` only builds elements for the
    /// active tab's panes (and only the zoomed pane when a tab is zoomed), so a
    /// pane that has not rendered inside [`ONSCREEN_WINDOW`] is hidden.
    fn looks_onscreen(&self) -> bool {
        self.last_render_at
            .is_some_and(|at| at.elapsed() < ONSCREEN_WINDOW)
    }

    /// Whether a grid change may request a window repaint right now.
    ///
    /// On screen: always. Off screen: at most once per
    /// [`OFFSCREEN_REPAINT_INTERVAL`], so a background pane costs a trickle of
    /// repaints instead of one per PTY batch. Throttling instead of suppressing
    /// is deliberate — a pane that is wrongly judged hidden still refreshes
    /// within that bound rather than freezing.
    fn request_repaint_allowed(&mut self) -> bool {
        if self.looks_onscreen() {
            return true;
        }
        let now = Instant::now();
        let due = self
            .last_offscreen_notify_at
            .is_none_or(|at| now.duration_since(at) >= OFFSCREEN_REPAINT_INTERVAL);
        if due {
            self.last_offscreen_notify_at = Some(now);
        }
        due
    }

    /// Working directory of the attached PTY, if known.
    pub fn working_directory(&self, cx: &App) -> Option<PathBuf> {
        self.terminal_entity()
            .and_then(|t| t.read(cx).working_directory())
    }

    /// Normalized foreground process / script name, if the PTY has one.
    pub fn foreground_process_command_name(&self, cx: &App) -> Option<String> {
        self.terminal_entity()
            .and_then(|t| t.read(cx).foreground_process_command_name())
    }

    /// Spawned shell pid for this pane, when the PTY is local.
    pub fn shell_pid(&self, cx: &App) -> Option<u32> {
        self.terminal_entity().and_then(|t| t.read(cx).shell_pid())
    }

    /// Current grid selection, if any.
    pub fn selection_text(&self, cx: &App) -> Option<String> {
        self.terminal_entity()
            .and_then(|t| t.read(cx).last_content().selection_text.clone())
    }

    /// Type into this pane's PTY.
    pub fn input_bytes(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let typed = !bytes.is_empty();
        if let Some(term) = self.terminal_entity().cloned() {
            term.update(cx, |term, _| term.input(bytes));
        }
        if typed {
            self.note_user_typed(cx);
        }
    }

    fn note_user_typed(&self, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::UserTyped);
    }

    /// Visible screen (no scrollback), for the control surface `capture` verb.
    pub fn visible_screen_text(&self, cx: &App) -> String {
        self.terminal_entity()
            .map(|t| t.read(cx).visible_screen_text())
            .unwrap_or_default()
    }

    /// Scroll this pane to an OSC 133 absolute line (Run Ledger Anchor).
    pub fn scroll_to_anchor(&self, line: i32, column: usize, cx: &mut App) {
        if let Some(term) = self.terminal_entity().cloned() {
            term.update(cx, |term, cx| {
                term.scroll_to_absolute(line, column);
                cx.notify();
            });
        }
    }

    pub(crate) fn blocks(&self) -> &crate::plugin_block::BlockRegistry {
        &self.blocks
    }

    pub(crate) fn apply_block_render(
        &mut self,
        plugin_id: &str,
        run_id: plugin_protocol::v2::RunId,
        tree: plugin_protocol::v2::Widget,
        granted: bool,
        ledger_anchor: Option<run_ledger::Anchor>,
        existing_id: Option<plugin_protocol::v2::BlockId>,
        cx: &mut Context<Self>,
    ) -> crate::plugin_block::ApplyBlock {
        use crate::plugin_block::ApplyBlock;
        let out =
            self.blocks
                .apply_render(plugin_id, run_id, tree, granted, ledger_anchor, existing_id);
        if matches!(out, ApplyBlock::Inserted | ApplyBlock::Replaced) {
            self.sync_blocks_to_terminal(cx);
        }
        out
    }

    pub(crate) fn mark_blocks_stale(&mut self, plugin_id: &str) {
        self.blocks.mark_plugin_stale(plugin_id);
    }

    pub(crate) fn mark_missing_blocks_stale(&mut self, live: &std::collections::BTreeSet<String>) {
        self.blocks.mark_missing_stale(live);
    }

    pub(crate) fn set_blocks_frozen(&mut self, frozen: bool, cx: &mut Context<Self>) {
        if !frozen {
            self.blocks.invalidate_layouts();
        }
        if let Some(term) = self.terminal_entity().cloned() {
            term.update(cx, |term, _| term.set_blocks_frozen(frozen));
        }
        if !frozen {
            self.sync_blocks_to_terminal(cx);
        }
    }

    pub(crate) fn sync_block_lifecycle(&mut self, cx: &mut Context<Self>) {
        let Some(term) = self.terminal_entity().cloned() else {
            return;
        };
        let hist = term.read(cx).history_size();
        if hist < self.last_block_history {
            let removed = (self.last_block_history - hist) as i32;
            self.blocks.rebase_after_history_shrink(removed);
        }
        self.last_block_history = hist;
        self.sync_blocks_to_terminal(cx);
    }

    fn sync_blocks_to_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(term) = self.terminal_entity().cloned() else {
            return;
        };
        let cols = term
            .read(cx)
            .last_content()
            .terminal_bounds
            .num_columns()
            .min(u16::MAX as usize) as u16;
        let frozen = term.read(cx).row_geometry().is_frozen();
        self.blocks.relayout(cols.max(1), frozen);
        let blocks = self.blocks.geometry_blocks();
        let live: std::collections::BTreeSet<_> = blocks.iter().map(|b| b.id).collect();
        self.blocks.retain_live(&live);
        term.update(cx, |term, _| {
            let keep: std::collections::BTreeSet<_> =
                term.row_geometry().blocks().map(|b| b.id).collect();
            for id in keep {
                if !blocks.iter().any(|b| b.id == id) {
                    term.remove_block(id);
                }
            }
            for b in blocks {
                term.upsert_block(b);
            }
        });
    }

    fn attach_terminal(
        &mut self,
        terminal: Entity<Terminal>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            &terminal,
            window,
            |this, terminal, event, window, cx| match event {
                Event::Wakeup | Event::SelectionsChanged => {
                    // Poll command-finish notify (M14 → matrix).
                    let (min_secs, mode) = {
                        let settings = TerminalSettings::get_global(cx);
                        (
                            settings.notify_on_command_finish_secs,
                            settings.notify_on_command_finish_mode,
                        )
                    };
                    if min_secs > 0 && mode != NotifyOnCommandFinish::Never {
                        if let Some(dur) =
                            terminal.update(cx, |t, cx| t.poll_command_finish(min_secs, cx))
                        {
                            let should_notify = match mode {
                                NotifyOnCommandFinish::Never => false,
                                NotifyOnCommandFinish::Unfocused => !window.is_window_active(),
                                NotifyOnCommandFinish::Always => true,
                            };
                            if should_notify {
                                notify_command_finished(dur);
                            }
                        }
                    }
                    // A pane that is not on screen must not drive the window's
                    // repaint loop: a coding agent streaming in a background tab
                    // otherwise requests ~250 repaints/second (one per 4 ms PTY
                    // batch) while the user types in a different pane. The grid
                    // is still updated — only the repaint request is throttled.
                    if this.request_repaint_allowed() {
                        cx.notify();
                    }
                }
                Event::BlinkChanged(blinking) => {
                    this.terminal_wants_blink = *blinking;
                    cx.notify();
                }
                Event::TitleChanged => {
                    this.title = terminal.read(cx).title(true).into();
                    cx.emit(TermViewEvent::TitleChanged);
                    cx.notify();
                }
                Event::Bell => {
                    handle_bell(window, cx);
                    cx.emit(TermViewEvent::Bell);
                    cx.notify();
                }
                Event::CopiedToClipboard => {
                    this.show_copy_toast(cx);
                }
                Event::CloseTerminal => {
                    this.title = "exited".into();
                    cx.emit(TermViewEvent::TitleChanged);
                    cx.notify();
                }
                Event::NewNavigationTarget(_) => {}
                Event::Open(target) => {
                    open_navigation_target(target, cx);
                }
                // Ignore OSC 9 / OSC 777 desktop notification requests from
                // shell hooks and terminal applications.
                Event::Notify(_) => {}
                Event::RunStarted {
                    command,
                    cwd,
                    inferred,
                    line,
                    column,
                } => {
                    cx.emit(TermViewEvent::RunStarted {
                        command: command.clone(),
                        cwd: cwd.clone(),
                        inferred: *inferred,
                        line: *line,
                        column: *column,
                    });
                }
                Event::RunFinished { exit_code } => {
                    cx.emit(TermViewEvent::RunFinished {
                        exit_code: *exit_code,
                    });
                }
                Event::GutterClicked { line } => {
                    cx.emit(TermViewEvent::GutterClicked { line: *line });
                }
            },
        )
        .detach();

        self.title = terminal.read(cx).title(true).into();
        self.terminal = TerminalSlot::Ready(terminal);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub fn title(&self) -> &str {
        let title = self.title.as_ref();
        if title.is_empty() { "shell" } else { title }
    }

    pub fn terminal_entity(&self) -> Option<&Entity<Terminal>> {
        match &self.terminal {
            TerminalSlot::Ready(t) => Some(t),
            _ => None,
        }
    }

    /// Whether the attached PTY has a non-shell foreground job.
    pub fn looks_busy(&self, cx: &App) -> bool {
        self.terminal_entity()
            .map(|t| t.read(cx).looks_busy())
            .unwrap_or(false)
    }

    /// Apply a window-scoped font size override (zoom). `None` restores settings size.
    pub fn set_font_size_override(&mut self, size: Option<Pixels>, cx: &mut Context<Self>) {
        self.font_size_override = size;
        cx.notify();
    }

    pub fn font_size_override(&self) -> Option<Pixels> {
        self.font_size_override
    }

    /// Effective font size: zoom override or settings (clamped).
    pub fn effective_font_size(&self, cx: &App) -> Pixels {
        effective_font_size(self.font_size_override, cx)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.terminal_entity().cloned() else {
            return;
        };

        // Clipboard shortcuts are handled by on_action(Copy/Paste/PasteText);
        // skip only those combinations so plain Ctrl+C/V still reach the PTY.
        if is_clipboard_shortcut(&event.keystroke) {
            return;
        }

        // Reset cursor blink solid window on any keystroke (M11).
        self.last_input_at = Instant::now();

        let option_as_meta = TerminalSettings::get_global(cx).option_as_meta;
        let handled = terminal.update(cx, |term, _| {
            term.try_keystroke(&event.keystroke, option_as_meta)
        });
        if handled {
            self.note_user_typed(cx);
            cx.notify();
        }
    }

    /// Effective cursor blink opacity for the paint path (M11).
    pub fn blink_alpha(&self, cx: &App) -> f32 {
        let settings = TerminalSettings::get_global(cx).blinking;
        cursor_blink_alpha(
            self.last_input_at.elapsed(),
            self.terminal_wants_blink,
            settings,
        )
    }

    /// Whether the cursor animation should keep requesting frames.
    pub fn blink_needs_animation(&self, cx: &App) -> bool {
        match TerminalSettings::get_global(cx).blinking {
            TerminalBlink::Off => false,
            TerminalBlink::On => true,
            TerminalBlink::TerminalControlled => self.terminal_wants_blink,
        }
    }

    /// Show a brief bottom toast after text lands on the clipboard.
    fn show_copy_toast(&mut self, cx: &mut Context<Self>) {
        let hide = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(1600))
                .await;
            this.update(cx, |this, cx| {
                this.copy_toast = None;
                cx.notify();
            })
            .ok();
        });
        self.copy_toast = Some(CopyToast { _hide: hide });
        cx.notify();
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.copy(Some(true)));
            cx.notify();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        self.paste_from_clipboard(cx);
    }

    fn paste_text(&mut self, _: &PasteText, _window: &mut Window, cx: &mut Context<Self>) {
        self.paste_text_from_clipboard(cx);
    }

    /// Full paste: image → temp path, file paths → quoted paths, else text.
    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return;
        };
        let Some(terminal) = self.terminal_entity().cloned() else {
            return;
        };

        match clipboard.entries().first() {
            Some(ClipboardEntry::Image(image)) if !image.bytes().is_empty() => {
                match write_clipboard_image_to_temp(image) {
                    Ok(path) => {
                        let text = format!("{} ", shell_quote_path(&path));
                        terminal.update(cx, |term, _| term.paste(&text));
                        self.note_user_typed(cx);
                        cx.notify();
                    }
                    Err(err) => {
                        log::error!("failed to write clipboard image to temp file: {err}");
                    }
                }
            }
            Some(ClipboardEntry::ExternalPaths(paths)) => {
                let text = format_external_paths(paths.paths());
                if !text.trim().is_empty() {
                    terminal.update(cx, |term, _| term.paste(&text));
                    self.note_user_typed(cx);
                    cx.notify();
                }
            }
            _ => {
                if let Some(text) = clipboard.text() {
                    terminal.update(cx, |term, _| term.paste(&text));
                    self.note_user_typed(cx);
                    cx.notify();
                }
            }
        }
    }

    /// Text-only paste (never images or file-path conversion).
    fn paste_text_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.paste(&text));
            self.note_user_typed(cx);
            cx.notify();
        }
    }

    fn clear(&mut self, _: &Clear, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.clear());
            cx.notify();
        }
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.select_all());
            cx.notify();
        }
    }

    fn scroll_line_up(&mut self, _: &ScrollLineUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_alt_screen(cx) {
            return;
        }
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.scroll_line_up());
            cx.notify();
        }
    }

    fn scroll_line_down(
        &mut self,
        _: &ScrollLineDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_alt_screen(cx) {
            return;
        }
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.scroll_line_down());
            cx.notify();
        }
    }

    fn scroll_page_up(&mut self, _: &ScrollPageUp, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_alt_screen(cx) {
            return;
        }
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.scroll_page_up());
            cx.notify();
        }
    }

    fn scroll_page_down(
        &mut self,
        _: &ScrollPageDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_alt_screen(cx) {
            return;
        }
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.scroll_page_down());
            cx.notify();
        }
    }

    fn scroll_to_top(&mut self, _: &ScrollToTop, _window: &mut Window, cx: &mut Context<Self>) {
        if self.is_alt_screen(cx) {
            return;
        }
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.scroll_to_top());
            cx.notify();
        }
    }

    fn scroll_to_bottom(
        &mut self,
        _: &ScrollToBottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_alt_screen(cx) {
            return;
        }
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.scroll_to_bottom());
            cx.notify();
        }
    }

    fn toggle_vi_mode(&mut self, _: &ToggleViMode, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.toggle_vi_mode());
            cx.notify();
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn send_text(&mut self, text: &SendText, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.input(text.0.as_bytes().to_vec()));
            self.note_user_typed(cx);
            cx.notify();
        }
    }

    fn send_keystroke(
        &mut self,
        key: &SendKeystroke,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(keystroke) = Keystroke::parse(&key.0) else {
            log::warn!("invalid SendKeystroke payload: {}", key.0);
            return;
        };
        let option_as_meta = TerminalSettings::get_global(cx).option_as_meta;
        if let Some(terminal) = self.terminal_entity().cloned() {
            let handled =
                terminal.update(cx, |term, _| term.try_keystroke(&keystroke, option_as_meta));
            if handled {
                self.note_user_typed(cx);
                cx.notify();
            }
        }
    }

    fn is_alt_screen(&self, cx: &App) -> bool {
        self.terminal_entity()
            .map(|t| t.read(cx).last_content().mode.contains(Modes::ALT_SCREEN))
            .unwrap_or(false)
    }

    fn new_tab(&mut self, _: &NewTab, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestNewTab);
    }

    fn next_tab(&mut self, _: &NextTab, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestNextTab);
    }

    fn prev_tab(&mut self, _: &PrevTab, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestPrevTab);
    }

    fn reload_settings(
        &mut self,
        _: &ReloadSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(TermViewEvent::RequestReloadSettings);
    }

    fn cycle_theme(&mut self, _: &CycleTheme, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestCycleTheme);
    }

    fn open_settings(&mut self, _: &OpenSettings, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestOpenSettings);
    }
}

/// Hover tooltip previewing the hyperlink/path under the pointer (M16).
struct LinkPreview {
    text: SharedString,
}

impl Render for LinkPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let bg = palette
            .background
            .blend(gpui::Hsla::black().opacity(0.4))
            .alpha(1.0);
        div()
            .px_2()
            .py_1()
            .rounded(gpui::px(4.0))
            .bg(bg)
            .text_color(palette.foreground)
            .text_size(gpui::px(12.0))
            .font_family(sleipnir_settings::default_font_family())
            .child(self.text.clone())
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Being rendered *is* the visibility signal: only panes that
        // `AppShell::render_content` emits get here (see `looks_onscreen`).
        self.last_render_at = Some(Instant::now());
        let palette = TerminalPalette::get_global(cx);
        let focused = self.focus_handle.is_focused(window);
        let show_copy_toast = self.copy_toast.is_some();
        // Toast chrome: dark surface, lime border/dot (matches common terminal UX).
        let toast_bg = palette
            .background
            .blend(gpui::Hsla::black().opacity(0.35))
            .alpha(1.0);
        let toast_border = palette.ansi[2].opacity(0.9);
        let toast_dot = palette.ansi[2];
        let toast_fg = palette
            .foreground
            .blend(palette.ansi[3].opacity(0.15))
            .alpha(1.0);

        div()
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(palette.background)
            .text_color(palette.foreground)
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::paste_text))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::scroll_line_up))
            .on_action(cx.listener(Self::scroll_line_down))
            .on_action(cx.listener(Self::scroll_page_up))
            .on_action(cx.listener(Self::scroll_page_down))
            .on_action(cx.listener(Self::scroll_to_top))
            .on_action(cx.listener(Self::scroll_to_bottom))
            .on_action(cx.listener(Self::toggle_vi_mode))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::send_text))
            .on_action(cx.listener(Self::send_keystroke))
            .on_action(cx.listener(Self::new_tab))
            // CloseTab (⌘W) is intentionally NOT handled here. TermView would
            // nest-update AppShell and drop *this* entity while it is still
            // leased for the action — that panics and tears down the window.
            // AppShell owns the close path (close active pane, or tab if last)
            // and receives the action via bubble phase, same as SplitRight.
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::reload_settings))
            .on_action(cx.listener(Self::cycle_theme))
            .on_action(cx.listener(Self::open_settings))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(match &self.terminal {
                TerminalSlot::Loading => div()
                    .id("term-loading")
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(0x6c7086))
                    .child("Starting shell…")
                    .into_any_element(),
                TerminalSlot::Failed(err) => div()
                    .id("term-failed")
                    .size_full()
                    .p_4()
                    .text_color(rgb(0xf38ba8))
                    .child(err.clone())
                    .into_any_element(),
                TerminalSlot::Ready(terminal) => {
                    let hovered = terminal
                        .read(cx)
                        .last_content()
                        .last_hovered_word
                        .as_ref()
                        .map(|w| w.word.clone());

                    let a11y_text: SharedString = terminal.read(cx).visible_screen_text().into();
                    let body = div()
                        .id("term-view-body")
                        .size_full()
                        .p_2()
                        // Read-only accessibility: VoiceOver reads the visible
                        // screen as the terminal's value (Ghostty-parity, opt-in
                        // there; we keep it always-on and read-only).
                        .role(gpui::Role::MultilineTextInput)
                        .aria_label("Terminal")
                        .aria_value(a11y_text);
                    let body = if let Some(word) = hovered {
                        body.tooltip(move |_window, cx| {
                            let text: SharedString = word.clone().into();
                            cx.new(move |_| LinkPreview { text }).into()
                        })
                    } else {
                        body
                    };

                    body.child(TermElement::new(
                        terminal.clone(),
                        cx.entity(),
                        self.focus_handle.clone(),
                        focused,
                        self.font_size_override,
                        self.last_input_at,
                        self.terminal_wants_blink,
                    ))
                    .into_any_element()
                }
            })
            .when(show_copy_toast, |el| {
                el.child(
                    div()
                        .id("copy-toast")
                        .absolute()
                        .bottom(gpui::px(14.0))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        // Don't steal mouse from the terminal under the toast.
                        .occlude()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py_1p5()
                                .rounded(gpui::px(6.0))
                                .bg(toast_bg)
                                .border_1()
                                .border_color(toast_border)
                                .shadow_md()
                                .child(div().size(gpui::px(7.0)).rounded_full().bg(toast_dot))
                                .child(
                                    div()
                                        .text_size(gpui::px(12.0))
                                        .text_color(toast_fg)
                                        .font_family(sleipnir_settings::default_font_family())
                                        .child("copied to clipboard"),
                                ),
                        ),
                )
            })
    }
}

/// Best-effort notification when a long-running command finishes (M14).
fn notify_command_finished(dur: std::time::Duration) {
    let secs = dur.as_secs();
    notify_message("Sleipnir", &format!("Command finished after {secs}s"));
}

/// Escape a value for interpolation into an AppleScript `"..."` string.
///
/// Both `\` and `"` must be escaped. A plugin-controlled title that is
/// interpolated raw can close the string and run the rest as AppleScript
/// (`osascript` will execute it). Newlines are flattened so they cannot
/// terminate the statement.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ")
}

/// The `-e` script passed to `osascript`. Title and message are both escaped
/// so a HostCall::Notify cannot break out of the string literals.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn applescript_notification_script(title: &str, message: &str) -> String {
    format!(
        "display notification \"{}\" with title \"{}\"",
        applescript_escape(message),
        applescript_escape(title)
    )
}

/// Notification program for a given desktop platform.
#[cfg(any(target_os = "linux", test))]
fn notification_program_for(windows: bool, linux: bool) -> &'static str {
    if windows {
        "powershell"
    } else if linux {
        "notify-send"
    } else {
        "osascript"
    }
}

/// Arguments passed directly to `notify-send`, without shell interpolation.
#[cfg(any(target_os = "linux", test))]
fn linux_notification_args<'a>(title: &'a str, message: &'a str) -> [&'a str; 4] {
    ["--app-name", "Sleipnir", title, message]
}

/// Best-effort desktop notification (OSC 9 / 777 / command finish / HostCall).
///
/// The single notification path. HostCall::Notify must use this — a second
/// builder would reintroduce the AppleScript injection hole.
pub(crate) fn notify_message(title: &str, message: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = applescript_notification_script(title, message);
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .spawn();
    }
    #[cfg(windows)]
    {
        // Best-effort toast via PowerShell. Failures are silent.
        let title = title.replace('\'', "''");
        let message = message.replace('\'', "''");
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $text = $template.GetElementsByTagName('text'); \
             $text.Item(0).AppendChild($template.CreateTextNode('{title}')) > $null; \
             $text.Item(1).AppendChild($template.CreateTextNode('{message}')) > $null; \
             $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Sleipnir').Show($toast)"
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        match std::process::Command::new(notification_program_for(false, true))
            .args(linux_notification_args(title, message))
            .spawn()
        {
            Ok(mut child) => {
                std::thread::spawn(move || match child.wait() {
                    Ok(status) if status.success() => {}
                    Ok(status) => log::warn!("notify-send exited with {status}"),
                    Err(err) => log::warn!("failed waiting for notify-send: {err}"),
                });
            }
            Err(err) => log::warn!("failed to start notify-send: {err}"),
        }
    }

    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (title, message);
    }
}

/// Clamp font size for zoom / settings (pt).
pub const FONT_SIZE_MIN: f32 = 8.0;
pub const FONT_SIZE_MAX: f32 = 72.0;
pub const FONT_SIZE_STEP: f32 = 1.0;

/// Effective font size from optional override + settings.
pub fn effective_font_size(override_size: Option<Pixels>, cx: &App) -> Pixels {
    let base = override_size
        .or_else(|| TerminalSettings::get_global(cx).font_size)
        .unwrap_or(gpui::px(14.));
    let v = f32::from(base).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
    gpui::px(v)
}

fn handle_bell(window: &mut Window, cx: &mut Context<TermView>) {
    match TerminalSettings::get_global(cx).bell {
        TerminalBell::Off => {}
        TerminalBell::System => {
            window.play_system_bell();
        }
        TerminalBell::Visual => {
            // Tab flash is applied by AppShell via TermViewEvent::Bell.
        }
    }
}

/// Whether a keystroke is reserved for clipboard actions (Copy/Paste/PasteText).
///
/// Plain `Ctrl+C` / `Ctrl+V` must **not** match so they reach the PTY as ETX/SYN
/// and can interrupt or pass control sequences to foreground programs.
fn is_clipboard_shortcut(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    let is_c = keystroke.key.eq_ignore_ascii_case("c");
    let is_v = keystroke.key.eq_ignore_ascii_case("v");

    // Cmd+C/V and Ctrl+Shift+C/V. Plain Ctrl+C always stays with the PTY.
    // On Windows, Ctrl+V is the shipped paste binding (never Ctrl+C).
    (modifiers.platform && (is_c || is_v))
        || (modifiers.control && modifiers.shift && !modifiers.platform && (is_c || is_v))
        || (cfg!(windows) && modifiers.control && !modifiers.shift && !modifiers.platform && is_v)
}

/// Open web URLs, and path-like targets when `path_links` is enabled (M12).
fn open_navigation_target(target: &MaybeNavigationTarget, cx: &App) {
    match target {
        MaybeNavigationTarget::Url(url) if is_web_url(url) => {
            log::info!("opening url: {url}");
            cx.open_url(url);
        }
        MaybeNavigationTarget::Url(url) => {
            log::debug!("ignoring non-web url: {url}");
        }
        MaybeNavigationTarget::PathLike(path) => {
            if !TerminalSettings::get_global(cx).path_links {
                log::debug!("path_links disabled; ignoring {}", path.maybe_path);
                return;
            }
            open_path_like_target(path);
        }
    }
}

/// Parsed path target with optional line/column suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPathTarget {
    pub path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// Split `path`, `path:line`, or `path:line:col` (and optional `file://` prefix).
pub fn parse_path_line_col(input: &str) -> ParsedPathTarget {
    let mut s = input.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        s = rest;
        // file:///abs → /abs; file://localhost/abs handled loosely
        if let Some(rest) = s.strip_prefix("localhost") {
            s = rest;
        }
    }
    // Strip a single leading slash after host-less file: (//path → /path already)
    let (path, line, column) = split_path_line_col(s);
    ParsedPathTarget { path, line, column }
}

fn split_path_line_col(s: &str) -> (String, Option<u32>, Option<u32>) {
    // From the right: optional :col, then :line. A Windows drive prefix
    // (`C:` / `/C:`) is not treated as a suffix separator.
    let drive = windows_drive_prefix_len(s);
    let searchable = &s[drive..];
    let mut col_start = None;
    let mut line_start = None;
    if let Some(rel) = searchable.rfind(':') {
        let colon = drive + rel;
        let after = &s[colon + 1..];
        if !after.is_empty() && after.bytes().all(|b| b.is_ascii_digit()) {
            col_start = Some(colon);
            let before = &s[drive..colon];
            if let Some(rel2) = before.rfind(':') {
                let colon2 = drive + rel2;
                let mid = &s[colon2 + 1..colon];
                if !mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()) {
                    line_start = Some(colon2);
                }
            }
        }
    }
    match (line_start, col_start) {
        (Some(ls), Some(cs)) => {
            let path = s[..ls].to_string();
            let line = s[ls + 1..cs].parse().ok();
            let column = s[cs + 1..].parse().ok();
            (path, line, column)
        }
        (None, Some(cs)) => {
            let path = s[..cs].to_string();
            let line = s[cs + 1..].parse().ok();
            (path, line, None)
        }
        _ => (s.to_string(), None, None),
    }
}

/// Length of a leading `C:` / `/C:` drive prefix so `:line` parsing ignores it.
fn windows_drive_prefix_len(s: &str) -> usize {
    let (offset, rest) = if let Some(stripped) = s.strip_prefix('/') {
        (1, stripped)
    } else {
        (0, s)
    };
    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        if bytes.len() == 2 || bytes[2] == b'\\' || bytes[2] == b'/' {
            return offset + 2;
        }
    }
    0
}

/// Resolve a path-like target against cwd and open if it exists.
fn open_path_like_target(path: &terminal::PathLikeTarget) {
    let parsed = parse_path_line_col(&path.maybe_path);
    if parsed.path.is_empty() {
        return;
    }
    // Skip method-call-looking tokens when they don't exist on disk.
    if looks_like_method_call(&parsed.path) {
        let candidate = resolve_path(&parsed.path, path.working_directory.as_deref());
        if !candidate.exists() {
            log::debug!("skipping method-like path target: {}", parsed.path);
            return;
        }
    }
    let candidate = resolve_path(&parsed.path, path.working_directory.as_deref());
    if !candidate.exists() {
        log::debug!(
            "path-like target does not exist: {} (resolved {})",
            parsed.path,
            candidate.display()
        );
        return;
    }
    log::info!(
        "opening path: {} (line={:?} col={:?})",
        candidate.display(),
        parsed.line,
        parsed.column
    );
    // Existence-checked above → no panic. Line/col are not passed (the
    // default app may ignore them).
    open_existing_path(&candidate);
}

/// Program used to open paths. `None` on Windows (`cmd /C start`).
pub fn path_opener_program() -> Option<&'static str> {
    path_opener_program_for(cfg!(windows), cfg!(target_os = "linux"))
}

/// Select the native path opener for Windows, Linux, or macOS.
pub fn path_opener_program_for(windows: bool, linux: bool) -> Option<&'static str> {
    if windows {
        None
    } else if linux {
        Some("xdg-open")
    } else {
        Some("open")
    }
}

pub(crate) fn open_existing_path(candidate: &Path) {
    #[cfg(windows)]
    {
        match std::process::Command::new("cmd")
            .args(["/C", "start", "", &candidate.to_string_lossy()])
            .spawn()
        {
            Ok(_) => {}
            Err(err) => log::warn!("failed to open {}: {err}", candidate.display()),
        }
    }
    #[cfg(not(windows))]
    {
        let program = path_opener_program().expect("non-Windows platforms have a path opener");
        match std::process::Command::new(program).arg(candidate).spawn() {
            Ok(_) => {}
            Err(err) => log::warn!(
                "failed to start {program} for {}: {err}",
                candidate.display()
            ),
        }
    }
}

fn resolve_path(path_str: &str, cwd: Option<&Path>) -> PathBuf {
    let p = Path::new(path_str);
    if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(p)
    } else {
        p.to_path_buf()
    }
}

fn looks_like_method_call(s: &str) -> bool {
    // e.g. foo.bar() or obj.method — common false positives from path regexes.
    s.contains("()") || (s.contains('.') && s.contains('(') && s.ends_with(')'))
}

fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("ftp://")
}

/// Monotonic counter for unique temp file names (avoids clock-regression issues).
static PASTE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write a clipboard image to a unique temp file and return its absolute path.
fn write_clipboard_image_to_temp(image: &gpui::Image) -> Result<PathBuf, String> {
    let ext = image.format().extension();
    let counter = PASTE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("sleipnir-paste-{pid}-{counter}.{ext}"));
    std::fs::write(&path, image.bytes()).map_err(|e| e.to_string())?;
    log::info!("pasted clipboard image to {}", path.display());
    Ok(path)
}

/// Quote a filesystem path for safe insertion into a shell command line.
fn shell_quote_path(path: &Path) -> String {
    quote_path_for_shell(path)
}

/// Quote `path` for the compiling OS shell (POSIX or PowerShell).
pub fn quote_path_for_shell(path: &Path) -> String {
    quote_path_for_shell_os(path, cfg!(windows))
}

/// Quote `path`. `windows = true` uses PowerShell single-quote rules.
pub fn quote_path_for_shell_os(path: &Path, windows: bool) -> String {
    let s = path.to_string_lossy();
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '/' | '\\' | '.' | '_' | '-' | '=' | ',' | ':' | '@' | '+'
            )
    });
    if safe {
        return s.into_owned();
    }
    if windows {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Format Finder/file-manager paths for paste: leading space, quoted paths, trailing space.
fn format_external_paths(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push(' ');
        out.push_str(&shell_quote_path(path));
    }
    if !out.is_empty() {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(keystroke: &str) -> Keystroke {
        Keystroke::parse(keystroke).unwrap_or_else(|err| panic!("parse {keystroke:?}: {err}"))
    }

    #[test]
    fn plain_control_c_and_v_are_sent_to_the_terminal() {
        assert!(!is_clipboard_shortcut(&parse("ctrl-c")));
        // Windows ships Ctrl+V as paste; macOS leaves plain Ctrl+V for the PTY.
        assert_eq!(is_clipboard_shortcut(&parse("ctrl-v")), cfg!(windows));
    }

    #[test]
    fn configured_clipboard_shortcuts_are_reserved() {
        for shortcut in [
            "cmd-c",
            "cmd-v",
            "ctrl-shift-c",
            "ctrl-shift-v",
            "ctrl-cmd-v",
        ] {
            assert!(
                is_clipboard_shortcut(&parse(shortcut)),
                "expected clipboard shortcut reserved: {shortcut}"
            );
        }
    }

    #[test]
    fn unrelated_or_unbound_combinations_are_not_reserved() {
        for shortcut in [
            "c",
            "v",
            "ctrl-x",
            "ctrl-a",
            "cmd-x",
            "ctrl-shift-x",
            "alt-c",
            "ctrl-alt-c",
            "shift-c",
        ] {
            assert!(
                !is_clipboard_shortcut(&parse(shortcut)),
                "expected not reserved for terminal: {shortcut}"
            );
        }
    }

    #[test]
    fn clipboard_routing_is_case_insensitive_for_c_and_v() {
        // Keystroke::parse maps an uppercase letter to shift+lowercase; so
        // "ctrl-C" is equivalent to the reserved clipboard combo ctrl-shift-c.
        assert!(is_clipboard_shortcut(&parse("ctrl-C")));
        assert!(is_clipboard_shortcut(&parse("ctrl-V")));
        assert!(is_clipboard_shortcut(&parse("cmd-C")));
        assert!(is_clipboard_shortcut(&parse("cmd-V")));
        assert!(is_clipboard_shortcut(&parse("ctrl-shift-C")));
        assert!(is_clipboard_shortcut(&parse("ctrl-shift-V")));

        // Runtime events may still carry an uppercase key without shift; those
        // must use case-insensitive key matching, not parse-time shift folding.
        let mut plain_ctrl_c = parse("ctrl-c");
        plain_ctrl_c.key = "C".into();
        assert!(!is_clipboard_shortcut(&plain_ctrl_c));

        let mut plain_ctrl_v = parse("ctrl-v");
        plain_ctrl_v.key = "V".into();
        assert_eq!(is_clipboard_shortcut(&plain_ctrl_v), cfg!(windows));

        let mut cmd_c = parse("cmd-c");
        cmd_c.key = "C".into();
        assert!(is_clipboard_shortcut(&cmd_c));
    }

    #[test]
    fn shell_quote_path_quotes_unsafe_paths() {
        assert_eq!(
            quote_path_for_shell(Path::new("/tmp/safe-name")),
            "/tmp/safe-name"
        );
        assert_eq!(
            quote_path_for_shell(Path::new("/tmp/has space")),
            "'/tmp/has space'"
        );
        assert_eq!(
            quote_path_for_shell_os(Path::new("/tmp/o'reilly"), false),
            "'/tmp/o'\\''reilly'"
        );
        assert_eq!(quote_path_for_shell(Path::new("")), "''");
        assert_eq!(
            quote_path_for_shell_os(Path::new(r"C:\Program Files\x"), true),
            r"'C:\Program Files\x'"
        );
        assert_eq!(
            quote_path_for_shell_os(Path::new(r"C:\o'reilly"), true),
            r"'C:\o''reilly'"
        );
        assert_eq!(
            quote_path_for_shell_os(Path::new(r"C:\safe"), true),
            r"C:\safe"
        );
    }

    #[test]
    fn format_external_paths_joins_quoted_paths() {
        let paths = vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b c")];
        assert_eq!(format_external_paths(&paths), " /tmp/a '/tmp/b c' ");
        assert_eq!(format_external_paths(&[]), "");
    }

    #[test]
    fn is_web_url_accepts_common_schemes() {
        assert!(is_web_url("https://example.com"));
        assert!(is_web_url("HTTP://example.com"));
        assert!(is_web_url("mailto:a@b.com"));
        assert!(is_web_url("ftp://files.example"));
        assert!(!is_web_url("file:///tmp/x"));
        assert!(!is_web_url("not-a-url"));
    }

    #[test]
    fn splits_line_column_suffix() {
        assert_eq!(
            parse_path_line_col("src/main.rs:10:2"),
            ParsedPathTarget {
                path: "src/main.rs".into(),
                line: Some(10),
                column: Some(2),
            }
        );
        assert_eq!(
            parse_path_line_col("src/main.rs:10"),
            ParsedPathTarget {
                path: "src/main.rs".into(),
                line: Some(10),
                column: None,
            }
        );
        assert_eq!(
            parse_path_line_col("/tmp/foo"),
            ParsedPathTarget {
                path: "/tmp/foo".into(),
                line: None,
                column: None,
            }
        );
        assert_eq!(
            parse_path_line_col("file:///Users/me/x.rs:3:4"),
            ParsedPathTarget {
                path: "/Users/me/x.rs".into(),
                line: Some(3),
                column: Some(4),
            }
        );
        assert_eq!(
            parse_path_line_col(r"C:\Windows\win.ini:12"),
            ParsedPathTarget {
                path: r"C:\Windows\win.ini".into(),
                line: Some(12),
                column: None,
            }
        );
        assert_eq!(
            parse_path_line_col(r"C:\foo\bar.rs:10:2"),
            ParsedPathTarget {
                path: r"C:\foo\bar.rs".into(),
                line: Some(10),
                column: Some(2),
            }
        );
        assert_eq!(
            parse_path_line_col("C:/Users/me/x.rs:3"),
            ParsedPathTarget {
                path: "C:/Users/me/x.rs".into(),
                line: Some(3),
                column: None,
            }
        );
    }

    #[test]
    fn resolve_path_joins_relative_to_cwd() {
        let cwd = Path::new("/project");
        assert_eq!(
            resolve_path("src/main.rs", Some(cwd)),
            PathBuf::from("/project/src/main.rs")
        );
        assert_eq!(resolve_path("/abs/x", Some(cwd)), PathBuf::from("/abs/x"));
    }

    #[test]
    fn method_call_guardrail() {
        assert!(looks_like_method_call("foo.bar()"));
        assert!(!looks_like_method_call("src/main.rs"));
    }

    /// Regression: GPUI binds the printed key name. `cmd-plus` / `cmd-minus`
    /// parse as keys "plus"/"minus", which the platform never emits for ⌘+/⌘-.
    /// The shipped binding table must use `cmd-+` and `cmd--` (and `cmd-=` / `cmd-0`).
    #[test]
    fn font_zoom_shipped_keystrokes_parse_as_platform_keys() {
        let bindings = font_zoom_key_bindings();
        let want_plus = if cfg!(target_os = "macos") {
            "cmd-+"
        } else {
            "ctrl-shift-+"
        };
        let want_minus = if cfg!(target_os = "macos") {
            "cmd--"
        } else {
            "ctrl-shift--"
        };
        assert!(
            bindings
                .iter()
                .any(|(k, a)| *k == want_plus && *a == "increase_font_size"),
            "shipped table must include {want_plus} for increase"
        );
        assert!(
            bindings
                .iter()
                .any(|(k, a)| *k == want_minus && *a == "decrease_font_size"),
            "shipped table must include {want_minus} for decrease"
        );
        assert!(
            !bindings
                .iter()
                .any(|(k, _)| k.ends_with("-plus") || k.ends_with("-minus")),
            "must not use plus/minus key names (never-emitted)"
        );

        for (keystroke, action) in bindings {
            let ks = Keystroke::parse(keystroke).unwrap_or_else(|err| {
                panic!("shipped font-zoom keystroke {keystroke:?} must parse: {err}")
            });
            if cfg!(target_os = "macos") {
                assert!(
                    ks.modifiers.platform,
                    "{keystroke} must include cmd/platform"
                );
            } else {
                assert!(ks.modifiers.control, "{keystroke} must include ctrl");
            }
            match *action {
                "increase_font_size" => {
                    assert!(
                        ks.key == "+" || ks.key == "=",
                        "{keystroke} parsed key={:?}, want + or =",
                        ks.key
                    );
                }
                "decrease_font_size" => {
                    assert_eq!(ks.key, "-", "{keystroke} must parse to key \"-\"");
                }
                "reset_font_size" => {
                    assert_eq!(ks.key, "0", "{keystroke} must parse to key \"0\"");
                }
                other => panic!("unexpected action id {other}"),
            }
        }

        // Contrast: the incorrect strings the bug used parse to dead keys.
        let wrong_plus = Keystroke::parse("cmd-plus").expect("cmd-plus still parses as a string");
        assert_eq!(
            wrong_plus.key, "plus",
            "cmd-plus binds dead key name \"plus\" (platform never emits this)"
        );
        let wrong_minus =
            Keystroke::parse("cmd-minus").expect("cmd-minus still parses as a string");
        assert_eq!(
            wrong_minus.key, "minus",
            "cmd-minus binds dead key name \"minus\" (platform never emits this)"
        );
    }

    #[test]
    fn desktop_commands_are_platform_specific() {
        assert_eq!(path_opener_program_for(false, false), Some("open"));
        assert_eq!(path_opener_program_for(true, false), None);
        assert_eq!(path_opener_program_for(false, true), Some("xdg-open"));
        assert_eq!(notification_program_for(false, false), "osascript");
        assert_eq!(notification_program_for(true, false), "powershell");
        assert_eq!(notification_program_for(false, true), "notify-send");
    }

    #[test]
    fn linux_notification_arguments_do_not_use_a_shell() {
        assert_eq!(
            linux_notification_args("Sleipnir", "build; rm -rf /"),
            ["--app-name", "Sleipnir", "Sleipnir", "build; rm -rf /"]
        );
    }

    #[test]
    fn applescript_title_and_message_cannot_break_out_of_the_string_literal() {
        // A plugin-controlled title with a raw quote used to close the
        // AppleScript string and run the rest (`do shell script ...`).
        let title = r#"pwned" & (do shell script "id") & "x"#;
        let message = r#"hi" & (do shell script "uname") & "y"#;
        let script = applescript_notification_script(title, message);
        assert_eq!(
            script,
            r#"display notification "hi\" & (do shell script \"uname\") & \"y" with title "pwned\" & (do shell script \"id\") & \"x""#
        );
        // Backslash must be doubled so a trailing \" cannot close the literal.
        let script = applescript_notification_script(r#"end\"#, r#"body\"#);
        assert_eq!(
            script,
            r#"display notification "body\\" with title "end\\""#
        );
        assert!(
            !script.contains(r#"with title "end""#),
            "raw title interpolation would close the string early"
        );
    }

    /// Regression: the plugin status chip was rendered immediately after
    /// `tab_scroll`, which is `None` with a single tab. The chip then collapsed
    /// left and sat against the macOS traffic lights, inside the titlebar drag
    /// region. It must come after `trailing_drag` so it stays at the trailing
    /// end of the chrome band regardless of tab count.
    #[test]
    fn plugin_status_chip_sits_after_the_trailing_drag_region() {
        let sources = all_ui_sources();
        let src = sources
            .iter()
            .find(|src| src.contains("render_plugin_status_chip(&tokens, cx)"))
            .expect("chrome band renders the plugin status chip");
        let trailing = src
            .find(".child(trailing_drag)")
            .expect("chrome band has a trailing drag region");
        let chip = src
            .find("render_plugin_status_chip(&tokens, cx)")
            .expect("chip call site");
        assert!(
            trailing < chip,
            "the chip must follow trailing_drag, else a single-tab window \
             collapses it onto the traffic lights"
        );
    }

    /// ADR-0016 §7 requires an indicator a *plugin* cannot suppress. The host
    /// still owns the zero case, and "0 plugins" is chrome spent on the state
    /// that carries no information, so the chip is hidden at zero.
    #[test]
    fn plugin_status_chip_is_hidden_when_no_plugins_run() {
        let sources = all_ui_sources();
        let src = sources
            .iter()
            .find(|src| src.contains(r#".id("plugin-status-chip")"#))
            .expect("plugin-status-chip is rendered somewhere");
        let chip = src
            .find(r#".id("plugin-status-chip")"#)
            .expect("plugin-status-chip");
        let body = &src[chip..];
        let guard = body
            .find(".when(n > 0")
            .expect("chip content must be gated on a non-zero plugin count");
        let label = body
            .find("running_indicator_label(n)")
            .expect("chip renders the running-indicator label");
        assert!(
            guard < label,
            "the label must sit inside the n > 0 guard, not outside it"
        );
    }

    /// Regression: clicking Close on the confirm dialog must not hit the
    /// full-size backdrop. Other overlays (settings/update/palette) already
    /// stop mouse_down on the panel; this dialog originally omitted that.
    #[test]
    fn close_confirm_panel_stops_backdrop_clicks() {
        let sources = all_ui_sources();
        let src = sources
            .iter()
            .find(|src| src.contains(r#".id("close-confirm-panel")"#))
            .expect("close-confirm-panel is rendered somewhere");
        let panel = src
            .find(r#".id("close-confirm-panel")"#)
            .expect("close-confirm-panel");
        let backdrop = src
            .find(r#".id("close-confirm-backdrop")"#)
            .expect("close-confirm-backdrop");
        assert!(
            panel < backdrop,
            "panel markup should precede backdrop markup"
        );
        assert!(
            src[panel..backdrop].contains("stop_propagation"),
            "close-confirm panel must stop mouse_down so Close is not cancelled by the backdrop"
        );
    }

    /// Modal overlays sit as siblings above TermElement, not as ancestors.
    /// GPUI's `on_scroll_wheel` fires when `hitbox.should_handle_scroll()` is
    /// true for *every* hitbox under the pointer. Only `.occlude()`
    /// (`HitboxBehavior::BlockMouse`) drops the terminal from that list.
    /// `stop_propagation` on mouse_down does not.
    fn overlay_builder_prefix<'a>(src: &'a str, id: &str) -> Option<&'a str> {
        let needle = format!(r#".id("{id}")"#);
        let start = src.find(&needle)?;
        let after = &src[start..];
        let child = after
            .find(".child(")
            .unwrap_or_else(|| panic!("{id} has no .child("));
        Some(&after[..child])
    }

    /// Every `.rs` under `src/` except this file, so extracting a renderer into
    /// a new module never silently drops it from the scans below. This file is
    /// skipped because the assertions themselves contain the markup they look
    /// for, which would otherwise satisfy every check trivially.
    /// ADR-0018: a future patch must not silently reintroduce `line * line_height`
    /// for a y coordinate. Widget-local cell rows (plugin_block paint) are
    /// integer cells, not grid display lines, and are excluded.
    #[test]
    fn host_y_coordinates_go_through_row_geometry() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let term = std::fs::read_to_string(src_dir.join("term_element.rs")).unwrap();
        for needle in [
            "as f32 * dimensions.line_height",
            "as f32 * line_height",
            "as f32 + 0.5) * dimensions.line_height",
            "as f32 + 1.0) * dimensions.line_height",
            "as f32 + 0.5) * line_height",
        ] {
            assert!(
                !term.contains(needle),
                "term_element.rs must not compute a y as line * line_height ({needle})"
            );
        }
        assert!(
            term.contains("y_for_display") && term.contains("PaintMap"),
            "term_element.rs must route y through RowGeometry via PaintMap"
        );
        let terminal_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("terminal")
            .join("src");
        for name in ["mappings/mouse.rs", "terminal.rs"] {
            let src = std::fs::read_to_string(terminal_dir.join(name)).unwrap();
            assert!(
                !src.contains("pos.y / cur_size.line_height")
                    && !src.contains("pos.y / terminal_bounds.line_height"),
                "{name} must not divide y by line_height to get a line"
            );
        }
        assert!(
            !std::fs::read_to_string(terminal_dir.join("terminal.rs"))
                .unwrap()
                .contains("scroll_px %="),
            "scroll remainder must be retained, not discarded with modulo"
        );
    }

    fn all_ui_sources() -> Vec<String> {
        fn walk(dir: &std::path::Path, skip: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read src dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, skip, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") && path != skip {
                    out.push(std::fs::read_to_string(&path).expect("read source"));
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this = src.join("sleipnir_ui.rs");
        let mut out = Vec::new();
        walk(&src, &this, &mut out);
        assert!(!out.is_empty(), "no sources found under {}", src.display());
        out
    }

    #[test]
    fn modal_overlays_occlude_so_terminal_does_not_scroll() {
        let sources = all_ui_sources();
        for id in [
            "settings-overlay",
            "update-overlay",
            "palette-overlay",
            "close-confirm-overlay",
            "diff-overlay",
        ] {
            let prefix = sources
                .iter()
                .find_map(|src| overlay_builder_prefix(src, id))
                .unwrap_or_else(|| panic!("{id} is not rendered anywhere in this crate"));
            assert!(
                prefix.contains(".occlude()"),
                "{id} must .occlude() so wheel events do not reach TermElement under the overlay"
            );
        }
    }

    #[test]
    fn chrome_hides_action_buttons() {
        let src = include_str!("chrome/tab_strip.rs");
        assert!(
            !src.contains(r#".id("diff-chrome-button")"#),
            "Diff should be keyboard-only in the chrome"
        );
        assert!(
            !src.contains(r#".id("strip-new-tab")"#),
            "new tab should be keyboard-only in the chrome"
        );
        assert!(
            all_ui_sources()
                .iter()
                .any(|src| src.contains("render_desktop_titlebar_end")),
            "chrome band must host non-macOS desktop caption buttons"
        );
    }

    /// Empty-region window move lives on `chrome-drag-trailing`. A height-less
    /// wrapper (the old `chrome-trailing` row) collapses `h_full()` to 0px,
    /// and `app_owns_titlebar_drag` then leaves no native fallback.
    #[test]
    fn chrome_trailing_drag_is_a_direct_band_child() {
        let sources = all_ui_sources();
        let band_module = sources
            .iter()
            .find(|src| src.contains(r#".id("chrome-band")"#))
            .expect("chrome-band");
        assert!(
            sources
                .iter()
                .all(|src| !src.contains(r#".id("chrome-trailing")"#)),
            "do not wrap chrome-drag-trailing in a height-less row"
        );
        let band = band_module
            .find(r#".id("chrome-band")"#)
            .expect("chrome-band");
        assert!(
            band_module[band..].contains(".child(trailing_drag)"),
            "chrome-band must parent trailing_drag directly so h_full() resolves against the band height"
        );
        assert!(
            sources
                .iter()
                .any(|src| src.contains("WindowControlArea::Drag")),
            "desktop drag requires WindowControlArea::Drag"
        );
    }

    #[test]
    fn running_plugins_indicator_is_always_in_chrome_and_not_after_plugin_status() {
        let sources = all_ui_sources();
        let band_module = sources
            .iter()
            .find(|src| src.contains(r#".id("chrome-band")"#))
            .expect("chrome-band");
        let chip = band_module
            .find("render_plugin_status_chip")
            .expect("running-plugins indicator must be host-drawn in the chrome band");
        let plugin_status = band_module.find("render_plugin_chrome_status");
        assert!(
            plugin_status.is_none_or(|p| chip < p),
            "plugin status items must sit after the running-plugins chip so they cannot cover it"
        );
        assert!(
            sources
                .iter()
                .any(|src| src.contains(r#".id("plugin-status-chip")"#)),
            "the indicator id must exist so it cannot be omitted by a contribution"
        );
        assert!(
            sources
                .iter()
                .any(|src| src.contains("render_desktop_titlebar_end")),
            "desktop caption buttons must ship on the chrome band"
        );
    }
}
