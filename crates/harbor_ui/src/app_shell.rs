//! Single-window multi-tab shell for harbor (HIG-aligned chrome).

use gpui::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    actions, div, px, prelude::FluentBuilder as _,
};
use harbor_settings::{TerminalPalette, TerminalSettings, ThemeName};

use crate::TermView;
use crate::chrome::{ChromeGeometry, ChromeTokens, active_after_close};

actions!(
    harbor,
    [
        /// Open a new terminal tab.
        NewTab,
        /// Close the active terminal tab.
        CloseTab,
        /// Activate the next tab.
        NextTab,
        /// Activate the previous tab.
        PrevTab,
        /// Reload `~/.config/harbor/settings.json`.
        ReloadSettings,
        /// Cycle built-in theme (mocha → macchiato → frappe → latte).
        CycleTheme,
    ]
);

struct Tab {
    id: u64,
    view: Entity<TermView>,
}

/// Window root: unified chrome band + active terminal.
pub struct AppShell {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    focus_handle: FocusHandle,
    /// Empty-region drag: true after mouse-down on a drag strip until move/up.
    should_move: bool,
    tab_scroll_handle: ScrollHandle,
    /// Tab id currently under the pointer (for hover close / hover fill).
    hovered_tab: Option<u64>,
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
            focus_handle: cx.focus_handle(),
            should_move: false,
            tab_scroll_handle: ScrollHandle::new(),
            hovered_tab: None,
        };
        shell.add_tab(window, cx);
        shell
    }

    fn sync_window_title(&self, window: &mut Window, cx: &App) {
        let title = self
            .tabs
            .get(self.active)
            .map(|t| t.view.read(cx).title())
            .unwrap_or("Harbor");
        window.set_window_title(title);
    }

    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
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
        self.tabs.push(Tab { id, view });
        self.active = self.tabs.len() - 1;
        self.focus_active(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        cx.notify();
    }

    pub(crate) fn add_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(window, cx);
    }

    pub(crate) fn close_active_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_active_tab(window, cx);
    }

    pub(crate) fn next_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.next_tab(window, cx);
    }

    pub(crate) fn prev_tab_public(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prev_tab(window, cx);
    }

    fn close_tab_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
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
        if let Some(tab) = self.tabs.get(self.active) {
            let handle = tab.view.focus_handle(cx);
            window.focus(&handle, cx);
        }
    }

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(window, cx);
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_active_tab(window, cx);
    }

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.next_tab(window, cx);
    }

    fn on_prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        self.prev_tab(window, cx);
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
        let mut settings = TerminalSettings::get_global(cx).clone();
        settings.theme = match settings.theme {
            ThemeName::Mocha => ThemeName::Macchiato,
            ThemeName::Macchiato => ThemeName::Frappe,
            ThemeName::Frappe => ThemeName::Latte,
            ThemeName::Latte => ThemeName::Mocha,
        };
        log::info!("theme -> {:?}", settings.theme);
        TerminalSettings::apply(settings, cx);
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
                let title: SharedString = tab.view.read(cx).title().into();
                let is_active = ix == active;
                let is_hovered = hovered == Some(tab.id);
                let show_close = is_active || is_hovered;
                let tab_id = tab.id;

                let bg = if is_active {
                    tokens.active_tab_bg()
                } else if is_hovered {
                    tokens.hover
                } else {
                    // Transparent over surface
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
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered {
                            this.hovered_tab = Some(tab_id);
                        } else if this.hovered_tab == Some(tab_id) {
                            this.hovered_tab = None;
                        }
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate(ix, window, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(title),
                    )
                    .when(show_close, |el| {
                        el.child(
                            div()
                                .id(("tab-close", tab_id))
                                .size(geo.close_hit)
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .text_color(tokens.fg_muted)
                                .hover(|s| s.bg(tokens.hover).text_color(tokens.fg))
                                .cursor_pointer()
                                .child("×")
                                .on_mouse_down(MouseButton::Left, |_, _, _| {
                                    // Absorb so empty-region drag never sees this.
                                })
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    // Stop activate on the parent tab.
                                    cx.stop_propagation();
                                    this.close_tab_at(ix, window, cx);
                                })),
                        )
                    })
            }));

        let new_tab = div()
            .id("new-tab")
            .size(geo.new_tab_hit)
            .flex()
            .items_center()
            .justify_center()
            .rounded(geo.tab_radius)
            .text_color(tokens.fg_muted)
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(tokens.hover).text_color(tokens.fg))
            .child("+")
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .on_click(cx.listener(|this, _, window, cx| {
                this.add_tab(window, cx);
            }));

        let chrome_band = div()
            .id("chrome-band")
            .h(geo.height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .bg(tokens.surface)
            .border_b_1()
            .border_color(tokens.border)
            .child(leading_drag)
            .child(tab_scroll)
            .child(trailing_drag)
            .child(new_tab);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(tokens.content_bg)
            .track_focus(&self.focus_handle)
            .key_context("AppShell")
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_reload_settings))
            .on_action(cx.listener(Self::on_cycle_theme))
            .child(chrome_band)
            .child(
                div()
                    .flex_1()
                    .size_full()
                    .min_h_0()
                    .child(if let Some(tab) = self.tabs.get(active) {
                        tab.view.clone().into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
    }
}
