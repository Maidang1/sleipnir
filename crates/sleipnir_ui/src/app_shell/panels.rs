//! Side panels and dialogs: pane facts, run ledger, history search, and the
//! shared confirm dialog.
//!
//! A child module of `app_shell` so these can read the shell's private panel
//! state without widening it to the crate.

use gpui::{
    App, BorrowAppContext as _, ClickEvent, Context, Hsla, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, deferred, div, px,
};
use sleipnir_settings::TerminalSettings;

use super::{AppShell, ConfirmKind};
use crate::chrome::ChromeTokens;
use crate::run_ledger_global::RunLedgerGlobal;
use crate::ui_mode::OverlayKind;

impl AppShell {
    pub(super) fn render_pane_facts(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::chrome::pane_facts::localhost_copy;
        let facts = self
            .active_pane_key()
            .and_then(|pane| self.facts.facts_for(pane))
            .cloned()
            .unwrap_or_default();

        let mut body = div()
            .id("pane-facts-body")
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .pb_3()
            .overflow_y_scroll();

        if let Some(cwd) = facts.cwd.as_ref() {
            body = body.child(facts_section(
                tokens,
                "Directory",
                vec![cwd.display().to_string()],
            ));
        }
        if let Some(name) = facts.foreground.as_ref() {
            body = body.child(facts_section(tokens, "Foreground", vec![name.clone()]));
        }
        if !facts.tree.is_empty() {
            let lines: Vec<String> = facts
                .tree
                .iter()
                .map(|row| {
                    let pad = "  ".repeat(row.depth);
                    match row.name.as_deref() {
                        Some(name) => format!("{pad}{name}  {}", row.pid),
                        None => format!("{pad}{}", row.pid),
                    }
                })
                .collect();
            body = body.child(facts_section(tokens, "Processes", lines));
        }
        if !facts.ports.is_empty() {
            let mut port_col = div().flex().flex_col().gap_1();
            for port in &facts.ports {
                let label: SharedString = port.addr.clone().into();
                let copy = localhost_copy(&port.addr);
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().text_color(tokens.fg).child(label));
                if let Some(text) = copy {
                    row = row.child(
                        div()
                            .id(("copy-port", port.pid as u64))
                            .text_xs()
                            .text_color(tokens.accent)
                            .cursor_pointer()
                            .child("copy")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    text.clone(),
                                ));
                            })),
                    );
                }
                port_col = port_col.child(row);
            }
            body = body
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.fg_muted)
                        .child("Ports"),
                )
                .child(port_col);
        }

        deferred(
            div()
                .id("pane-facts-overlay")
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(280.0))
                .flex()
                .flex_col()
                .bg(tokens.surface)
                .border_l_1()
                .border_color(tokens.border)
                .occlude()
                .child(
                    div()
                        .id("pane-facts-header")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .px_3()
                        .py_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.fg)
                                .child("Pane"),
                        )
                        .child(
                            div()
                                .id("pane-facts-close")
                                .text_xs()
                                .text_color(tokens.fg_muted)
                                .cursor_pointer()
                                .child("Esc")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_pane_facts(cx);
                                })),
                        ),
                )
                .child(body),
        )
    }

    pub(super) fn active_tombstone(&self, cx: &App) -> Option<crate::chrome::tombstone::Tombstone> {
        if !TerminalSettings::get_global(cx).show_tombstone {
            return None;
        }
        let tab = self.tabs.get(self.active)?;
        let pane = tab.tree.pane_key_for_id(tab.active_pane)?;
        let ledger = cx.try_global::<RunLedgerGlobal>()?;
        self.tombstone_gate
            .banner(&ledger.snapshot(), pane, ledger.launch_id())
    }

    pub(super) fn render_tombstone(
        &self,
        tokens: &ChromeTokens,
        stone: crate::chrome::tombstone::Tombstone,
    ) -> impl IntoElement {
        div()
            .id("tombstone-banner")
            .absolute()
            .top(px(4.0))
            .left_4()
            .right_4()
            .px_3()
            .py_1()
            .rounded(px(6.0))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .text_xs()
            .text_color(tokens.fg_muted)
            .child(stone.summary)
    }

    pub(super) fn render_run_ledger(
        &self,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::run_ledger_panel::{can_jump, group_label, row_summary, rows_from_runs};
        let ledger = cx.try_global::<RunLedgerGlobal>();
        let (rows, launch) = match ledger {
            Some(g) => (rows_from_runs(&g.snapshot()), g.launch_id()),
            None => (Vec::new(), run_ledger::LaunchId::nil()),
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut body = div()
            .id("run-ledger-body")
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .pb_3()
            .overflow_y_scroll();
        let mut last_group = "";
        for (i, row) in rows.iter().enumerate() {
            let group = group_label(row, now, launch);
            if group != last_group {
                last_group = group;
                body = body.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.fg_muted)
                        .child(group),
                );
            }
            let summary: SharedString = row_summary(row).into();
            let pane = row.pane;
            let id = row.id;
            let jump = can_jump(row, launch);
            body = body.child(
                div()
                    .id(("ledger-row", i))
                    .text_xs()
                    .text_color(if jump { tokens.fg } else { tokens.fg_muted })
                    .cursor_pointer()
                    .child(summary)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if cx.has_global::<RunLedgerGlobal>() {
                            cx.update_global(|g: &mut RunLedgerGlobal, _| {
                                g.mark_run_seen(id);
                            });
                        }
                        this.jump_to_ledger_row(pane, Some(id), window, cx);
                        cx.notify();
                    })),
            );
        }
        let _ = window;
        deferred(
            div()
                .id("run-ledger-overlay")
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(300.0))
                .flex()
                .flex_col()
                .bg(tokens.surface)
                .border_l_1()
                .border_color(tokens.border)
                .occlude()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.fg)
                        .child("Run Ledger"),
                )
                .child(body),
        )
    }

    pub(super) fn render_history_search(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::chrome::history_search::{filter_history, parse_history_file};
        let text = std::env::var("HISTFILE")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .or_else(|| {
                dirs::home_dir().and_then(|h| {
                    std::fs::read_to_string(h.join(".zsh_history"))
                        .ok()
                        .or_else(|| std::fs::read_to_string(h.join(".bash_history")).ok())
                })
            })
            .or_else(|| {
                dirs::data_dir().and_then(|d| {
                    std::fs::read_to_string(
                        d.join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt"),
                    )
                    .ok()
                })
            })
            .unwrap_or_default();
        let hits = parse_history_file(&text);
        let shown = filter_history(&hits, &self.history_query, 20);
        let mut list = div().flex().flex_col().gap_1().px_3().pb_3();
        for (i, hit) in shown.iter().enumerate() {
            let cmd = hit.command.clone();
            list = list.child(
                div()
                    .id(("hist", i))
                    .text_xs()
                    .text_color(tokens.fg)
                    .cursor_pointer()
                    .child(hit.command.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(view) = this.active_view(cx) {
                            view.update(cx, |v, cx| v.input_bytes(cmd.clone().into_bytes(), cx));
                        }
                        this.mode.close(OverlayKind::History);
                        this.history_query.clear();
                        cx.notify();
                    })),
            );
        }
        deferred(
            div()
                .id("history-search-overlay")
                .absolute()
                .top(px(48.0))
                .left_0()
                .right_0()
                .mx_auto()
                .w(px(420.0))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .rounded(px(8.0))
                .occlude()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(tokens.fg_muted)
                        .child(format!("History · {}", self.history_query)),
                )
                .child(list),
        )
    }

    pub(super) fn render_close_confirm(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (title, message, ok_label) = match self.close_confirm.as_ref() {
            Some(s) if s.kind == ConfirmKind::ClearRunLedger => {
                ("Clear Run Ledger?", s.message.clone(), "Clear")
            }
            Some(s) => ("Close pane?", s.message.clone(), "Close"),
            None => (
                "Close pane?",
                SharedString::from("Close this pane?"),
                "Close",
            ),
        };

        let panel = div()
            .id("close-confirm-panel")
            .w(px(360.0))
            .rounded(px(10.0))
            .bg(tokens.content_bg)
            .border_1()
            .border_color(tokens.border)
            .shadow_lg()
            .flex()
            .flex_col()
            .overflow_hidden()
            // Keep clicks inside the panel from reaching the backdrop.
            // Without this, mouse_down on Close/Cancel hits the full-size
            // backdrop first and cancel wins — the pane never closes.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg)
                    .child(title),
            )
            .child(
                div()
                    .px_4()
                    .pb_4()
                    .text_size(px(13.0))
                    .text_color(tokens.fg_muted)
                    .child(message),
            )
            .child(
                div()
                    .px_4()
                    .pb_4()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("close-confirm-cancel")
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|el| el.bg(tokens.hover))
                            .text_size(px(13.0))
                            .text_color(tokens.fg)
                            .child("Cancel")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_close_cancel(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id("close-confirm-ok")
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(tokens.accent)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(gpui::hsla(0.0, 0.0, 1.0, 1.0))
                            .child(ok_label)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.confirm_close_proceed(window, cx);
                            })),
                    ),
            );

        deferred(
            div()
                .id("close-confirm-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                // BlockMouse: otherwise TermElement under the overlay still
                // sees should_handle_scroll() and the terminal scrolls too.
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("close-confirm-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(Hsla::black().opacity(0.5))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.confirm_close_cancel(window, cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }
}

fn facts_section(tokens: &ChromeTokens, title: &str, lines: Vec<String>) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap_1().child(
        div()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(tokens.fg_muted)
            .child(SharedString::from(title.to_string())),
    );
    for line in lines {
        col = col.child(
            div()
                .text_xs()
                .text_color(tokens.fg)
                .child(SharedString::from(line)),
        );
    }
    col
}
