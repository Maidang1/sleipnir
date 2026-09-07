//! Terminal context menu: right-click on a terminal pane in normal mode.
//!
//! A child module of `app_shell` so it can drive the shell's private menu
//! state without widening it to the crate. Keyboard navigation (↑/↓/Enter/
//! Esc) lives in the shell's capture key handler.

use gpui::{
    ClickEvent, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, deferred, div, px,
    prelude::FluentBuilder as _,
};

use super::AppShell;
use crate::chrome::ChromeTokens;
use crate::chrome::pixel;

/// One actionable row of the terminal context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalMenuItem {
    Copy,
    Paste,
    OpenLink,
    SplitRight,
    SplitDown,
    Find,
    Clear,
}

fn menu_item_label(item: TerminalMenuItem) -> SharedString {
    match item {
        TerminalMenuItem::Copy => "Copy".into(),
        TerminalMenuItem::Paste => "Paste".into(),
        TerminalMenuItem::OpenLink => "Open Link".into(),
        TerminalMenuItem::SplitRight => "Split Right".into(),
        TerminalMenuItem::SplitDown => "Split Down".into(),
        TerminalMenuItem::Find => "Find in Scrollback…".into(),
        TerminalMenuItem::Clear => "Clear Scrollback".into(),
    }
}

/// Rows shown above a divider.
fn has_leading_divider(item: TerminalMenuItem) -> bool {
    matches!(
        item,
        TerminalMenuItem::SplitRight | TerminalMenuItem::Find
    )
}

impl AppShell {
    /// Rows for the current menu state, in display order.
    pub(crate) fn terminal_menu_items(&self) -> Vec<TerminalMenuItem> {
        let Some(state) = self.terminal_menu.as_ref() else {
            return Vec::new();
        };
        let mut items = vec![TerminalMenuItem::Copy, TerminalMenuItem::Paste];
        if state.link.is_some() {
            items.push(TerminalMenuItem::OpenLink);
        }
        items.push(TerminalMenuItem::SplitRight);
        items.push(TerminalMenuItem::SplitDown);
        items.push(TerminalMenuItem::Find);
        items.push(TerminalMenuItem::Clear);
        items
    }

    /// Execute a menu row. The menu is always closed first, mirroring the
    /// click path, so keyboard activation cannot leave it open.
    pub(crate) fn run_terminal_menu_item(
        &mut self,
        item: TerminalMenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let link = self.terminal_menu.as_ref().and_then(|s| s.link.clone());
        self.terminal_menu = None;
        match item {
            TerminalMenuItem::Copy => {
                if let Some(term) = self.active_terminal_entity(cx) {
                    term.update(cx, |t, _| t.copy(Some(true)));
                }
            }
            TerminalMenuItem::Paste => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    if let Some(term) = self.active_terminal_entity(cx) {
                        term.update(cx, |t, _| t.paste(&text));
                    }
                }
            }
            TerminalMenuItem::OpenLink => {
                if let Some(link) = link {
                    crate::open_navigation_target(&link, cx);
                }
            }
            TerminalMenuItem::SplitRight => {
                self.split_active(crate::pane_tree::SplitAxis::Horizontal, window, cx)
            }
            TerminalMenuItem::SplitDown => {
                self.split_active(crate::pane_tree::SplitAxis::Vertical, window, cx)
            }
            TerminalMenuItem::Find => self.open_find(window, cx),
            TerminalMenuItem::Clear => {
                if let Some(term) = self.active_terminal_entity(cx) {
                    term.update(cx, |t, _| t.clear());
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn render_terminal_menu(
        &self,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> impl IntoElement {
        let state = self
            .terminal_menu
            .as_ref()
            .expect("terminal menu state checked by caller");
        let items = self.terminal_menu_items();
        let selected = state.selected.min(items.len().saturating_sub(1));
        let border_w = pixel::PIXEL_BORDER;

        // Keep the panel inside the window: menus opened near an edge would
        // otherwise render off-screen.
        let viewport = window.viewport_size();
        let menu_w = px(190.0);
        let menu_h = px(items.len() as f32 * 28.0 + 16.0);
        let x = state.position.x.min((viewport.width - menu_w).max(px(0.0)));
        let y = state.position.y.min((viewport.height - menu_h).max(px(0.0)));

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            if has_leading_divider(*item) {
                rows.push(
                    div()
                        .mx_2()
                        .my_1()
                        .border_t(border_w)
                        .border_color(tokens.border)
                        .into_any_element(),
                );
            }
            let item = *item;
            let is_selected = index == selected;
            rows.push(
                div()
                    .id(SharedString::from(format!("terminal-menu-item-{index}")))
                    .px_3()
                    .py_1()
                    .text_sm()
                    .text_color(tokens.fg)
                    .cursor_pointer()
                    .when(is_selected, |el| el.bg(tokens.hover))
                    .hover(|el| el.bg(tokens.hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.run_terminal_menu_item(item, window, cx);
                        cx.stop_propagation();
                    }))
                    .child(menu_item_label(item))
                    .into_any_element(),
            );
        }

        deferred(
            div()
                .id("terminal-menu-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.terminal_menu = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                    this.terminal_menu = None;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| {
                    this.terminal_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("terminal-menu-panel")
                        .absolute()
                        .left(x)
                        .top(y)
                        .min_w(menu_w)
                        .py_1()
                        .rounded(px(0.0))
                        .border(border_w)
                        .border_color(tokens.border)
                        .bg(tokens.content_bg)
                        .shadow(pixel::hard_shadow())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                        .children(rows),
                ),
        )
    }
}
