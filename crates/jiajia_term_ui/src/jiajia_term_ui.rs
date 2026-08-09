//! Terminal UI for jiajia-term (M2: local PTY + input).

mod term_element;

pub use term_element::TermElement;

use collections::HashMap;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Styled as _, Task,
    Window, div, rgb,
};
use jiajia_settings::{AlternateScroll, TerminalPalette, TerminalSettings};
use terminal::{Copy, Event, Paste, Terminal, TerminalBuilder};
use util::paths::PathStyle;

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
        let settings = TerminalSettings::get_global(cx).clone();
        let window_id = window.window_handle().window_id().as_u64();
        let cwd = dirs::home_dir();
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
                    this.terminal = TerminalSlot::Failed(format!("failed to open PTY: {err:#}").into());
                    cx.notify();
                }
            })
            .ok();
        });

        Self {
            terminal: TerminalSlot::Loading,
            focus_handle: cx.focus_handle(),
            title: "jiajia-term".into(),
            _spawn: spawn,
        }
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
            title: "jiajia-term (display)".into(),
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
        cx.subscribe_in(&terminal, window, |this, terminal, event, window, cx| {
            match event {
                Event::Wakeup | Event::SelectionsChanged | Event::BlinkChanged(_) => {
                    cx.notify();
                }
                Event::TitleChanged | Event::BreadcrumbsChanged => {
                    this.title = terminal.read(cx).title(true).into();
                    cx.notify();
                }
                Event::Bell => {
                    // Optional: system bell later.
                    cx.notify();
                }
                Event::CloseTerminal => {
                    this.title = "exited".into();
                    cx.notify();
                }
                Event::NewNavigationTarget(_) | Event::Open(_) => {}
            }
            let _ = window;
        })
        .detach();

        self.title = terminal.read(cx).title(true).into();
        self.terminal = TerminalSlot::Ready(terminal);
        window.focus(&self.focus_handle, cx);
        cx.notify();
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

        // Clipboard shortcuts (also bound via keymap when present).
        let mods = &event.keystroke.modifiers;
        if (mods.platform || mods.control) && event.keystroke.key.eq_ignore_ascii_case("c") {
            let has_selection = terminal
                .read(cx)
                .last_content()
                .selection_text
                .as_ref()
                .is_some_and(|t| !t.is_empty());
            if has_selection {
                terminal.update(cx, |term, _| term.copy(Some(true)));
                cx.notify();
                return;
            }
        }
        if (mods.platform || mods.control) && event.keystroke.key.eq_ignore_ascii_case("v") {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                terminal.update(cx, |term, _| term.paste(&text));
                cx.notify();
            }
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
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if let Some(terminal) = self.terminal_entity().cloned() {
            terminal.update(cx, |term, _| term.paste(&text));
            cx.notify();
        }
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
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(0x313244))
                    .text_sm()
                    .text_color(rgb(0xa6adc8))
                    .child(self.title.clone()),
            )
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
