//! The bottom-right agent HUD: a minimal, collapsible readout of which panes
//! run a known coding agent.
//!
//! A child module of `app_shell` so it can read the shell's private tab/pane
//! state without widening it to the crate.
//!
//! This is deliberately the *host's* smallest honest view: it reports only
//! process status ("this pane's foreground command is a known agent"), never
//! turn or task progress. It derives its rows the same way the tab chips do —
//! [`crate::chrome::agent::identify`] on each leaf's foreground command — so it
//! needs no plugin, no socket, and no protocol round-trip. The richer built-in
//! Agents panel (opened from the command palette) stays available for
//! coordination; this corner panel is just the always-on status glance the
//! user asked for. Clicking a row jumps to the pane that runs it.

use gpui::{
    ClickEvent, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, deferred, div, px, svg,
};

use super::AppShell;
use crate::chrome::ChromeTokens;
use crate::chrome::agent::{self};
use crate::pane_tree::PaneId;

/// One running agent, resolved to the tab/pane that hosts it. Pure data so the
/// row computation is unit-testable without a window.
#[derive(Clone, Debug)]
pub(crate) struct AgentHudRow {
    /// Index into `AppShell::tabs`, so a click can activate the right tab.
    pub tab_index: usize,
    /// The leaf that runs the agent, so the click focuses that exact pane.
    pub pane_id: PaneId,
    /// Agent identity from the catalog (`"claude"`, `"codex"`, …).
    pub agent_id: &'static str,
    /// Catalog icon path and mark color, shared with the tab chips.
    pub icon: &'static str,
    pub color: Hsla,
    /// Short label: the tab's path label, so several agents stay distinct.
    pub label: String,
}

impl AppShell {
    /// Every pane whose foreground command is a known coding agent, in tab then
    /// tree order. One row per agent pane.
    pub(crate) fn agent_hud_rows(&self, cx: &gpui::App) -> Vec<AgentHudRow> {
        let mut rows = Vec::new();
        for (tab_index, tab) in self.tabs.iter().enumerate() {
            let mut leaves = Vec::new();
            tab.tree.leaves(&mut leaves);
            let label = tab.path_label(cx).to_string();
            for (pane_id, view) in leaves {
                let Some(kind) = view
                    .read(cx)
                    .foreground_process_command_name(cx)
                    .as_deref()
                    .and_then(agent::identify)
                else {
                    continue;
                };
                rows.push(AgentHudRow {
                    tab_index,
                    pane_id,
                    agent_id: kind.id,
                    icon: kind.icon,
                    color: kind.color,
                    label: label.clone(),
                });
            }
        }
        rows
    }

    /// The minimal bottom-right HUD. Nothing renders when no agent is running,
    /// so the corner is empty in the common case. Collapsed shows one dot +
    /// count; expanded lists one clickable row per agent pane.
    pub(super) fn render_agent_hud(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let rows = self.agent_hud_rows(cx);
        if rows.is_empty() {
            return div().into_any_element();
        }
        let collapsed = self.agent_hud_collapsed;

        let header = div()
            .id("agent-hud-header")
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .cursor_pointer()
            .hover(|el| el.bg(tokens.hover))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_agent_hud(cx);
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.accent)
                    .child(if collapsed { "▸" } else { "▾" }),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg)
                    .child(format!("Agents · {}", rows.len())),
            );

        let mut panel = div()
            .id("agent-hud-panel")
            .flex()
            .flex_col()
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .shadow(crate::chrome::pixel::hard_shadow())
            .overflow_hidden()
            .child(header);

        if !collapsed {
            let mut list = div().flex().flex_col().pb_1();
            for row in rows {
                let tab_index = row.tab_index;
                let pane_id = row.pane_id;
                list = list.child(
                    div()
                        .id(("agent-hud-row", agent_hud_row_id(tab_index, pane_id)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .px_2()
                        .py_0p5()
                        .cursor_pointer()
                        .hover(|el| el.bg(tokens.hover))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.reveal_agent_pane(tab_index, pane_id, window, cx);
                        }))
                        .child(
                            svg()
                                .path(row.icon)
                                .flex_shrink_0()
                                .w(px(11.0))
                                .h(px(11.0))
                                .text_color(row.color),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.fg)
                                .whitespace_nowrap()
                                .child(row.agent_id.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_xs()
                                .text_color(tokens.fg_muted)
                                .child(row.label),
                        ),
                );
            }
            panel = panel.child(list);
        }

        // Deferred so the corner panel paints above the terminal content but
        // stays a sibling (never an ancestor) of the panes underneath.
        deferred(
            div()
                .absolute()
                .bottom(px(12.0))
                .right(px(12.0))
                .flex()
                .justify_end()
                .child(panel.min_w(px(160.0)).max_w(px(280.0)).occlude()),
        )
        .into_any_element()
    }

    /// Flip the HUD between the one-line summary and the full row list.
    pub(super) fn toggle_agent_hud(&mut self, cx: &mut Context<Self>) {
        self.agent_hud_collapsed = !self.agent_hud_collapsed;
        cx.notify();
    }

    /// Activate the tab that owns `pane_id` and focus that leaf.
    fn reveal_agent_pane(
        &mut self,
        tab_index: usize,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tab_index >= self.tabs.len() {
            return;
        }
        self.activate(tab_index, window, cx);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.tree.pane_key_for_id(pane_id).is_some() {
                tab.active_pane = pane_id;
            }
        }
        self.focus_active(window, cx);
        cx.notify();
    }
}

/// Stable element id for a HUD row: tab and pane ids are both small and unique
/// within a window, so packing them keeps rows distinct across re-renders.
fn agent_hud_row_id(tab_index: usize, pane_id: PaneId) -> u64 {
    ((tab_index as u64) << 32) | (pane_id as u64 & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_ids_are_unique_per_tab_and_pane() {
        assert_ne!(agent_hud_row_id(0, 1), agent_hud_row_id(1, 1));
        assert_ne!(agent_hud_row_id(0, 1), agent_hud_row_id(0, 2));
        assert_eq!(agent_hud_row_id(2, 5), agent_hud_row_id(2, 5));
    }
}
