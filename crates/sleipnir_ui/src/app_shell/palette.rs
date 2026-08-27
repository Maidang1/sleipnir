//! Command palette: the overlay, its key handling and the query state.
//!
//! A child module of `app_shell` so it can drive the shell's private palette
//! state without widening it to the crate.

use gpui::{
    ClickEvent, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, deferred, div,
    prelude::FluentBuilder as _, px, relative,
};

use super::AppShell;
use crate::chrome::ChromeTokens;
use crate::command_palette::{CommandId, filter_commands};
use crate::ui_mode::OverlayKind;

impl AppShell {
    pub(super) fn open_palette(&mut self, cx: &mut Context<Self>) {
        self.mode.open(OverlayKind::Palette);
        self.palette_query.clear();
        self.palette_selected = 0;
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn palette_key_down(
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

    pub(super) fn render_command_palette(
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
}
