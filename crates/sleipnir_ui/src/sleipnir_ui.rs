//! Terminal UI for sleipnir (M2 PTY input, M3 tabs + URL open, HIG chrome).

mod app_shell;
mod chrome;
mod command_palette;
mod pane_tree;
mod session;
mod term_element;

pub use app_shell::{
    ActivateTab, AppShell, CheckForUpdates, CloseTab, CycleTheme, Find, FindNext, FindPrev,
    FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, NewTab, NextTab, OpenSettings,
    PrevTab, ReloadSettings, SplitDown, SplitRight, ToggleCommandPalette, UpdateUiState,
};
pub use chrome::{ChromeGeometry, ChromeTokens, active_after_close, contrast_ratio};
pub use command_palette::{CommandId, CommandItem, commands as palette_commands};
pub use pane_tree::{
    Branch, CloseOutcome, Direction, MIN_RATIO, PaneId, PaneNode, PaneRect, SplitAxis, SplitPath,
    neighbor,
};
pub use session::{SessionFile, SessionNode, SessionTab, load_session, save_session, session_path};
pub use term_element::TermElement;

use collections::HashMap;
use gpui::{
    App, AppContext as _, ClipboardEntry, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, ParentElement as _, Render,
    SharedString, Styled as _, Task, Window, div, rgb,
};
use sleipnir_settings::{AlternateScroll, TerminalPalette, TerminalSettings};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    _spawn: Task<()>,
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

        let builder_task = TerminalBuilder::new(
            cwd,
            None,
            settings.shell.clone(),
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
            _spawn: spawn,
        }
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
        cx.subscribe_in(&terminal, window, |this, terminal, event, _window, cx| {
            match event {
                Event::Wakeup | Event::SelectionsChanged | Event::BlinkChanged(_) => {
                    cx.notify();
                }
                Event::TitleChanged | Event::BreadcrumbsChanged => {
                    this.title = terminal.read(cx).title(true).into();
                    cx.emit(TermViewEvent::TitleChanged);
                    cx.notify();
                }
                Event::Bell => {
                    cx.notify();
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
            }
        })
        .detach();

        self.title = terminal.read(cx).title(true).into();
        self.terminal = TerminalSlot::Ready(terminal);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub fn title(&self) -> &str {
        let title = self.title.as_ref();
        if title.is_empty() {
            "shell"
        } else {
            title
        }
    }

    pub fn terminal_entity(&self) -> Option<&Entity<Terminal>> {
        match &self.terminal {
            TerminalSlot::Ready(t) => Some(t),
            _ => None,
        }
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
        // skip them here so they don't get double-processed or sent to the PTY.
        let mods = &event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if (mods.platform || mods.control)
            && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("v"))
        {
            return;
        }

        let option_as_meta = TerminalSettings::get_global(cx).option_as_meta;
        let handled = terminal.update(cx, |term, _| {
            term.try_keystroke(&event.keystroke, option_as_meta)
        });
        if handled {
            cx.notify();
        }
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

    fn scroll_line_down(&mut self, _: &ScrollLineDown, _window: &mut Window, cx: &mut Context<Self>) {
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

    fn scroll_page_down(&mut self, _: &ScrollPageDown, _window: &mut Window, cx: &mut Context<Self>) {
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
            .map(|t| {
                t.read(cx)
                    .last_content()
                    .mode
                    .contains(Modes::ALT_SCREEN)
            })
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

    fn reload_settings(&mut self, _: &ReloadSettings, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestReloadSettings);
    }

    fn cycle_theme(&mut self, _: &CycleTheme, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestCycleTheme);
    }

    fn open_settings(&mut self, _: &OpenSettings, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TermViewEvent::RequestOpenSettings);
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let focused = self.focus_handle.is_focused(window);

        div()
            .size_full()
            .flex()
            .flex_col()
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
                TerminalSlot::Ready(terminal) => div()
                    .id("term-view-body")
                    .size_full()
                    .p_2()
                    .child(TermElement::new(
                        terminal.clone(),
                        self.focus_handle.clone(),
                        focused,
                    ))
                    .into_any_element(),
            })
    }
}

/// Open only web URLs (M3 scope). Paths are ignored for now.
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
            log::debug!("ignoring path-like target in M3: {}", path.maybe_path);
        }
    }
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
    let s = path.to_string_lossy();
    if s.is_empty() {
        return "''".to_string();
    }
    // Unquoted only when clearly safe for common shells.
    let safe = s.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '=' | ',' | ':' | '@' | '+')
    });
    if safe {
        s.into_owned()
    } else {
        // POSIX single-quote: wrap and escape embedded ' as '\''
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
