use gpui::App;
use plugin_host::resident::Inbound;
use plugin_protocol::v2::{HostCall, HostCallResult, PaneKey, RenderTarget, RunId};
use std::collections::BTreeMap;

use crate::app_shell::AppShell;
use crate::plugin_runtime;
use crate::run_ledger_global::RunLedgerGlobal;

#[derive(Default)]
pub(crate) struct PluginDispatcher {
    live: Vec<(String, uuid::Uuid)>,
}

#[derive(Default)]
struct WindowRoutes {
    panes: BTreeMap<PaneKey, usize>,
    runs: BTreeMap<RunId, PaneKey>,
    preferred: Option<usize>,
    window_count: usize,
}

impl WindowRoutes {
    fn destinations(&self, message: &Inbound) -> Vec<usize> {
        let destination = match message {
            Inbound::Render {
                target: RenderTarget::Status,
                ..
            } => {
                return (0..self.window_count).collect();
            }
            Inbound::Render {
                target: RenderTarget::Block { anchor },
                ..
            } => self
                .runs
                .get(anchor)
                .and_then(|pane| self.panes.get(pane))
                .copied(),
            Inbound::Render {
                target: RenderTarget::Panel { pane },
                ..
            } => self.panes.get(pane).copied().or(self.preferred),
            Inbound::Call {
                call: HostCall::ReadScreen { pane } | HostCall::DrawScene { pane, .. },
                ..
            } => self.panes.get(pane).copied(),
            Inbound::Call {
                call: HostCall::ScrollToRun { run_id },
                ..
            } => self
                .runs
                .get(run_id)
                .and_then(|pane| self.panes.get(pane))
                .copied(),
            Inbound::Call { .. } => self.preferred,
        };
        destination.into_iter().collect()
    }
}

impl PluginDispatcher {
    pub(crate) fn pump(&mut self, cx: &mut App) {
        let Some(supervisor) = plugin_runtime::supervisor(cx) else {
            return;
        };
        let inbound = supervisor.drain_all_inbound();
        let live = supervisor.live_instances();
        let changed = self.live != live;
        self.live = live;
        if inbound.is_empty() && !changed {
            return;
        }
        let windows: Vec<_> = cx
            .windows()
            .into_iter()
            .filter_map(|handle| handle.downcast::<AppShell>())
            .collect();
        let active = cx
            .active_window()
            .and_then(|handle| handle.downcast::<AppShell>());
        let mut routes = WindowRoutes {
            preferred: windows
                .iter()
                .position(|handle| Some(*handle) == active)
                .or_else(|| (!windows.is_empty()).then_some(0)),
            window_count: windows.len(),
            ..WindowRoutes::default()
        };
        if cx.has_global::<RunLedgerGlobal>() {
            routes.runs = cx
                .global::<RunLedgerGlobal>()
                .snapshot()
                .into_iter()
                .map(|run| (run.id, run.pane))
                .collect();
        }
        for (index, handle) in windows.iter().enumerate() {
            if let Ok((terminals, panels)) =
                handle.update(cx, |shell, _, _| shell.terminal_and_panel_keys())
            {
                routes.panes.extend(
                    terminals
                        .into_iter()
                        .chain(panels)
                        .map(|pane| (pane, index)),
                );
            }
        }
        for (plugin_id, message) in inbound {
            let destinations = routes.destinations(&message);
            let call_id = match &message {
                Inbound::Call { id, .. } => Some(*id),
                _ => None,
            };
            if destinations.is_empty() {
                if let Some(id) = call_id {
                    plugin_runtime::reply_host_call(
                        &plugin_id,
                        id,
                        HostCallResult::Error {
                            message: "target pane or window is no longer available".into(),
                        },
                        cx,
                    );
                }
                continue;
            }
            let live_panes = if matches!(
                &message,
                Inbound::Call {
                    call: HostCall::ListPanes | HostCall::ReadScreen { .. },
                    ..
                }
            ) {
                crate::control_surface::live_terminal_panes(cx)
            } else {
                Vec::new()
            };
            for index in destinations {
                let applied = windows[index].update(cx, |shell, window, cx| {
                    shell.apply_plugin_inbound(
                        &plugin_id,
                        message.clone(),
                        &live_panes,
                        window,
                        cx,
                    );
                    cx.notify();
                    shell.terminal_and_panel_keys()
                });
                match applied {
                    Ok((terminals, panels)) => {
                        routes.panes.retain(|_, owner| *owner != index);
                        routes.panes.extend(
                            terminals
                                .into_iter()
                                .chain(panels)
                                .map(|pane| (pane, index)),
                        );
                    }
                    Err(_) => {
                        if let Some(id) = call_id {
                            plugin_runtime::reply_host_call(
                                &plugin_id,
                                id,
                                HostCallResult::Error {
                                    message: "target window closed".into(),
                                },
                                cx,
                            );
                        }
                    }
                }
            }
        }
        if changed {
            for handle in windows {
                let _ = handle.update(cx, |shell, _, cx| {
                    shell.sync_plugin_surfaces(cx);
                    cx.notify();
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_protocol::v2::Widget;
    use uuid::Uuid;

    fn two_windows() -> WindowRoutes {
        WindowRoutes {
            panes: BTreeMap::from([(Uuid::from_u128(1), 0), (Uuid::from_u128(2), 1)]),
            runs: BTreeMap::from([(Uuid::from_u128(10), Uuid::from_u128(2))]),
            preferred: Some(0),
            window_count: 2,
        }
    }

    #[test]
    fn block_routes_to_its_owner_not_the_active_window() {
        let message = Inbound::Render {
            id: 1,
            target: RenderTarget::Block {
                anchor: Uuid::from_u128(10),
            },
            tree: Widget::Sep,
        };
        assert_eq!(two_windows().destinations(&message), vec![1]);
    }

    #[test]
    fn panel_routes_to_existing_owner_and_new_panels_use_preferred_window() {
        let mut routes = two_windows();
        let message = Inbound::Render {
            id: 1,
            target: RenderTarget::Panel {
                pane: Uuid::from_u128(3),
            },
            tree: Widget::Sep,
        };
        assert_eq!(routes.destinations(&message), vec![0]);
        routes.panes.insert(Uuid::from_u128(3), 1);
        assert_eq!(routes.destinations(&message), vec![1]);
    }

    #[test]
    fn screen_calls_route_once_to_the_owner_and_missing_targets_do_not_fall_back() {
        let mut routes = two_windows();
        let message = Inbound::Call {
            id: 1,
            call: HostCall::ReadScreen {
                pane: Uuid::from_u128(2),
            },
        };
        assert_eq!(routes.destinations(&message), vec![1]);
        routes.panes.remove(&Uuid::from_u128(2));
        assert!(routes.destinations(&message).is_empty());
    }

    #[test]
    fn scroll_to_run_routes_to_the_runs_window_and_missing_runs_do_not_fall_back() {
        let mut routes = two_windows();
        let message = Inbound::Call {
            id: 1,
            call: HostCall::ScrollToRun {
                run_id: Uuid::from_u128(10),
            },
        };
        assert_eq!(routes.destinations(&message), vec![1]);
        routes.runs.remove(&Uuid::from_u128(10));
        assert!(routes.destinations(&message).is_empty());
    }

    #[test]
    fn status_is_broadcast_but_host_calls_are_not() {
        assert_eq!(
            two_windows().destinations(&Inbound::Render {
                id: 1,
                target: RenderTarget::Status,
                tree: Widget::Sep
            }),
            vec![0, 1]
        );
        assert_eq!(
            two_windows().destinations(&Inbound::Call {
                id: 2,
                call: HostCall::ListPanes
            }),
            vec![0]
        );
        assert!(
            WindowRoutes::default()
                .destinations(&Inbound::Call {
                    id: 2,
                    call: HostCall::ListPanes
                })
                .is_empty()
        );
    }
}
