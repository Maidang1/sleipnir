//! Per-window plugin surface owner (code-quality handoff item 1, slice 1).
//!
//! `PluginRuntime` stays a process `Global` (supervisor, catalog, 16 ms pump).
//! What is *per window* — the panel surfaces, the chrome contributions, and the
//! debounced event watch — lives here, in one owner that [`crate::app_shell::AppShell`]
//! holds as a single field. AppShell paints from it and keeps `InputMode`,
//! tabs, and focus; it no longer carries three sibling registries.
//!
//! The three registries are **private**. Every caller goes through a method on
//! `PluginHost` (`apply_panel_render`, `apply_chrome_status`, `sync_live`,
//! `watch_*`, the panel accessors), so this is an ownership move, not a struct
//! that re-exposes the same three fields under a new name.

use std::collections::BTreeSet;
use uuid::Uuid;

use crate::pane_tree::PaneKey;
use crate::plugin_chrome::{
    ApplyChrome, ChromeRegistry, PaletteContribution, PluginTabBadge, StatusLayout,
};
use crate::plugin_event_watch::{PaneUiFacts, PluginEventWatch};
use crate::plugin_panel::{ApplyPanel, PanelRegistry, PanelSurface};
use crate::plugin_surface::StaleRegistry as _;
use plugin_protocol::v2::{HostEvent, Widget};

/// Window-scoped plugin state: panel surfaces, chrome contributions, and the
/// polled pane-fact watch. Owned by one `AppShell`.
#[derive(Default)]
pub(crate) struct PluginHost {
    panels: PanelRegistry,
    chrome: ChromeRegistry,
    watch: PluginEventWatch,
}

impl PluginHost {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    // --- Panel surfaces -----------------------------------------------------

    /// The surface mounted at `pane`, if any. Painting reads this.
    pub(crate) fn panel(&self, pane: PaneKey) -> Option<&PanelSurface> {
        self.panels.get(pane)
    }

    /// Apply a whole-tree panel `Render`. Pure registry decision; the caller
    /// performs the pane_tree split for [`ApplyPanel::Create`].
    pub(crate) fn apply_panel_render(
        &mut self,
        plugin_id: &str,
        owner_instance_id: Uuid,
        pane: PaneKey,
        tree: Widget,
        granted: bool,
        terminal_panes: &BTreeSet<PaneKey>,
    ) -> ApplyPanel {
        self.panels
            .apply_render(plugin_id, owner_instance_id, pane, tree, granted, terminal_panes)
    }

    /// Drop a single panel surface (failed create, closed leaf).
    pub(crate) fn remove_panel(&mut self, pane: PaneKey) {
        self.panels.remove(pane);
    }

    /// Drop every listed panel surface (tab / window close).
    pub(crate) fn remove_panels(&mut self, keys: impl IntoIterator<Item = PaneKey>) {
        self.panels.remove_all(keys);
    }

    /// Snapshot surfaces for the given keys, for transfer to another window.
    pub(crate) fn clone_panel_surfaces(
        &self,
        keys: impl IntoIterator<Item = PaneKey>,
    ) -> Vec<PanelSurface> {
        self.panels.clone_surfaces(keys)
    }

    /// Adopt transferred surfaces (tab detach into a new window).
    pub(crate) fn insert_panel_surfaces(&mut self, surfaces: impl IntoIterator<Item = PanelSurface>) {
        self.panels.insert_surfaces(surfaces);
    }

    /// Mark surfaces whose owning instance is no longer live as stale (the last
    /// tree stays on screen, visibly dimmed).
    pub(crate) fn mark_panels_stale(&mut self, live: &BTreeSet<Uuid>) {
        self.panels.mark_missing_stale(live);
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
