//! Right-side agent status panel: a squeezed sidebar that shows running
//! agents with their coordination status, driven by the plugin hook data.
//!
//! When `plugins.agent_panel` is true and at least one agent is running,
//! the panel is rendered as a fixed-width column on the right side of the
//! content area (squeezing the terminal panes). When no agents are
//! running the panel disappears and the terminal reclaims the full width.
//!
//! Process identity comes from `chrome::agent::identify` on each leaf's
//! foreground command. Running status comes from the Run Ledger: if the
//! pane has a `Running` run, the agent is running; otherwise it has
//! exited or its status is unknown.

use gpui::{
    ClickEvent, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, svg,
};

use super::AppShell;
use crate::chrome::ChromeTokens;
use crate::chrome::agent::{self};
use crate::pane_tree::PaneId;
use crate::run_ledger_global::RunLedgerGlobal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentRunStatus {
    Running,
    Exited,
    Unknown,
}

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
    /// Running status derived from the Run Ledger.
    pub status: AgentRunStatus,
}

impl AppShell {
    /// Every pane whose foreground command is a known coding agent, in tab then
    /// tree order. One row per agent pane.
    pub(crate) fn agent_hud_rows(&self, cx: &gpui::App) -> Vec<AgentHudRow> {
        let ledger = cx.try_global::<RunLedgerGlobal>();
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
                let pane_key = tab.tree.pane_key_for_id(pane_id);
                let status = match (ledger, pane_key) {
                    (Some(lg), Some(pk)) => {
                        let runs: Vec<_> = lg.snapshot();
                        let pane_runs: Vec<_> = runs
                            .iter()
                            .filter(|r| r.pane == pk)
                            .collect();
                        if pane_runs.iter().any(|r| r.state == run_ledger::RunState::Running) {
                            AgentRunStatus::Running
                        } else if pane_runs.iter().any(|r| r.state.is_finished()) {
                            AgentRunStatus::Exited
                        } else {
                            AgentRunStatus::Unknown
                        }
                    }
                    _ => AgentRunStatus::Unknown,
                };
                rows.push(AgentHudRow {
                    tab_index,
                    pane_id,
                    agent_id: kind.id,
                    icon: kind.icon,
                    color: kind.color,
                    label: label.clone(),
                    status,
                });
            }
        }
        rows
    }

    /// The right-side squeezed agent panel. Renders a fixed-width column with
    /// a header and one clickable row per agent pane.
    pub(super) fn render_agent_panel(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let rows = self.agent_hud_rows(cx);
        if rows.is_empty() {
            return div().into_any_element();
        }

        let border_w = crate::chrome::pixel::PIXEL_BORDER;

        let running_count = rows
            .iter()
            .filter(|r| r.status == AgentRunStatus::Running)
            .count();
        let header_label = if running_count > 0 && running_count < rows.len() {
            format!("Agents · {} ({} running)", rows.len(), running_count)
        } else {
            format!("Agents · {}", rows.len())
        };

        let header = div()
            .id("agent-panel-header")
            .flex()
            .flex_row()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1p5()
            .border_b(border_w)
            .border_color(tokens.border)
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg)
                    .child(header_label),
            );

        let mut list = div()
            .id("agent-panel-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .py_1()
            .overflow_y_scroll();
        for row in rows {
            let tab_index = row.tab_index;
            let pane_id = row.pane_id;

            let (status_dot, status_color, status_label) = match row.status {
                AgentRunStatus::Running => ("●", tokens.accent, "running"),
                AgentRunStatus::Exited => ("○", tokens.fg_muted, "exited"),
                AgentRunStatus::Unknown => ("?", tokens.fg_muted, "unknown"),
            };

            list = list.child(
                div()
                    .id(("agent-panel-row", agent_panel_row_id(tab_index, pane_id)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.reveal_agent_pane(tab_index, pane_id, window, cx);
                    }))
                    .child(
                        svg()
                            .path(row.icon)
                            .flex_shrink_0()
                            .w(px(12.0))
                            .h(px(12.0))
                            .text_color(row.color),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(tokens.fg)
                                            .whitespace_nowrap()
                                            .child(row.agent_id.to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(3.0))
                                            .child(
                                                div()
                                                    .text_size(px(8.0))
                                                    .text_color(status_color)
                                                    .child(status_dot),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.0))
                                                    .text_color(status_color)
                                                    .child(status_label),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(tokens.fg_muted)
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(row.label),
                            ),
                    ),
            );
        }

        div()
            .id("agent-panel")
            .flex_shrink_0()
            .w(px(200.0))
            .h_full()
            .flex()
            .flex_col()
            .bg(tokens.surface)
            .border_l(border_w)
            .border_color(tokens.border)
            .child(header)
            .child(list)
            .into_any_element()
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

/// Stable element id for a panel row: tab and pane ids are both small and
/// unique within a window, so packing them keeps rows distinct across
/// re-renders.
fn agent_panel_row_id(tab_index: usize, pane_id: PaneId) -> u64 {
    ((tab_index as u64) << 32) | (pane_id as u64 & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_ids_are_unique_per_tab_and_pane() {
        assert_ne!(agent_panel_row_id(0, 1), agent_panel_row_id(1, 1));
        assert_ne!(agent_panel_row_id(0, 1), agent_panel_row_id(0, 2));
        assert_eq!(agent_panel_row_id(2, 5), agent_panel_row_id(2, 5));
    }
}
