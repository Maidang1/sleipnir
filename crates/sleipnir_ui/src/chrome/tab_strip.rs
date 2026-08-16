//! Tab strip / shared tab-chip rendering.

use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use run_ledger::Badge;
use sleipnir_settings::{TerminalPalette, TerminalSettings};

use crate::app_shell::{AppShell, PaneDrag, Tab, TabDragPreview};
use crate::chrome::agent::{self, AgentKind};
use crate::chrome::git_status::cached_git_snapshot;
use crate::chrome::workspace::{WorkspaceKey, group_tabs};
use crate::chrome::{ChromeGeometry, ChromeTokens};
use crate::run_ledger_global::RunLedgerGlobal;

/// Side rail vs top strip: same chip, different sizing and active fill.
#[derive(Clone, Copy)]
pub(crate) enum TabChipLayout {
    Rail,
    Strip,
}

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
                chips.push(render_tab_chip(
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
                    TabChipLayout::Strip,
                    cx,
                ));
            }
        }

        let plus = div()
            .id("strip-new-tab")
            .h(geo.tab_height)
            .min_w(geo.new_tab_hit)
            .px(geo.tab_px)
            .rounded(geo.tab_radius)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .text_color(tokens.fg_muted)
            .text_sm()
            .cursor_pointer()
            .hover(|el| el.bg(tokens.hover).text_color(tokens.fg))
            .child("+")
            .on_click(cx.listener(|this, _, window, cx| {
                this.add_tab(window, cx);
            }))
            .on_drop::<PaneDrag>(cx.listener(|this, dragged: &PaneDrag, window, cx| {
                let insert_at = this.tabs.len();
                this.extract_pane_to_tab(dragged.pane_id, insert_at, window, cx);
            }));

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
            .child(plus)
    }
}

pub(crate) fn render_tab_chip(
    tab: &Tab,
    ix: usize,
    is_active: bool,
    is_hovered: bool,
    is_bell: bool,
    rename_buffer: Option<String>,
    badge: Option<Badge>,
    agent: Option<AgentKind>,
    tokens: &ChromeTokens,
    geo: &ChromeGeometry,
    palette: &TerminalPalette,
    layout: TabChipLayout,
    cx: &mut Context<AppShell>,
) -> gpui::AnyElement {
    let title: gpui::SharedString = if let Some(buffer) = rename_buffer.as_ref() {
        format!("{buffer}|").into()
    } else {
        match layout {
            TabChipLayout::Rail => tab.title(cx),
            TabChipLayout::Strip => tab.path_label(cx),
        }
    };
    let tab_id = tab.id;
    let is_renaming = rename_buffer.is_some();
    let failed = tab_has_failed_attention(badge);
    let bg = chip_background(
        layout,
        is_active,
        is_hovered,
        is_bell,
        failed,
        tokens,
        palette,
    );
    let fg = if is_active || is_bell || is_hovered || failed {
        tokens.fg
    } else {
        tokens.fg_muted
    };
    let (branch, added, deleted) = match layout {
        TabChipLayout::Rail => tab_git_facts(tab, cx),
        TabChipLayout::Strip => (None, 0, 0),
    };
    let show_subtitle = matches!(layout, TabChipLayout::Rail)
        && (branch.is_some() || added > 0 || deleted > 0);
    let element_id = match layout {
        TabChipLayout::Rail => ("tab-row", tab_id),
        TabChipLayout::Strip => ("tab", tab_id),
    };
    let chip_height = match layout {
        TabChipLayout::Rail => geo.rail_row_height,
        TabChipLayout::Strip => geo.tab_height,
    };

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
        )
        .when(show_subtitle, |el| {
            el.child(render_git_subtitle(
                ix,
                tab_id,
                branch,
                added,
                deleted,
                tokens,
                palette,
                cx,
            ))
        });

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
    let chip = match layout {
        TabChipLayout::Rail => chip.w_full(),
        TabChipLayout::Strip => chip.min_w(geo.tab_min_width).max_w(geo.tab_max_width),
    };

    chip.on_hover(cx.listener(move |this, hovered, _, cx| {
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
    .child(body)
    .into_any_element()
}

pub(crate) fn tab_git_facts(tab: &Tab, cx: &App) -> (Option<String>, u32, u32) {
    match tab.workspace_cwd(cx) {
        Some(cwd) => cached_git_snapshot(&cwd)
            .map(|snap| (Some(snap.branch), snap.added, snap.deleted))
            .unwrap_or((None, 0, 0)),
        None => (None, 0, 0),
    }
}

fn render_git_subtitle(
    ix: usize,
    tab_id: u64,
    branch: Option<String>,
    added: u32,
    deleted: u32,
    tokens: &ChromeTokens,
    palette: &TerminalPalette,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let dirty = added > 0 || deleted > 0;
    div()
        .id(("tab-git", tab_id))
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(tokens.fg_muted)
        .when(dirty, |el| {
            el.cursor_pointer()
                .hover(|style| style.text_color(tokens.fg))
                .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    let _ = event;
                    cx.stop_propagation();
                    this.activate(ix, window, cx);
                    if !this.diff_open {
                        this.toggle_diff(window, cx);
                    } else {
                        this.refresh_diff(true, window, cx);
                    }
                }))
        })
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

/// Title / path / branch text that shrinks inside the chip and paints `…`.
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
    layout: TabChipLayout,
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
        return match layout {
            // Rail selected connects to the content column.
            TabChipLayout::Rail => tokens.content_bg,
            // Strip sits on the titlebar: a solid content_bg card reads too
            // bright / too tall. Use a whisper of foreground instead.
            TabChipLayout::Strip => tokens.fg.opacity(0.06),
        };
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
        let bg = chip_background(
            TabChipLayout::Strip,
            true,
            false,
            false,
            false,
            &tokens,
            &palette,
        );
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
        let bg = chip_background(
            TabChipLayout::Strip,
            false,
            false,
            false,
            true,
            &tokens,
            &palette,
        );
        assert!((bg.a - 0.12).abs() < 1e-5);
        assert_eq!(bg.h, palette.ansi[1].h);
    }
}
