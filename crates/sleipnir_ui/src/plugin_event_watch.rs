//! Debounced pane-fact deltas for the plugin event bus.
//!
//! Cwd, foreground agent, focus and listen ports are polled, not pushed.
//! Emitting on every frame would storm every `SubscribeEvents` plugin.
//! This store emits a delta only when a value actually changes.

use plugin_protocol::v2::HostEvent;
use run_ledger::PaneKey;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use crate::chrome::pane_facts::ListenPort;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneUiFacts {
    pub pane: PaneKey,
    pub cwd: Option<String>,
    pub agent: Option<String>,
}

#[derive(Default)]
struct WatchedPane {
    cwd: Option<String>,
    agent: Option<String>,
    ports: BTreeSet<(u32, String)>,
}

/// Last-seen pane facts. Pure: no GPUI, no I/O.
#[derive(Default)]
pub struct PluginEventWatch {
    last_poll: Option<Instant>,
    last_focus: Option<PaneKey>,
    panes: HashMap<PaneKey, WatchedPane>,
    pub ports_inflight: bool,
}

impl PluginEventWatch {
    pub fn due(&mut self, now: Instant, interval: Duration) -> bool {
        if self
            .last_poll
            .is_some_and(|at| now.duration_since(at) < interval)
        {
            return false;
        }
        self.last_poll = Some(now);
        true
    }

    /// Compare cheap UI-thread facts (cwd / agent / focus) to the last emit.
    pub fn ingest_ui(&mut self, focus: Option<PaneKey>, facts: &[PaneUiFacts]) -> Vec<HostEvent> {
        let live: BTreeSet<PaneKey> = facts.iter().map(|f| f.pane).collect();
        self.panes.retain(|pane, _| live.contains(pane));

        let mut out = Vec::new();
        if focus != self.last_focus {
            self.last_focus = focus;
            if let Some(pane) = focus {
                out.push(HostEvent::PaneFocused { pane });
            }
        }
        for fact in facts {
            let entry = self.panes.entry(fact.pane).or_default();
            if fact.cwd != entry.cwd {
                entry.cwd = fact.cwd.clone();
                if let Some(cwd) = fact.cwd.clone() {
                    out.push(HostEvent::CwdChanged {
                        pane: fact.pane,
                        cwd,
                    });
                }
            }
            if fact.agent != entry.agent {
                entry.agent = fact.agent.clone();
                out.push(HostEvent::ForegroundChanged {
                    pane: fact.pane,
                    agent: fact.agent.clone(),
                });
            }
        }
        out
    }

    /// Compare a port snapshot to the last emit. Only *new* listens fire
    /// `PortOpened`; closing a port is not an event in v2.
    pub fn ingest_ports(&mut self, pane: PaneKey, ports: &[ListenPort]) -> Vec<HostEvent> {
        let incoming: BTreeSet<(u32, String)> =
            ports.iter().map(|p| (p.pid, p.addr.clone())).collect();
        let entry = self.panes.entry(pane).or_default();
        let mut out = Vec::new();
        for (pid, addr) in &incoming {
            if entry.ports.insert((*pid, addr.clone())) {
                out.push(HostEvent::PortOpened {
                    pane,
                    pid: *pid,
                    addr: addr.clone(),
                });
            }
        }
        entry.ports = incoming;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn pane(n: u128) -> PaneKey {
        Uuid::from_u128(n)
    }

    #[test]
    fn due_debounces_within_the_interval() {
        let mut w = PluginEventWatch::default();
        let t0 = Instant::now();
        assert!(w.due(t0, Duration::from_secs(1)));
        assert!(!w.due(t0 + Duration::from_millis(200), Duration::from_secs(1)));
        assert!(w.due(t0 + Duration::from_secs(1), Duration::from_secs(1)));
    }

    #[test]
    fn ingest_ui_emits_only_on_change() {
        let mut w = PluginEventWatch::default();
        let p = pane(1);
        let facts = [PaneUiFacts {
            pane: p,
            cwd: Some("/a".into()),
            agent: Some("claude".into()),
        }];
        let first = w.ingest_ui(Some(p), &facts);
        assert!(
            first
                .iter()
                .any(|e| matches!(e, HostEvent::PaneFocused { pane } if *pane == p))
        );
        assert!(
            first
                .iter()
                .any(|e| matches!(e, HostEvent::CwdChanged { cwd, .. } if cwd == "/a"))
        );
        assert!(first.iter().any(|e| matches!(
            e,
            HostEvent::ForegroundChanged { agent, .. } if agent.as_deref() == Some("claude")
        )));
        let second = w.ingest_ui(Some(p), &facts);
        assert!(second.is_empty(), "unchanged facts must not re-emit");
    }

    #[test]
    fn ingest_ports_emits_only_new_listens() {
        let mut w = PluginEventWatch::default();
        let p = pane(1);
        let a = ListenPort {
            pid: 7,
            addr: "127.0.0.1:3000".into(),
        };
        let first = w.ingest_ports(p, std::slice::from_ref(&a));
        assert_eq!(first.len(), 1);
        let again = w.ingest_ports(p, std::slice::from_ref(&a));
        assert!(again.is_empty());
        let b = ListenPort {
            pid: 7,
            addr: "127.0.0.1:3001".into(),
        };
        let added = w.ingest_ports(p, &[a, b]);
        assert_eq!(added.len(), 1);
        assert!(matches!(
            &added[0],
            HostEvent::PortOpened { addr, .. } if addr == "127.0.0.1:3001"
        ));
    }
}
