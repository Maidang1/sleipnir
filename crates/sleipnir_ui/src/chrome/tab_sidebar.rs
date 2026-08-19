//! Left tab rail: workspace-grouped vertical tab rows.

use gpui::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use sleipnir_settings::{TabPlacement, TerminalPalette, TerminalSettings};

use crate::app_shell::{AppShell, PaneDrag};
use crate::chrome::agent;
use crate::chrome::tab_strip::{render_tab_chip, tab_badge_for, tab_git_facts, TabChipLayout};
use crate::chrome::workspace::{WorkspaceKey, group_tabs};
use crate::chrome::{ChromeGeometry, ChromeTokens};

impl AppShell {
    pub(crate) fn render_tab_sidebar(
        &self,
        tokens: &ChromeTokens,
        geo: &ChromeGeometry,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let settings = TerminalSettings::get_global(cx);
        let show_icons = settings.agent_icons;
        let active = self.active;
        let hovered = self.hovered_tab;

        let keys: Vec<WorkspaceKey> = self
            .tabs
            .iter()
            .map(|tab| WorkspaceKey::of(tab.workspace_cwd(cx).as_deref()))
            .collect();
        let groups = group_tabs(keys.into_iter().enumerate());

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (_key, indices) in groups {
            for ix in indices {
                let Some(tab) = self.tabs.get(ix) else {
                    continue;
                };
                let badge = tab_badge_for(tab, cx);
                let agent = if show_icons {
                    agent::identify_tab(tab, cx)
                } else {
                    None
                };
                rows.push(render_tab_chip(
                    tab,
                    ix,
                    ix == active,
                    hovered == Some(tab.id),
                    self.bell_flash_tabs.contains(&tab.id),
                    self.rename
                        .as_ref()
                        .filter(|state| state.tab_id == tab.id)
                        .map(|state| state.buffer.clone()),
                    badge,
                    agent,
                    tokens,
                    geo,
                    &palette,
                    TabChipLayout::Rail,
                    cx,
                ));
            }
        }

        let header = self
            .attach_empty_drag("sidebar-header", cx)
            .h(geo.sidebar_header)
            .w_full()
            .flex_shrink_0();

        let plus = div()
            .id("sidebar-new-tab")
            .h(geo.new_tab_hit)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .px(geo.tab_px)
            .flex_shrink_0()
            .text_color(tokens.fg_muted)
            .text_sm()
            .cursor_pointer()
            .hover(|el| el.bg(tokens.hover).text_color(tokens.fg))
            .child("+  New tab")
            .on_click(cx.listener(|this, _, window, cx| {
                this.add_tab(window, cx);
            }))
            .on_drop::<PaneDrag>(cx.listener(|this, dragged: &PaneDrag, window, cx| {
                let insert_at = this.tabs.len();
                this.extract_pane_to_tab(dragged.pane_id, insert_at, window, cx);
            }));

        div()
            .id("tab-sidebar")
            .w(geo.sidebar_width)
            .h_full()
            .flex()
            .flex_col()
            .bg(tokens.surface)
            .border_r_1()
            .border_color(tokens.border)
            .child(header)
            .child(
                div()
                    .id("tab-sidebar-scroller")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px_1()
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .track_scroll(&self.tab_scroll_handle)
                    .children(rows),
            )
            .child(plus)
    }

    pub(crate) fn render_content_title(
        &self,
        tokens: &ChromeTokens,
        geo: &ChromeGeometry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab = self.tabs.get(self.active);
        let workspace = tab
            .map(|tab| WorkspaceKey::of(tab.workspace_cwd(cx).as_deref()).name())
            .unwrap_or_else(|| "~".into());
        let title = tab
            .map(|tab| tab.title(cx).to_string())
            .unwrap_or_else(|| "shell".into());
        let label: gpui::SharedString = format!("{workspace} · {title}").into();
        let palette = TerminalPalette::get_global(cx);

        div()
            .id("content-title-row")
            .h(geo.content_title_height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .min_w_0()
            .bg(tokens.content_bg)
            .child(
                self.attach_empty_drag("content-title", cx)
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_3()
                    .text_sm()
                    .text_color(tokens.fg_muted)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(self.render_diff_chrome_button(tokens, &palette, cx)),
            )
            .child(self.render_windows_titlebar_end(tokens, window, cx))
    }

    /// Always-visible chrome control that opens the diff inspector.
    pub(crate) fn render_diff_chrome_button(
        &self,
        tokens: &ChromeTokens,
        palette: &TerminalPalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (_, added, deleted) = self
            .tabs
            .get(self.active)
            .map(|tab| tab_git_facts(tab, cx))
            .unwrap_or((None, 0, 0));
        let open = self.diff_open;
        div()
            .id("diff-chrome-button")
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .px_2()
            .py_0p5()
            .rounded(px(4.0))
            .text_xs()
            .cursor_pointer()
            .when(open, |el| {
                el.bg(tokens.accent.opacity(0.2)).text_color(tokens.fg)
            })
            .when(!open, |el| {
                el.text_color(tokens.fg_muted)
                    .hover(|style| style.bg(tokens.hover).text_color(tokens.fg))
            })
            .child(gpui::SharedString::from("Diff"))
            .when(added > 0, |el| {
                el.child(
                    div()
                        .text_color(palette.ansi[2])
                        .child(gpui::SharedString::from(format!("+{added}"))),
                )
            })
            .when(deleted > 0, |el| {
                el.child(
                    div()
                        .text_color(palette.ansi[1])
                        .child(gpui::SharedString::from(format!("−{deleted}"))),
                )
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_diff(window, cx);
            }))
    }
}

/// Whether the window should use the side rail (vs the top strip).
pub(crate) fn is_side_placement(cx: &App) -> bool {
    TerminalSettings::get_global(cx).tab_placement == TabPlacement::Side
}
