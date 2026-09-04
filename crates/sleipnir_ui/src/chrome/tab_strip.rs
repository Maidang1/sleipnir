//! Tab strip / shared tab-chip rendering.

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    deferred, div, prelude::FluentBuilder as _, px, svg,
};
use run_ledger::Badge;
use sleipnir_settings::{TerminalPalette, TerminalSettings};

use crate::app_shell::{AppShell, PaneDrag, Tab, TabDragPreview, TabMenuState};
use crate::chrome::agent::{self, AgentKind};
use crate::chrome::workspace::{WorkspaceKey, group_tabs};
use crate::chrome::{ChromeGeometry, ChromeTokens};
use crate::run_ledger_global::RunLedgerGlobal;

impl AppShell {
    pub(crate) fn render_tab_strip(
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

        let mut chips: Vec<gpui::AnyElement> = Vec::new();
        // Drop-target bar is only meaningful while a drag is actually live;
        // stale state from an aborted drag must never paint.
        let drop_target = if cx.has_active_drag() {
            self.tab_drop_target
        } else {
            None
        };
        for (_key, indices) in groups {
            for ix in indices {
                let Some(tab) = self.tabs.get(ix) else {
                    continue;
                };
                let badge = tab_badge_for(tab, cx);
                let keys = tab.tree.all_pane_keys();
                let plugin_badges = self.plugin_badges_for_tab(&keys, ix == active);
                // The Failed wash is the ledger's own verdict; plugin badges
                // can never set or suppress it.
                let failed = tab_has_failed_attention(badge);
                let agent = if show_icons {
                    agent::identify_tab(tab, cx)
                } else {
                    None
                };
                chips.push(render_tab_chip(
                    tab,
                    ix,
                    ix == active,
                    hovered == Some(tab.id),
                    drop_target == Some(tab.id),
                    self.bell_flash_tabs.contains(&tab.id),
                    self.rename
                        .as_ref()
                        .filter(|state| state.tab_id == tab.id)
                        .map(|state| state.buffer.clone()),
                    badge,
                    plugin_badges,
                    failed,
                    agent,
                    tokens,
                    geo,
                    &palette,
                    cx,
                ));
            }
        }

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
            .children(chips)
    }
}

pub(crate) struct TabPathPreview {
    text: SharedString,
}

impl Render for TabPathPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = TerminalPalette::get_global(cx);
        let tokens = ChromeTokens::from_palette(&palette, window.is_window_active());
        div()
            .px_3()
            .py_1()
            .rounded(px(6.0))
            .bg(tokens.hover)
            .border_1()
            .border_color(tokens.border)
            .shadow_lg()
            .text_sm()
            .text_color(tokens.fg)
            .child(self.text.clone())
    }
}

pub(crate) fn render_tab_chip(
    tab: &Tab,
    ix: usize,
    is_active: bool,
    is_hovered: bool,
    is_drop_target: bool,
    is_bell: bool,
    rename_buffer: Option<String>,
    _badge: Option<Badge>,
    plugin_badges: Vec<crate::plugin_chrome::PluginTabBadge>,
    failed: bool,
    agent: Option<AgentKind>,
    tokens: &ChromeTokens,
    geo: &ChromeGeometry,
    palette: &TerminalPalette,
    cx: &mut Context<AppShell>,
) -> gpui::AnyElement {
    let title: gpui::SharedString = if let Some(buffer) = rename_buffer.as_ref() {
        format!("{buffer}|").into()
    } else {
        tab.path_label(cx)
    };
    let tab_id = tab.id;
    let path: SharedString = tab
        .workspace_cwd(cx)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~".to_string())
        .into();
    let is_renaming = rename_buffer.is_some();
    let bg = chip_background(is_active, is_hovered, is_bell, failed, tokens, palette);
    let fg = if is_active || is_bell || is_hovered || failed {
        tokens.fg
    } else {
        tokens.fg_muted
    };
    let element_id = ("tab", tab_id);
    let chip_height = geo.tab_height;

    let body = div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .flex()
        .flex_col()
        .justify_center()
        .child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_row()
                .items_center()
                .child(truncated_label(title.clone())),
        );

    let chip = div()
        .id(element_id)
        .h(chip_height)
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
        .when(is_bell, |el| el.border_1().border_color(tokens.accent));
    let chip = chip.min_w(geo.tab_min_width).max_w(geo.tab_max_width);

    let chip = chip
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
            cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                this.tab_menu = Some(TabMenuState {
                    tab_id,
                    position: event.position,
                });
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _, window, cx| {
                this.request_close_tab(tab_id, window, cx);
                cx.stop_propagation();
            }),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if this.rename.as_ref().is_some_and(|s| s.tab_id == tab_id) {
                return;
            }
            this.activate(ix, window, cx);
        }))
        .on_drag(tab_id, {
            move |_dragged: &u64, _offset, _window, cx| {
                let value = title.clone();
                cx.new(move |_| TabDragPreview { title: value })
            }
        })
        .on_drop::<u64>(cx.listener(move |this, dragged: &u64, window, cx| {
            this.tab_drop_target = None;
            this.reorder_tab(*dragged, tab_id, window, cx);
        }))
        .on_drop::<PaneDrag>(cx.listener(move |this, dragged: &PaneDrag, window, cx| {
            this.tab_drop_target = None;
            let insert_at = this.tabs.iter().position(|t| t.id == tab_id).unwrap_or(0);
            this.extract_pane_to_tab(dragged.pane_id, insert_at, window, cx);
        }))
        .on_drag_move::<u64>(cx.listener(move |this, _, _, cx| {
            if this.tab_drop_target != Some(tab_id) {
                this.tab_drop_target = Some(tab_id);
                cx.notify();
            }
        }))
        .on_drag_move::<PaneDrag>(cx.listener(move |this, _, _, cx| {
            if this.tab_drop_target != Some(tab_id) {
                this.tab_drop_target = Some(tab_id);
                cx.notify();
            }
        }))
        .when(is_drop_target, |el| {
            // Insertion bar on the chip's leading edge.
            el.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(2.0))
                    .bg(tokens.accent),
            )
        })
        .when_some(agent, |el, kind| el.child(render_agent_mark(kind)))
        .child(body)
        .children(plugin_badges.into_iter().map(|b| {
            let color = match b.tone {
                plugin_protocol::v2::Tone::Ok => tokens.ok,
                plugin_protocol::v2::Tone::Warn => tokens.warn,
                plugin_protocol::v2::Tone::Err => tokens.err,
                plugin_protocol::v2::Tone::Accent => tokens.accent,
                plugin_protocol::v2::Tone::Dim => tokens.fg_muted,
                plugin_protocol::v2::Tone::Fg => tokens.fg,
            };
            div()
                .id(SharedString::from(format!(
                    "plugin-tab-badge-{tab_id}-{}",
                    b.plugin_id
                )))
                .flex_shrink_0()
                .px_1()
                .rounded(px(3.0))
                .bg(tokens.surface)
                .text_xs()
                .text_color(color)
                .child(format!("{}:{}", b.plugin_id, b.text))
        }))
        .when(is_hovered, |el| {
            el.child(
                div()
                    .id(("tab-close", tab_id))
                    .flex_shrink_0()
                    .px_1()
                    .rounded(px(3.0))
                    .text_xs()
                    .hover(|el| el.bg(tokens.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.request_close_tab(tab_id, window, cx);
                        cx.stop_propagation();
                    }))
                    .child("✕"),
            )
        });

    if is_renaming {
        chip.into_any_element()
    } else {
        chip.tooltip(move |_window, cx| {
            let text = path.clone();
            cx.new(move |_| TabPathPreview { text }).into()
        })
        .into_any_element()
    }
}

impl AppShell {
    pub(crate) fn render_tab_menu(
        &self,
        tokens: &ChromeTokens,
        _window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> impl IntoElement {
        let state = self.tab_menu.expect("tab menu state checked by caller");
        let row = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_3()
                .py_1()
                .text_sm()
                .text_color(tokens.fg)
                .cursor_pointer()
                .hover(|el| el.bg(tokens.hover))
                .child(label)
        };

        let close_menu = |this: &mut AppShell, cx: &mut Context<AppShell>| {
            this.tab_menu = None;
            cx.notify();
        };

        deferred(
            div()
                .id("tab-menu-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| close_menu(this, cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, _, cx| close_menu(this, cx)),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, _, cx| close_menu(this, cx)),
                )
                .child(
                    div()
                        .id("tab-menu-panel")
                        .absolute()
                        .left(state.position.x)
                        .top(state.position.y)
                        .min_w(px(160.0))
                        .py_1()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(tokens.border)
                        .bg(tokens.content_bg)
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                        .child(row("tab-menu-rename", "Rename Tab").on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.tab_menu = None;
                                this.begin_rename(state.tab_id, cx);
                                cx.stop_propagation();
                            },
                        )))
                        .child(
                            row("tab-menu-duplicate", "Duplicate Tab").on_click(cx.listener(
                                move |this, _, window, cx| {
                                    let cwd = this
                                        .tabs
                                        .iter()
                                        .find(|tab| tab.id == state.tab_id)
                                        .and_then(|tab| tab.workspace_cwd(cx));
                                    this.tab_menu = None;
                                    this.add_tab_at(cwd, window, cx);
                                    cx.stop_propagation();
                                },
                            )),
                        )
                        .child(row("tab-menu-close", "Close Tab").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.tab_menu = None;
                                this.request_close_tab(state.tab_id, window, cx);
                                cx.stop_propagation();
                            },
                        )))
                        .child(row("tab-menu-close-others", "Close Other Tabs").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.tab_menu = None;
                                for index in (0..this.tabs.len()).rev() {
                                    if this.tabs[index].id != state.tab_id {
                                        this.close_tab_at(index, window, cx);
                                    }
                                }
                                cx.stop_propagation();
                            }),
                        )),
                ),
        )
    }
}

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
    svg()
        .path(kind.icon)
        .flex_shrink_0()
        .w(px(12.0))
        .h(px(12.0))
        .text_color(kind.color)
}

pub(crate) fn tab_badge_for(tab: &crate::app_shell::Tab, cx: &App) -> Option<Badge> {
    let ledger = cx.try_global::<RunLedgerGlobal>()?;
    let keys = tab.tree.all_pane_keys();
    ledger.badge_for(&keys, ledger.now_ms())
}

/// Failed Attention is a faint red wash on the chip, not a glyph.
/// Running / succeeded never draw on the tab (no yellow dots).
pub(crate) fn tab_has_failed_attention(badge: Option<Badge>) -> bool {
    matches!(badge, Some(b) if b.kind == run_ledger::BadgeKind::Failed)
}

fn chip_background(
    is_active: bool,
    is_hovered: bool,
    is_bell: bool,
    failed: bool,
    tokens: &ChromeTokens,
    palette: &TerminalPalette,
) -> gpui::Hsla {
    if is_bell {
        return tokens.accent.opacity(0.35);
    }
    if failed {
        // Whole-tab wash. A bit stronger when the tab is selected.
        let amount = if is_active { 0.18 } else { 0.12 };
        return palette.ansi[1].opacity(amount);
    }
    if is_active {
        // Strip sits on the titlebar: a solid content_bg card reads too
        // bright / too tall. Use a whisper of foreground instead.
        return tokens.fg.opacity(0.06);
    }
    if is_hovered {
        return tokens.hover.opacity(0.55);
    }
    gpui::hsla(0.0, 0.0, 0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use run_ledger::BadgeKind;

    fn badge(kind: BadgeKind) -> Badge {
        Badge {
            kind,
            count: 1,
            elapsed_ms: 0,
        }
    }

    #[test]
    fn tab_chrome_washes_failed_and_hides_running_dots() {
        assert!(tab_has_failed_attention(Some(badge(BadgeKind::Failed))));
        assert!(!tab_has_failed_attention(Some(badge(BadgeKind::Running))));
        assert!(!tab_has_failed_attention(Some(badge(BadgeKind::Succeeded))));
        assert!(!tab_has_failed_attention(None));
    }

    #[test]
    fn strip_selected_fill_is_a_whisper() {
        use sleipnir_settings::{Appearance, ThemeName, palette_for_theme};
        let palette = palette_for_theme(ThemeName::Mocha, Appearance::Dark);
        let tokens = ChromeTokens::from_palette(&palette, true);
        let bg = chip_background(true, false, false, false, &tokens, &palette);
        assert!(
            bg.a < 0.12,
            "selected strip fill must stay faint, got alpha {}",
            bg.a
        );
    }

    #[test]
    fn failed_tab_is_a_red_wash() {
        use sleipnir_settings::{Appearance, ThemeName, palette_for_theme};
        let palette = palette_for_theme(ThemeName::Mocha, Appearance::Dark);
        let tokens = ChromeTokens::from_palette(&palette, true);
        let bg = chip_background(false, false, false, true, &tokens, &palette);
        assert!((bg.a - 0.12).abs() < 1e-5);
        assert_eq!(bg.h, palette.ansi[1].h);
    }
}
