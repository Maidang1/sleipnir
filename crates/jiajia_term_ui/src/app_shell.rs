//! Single-window multi-tab shell for jiajia-term (M3).

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, actions, div,
};
use jiajia_settings::{TerminalPalette, TerminalSettings, ThemeName};

use crate::TermView;

actions!(
    jiajia_term,
    [
        /// Open a new terminal tab.
        NewTab,
        /// Close the active terminal tab.
        CloseTab,
        /// Activate the next tab.
        NextTab,
        /// Activate the previous tab.
        PrevTab,
        /// Reload `~/.config/jiajia-term/settings.json`.
        ReloadSettings,
        /// Cycle built-in theme (mocha → macchiato → frappe → latte).
        CycleTheme,
    ]
);

struct Tab {
    id: u64,
    view: Entity<TermView>,
}

/// Window root: tab bar + active terminal.
pub struct AppShell {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    focus_handle: FocusHandle,
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
        };
        shell.add_tab(window, cx);
        shell
    }

    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let shell = cx.weak_entity();
        let view = cx.new(|cx| TermView::new_local_in_shell(Some(shell), window, cx));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&view, |_, _, _: &crate::TermViewEvent, cx| {
            cx.notify();
        })
        .detach();
        self.tabs.push(Tab { id, view });
        self.active = self.tabs.len() - 1;
        self.focus_active(window, cx);
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

    fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = self.active.min(self.tabs.len() - 1);
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            // Always keep at least one tab.
            self.add_tab(window, cx);
            return;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        self.focus_active(window, cx);
        cx.notify();
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active = index;
            self.focus_active(window, cx);
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
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let active = self.active;
        let bar_bg = palette.background.opacity(0.92);
        let border = palette.ansi[8];
        let muted = palette.ansi[8];
        let active_bg = palette.selection.opacity(0.55);
        let active_fg = palette.foreground;
        let inactive_fg = muted;

        let tab_bar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .bg(bar_bg)
            .border_b_1()
            .border_color(border)
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let title: SharedString = tab.view.read(cx).title().into();
                let is_active = ix == active;
                let bg = if is_active { active_bg } else { bar_bg };
                let fg = if is_active { active_fg } else { inactive_fg };
                div()
                    .id(("tab", tab.id))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(bg)
                    .text_color(fg)
                    .text_sm()
                    .cursor_pointer()
                    .child(title)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate(ix, window, cx);
                    }))
            }))
            .child(
                div()
                    .id("new-tab")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_color(inactive_fg)
                    .text_sm()
                    .cursor_pointer()
                    .child("+")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_tab(window, cx);
                    })),
            );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.background)
            .track_focus(&self.focus_handle)
            .key_context("AppShell")
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_reload_settings))
            .on_action(cx.listener(Self::on_cycle_theme))
            .child(tab_bar)
            .child(
                div()
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
