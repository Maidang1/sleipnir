//! Tab strip rendering extracted from `AppShell::render`.

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use run_ledger::{Badge, BadgeKind};
use sleipnir_settings::TerminalPalette;

use crate::app_shell::{AppShell, TabDragPreview};
use crate::chrome::{ChromeGeometry, ChromeTokens};
use crate::chrome::tab_badge::{badge_color, badge_label, format_elapsed};
use crate::run_ledger_global::RunLedgerGlobal;

impl AppShell {
    pub(crate) fn render_tab_strip(
        &self,
        tokens: &ChromeTokens,
        geo: &ChromeGeometry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let active = self.active;
        let hovered = self.hovered_tab;
        let badges: Vec<Option<Badge>> = self.tabs.iter().map(|tab| tab_badge_for(tab, cx)).collect();
        if badges
            .iter()
            .any(|b| matches!(b, Some(Badge { kind: BadgeKind::Running, .. })))
        {
            window.request_animation_frame();
        }

        let tabs = self.tabs.iter().zip(badges).enumerate().map(|(ix, (tab, badge))| {
            let title: gpui::SharedString = tab.title(cx);
            let is_active = ix == active;
            let is_hovered = hovered == Some(tab.id);
            let tab_id = tab.id;
            let rename_buffer = self
                .rename
                .as_ref()
                .filter(|s| s.tab_id == tab_id)
                .map(|s| s.buffer.clone());
            let is_renaming = rename_buffer.is_some();

            let is_bell = self.bell_flash_tabs.contains(&tab_id);
            let bg = if is_bell {
                tokens.accent.opacity(0.35)
            } else if is_active {
                tokens.active_tab_bg()
            } else if is_hovered {
                tokens.hover
            } else {
                // Transparent over the chrome band
                gpui::hsla(0.0, 0.0, 0.0, 0.0)
            };
            let fg = if is_active || is_bell || is_hovered {
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
                .when(is_bell, |el| el.border_1().border_color(tokens.accent))
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
                // Drag a tab to reorder it (drop before another tab).
                .on_drag(tab_id, {
                    let title = title.clone();
                    move |_dragged: &u64, _offset, _window, cx| {
                        let value = title.clone();
                        cx.new(move |_| TabDragPreview { title: value })
                    }
                })
                .on_drop::<u64>(cx.listener(move |this, dragged: &u64, window, cx| {
                    this.reorder_tab(*dragged, tab_id, window, cx);
                }))
                .when_some(badge, |el, badge| el.child(render_tab_badge(badge, &palette)))
                .child(if let Some(buffer) = rename_buffer {
                    let text: gpui::SharedString = format!("{buffer}|").into();
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
        });

        div()
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
            .children(tabs)
    }
}

fn tab_badge_for(tab: &crate::app_shell::Tab, cx: &App) -> Option<Badge> {
    let ledger = cx.try_global::<RunLedgerGlobal>()?;
    let keys = tab.tree.all_pane_keys();
    ledger.badge_for(&keys, ledger.now_ms())
}

fn render_tab_badge(badge: Badge, palette: &TerminalPalette) -> impl IntoElement {
    let color = badge_color(badge.kind, palette);
    let label = badge_label(badge.kind, badge.count);
    // Fixed slot so the title does not jump as elapsed ticks or the count changes.
    let slot = if badge.kind == BadgeKind::Running {
        px(52.0)
    } else {
        px(18.0)
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .flex_shrink_0()
        .min_w(slot)
        .text_color(color)
        .text_xs()
        .child(label)
        .when(badge.kind == BadgeKind::Running, |el| {
            el.child(
                div()
                    .whitespace_nowrap()
                    .child(format_elapsed(badge.elapsed_ms)),
            )
        })
}
