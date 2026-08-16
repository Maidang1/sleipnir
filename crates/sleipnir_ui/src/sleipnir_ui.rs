//! Terminal UI for sleipnir (M2 PTY input, M3 tabs + URL open, HIG chrome).

mod app_shell;
mod blink;
mod chrome;
mod command_palette;
mod keymap;
mod pane_tree;
mod run_ledger_global;
mod session;
mod term_element;

pub use blink::{BLINK_HALF_PERIOD, cursor_blink_alpha};

pub use app_shell::{
    ActivateTab, AppShell, CheckForUpdates, ClearRunLedger, CloseTab, CycleTheme, DecreaseFontSize,
    ExportScrollback, Find, FindNext, FindPrev, FocusPaneDown, FocusPaneLeft, FocusPaneRight,
    FocusPaneUp, IncreaseFontSize, JumpNextPrompt, JumpPrevPrompt, NewTab, NewWindow, NextTab,
    OpenQuickTerminal, OpenSettings, PrevTab, ReloadSettings, ResetFontSize, SplitDown,
    SplitRight, ToggleBroadcast, ToggleCommandPalette, TogglePaneZoom, ToggleQuickSelect,
    ToggleRunLedger, UpdateUiState, open_sleipnir_window,
};
pub use chrome::{ChromeGeometry, ChromeTokens, active_after_close, contrast_ratio};
pub use command_palette::{CommandId, CommandItem, commands as palette_commands};
pub use keymap::{
    BindingContext, BuiltinAction, BuiltinBinding, builtin_bindings, display_shortcut,
    font_zoom_key_bindings,
};
pub use pane_tree::{
    Branch, CloseOutcome, Direction, MIN_RATIO, PaneId, PaneNode, PaneRect, SplitAxis, SplitPath,
    neighbor,
};
pub use run_ledger_global::RunLedgerGlobal;
pub use session::{SessionFile, SessionNode, SessionTab, load_session, save_session, session_path};
pub use term_element::TermElement;

use collections::HashMap;
use gpui::{
    App, AppContext as _, ClipboardEntry, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div, rgb,
};
use gpui::Pixels;
use gpui::prelude::FluentBuilder as _;
use sleipnir_settings::{
    AlternateScroll, NotifyOnCommandFinish, TerminalBell, TerminalBlink, TerminalPalette,
    TerminalSettings,
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
    },
    /// The current command in this pane finished.
    RunFinished { exit_code: Option<i32> },
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
        let settings = TerminalSettings::get_global(cx).clone();
        let window_id = window.window_handle().window_id().as_u64();
        let cwd = cwd.or_else(dirs::home_dir);
        let mut env: HashMap<String, String> = std::env::vars().collect();
        for (k, v) in &settings.env {
            env.insert(k.clone(), v.clone());
        }
        let shell = terminal::apply_inject_to_shell(
            settings.shell.clone(),
            &mut env,
            settings.inject_osc133,
        );

        let builder_task = TerminalBuilder::new(
            cwd,
            None,
            shell,
            env,
            settings.cursor_shape,
            settings.alternate_scroll,
            settings.max_scroll_history_lines,
            settings.path_hyperlink_regexes.clone(),
            settings.path_hyperlink_timeout_ms,
            false,
            window_id,
            None,
            cx,
            Vec::new(),
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

    /// Keep display-only path for tests/demos.
    pub fn new_display_only(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = TerminalSettings::get_global(cx);
        let window_id = window.window_handle().window_id().as_u64();
        let builder = TerminalBuilder::new_display_only(
            settings.cursor_shape,
            AlternateScroll::On,
            settings.max_scroll_history_lines,
            window_id,
            cx.background_executor(),
            PathStyle::local(),
        );

        let terminal = cx.new(|cx| builder.subscribe(cx));
        let mut this = Self {
            terminal: TerminalSlot::Loading,
            focus_handle: cx.focus_handle(),
            title: "sleipnir (display)".into(),
            font_size_override: None,
            terminal_wants_blink: true,
            last_input_at: Instant::now(),
            copy_toast: None,
            last_render_at: None,
            last_offscreen_notify_at: None,
            _spawn: Task::ready(()),
        };
        this.attach_terminal(terminal, window, cx);
        this
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
                Event::TitleChanged | Event::BreadcrumbsChanged => {
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
                Event::Notify(message) => {
                    notify_message("Sleipnir", message);
                }
                Event::RunStarted {
                    command,
                    cwd,
                    inferred,
                } => {
                    cx.emit(TermViewEvent::RunStarted {
                        command: command.clone(),
                        cwd: cwd.clone(),
                        inferred: *inferred,
                    });
                }
                Event::RunFinished { exit_code } => {
                    cx.emit(TermViewEvent::RunFinished {
                        exit_code: *exit_code,
                    });
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

    pub fn write_output(&self, bytes: &[u8], cx: &mut Context<Self>) {
        if let TerminalSlot::Ready(terminal) = &self.terminal {
            terminal.update(cx, |term, cx| term.write_output(bytes, cx));
            cx.notify();
        }
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
                    cx.notify();
                }
            }
            _ => {
                if let Some(text) = clipboard.text() {
                    terminal.update(cx, |term, _| term.paste(&text));
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
        let toast_bg = palette.background.blend(gpui::Hsla::black().opacity(0.35)).alpha(1.0);
        let toast_border = palette.ansi[2].opacity(0.9);
        let toast_dot = palette.ansi[2];
        let toast_fg = palette.foreground.blend(palette.ansi[3].opacity(0.15)).alpha(1.0);

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

                    let a11y_text: SharedString =
                        terminal.read(cx).visible_screen_text().into();
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
                                .child(
                                    div()
                                        .size(gpui::px(7.0))
                                        .rounded_full()
                                        .bg(toast_dot),
                                )
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

/// Best-effort macOS desktop notification (OSC 9 / 777 / command finish).
fn notify_message(title: &str, message: &str) {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{escaped}\" with title \"{title}\"");
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .spawn();
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

    // Cmd+C/V and Ctrl+Shift+C/V. Plain Ctrl+C / Ctrl+V stay with the PTY.
    (modifiers.platform && (is_c || is_v))
        || (modifiers.control && modifiers.shift && !modifiers.platform && (is_c || is_v))
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
    ParsedPathTarget {
        path,
        line,
        column,
    }
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

/// Program used to open paths.
pub fn path_opener_program() -> Option<&'static str> {
    Some("open")
}

pub(crate) fn open_existing_path(candidate: &Path) {
    match std::process::Command::new("open").arg(candidate).spawn() {
        Ok(_) => {}
        Err(err) => log::warn!("failed to open {}: {err}", candidate.display()),
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

/// Quote `path` for POSIX shells.
pub fn quote_path_for_shell(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.is_empty() {
        return "''".to_string();
    }
    let safe = s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '/' | '.' | '_' | '-' | '=' | ',' | ':' | '@' | '+')
    });
    if safe {
        return s.into_owned();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
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
        assert!(!is_clipboard_shortcut(&parse("ctrl-v")));
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
        assert!(!is_clipboard_shortcut(&plain_ctrl_v));

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
            quote_path_for_shell(Path::new("/tmp/o'reilly")),
            "'/tmp/o'\\''reilly'"
        );
        assert_eq!(quote_path_for_shell(Path::new("")), "''");
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
        assert_eq!(
            resolve_path("/abs/x", Some(cwd)),
            PathBuf::from("/abs/x")
        );
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
        let want_plus = "cmd-+";
        let want_minus = "cmd--";
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
            !bindings.iter().any(|(k, _)| k.ends_with("-plus") || k.ends_with("-minus")),
            "must not use plus/minus key names (never-emitted)"
        );

        for (keystroke, action) in bindings {
            let ks = Keystroke::parse(keystroke).unwrap_or_else(|err| {
                panic!("shipped font-zoom keystroke {keystroke:?} must parse: {err}")
            });
            assert!(
                ks.modifiers.platform,
                "{keystroke} must include cmd/platform"
            );
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
    fn path_opener_is_open() {
        assert_eq!(path_opener_program(), Some("open"));
    }

    /// Regression: clicking Close on the confirm dialog must not hit the
    /// full-size backdrop. Other overlays (settings/update/palette) already
    /// stop mouse_down on the panel; this dialog originally omitted that.
    #[test]
    fn close_confirm_panel_stops_backdrop_clicks() {
        let src = include_str!("app_shell.rs");
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
    fn overlay_builder_prefix<'a>(src: &'a str, id: &str) -> &'a str {
        let needle = format!(r#".id("{id}")"#);
        let start = src
            .find(&needle)
            .unwrap_or_else(|| panic!("{id} missing from app_shell.rs"));
        let after = &src[start..];
        let child = after
            .find(".child(")
            .unwrap_or_else(|| panic!("{id} has no .child("));
        &after[..child]
    }

    #[test]
    fn modal_overlays_occlude_so_terminal_does_not_scroll() {
        let src = include_str!("app_shell.rs");
        for id in [
            "settings-overlay",
            "update-overlay",
            "palette-overlay",
            "close-confirm-overlay",
        ] {
            let prefix = overlay_builder_prefix(src, id);
            assert!(
                prefix.contains(".occlude()"),
                "{id} must .occlude() so wheel events do not reach TermElement under the overlay"
            );
        }
    }
}
