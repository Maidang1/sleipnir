//! Per-window plugin surface owner (code-quality handoff item 1, slices 1–2b).
//!
//! `PluginRuntime` stays a process `Global` (supervisor, catalog, 16 ms pump).
//! What is *per window* — the chrome contributions and the debounced event
//! watch — lives here, in one owner that [`crate::app_shell::AppShell`] holds
//! as a single field. Panel surfaces are owned by the pane tree
//! (`LeafContent::Panel(PanelView)`) since slice 2b.

use std::collections::BTreeSet;
use uuid::Uuid;

use crate::pane_tree::PaneKey;
use crate::plugin_chrome::{
    ApplyChrome, ChromeRegistry, PaletteContribution, PluginTabBadge, StatusLayout,
};
use crate::plugin_event_watch::{PaneUiFacts, PluginEventWatch};
use plugin_protocol::v2::{HostEvent, Widget};

/// Window-scoped plugin state: chrome contributions and the polled pane-fact
/// watch. Panel surfaces are tree-owned since slice 2b. Owned by one `AppShell`.
#[derive(Default)]
pub(crate) struct PluginHost {
    chrome: ChromeRegistry,
    watch: PluginEventWatch,
}

impl PluginHost {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    // --- Chrome contributions ----------------------------------------------

    /// Apply a status contribution. Pure registry decision.
    pub(crate) fn apply_chrome_status(
        &mut self,
        plugin_id: &str,
        instance_id: Uuid,
        tree: Widget,
        granted: bool,
        active_pane: Option<PaneKey>,
    ) -> ApplyChrome {
        self.chrome
            .apply_status(plugin_id, instance_id, tree, granted, active_pane)
    }

    /// Drop chrome for instances that are no longer live. Returns whether
    /// anything changed (the caller rebuilds palette items on `true`).
    pub(crate) fn sync_chrome_live(&mut self, live: &BTreeSet<Uuid>) -> bool {
        self.chrome.sync_live(live)
    }

    pub(crate) fn chrome_is_empty(&self) -> bool {
        self.chrome.is_empty()
    }

    /// Lay out the status band. `&mut` because the layout is memoized.
    pub(crate) fn chrome_status_layout(&mut self, cols: u16) -> Option<&StatusLayout> {
        self.chrome.status_layout(cols)
    }

    pub(crate) fn chrome_badges_for_tab(
        &self,
        tab_panes: &[PaneKey],
        tab_is_active: bool,
    ) -> Vec<PluginTabBadge> {
        self.chrome.badges_for_tab(tab_panes, tab_is_active)
    }

    pub(crate) fn chrome_palette_entries(&self) -> &[PaletteContribution] {
        self.chrome.palette_entries()
    }

    // --- Event watch --------------------------------------------------------

    /// True when the 1 s poll throttle has elapsed. Must be *called* every tick
    /// so a quiet resident still gets polled facts.
    pub(crate) fn watch_due(&mut self, now: std::time::Instant, interval: std::time::Duration) -> bool {
        self.watch.due(now, interval)
    }

    /// Diff cheap UI-thread facts (cwd / agent / focus) against the last emit.
    pub(crate) fn watch_ingest_ui(
        &mut self,
        focus: Option<PaneKey>,
        facts: &[PaneUiFacts],
    ) -> Vec<HostEvent> {
        self.watch.ingest_ui(focus, facts)
    }

    /// Diff a port snapshot against the last emit.
    pub(crate) fn watch_ingest_ports(
        &mut self,
        pane: PaneKey,
        ports: &[crate::chrome::pane_facts::ListenPort],
    ) -> Vec<HostEvent> {
        self.watch.ingest_ports(pane, ports)
    }

    pub(crate) fn watch_ports_inflight(&self) -> bool {
        self.watch.ports_inflight
    }

    pub(crate) fn set_watch_ports_inflight(&mut self, inflight: bool) {
        self.watch.ports_inflight = inflight;
    }
}
