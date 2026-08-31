//! Side panels and dialogs: pane facts, run ledger, history search, plugin
//! monitor / consent, and the shared confirm dialog.
//!
//! A child module of `app_shell` so these can read the shell's private panel
//! state without widening it to the crate.

use super::{AppShell, ConfirmKind, TogglePaneFacts};
use crate::chrome::ChromeTokens;
use crate::run_ledger_global::RunLedgerGlobal;
use crate::ui_mode::{OverlayKind, PANE_FACTS_MAX_AGE, PaneFactsState};
use gpui::{
    AppContext as _, BorrowAppContext as _, ClickEvent, Context, Hsla, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, deferred, div, prelude::FluentBuilder as _, px,
};

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

    pub(super) fn render_plugin_monitor(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::plugin_monitor_panel::{
            format_activity, format_pid, format_uptime, live_plugin_count, rows_from_snapshots,
            running_indicator_label, state_label, tier_badge,
        };
        let snapshots = crate::plugin_runtime::snapshots(cx);
        let names = crate::plugin_runtime::catalog_names(cx);
        let tiers = crate::plugin_runtime::grant_tiers();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let running = live_plugin_count(&snapshots);
        let mut rows = rows_from_snapshots(&snapshots, &names, &tiers, now);
        let drops = self.plugin_calls.dropped_counts();
        for row in &mut rows {
            row.host_calls_dropped = drops.get(&row.plugin_id).copied().unwrap_or(0);
        }

        let mut body = div()
            .id("plugin-monitor-body")
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .pb_3()
            .overflow_y_scroll();

        if rows.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child("No plugins running."),
            );
        }

        for (i, row) in rows.iter().enumerate() {
            let uptime = format_uptime(now.saturating_sub(row.started_at_ms));
            let activity = format_activity(now.saturating_sub(row.last_activity_ms));
            let plugin_id = row.plugin_id.clone();
            let stderr: String = row.stderr.join("\n");
            let mut card = div()
                .id(("plugin-row", i))
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded(px(6.0))
                .bg(tokens.content_bg)
                .border_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.fg)
                                .child(row.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.fg_muted)
                                .child(tier_badge(row.tier)),
                        ),
                )
                .child(div().text_xs().text_color(tokens.fg_muted).child(format!(
                    "{} · {} · up {} · {} · in-flight {} · restarts {}",
                    state_label(row.state),
                    format_pid(row.pid),
                    uptime,
                    activity,
                    row.in_flight,
                    row.restart_count
                )))
                .child(
                    div()
                        .text_xs()
                        .text_color(tokens.fg_disabled)
                        .child(format!(
                            "dropped {} · events dropped {} · malformed {} · host calls dropped {}",
                            row.inbound_dropped,
                            row.events_dropped,
                            row.malformed_lines,
                            row.host_calls_dropped
                        )),
                );
            if !stderr.is_empty() {
                card = card.child(div().text_xs().text_color(tokens.fg_muted).child(stderr));
            }
            card = card.child(
                div()
                    .id(("plugin-kill", i))
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_xs()
                    .text_color(tokens.accent)
                    .child("Kill")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.kill_plugin(plugin_id.clone(), cx);
                    })),
            );
            body = body.child(card);
        }

        deferred(
            div()
                .id("plugin-monitor-overlay")
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(340.0))
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
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tokens.fg)
                                .child("Plugin Monitor"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(tokens.fg_muted)
                                // Only meaningful once something is running: the
                                // panel title plus the empty-state line already
                                // say "no plugins", so a "0 plugins" counter here
                                // is the same fact a third time.
                                .when(running > 0, |el| el.child(running_indicator_label(running))),
                        ),
                )
                .child(body),
        )
    }

    pub(super) fn render_plugin_consent(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::plugin_monitor_panel::{
            approve_label, capability_label, consent_copy, deny_label, tier_badge,
        };
        let prompt = self.plugin_consent.as_ref().map(|p| p.prompt.clone());
        let (title, lead, warning, caps, tier) = match prompt.as_ref() {
            Some(prompt) => {
                let copy = consent_copy(prompt);
                (
                    copy.title.to_string(),
                    copy.lead,
                    copy.is_security_warning,
                    prompt.missing.clone(),
                    prompt.tier,
                )
            }
            None => (
                "Plugin permission".into(),
                String::new(),
                false,
                Vec::new(),
                plugin_grants::Tier::Local,
            ),
        };

        let title_color = if warning { tokens.accent } else { tokens.fg };
        let mut cap_list = div().flex().flex_col().gap_1();
        for cap in &caps {
            cap_list = cap_list.child(
                div()
                    .text_size(px(12.0))
                    .text_color(tokens.fg)
                    .child(format!("· {}", capability_label(*cap))),
            );
        }

        let panel = div()
            .id("plugin-consent-panel")
            .w(px(420.0))
            .rounded(px(10.0))
            .bg(tokens.content_bg)
            .border_1()
            .border_color(if warning {
                tokens.accent
            } else {
                tokens.border
            })
            .shadow_lg()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(title_color)
                    .child(title),
            )
            .child(
                div()
                    .px_4()
                    .pb_2()
                    .text_size(px(12.0))
                    .text_color(tokens.fg_muted)
                    .child(tier_badge(tier)),
            )
            .child(
                div()
                    .px_4()
                    .pb_3()
                    .text_size(px(13.0))
                    .text_color(if warning { tokens.fg } else { tokens.fg_muted })
                    .when(warning, |el| el.font_weight(gpui::FontWeight::SEMIBOLD))
                    .child(lead),
            )
            .child(div().px_4().pb_3().child(cap_list))
            .child(
                div()
                    .px_4()
                    .pb_4()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(
                        // Approve is secondary: the safe default is Deny.
                        div()
                            .id("plugin-consent-approve")
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .hover(|el| el.bg(tokens.hover))
                            .text_size(px(13.0))
                            .text_color(tokens.fg)
                            .child(approve_label())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.approve_plugin_consent(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("plugin-consent-deny")
                            .px_3()
                            .py_1p5()
                            .rounded(px(6.0))
                            .bg(tokens.accent)
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .text_color(tokens.content_bg)
                            .child(deny_label())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.deny_plugin_consent(cx);
                            })),
                    ),
            );

        deferred(
            div()
                .id("plugin-consent-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("plugin-consent-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(tokens.content_bg.opacity(0.72))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.deny_plugin_consent(cx);
                            }),
                        ),
                )
                .child(panel),
        )
    }

    pub(super) fn render_plugin_status_chip(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use crate::plugin_monitor_panel::{live_plugin_count, running_indicator_label};
        let snapshots = crate::plugin_runtime::snapshots(cx);
        let n = live_plugin_count(&snapshots);
        let open = self.mode.is(OverlayKind::PluginMonitor);
        // Zero running plugins renders nothing: ADR-0016 §7 requires the
        // indicator to be unsuppressible *by a plugin*, and a plugin cannot
        // reach zero on its own behalf — the host owns this count. Showing
        // "0 plugins" permanently spends chrome on the state that carries no
        // information. The Monitor stays reachable from the palette.
        div()
            .id("plugin-status-chip")
            .flex_shrink_0()
            .when(n > 0, |el| {
                el.px_2()
                    .h_full()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_xs()
                    .when(open, |el| el.text_color(tokens.fg).bg(tokens.hover))
                    .when(!open, |el| el.text_color(tokens.fg_muted))
                    .hover(|el| el.bg(tokens.hover).text_color(tokens.fg))
                    .child(running_indicator_label(n))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_plugin_monitor(cx);
                    }))
            })
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

    pub(super) fn on_toggle_pane_facts(
        &mut self,
        _: &TogglePaneFacts,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_pane_facts(cx);
    }

    pub(super) fn toggle_pane_facts(&mut self, cx: &mut Context<Self>) {
        if self.mode.toggle(OverlayKind::PaneFacts) {
            self.refresh_pane_facts(cx);
        } else {
            self.discard_pane_facts();
        }
        cx.notify();
    }

    /// Close the facts panel and drop its cached snapshot. Any in-flight
    /// collection lands as stale, because it checks both the overlay and the
    /// pane it was started for before storing anything.
    pub(super) fn close_pane_facts(&mut self, cx: &mut Context<Self>) {
        self.mode.close(OverlayKind::PaneFacts);
        self.discard_pane_facts();
        cx.notify();
    }

    fn discard_pane_facts(&mut self) {
        self.facts = PaneFactsState::Idle;
    }

    /// Kick off an off-thread facts collection for the focused pane.
    ///
    /// `sysinfo` process-tree walks and `lsof` are far too slow to run on the
    /// UI thread, so the collection happens on the background executor and the
    /// result lands back through `PaneFactsState`. Results tagged with a stale
    /// pane, or arriving after the panel closed, are dropped.
    fn refresh_pane_facts(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane_key() else {
            self.facts = PaneFactsState::Idle;
            return;
        };
        let view = self.active_view(cx);
        let cwd = view.as_ref().and_then(|v| v.read(cx).working_directory(cx));
        let foreground = view
            .as_ref()
            .and_then(|v| v.read(cx).foreground_process_command_name(cx));
        let root = view.as_ref().and_then(|v| v.read(cx).shell_pid(cx));

        self.facts.begin_collection(pane);

        cx.spawn(async move |this, cx| {
            let facts = cx
                .background_spawn(async move {
                    crate::chrome::pane_facts::collect_live_facts(cwd, foreground, root)
                })
                .await;
            this.update(cx, |this, cx| {
                // Focus may have moved, or the panel closed, while we were off
                // thread. Either way this snapshot is no longer what is shown.
                if !this.mode.is(OverlayKind::PaneFacts) || this.active_pane_key() != Some(pane) {
                    return;
                }
                this.facts.finish_collection(pane, facts);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn refresh_pane_facts_if_stale(&mut self, cx: &mut Context<Self>) {
        if !self.mode.is(OverlayKind::PaneFacts) {
            return;
        }
        let Some(pane) = self.active_pane_key() else {
            return;
        };
        // Render calls this every frame; never stack a second collection for a
        // pane that already has one in flight.
        if self.facts.is_collecting_for(pane) {
            return;
        }
        if self.facts.needs_refresh_for(pane, PANE_FACTS_MAX_AGE) {
            self.refresh_pane_facts(cx);
        }
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
