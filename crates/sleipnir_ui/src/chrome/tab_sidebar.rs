//! Left tab rail: workspace headers + vertical tab rows.

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use run_ledger::Badge;
use sleipnir_settings::{TabPlacement, TerminalPalette, TerminalSettings};

use crate::app_shell::{AppShell, PaneDrag, Tab, TabDragPreview};
use crate::chrome::agent::{self, AgentKind};
use crate::chrome::git_status::cached_git_snapshot;
use crate::chrome::tab_strip::{render_tab_badge, tab_badge_for};
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
        for (group_ix, (key, indices)) in groups.into_iter().enumerate() {
            let header: gpui::SharedString = workspace_header_label(&key.name(), indices.len()).into();
            rows.push(
                div()
                    .id(("ws-header", group_ix))
                    .w_full()
                    .px(geo.tab_px)
                    .pt_2()
                    .pb_1()
                    .flex()
                    .flex_row()
                    .items_center()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg_muted)
                    .child(header)
                    .into_any_element(),
            );
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
                let (branch, added, deleted) = match tab.workspace_cwd(cx) {
                    Some(cwd) => cached_git_snapshot(&cwd)
                        .map(|snap| (Some(snap.branch), snap.added, snap.deleted))
                        .unwrap_or((None, 0, 0)),
                    None => (None, 0, 0),
                };
                rows.push(render_rail_row(
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
                    branch,
                    added,
                    deleted,
                    tokens,
                    geo,
                    &palette,
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

        self.attach_empty_drag("content-title", cx)
            .h(geo.content_title_height)
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .px_3()
            .min_w_0()
            .bg(tokens.content_bg)
            .text_sm()
            .text_color(tokens.fg_muted)
            .child(truncated_label(label))
    }
}

fn render_rail_row(
    tab: &Tab,
    ix: usize,
    is_active: bool,
    is_hovered: bool,
    is_bell: bool,
    rename_buffer: Option<String>,
    badge: Option<Badge>,
    agent: Option<AgentKind>,
    branch: Option<String>,
    added: u32,
    deleted: u32,
    tokens: &ChromeTokens,
    geo: &ChromeGeometry,
    palette: &TerminalPalette,
    cx: &mut Context<AppShell>,
) -> gpui::AnyElement {
    let title: gpui::SharedString = if let Some(buffer) = rename_buffer.as_ref() {
        format!("{buffer}|").into()
    } else {
        tab.title(cx)
    };
    let tab_id = tab.id;
    let is_renaming = rename_buffer.is_some();
    let bg = if is_bell {
        tokens.accent.opacity(0.35)
    } else if is_active {
        tokens.content_bg
    } else if is_hovered {
        tokens.hover
    } else {
        gpui::hsla(0.0, 0.0, 0.0, 0.0)
    };
    let fg = if is_active || is_bell || is_hovered {
        tokens.fg
    } else {
        tokens.fg_muted
    };
    let show_subtitle = branch.is_some() || added > 0 || deleted > 0;

    let title_line = div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .child(truncated_label(title.clone()))
        .when_some(badge, |el, badge| {
            el.child(render_tab_badge(badge, palette))
        });

    let body = div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .flex_col()
        .justify_center()
        .child(title_line)
        .when(show_subtitle, |el| {
            el.child(render_rail_subtitle(
                branch,
                added,
                deleted,
                tokens,
                palette,
            ))
        });

    div()
        .id(("tab-row", tab_id))
        .w_full()
        .h(geo.rail_row_height)
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
        .when(is_renaming, |el| el.border_1().border_color(tokens.accent))
        .when(is_bell, |el| el.border_1().border_color(tokens.accent))
        .on_hover(cx.listener(move |this, hovered, _, cx| {
            if *hovered {
                this.hovered_tab = Some(tab_id);
            } else if this.hovered_tab == Some(tab_id) {
                this.hovered_tab = None;
            }
            cx.notify();
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, _, cx| {
                this.begin_rename(tab_id, cx);
            }),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if this.rename.as_ref().is_some_and(|s| s.tab_id == tab_id) {
                return;
            }
            this.activate(ix, window, cx);
        }))
        .on_drag(tab_id, {
            let title = title.clone();
            move |_dragged: &u64, _offset, _window, cx| {
                let value = title.clone();
                cx.new(move |_| TabDragPreview { title: value })
            }
        })
        .on_drop::<u64>(cx.listener(move |this, dragged: &u64, window, cx| {
            let Some(from) = this.tabs.iter().find(|t| t.id == *dragged) else {
                return;
            };
            let Some(to) = this.tabs.iter().find(|t| t.id == tab_id) else {
                return;
            };
            let from_ws = WorkspaceKey::of(from.workspace_cwd(cx).as_deref());
            let to_ws = WorkspaceKey::of(to.workspace_cwd(cx).as_deref());
            if from_ws != to_ws {
                return;
            }
            this.reorder_tab(*dragged, tab_id, window, cx);
        }))
        .on_drop::<PaneDrag>(cx.listener(move |this, dragged: &PaneDrag, window, cx| {
            let insert_at = this.tabs.iter().position(|t| t.id == tab_id).unwrap_or(0);
            this.extract_pane_to_tab(dragged.pane_id, insert_at, window, cx);
        }))
        .when_some(agent, |el, kind| el.child(render_agent_mark(kind)))
        .when(
            agent.is_none() && TerminalSettings::get_global(cx).agent_icons,
            |el| el.child(render_shell_mark(tokens)),
        )
        .child(body)
        .into_any_element()
}

fn render_rail_subtitle(
    branch: Option<String>,
    added: u32,
    deleted: u32,
    tokens: &ChromeTokens,
    palette: &TerminalPalette,
) -> impl IntoElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(tokens.fg_muted)
        .when(branch.is_some(), |el| {
            el.child(
                div()
                    .flex_shrink_0()
                    .text_color(tokens.fg_muted)
                    .child("⎇"),
            )
        })
        .when_some(branch, |el, name| el.child(truncated_label(name)))
        .when(added > 0, |el| {
            el.child(
                div()
                    .flex_shrink_0()
                    .text_color(palette.ansi[2])
                    .child(gpui::SharedString::from(format!("+{added}"))),
            )
        })
        .when(deleted > 0, |el| {
            el.child(
                div()
                    .flex_shrink_0()
                    .text_color(palette.ansi[1])
                    .child(gpui::SharedString::from(format!("−{deleted}"))),
            )
        })
}

/// Title / branch text that shrinks inside the rail and paints `…`.
fn truncated_label(text: impl Into<gpui::SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

fn render_agent_mark(kind: AgentKind) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .w(px(16.0))
        .flex()
        .justify_center()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(kind.color)
        .child(kind.mark)
}

fn render_shell_mark(tokens: &ChromeTokens) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .w(px(16.0))
        .flex()
        .justify_center()
        .text_xs()
        .text_color(tokens.fg_muted)
        .child("⌂")
}

/// Whether the window should use the side rail (vs the top strip).
pub(crate) fn is_side_placement(cx: &App) -> bool {
    TerminalSettings::get_global(cx).tab_placement == TabPlacement::Side
}

/// Workspace header: directory basename plus the tab count in that group.
fn workspace_header_label(name: &str, count: usize) -> String {
    format!("{name} {count}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_header_includes_count() {
        assert_eq!(workspace_header_label("harbor", 1), "harbor 1");
        assert_eq!(workspace_header_label("TTY7", 3), "TTY7 3");
    }
}
