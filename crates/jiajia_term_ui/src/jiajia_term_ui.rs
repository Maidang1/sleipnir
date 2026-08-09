//! Display-oriented terminal UI for jiajia-term (M1: paint grid from `Terminal` content).

mod term_element;

pub use term_element::TermElement;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div, rgb,
};
use jiajia_settings::{AlternateScroll, TerminalPalette, TerminalSettings};
use terminal::{Event, Terminal, TerminalBuilder};
use util::paths::PathStyle;

/// Host view: owns a display-only `Terminal` and paints via [`TermElement`].
pub struct TermView {
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    title: SharedString,
}

impl Focusable for TermView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TermView {
    pub fn new_display_only(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = TerminalSettings::get_global(cx);
        let cursor_shape = settings.cursor_shape;
        let max_scroll = settings.max_scroll_history_lines;
        let window_id = window.window_handle().window_id().as_u64();
        let builder = TerminalBuilder::new_display_only(
            cursor_shape,
            AlternateScroll::On,
            max_scroll,
            window_id,
            cx.background_executor(),
            PathStyle::local(),
        );

        let terminal = cx.new(|cx| {
            // subscribe drains PTY events; display-only has none, but keeps API.
            builder.subscribe(cx)
        });

        cx.subscribe(&terminal, |_, _, event: &Event, cx| {
            if matches!(event, Event::Wakeup | Event::TitleChanged | Event::Bell) {
                cx.notify();
            }
        })
        .detach();

        Self {
            terminal,
            focus_handle: cx.focus_handle(),
            title: "jiajia-term".into(),
        }
    }

    pub fn terminal(&self) -> &Entity<Terminal> {
        &self.terminal
    }

    /// Inject ANSI/bytes into the display-only terminal grid.
    pub fn write_output(&self, bytes: &[u8], cx: &mut Context<Self>) {
        self.terminal.update(cx, |term, cx| {
            term.write_output(bytes, cx);
        });
        cx.notify();
    }

    pub fn write_demo_ansi(&self, cx: &mut Context<Self>) {
        let sample = concat!(
            "\x1b[1;37mjiajia-term\x1b[0m  M1 display-only grid\r\n",
            "\r\n",
            "  \x1b[31mred\x1b[0m \x1b[32mgreen\x1b[0m \x1b[33myellow\x1b[0m ",
            "\x1b[34mblue\x1b[0m \x1b[35mmagenta\x1b[0m \x1b[36mcyan\x1b[0m\r\n",
            "  \x1b[1;31mbright-red\x1b[0m \x1b[1;32mbright-green\x1b[0m ",
            "\x1b[1;34mbright-blue\x1b[0m\r\n",
            "  \x1b[38;2;250;179;135mtruecolor peach\x1b[0m ",
            "\x1b[48;2;30;30;46m\x1b[38;2;166;227;161m on bg\x1b[0m\r\n",
            "\r\n",
            "  cubox: █ ▓ ▒ ░  block: ▀ ▄ ─ │\r\n",
            "\r\n",
            "$ \x1b[32mready\x1b[0m for M2 PTY…\r\n",
        );
        self.write_output(sample.as_bytes(), cx);
    }
}

impl Render for TermView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let focused = self.focus_handle.is_focused(_window);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
            .text_color(palette.foreground)
            .track_focus(&self.focus_handle)
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
            .child(
                div()
                    .id("term-view-body")
                    .size_full()
                    .p_2()
                    .child(TermElement::new(
                        self.terminal.clone(),
                        self.focus_handle.clone(),
                        focused,
                    )),
            )
    }
}
