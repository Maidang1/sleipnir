//! Remote SSH host picker overlay.
//!
//! Lists hosts from both sources the user configured — their own
//! `~/.ssh/config` and `ssh.hosts` in `settings.json` — filtered by a fuzzy
//! query, and opens the selected one in a new tab running `ssh`.
//!
//! The list is resolved when the overlay opens rather than watched, so editing
//! `~/.ssh/config` on disk is picked up the next time the picker is shown.

use gpui::{
    AppContext as _, ClickEvent, Context, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, deferred, div, px,
};

use super::{AppShell, ChromeTokens};
use crate::app_shell::query::{QueryBox, QuerySurface};
use crate::pane_tree::PaneNode;
use crate::ssh::SshConnectPlan;
use sleipnir_ssh::{HostSource, SshHostEntry, filter_hosts, resolve_default_hosts};

/// State for the host picker. The filter query lives on the shell (next to the
/// other overlay query boxes); this holds the resolved hosts and the matches.
pub(crate) struct SshHostsState {
    /// Every host, resolved once when the overlay opened.
    pub(crate) hosts: Vec<SshHostEntry>,
    /// Indices into `hosts` matching the current query, best first.
    pub(crate) matches: Vec<usize>,
}

impl SshHostsState {
    pub(crate) fn new(hosts: Vec<SshHostEntry>) -> Self {
        let matches = (0..hosts.len()).collect();
        Self { hosts, matches }
    }

    /// Recompute `matches` for `query`.
    pub(crate) fn refresh_matches(&mut self, query: &str) {
        self.matches = filter_hosts(&self.hosts, query);
    }

    /// The highlighted host, if any.
    pub(crate) fn selected(&self, selected: usize) -> Option<&SshHostEntry> {
        self.matches.get(selected).map(|&index| &self.hosts[index])
    }
}

impl AppShell {
    /// Resolve the host list and open the picker overlay.
    pub(crate) fn open_ssh_hosts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = sleipnir_settings::TerminalSettings::get_global(cx).clone();
        let hosts = resolve_default_hosts(&settings.ssh.hosts, settings.ssh.read_ssh_config);
        self.ssh_hosts = Some(SshHostsState::new(hosts));
        self.ssh_hosts_input = QueryBox::seeded(String::new());
        self.refresh_ssh_host_matches();
        self.open_overlay(crate::ui_mode::OverlayKind::SshHosts, cx);
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Close the picker, dropping the resolved list.
    pub(crate) fn close_ssh_hosts(&mut self, cx: &mut Context<Self>) {
        self.ssh_hosts = None;
        self.close_overlay(crate::ui_mode::OverlayKind::SshHosts, cx);
    }

    /// Recompute matches after the query changed through the IME.
    pub(crate) fn refresh_ssh_host_matches(&mut self) {
        let query = self.ssh_hosts_input.text.clone();
        if let Some(state) = self.ssh_hosts.as_mut() {
            state.refresh_matches(&query);
            if self.ssh_hosts_input.selected >= state.matches.len() {
                self.ssh_hosts_input.selected = 0;
            }
        }
    }

    /// Keyboard handling while the picker owns the keyboard.
    pub(crate) fn ssh_hosts_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.ssh_hosts.is_none() {
            return false;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                self.close_ssh_hosts(cx);
                true
            }
            "enter" => {
                self.connect_selected_ssh_host(window, cx);
                true
            }
            "up" | "arrowup" => {
                let matches = self.ssh_hosts.as_ref().map_or(0, |s| s.matches.len());
                if matches > 0 {
                    self.ssh_hosts_input.selected = self.ssh_hosts_input.selected.saturating_sub(1);
                }
                cx.notify();
                true
            }
            "down" | "arrowdown" => {
                let matches = self.ssh_hosts.as_ref().map_or(0, |s| s.matches.len());
                if matches > 0 {
                    let next = self.ssh_hosts_input.selected + 1;
                    self.ssh_hosts_input.selected = next.min(matches - 1);
                }
                cx.notify();
                true
            }
            _ => {
                // Typing edits the query through the shared IME path.
                let items = self.ssh_hosts.as_ref().map_or(0, |s| s.matches.len());
                let changed = self.ssh_hosts_input.edit(&event.keystroke, items, &mut || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
                if changed {
                    self.refresh_ssh_host_matches();
                    cx.notify();
                }
                // Swallow everything else so terminal shortcuts don't fire.
                true
            }
        }
    }

    /// Connect to the highlighted host in a new tab.
    pub(crate) fn connect_selected_ssh_host(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.ssh_hosts_input.selected;
        let Some(entry) = self
            .ssh_hosts
            .as_ref()
            .and_then(|state| state.selected(selected))
            .cloned()
        else {
            return;
        };
        self.connect_ssh_host(entry, window, cx);
    }

    /// Connect to a specific host entry (used by the click path).
    fn connect_ssh_host_index(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self
            .ssh_hosts
            .as_ref()
            .and_then(|state| state.hosts.get(index))
            .cloned()
        else {
            return;
        };
        self.connect_ssh_host(entry, window, cx);
    }

    /// Open a new tab running `ssh` for `entry`.
    pub(crate) fn connect_ssh_host(
        &mut self,
        entry: SshHostEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = sleipnir_settings::TerminalSettings::get_global(cx).clone();
        let exe = std::env::current_exe().ok();
        let plan = SshConnectPlan::for_entry(entry, &settings, exe.as_deref());
        let title = plan.title.clone();
        let view = cx.new(|cx| crate::TermView::new_ssh(plan, window, cx));
        self.wire_term_view(&view, window, cx);

        let tab_id = self.next_id;
        self.next_id += 1;
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut tab = crate::app_shell::Tab {
            id: tab_id,
            tree: PaneNode::leaf(pane_id, view),
            active_pane: pane_id,
            custom_title: None,
            zoomed_pane: None,
        };
        if let Some(title) = title {
            tab.custom_title = Some(SharedString::from(title));
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.close_ssh_hosts(cx);
        self.commit_workspace(window, cx);
    }

    /// Render the host picker as a centered modal panel.
    pub(super) fn render_ssh_hosts(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use gpui::prelude::FluentBuilder as _;

        let border_w = crate::chrome::pixel::PIXEL_BORDER;
        let Some(state) = self.ssh_hosts.as_ref() else {
            return deferred(div());
        };

        let mut list = div().flex().flex_col().w_full();
        if state.matches.is_empty() {
            list = list.child(
                div()
                    .px_4()
                    .py_6()
                    .text_size(px(13.0))
                    .text_color(tokens.fg_muted)
                    .child(if state.hosts.is_empty() {
                        "No SSH hosts yet. Add one under `ssh.hosts` in settings.json, or configure ~/.ssh/config."
                    } else {
                        "No host matches the filter."
                    }),
            );
        } else {
            for (row, &index) in state.matches.iter().enumerate() {
                let host = &state.hosts[index];
                let selected = row == self.ssh_hosts_input.selected;
                let source_label = match host.source {
                    HostSource::SshConfig => "~/.ssh/config",
                    HostSource::App => "Sleipnir",
                };
                let alias = host.alias.clone();
                let display = host.display();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("ssh-host-{index}")))
                        .w_full()
                        .px_4()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .when(selected, |el| el.bg(tokens.accent.opacity(0.18)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.connect_ssh_host_index(index, window, cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(tokens.fg)
                                        .child(alias),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(tokens.fg_muted)
                                        .child(source_label),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(tokens.fg_muted)
                                .child(display),
                        ),
                );
            }
        }

        deferred(
            div()
                .id("ssh-hosts-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .justify_center()
                .pt(px(96.0))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.close_ssh_hosts(cx);
                    }),
                )
                .child(
                    div()
                        .id("ssh-hosts-panel")
                        .w(px(460.0))
                        .max_h(px(420.0))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .shadow(crate::chrome::pixel::hard_shadow())
                        .child(crate::chrome::pixel::pixel_panel_bg(
                            tokens.content_bg,
                            tokens.border,
                        ))
                        // Keep panel clicks from reaching the backdrop.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .px_4()
                                .pt_4()
                                .pb_2()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.fg)
                                .child("Connect to Host"),
                        )
                        .child(
                            div()
                                .relative()
                                .mx_4()
                                .mb_2()
                                .px_3()
                                .py_2()
                                .border(border_w)
                                .border_color(tokens.border)
                                .text_size(px(13.0))
                                .text_color(tokens.fg)
                                .child(if self.ssh_hosts_input.text.is_empty() {
                                    SharedString::from("Type to filter hosts…")
                                } else {
                                    SharedString::from(self.ssh_hosts_input.text.clone())
                                })
                                .child(self.query_input_canvas(QuerySurface::SshHosts, cx)),
                        )
                        .child(list),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(alias: &str, source: HostSource) -> SshHostEntry {
        SshHostEntry {
            alias: alias.into(),
            hostname: format!("{alias}.example.com"),
            source,
            ..SshHostEntry::default()
        }
    }

    fn state(aliases: &[&str]) -> SshHostsState {
        SshHostsState::new(
            aliases
                .iter()
                .map(|a| host(a, HostSource::App))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn new_state_matches_every_host() {
        let state = state(&["alpha", "beta", "gamma"]);
        assert_eq!(state.matches, vec![0, 1, 2]);
        assert_eq!(state.hosts.len(), 3);
    }

    #[test]
    fn refresh_matches_filters_and_ranks_prefix_first() {
        let mut state = state(&["staging", "production", "edge"]);
        state.refresh_matches("sta");
        assert_eq!(state.matches, vec![0]);
        assert_eq!(state.selected(0).map(|h| h.alias.as_str()), Some("staging"));

        state.refresh_matches("pro");
        assert_eq!(state.matches, vec![1]);

        state.refresh_matches("zzz");
        assert!(state.matches.is_empty());
        assert!(state.selected(0).is_none());
    }

    #[test]
    fn selected_is_none_past_the_end() {
        let state = state(&["only"]);
        assert!(state.selected(0).is_some());
        assert!(state.selected(1).is_none());
    }
}
