//! Plugin orchestration for the shell (ADR-0015/0016/0017/0018): event
//! polling, render/call application, consent gating, resident lifecycle, and
//! panel camera drag.
//!
//! This is a child module of `app_shell` so it can drive `AppShell` internals
//! while they stay private to the shell, matching `command_dispatch.rs` and
//! `panels.rs`.

use super::*;
use crate::plugin_surface::StaleRegistry;

/// Enough to finish a launch after the user approves. The dialog itself
/// renders [`crate::plugin_monitor_panel::ConsentPrompt`] only.
pub(super) struct PluginConsentPending {
    pub(super) prompt: crate::plugin_monitor_panel::ConsentPrompt,
    kind: PluginConsentKind,
    hash: plugin_grants::BinaryHash,
    request: Vec<plugin_protocol::v2::Capability>,
}
pub(super) enum PluginConsentKind {
    Command(plugin_host::LoadedPluginCommand),
    Resident(plugin_host::LoadedPlugin),
}

impl AppShell {
    pub(super) fn poll_plugin_events(&mut self, cx: &mut Context<Self>) {
        use crate::plugin_event_watch::PaneUiFacts;
        if !self
            .plugin_watch
            .due(std::time::Instant::now(), std::time::Duration::from_secs(1))
        {
            return;
        }
        let focus = self.active_pane_key();
        let mut facts = Vec::new();
        let mut port_jobs = Vec::new();
        for tab in &self.tabs {
            let mut leaves = Vec::new();
            tab.tree.leaves_with_keys(&mut leaves);
            for (pane, view) in leaves {
                let cwd = view
                    .read(cx)
                    .working_directory(cx)
                    .map(|p| p.to_string_lossy().into_owned());
                let fg = view.read(cx).foreground_process_command_name(cx);
                let agent = fg
                    .as_deref()
                    .and_then(crate::chrome::agent::identify)
                    .map(|kind| kind.id.to_string());
                port_jobs.push((pane, view.read(cx).shell_pid(cx)));
                facts.push(PaneUiFacts { pane, cwd, agent });
            }
        }
        for ev in self.plugin_watch.ingest_ui(focus, &facts) {
            crate::plugin_runtime::broadcast_event(ev, cx);
        }
        if self.plugin_watch.ports_inflight {
            return;
        }
        self.plugin_watch.ports_inflight = true;
        cx.spawn(async move |this, cx| {
            let mut found = Vec::new();
            for (pane, pid) in port_jobs {
                let facts = cx
                    .background_spawn(async move {
                        crate::chrome::pane_facts::collect_live_facts(None, None, pid)
                    })
                    .await;
                found.push((pane, facts.ports));
            }
            this.update(cx, |this, cx| {
                this.plugin_watch.ports_inflight = false;
                for (pane, ports) in found {
                    for ev in this.plugin_watch.ingest_ports(pane, &ports) {
                        crate::plugin_runtime::broadcast_event(ev, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn poll_plugin_inbound(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use plugin_host::resident::{ConnectionState, Inbound};
        use plugin_protocol::v2::RenderTarget;
        let inbound = crate::plugin_runtime::drain_all_inbound(cx);
        for (plugin_id, msg) in inbound {
            match msg {
                Inbound::Render {
                    target: RenderTarget::Panel { pane },
                    tree,
                    ..
                } => self.apply_panel_render(&plugin_id, pane, tree, window, cx),
                Inbound::Render {
                    target: RenderTarget::Status,
                    tree,
                    ..
                } => self.apply_chrome_status(&plugin_id, tree, cx),
                Inbound::Render {
                    target: RenderTarget::Block { anchor },
                    tree,
                    ..
                } => self.apply_block_render(&plugin_id, anchor, tree, cx),
                Inbound::Call { id, call } => {
                    self.handle_host_call(&plugin_id, id, call, window, cx)
                }
            }
        }
        let snapshots = crate::plugin_runtime::snapshots(cx);
        let live: std::collections::BTreeSet<String> = snapshots
            .into_iter()
            .filter(|snap| snap.state == ConnectionState::Live)
            .map(|snap| snap.plugin_id)
            .collect();
        // One sweep per mount, driven by `live` alone. Marking each non-live
        // snapshot individually first would be redundant: snapshots hold at
        // most one entry per plugin_id, so a non-live plugin is by definition
        // absent from `live` and the sweep already covers it.
        self.plugin_panels.mark_missing_stale(&live);
        self.mark_missing_blocks_stale(&live, cx);
        // Chrome is the exception: transient decoration is dropped, not
        // dimmed, so a dead plugin cannot leave a badge misreporting live
        // state (see `plugin_surface`).
        if self.plugin_chrome.sync_live(&live) {
            self.rebuild_palette_items();
        }
    }
    pub(super) fn apply_panel_render(
        &mut self,
        plugin_id: &str,
        pane: PaneKey,
        tree: plugin_protocol::v2::Widget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_panel::ApplyPanel;
        use plugin_protocol::v2::Capability;
        // Same source as the event bus: Hello.granted, not "the plugin asked".
        let granted = crate::plugin_runtime::has_grant(plugin_id, Capability::RenderPanel, cx);
        let mut terminals = std::collections::BTreeSet::new();
        for tab in &self.tabs {
            let mut out = Vec::new();
            tab.tree.leaves_with_keys(&mut out);
            for (key, _) in out {
                terminals.insert(key);
            }
        }
        match self
            .plugin_panels
            .apply_render(plugin_id, pane, tree, granted, &terminals)
        {
            ApplyPanel::Create { pane_key } => {
                if !self.insert_panel_leaf(pane_key, plugin_id, window, cx) {
                    self.plugin_panels.remove(pane_key);
                }
            }
            ApplyPanel::Replace { .. } => cx.notify(),
            ApplyPanel::DeniedGrant => {
                log::warn!("plugin {plugin_id} RenderPanel denied (no grant)");
            }
            ApplyPanel::DeniedTerminal => {
                log::warn!("plugin {plugin_id} tried to draw into a terminal pane");
            }
            ApplyPanel::DeniedOccupied => {
                log::warn!("plugin {plugin_id} tried to take another plugin's panel");
            }
        }
    }
    pub(super) fn apply_block_render(
        &mut self,
        plugin_id: &str,
        run_id: plugin_protocol::v2::RunId,
        tree: plugin_protocol::v2::Widget,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_block::ApplyBlock;
        use plugin_protocol::v2::Capability;
        let granted = crate::plugin_runtime::has_grant(plugin_id, Capability::RenderBlock, cx);
        let (pane, ledger_anchor, existing) = if cx.has_global::<RunLedgerGlobal>() {
            let snap = cx.global::<RunLedgerGlobal>().snapshot();
            snap.into_iter()
                .find(|r| r.id == run_id)
                .map(|r| {
                    let existing = self.view_for_pane(r.pane).and_then(|v| {
                        v.read(cx)
                            .blocks()
                            .iter()
                            .find(|s| s.run_id == run_id && s.plugin_id == plugin_id)
                            .map(|s| s.block_id)
                    });
                    (Some(r.pane), r.anchor, existing)
                })
                .unwrap_or((None, None, None))
        } else {
            (None, None, None)
        };
        let Some(pane) = pane else {
            log::warn!("plugin {plugin_id} RenderBlock: no Run for anchor");
            return;
        };
        let Some(view) = self.view_for_pane(pane) else {
            log::warn!("plugin {plugin_id} RenderBlock: no pane for Run");
            return;
        };
        let out = view.update(cx, |v, cx| {
            v.apply_block_render(
                plugin_id,
                run_id,
                tree,
                granted,
                ledger_anchor,
                existing,
                cx,
            )
        });
        match out {
            ApplyBlock::Inserted | ApplyBlock::Replaced => cx.notify(),
            ApplyBlock::DeniedGrant => {
                log::warn!("plugin {plugin_id} RenderBlock denied (no grant)");
            }
            ApplyBlock::DeniedAnchor => {
                log::warn!("plugin {plugin_id} RenderBlock denied (no process-local anchor)");
            }
        }
    }
    pub(super) fn mark_missing_blocks_stale(
        &mut self,
        live: &std::collections::BTreeSet<String>,
        cx: &mut Context<Self>,
    ) {
        for (_, view) in self.all_live_panes() {
            view.update(cx, |v, _| v.mark_missing_blocks_stale(live));
        }
    }
    pub(super) fn set_all_blocks_frozen(&mut self, frozen: bool, cx: &mut Context<Self>) {
        for (_, view) in self.all_live_panes() {
            view.update(cx, |v, cx| v.set_blocks_frozen(frozen, cx));
        }
    }
    pub(super) fn apply_chrome_status(
        &mut self,
        plugin_id: &str,
        tree: plugin_protocol::v2::Widget,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_chrome::ApplyChrome;
        use plugin_protocol::v2::Capability;
        let granted = crate::plugin_runtime::has_grant(plugin_id, Capability::RenderStatus, cx);
        let hint = self.active_pane_key();
        match self
            .plugin_chrome
            .apply_status(plugin_id, tree, granted, hint)
        {
            ApplyChrome::Applied => {
                self.rebuild_palette_items();
                cx.notify();
            }
            ApplyChrome::DeniedGrant => {
                log::warn!("plugin {plugin_id} RenderStatus denied (no grant)");
            }
        }
    }
    pub(super) fn insert_panel_leaf(
        &mut self,
        pane_key: PaneKey,
        plugin_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return false;
        };
        let target = tab.active_pane;
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let content = crate::LeafContent::Panel {
            plugin_id: plugin_id.to_string(),
        };
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        if !tab
            .tree
            .split_content(target, SplitAxis::Horizontal, new_id, pane_key, content)
        {
            return false;
        }
        tab.active_pane = new_id;
        self.commit_workspace(window, cx);
        true
    }
    pub(super) fn handle_host_call(
        &mut self,
        plugin_id: &str,
        id: plugin_protocol::v2::MessageId,
        call: plugin_protocol::v2::HostCall,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_host_calls::{
            CallPlan, cap_screen, error_result, filter_listed_panes, plan_call, read_screen_access,
        };
        use plugin_protocol::v2::{Capability, HostCallResult, PaneInfo};
        let granted: Vec<Capability> = [
            Capability::HostCallNotify,
            Capability::HostCallReadScreen,
            Capability::HostCallListPanes,
            Capability::HostCallOpenPane,
            Capability::HostCallDrawScene,
        ]
        .into_iter()
        .filter(|cap| crate::plugin_runtime::has_grant(plugin_id, *cap, cx))
        .collect();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let plan = plan_call(plugin_id, &call, &granted, &mut self.plugin_calls, now_ms);
        let result = match plan {
            CallPlan::Reply(result) => result,
            CallPlan::Notify { title, body } => {
                crate::notify_message(&title, &body);
                HostCallResult::Ok
            }
            CallPlan::ListPanes => {
                let live = crate::control_surface::live_terminal_panes(cx);
                let mut terminals = std::collections::BTreeSet::new();
                let mut infos = Vec::new();
                for (pane, view) in &live {
                    terminals.insert(*pane);
                    infos.push(PaneInfo {
                        pane: *pane,
                        cwd: view
                            .read(cx)
                            .working_directory(cx)
                            .map(|p| p.to_string_lossy().into_owned()),
                        title: Some(view.read(cx).title().to_string()),
                        busy: view.read(cx).looks_busy(cx),
                    });
                }
                HostCallResult::Panes {
                    panes: filter_listed_panes(infos, &terminals),
                }
            }
            CallPlan::ReadScreen { pane } => {
                let live = crate::control_surface::live_terminal_panes(cx);
                let mut terminals = std::collections::BTreeSet::new();
                for (key, _) in &live {
                    terminals.insert(*key);
                }
                let (_, panels) = self.terminal_and_panel_keys();
                match read_screen_access(pane, &terminals, &panels) {
                    Err(message) => error_result(message),
                    Ok(()) => match live.into_iter().find(|(key, _)| *key == pane) {
                        Some((_, view)) => HostCallResult::Screen {
                            text: cap_screen(view.read(cx).visible_screen_text(cx)),
                        },
                        None => error_result(format!("pane {pane} not found")),
                    },
                }
            }
            CallPlan::OpenPane { cwd, command } => self.execute_open_pane(cwd, command, window, cx),
            CallPlan::DrawScene { pane, scene } => {
                // A fresh scene from the plugin is authoritative, including its
                // camera: the host adopts it so the plugin's own controls (spin,
                // rescan, cd) keep the view in sync. Host-driven camera moves go
                // the other way and never resend the scene (see the camera
                // action path), so this cannot fight an in-progress drag.
                if self.plugin_panels.set_scene(pane, plugin_id, scene) {
                    window.refresh();
                    HostCallResult::SceneOk
                } else {
                    error_result("pane not found or not owned by this plugin")
                }
            }
        };
        if !crate::plugin_runtime::reply_host_call(plugin_id, id, result, cx) {
            log::debug!("plugin {plugin_id} Call {id} reply dropped (session gone)");
        }
    }
    /// Rotate a plugin panel's camera from a drag delta. The host owns the
    /// camera: it mutates the stored scene and repaints immediately (no plugin
    /// round-trip, so the motion is smooth), then reports the new camera to the
    /// plugin as a throttled `camera` action so the legend stays in sync.
    pub(super) fn drag_panel_camera(
        &mut self,
        pane_key: PaneKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.panel_drag.as_ref() else {
            return;
        };
        if drag.pane_key != pane_key {
            return;
        }
        let dx = f32::from(position.x) - f32::from(drag.last.x);
        let dy = f32::from(position.y) - f32::from(drag.last.y);
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        // Pixels-to-radians: a full panel width is roughly a half turn.
        const YAW_PER_PX: f32 = 0.01;
        const PITCH_PER_PX: f32 = 0.01;
        let Some(scene) = self.plugin_panels.scene(pane_key) else {
            return;
        };
        let mut camera = scene.camera;
        camera.yaw = wrap_camera_angle(camera.yaw + dx * YAW_PER_PX);
        camera.pitch = (camera.pitch - dy * PITCH_PER_PX).clamp(0.05, 1.35);
        self.plugin_panels.set_scene_camera(pane_key, camera);
        if let Some(drag) = self.panel_drag.as_mut() {
            drag.last = position;
        }
        cx.notify();
        self.push_panel_camera(pane_key, false, cx);
    }
    /// Zoom a plugin panel's camera from a scroll-wheel delta.
    pub(super) fn zoom_panel_camera(
        &mut self,
        pane_key: PaneKey,
        ev: &gpui::ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(scene) = self.plugin_panels.scene(pane_key) else {
            return;
        };
        // A line of wheel travel is one zoom step; pixel deltas are scaled down.
        let dy = match ev.delta {
            gpui::ScrollDelta::Lines(p) => p.y,
            gpui::ScrollDelta::Pixels(p) => f32::from(p.y) / 40.0,
        };
        if dy == 0.0 {
            return;
        }
        let mut camera = scene.camera;
        let factor = 1.0 + dy * 0.1;
        camera.zoom = (camera.zoom * factor).clamp(0.5, 2.5);
        self.plugin_panels.set_scene_camera(pane_key, camera);
        cx.notify();
        self.push_panel_camera(pane_key, false, cx);
    }
    /// End a camera drag and push the final camera unthrottled, so the plugin's
    /// legend settles on the exact resting view.
    pub(super) fn end_panel_camera_drag(&mut self, pane_key: PaneKey, cx: &mut Context<Self>) {
        let is_ours = self
            .panel_drag
            .as_ref()
            .is_some_and(|d| d.pane_key == pane_key);
        if !is_ours {
            return;
        }
        self.push_panel_camera(pane_key, true, cx);
        self.panel_drag = None;
    }
    /// Report a panel's current camera to its plugin as a `camera` action.
    ///
    /// Throttled unless `force`: the local repaint already happened, so this only
    /// keeps the plugin-owned legend in sync. Per the no-loopback rule the plugin
    /// answers `camera` by resending chrome only, never the scene, so this cannot
    /// bounce back and fight the drag.
    pub(super) fn push_panel_camera(&mut self, pane_key: PaneKey, force: bool, cx: &mut Context<Self>) {
        const THROTTLE_MS: u64 = 40;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if !force && now.saturating_sub(self.panel_camera_last_ms) < THROTTLE_MS {
            return;
        }
        let Some((plugin_id, surface_id)) = self
            .panel_drag
            .as_ref()
            .filter(|d| d.pane_key == pane_key)
            .map(|d| (d.plugin_id.clone(), d.surface_id))
            .or_else(|| {
                // Wheel zoom has no active drag; look the surface up directly.
                self.plugin_panels
                    .get(pane_key)
                    .map(|s| (s.plugin_id.clone(), s.surface_id))
            })
        else {
            return;
        };
        let Some(scene) = self.plugin_panels.scene(pane_key) else {
            return;
        };
        let camera = scene.camera;
        // Typed payload: the same serde `SceneCamera` the scene itself carries,
        // not a stringly key=value encoding.
        let arg = serde_json::to_string(&camera).unwrap_or_default();
        self.panel_camera_last_ms = now;
        crate::plugin_runtime::push_action(
            &plugin_id,
            surface_id,
            "camera".to_string(),
            Some(arg),
            cx,
        );
    }
    pub(super) fn terminal_and_panel_keys(
        &self,
    ) -> (
        std::collections::BTreeSet<PaneKey>,
        std::collections::BTreeSet<PaneKey>,
    ) {
        let mut terminals = std::collections::BTreeSet::new();
        let mut panels = std::collections::BTreeSet::new();
        for tab in &self.tabs {
            let mut all = Vec::new();
            tab.tree.walk_leaves(&mut all);
            for (_, key, content) in all {
                if content.is_terminal() {
                    terminals.insert(key);
                } else {
                    panels.insert(key);
                }
            }
        }
        (terminals, panels)
    }
    pub(super) fn execute_open_pane(
        &mut self,
        cwd: Option<String>,
        command: Option<crate::plugin_host_calls::OpenCommand>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> plugin_protocol::v2::HostCallResult {
        use crate::plugin_host_calls::error_result;
        use crate::session::resolve_cwd;
        use plugin_protocol::v2::HostCallResult;
        let cwd = match cwd.as_deref() {
            None => None,
            Some(raw) => {
                let resolved = resolve_cwd(Some(raw));
                if resolved.is_none() {
                    return error_result(format!("cwd not found: {raw}"));
                }
                resolved
            }
        };
        let argv = command.map(|c| (c.program, c.args));
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let tab_id = self.next_id;
        self.next_id += 1;
        let view = self.spawn_term_view(cwd, argv, window, cx);
        let pane_key = {
            let tab = crate::tab_convert::Tab {
                id: tab_id,
                tree: crate::pane_tree::PaneNode::leaf(pane_id, view),
                active_pane: pane_id,
                custom_title: None,
                zoomed_pane: None,
            };
            let key = tab.tree.pane_key_for_id(pane_id).expect("fresh leaf");
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            key
        };
        self.commit_workspace(window, cx);
        HostCallResult::Pane { pane: pane_key }
    }
    pub(super) fn refresh_plugin_commands(&mut self, cx: &mut Context<Self>) {
        crate::plugin_runtime::PluginRuntime::reload(cx);
        self.plugin_commands = crate::plugin_runtime::PluginRuntime::commands(cx);
        self.rebuild_palette_items();
        self.palette_selected = 0;
        self.start_resident_plugins(cx);
    }
    pub(super) fn run_plugin_contribution(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.plugin_chrome.palette_entries().get(index).cloned() else {
            return;
        };
        crate::plugin_runtime::push_action(
            &entry.plugin_id,
            entry.surface_id,
            entry.action,
            entry.arg,
            cx,
        );
    }
    pub(super) fn on_toggle_plugin_monitor(
        &mut self,
        _: &TogglePluginMonitor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command(CommandId::TogglePluginMonitor, window, cx);
    }
    pub(super) fn toggle_plugin_monitor(&mut self, cx: &mut Context<Self>) {
        self.mode.toggle(OverlayKind::PluginMonitor);
        cx.notify();
    }
    pub(super) fn close_plugin_monitor(&mut self, cx: &mut Context<Self>) {
        self.mode.close(OverlayKind::PluginMonitor);
        cx.notify();
    }
    pub(super) fn deny_plugin_consent(&mut self, cx: &mut Context<Self>) {
        // Deny writes nothing: a dismissed prompt must not become a grant.
        self.plugin_consent = None;
        self.mode.close(OverlayKind::PluginConsent);
        cx.notify();
    }
    pub(super) fn approve_plugin_consent(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.plugin_consent.take() else {
            return;
        };
        self.mode.close(OverlayKind::PluginConsent);
        let plugin_id = match &pending.kind {
            PluginConsentKind::Command(plugin) => plugin.plugin_id.clone(),
            PluginConsentKind::Resident(plugin) => plugin.manifest.id.clone(),
        };
        crate::plugin_runtime::save_grant(
            &plugin_id,
            &pending.request,
            &pending.hash,
            pending.prompt.tier,
        );
        match pending.kind {
            PluginConsentKind::Command(plugin) => self.invoke_plugin_command(plugin, cx),
            PluginConsentKind::Resident(plugin) => {
                self.connect_resident(plugin, pending.request, cx)
            }
        }
        // Another resident may still be waiting on first-run consent.
        self.start_resident_plugins(cx);
        cx.notify();
    }
    pub(super) fn kill_plugin(&mut self, plugin_id: String, cx: &mut Context<Self>) {
        crate::plugin_runtime::kill_plugin(&plugin_id, cx);
        cx.notify();
    }
    pub(super) fn run_plugin_command(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(plugin) = self.plugin_commands.get(index).cloned() else {
            return;
        };
        self.start_plugin_command(plugin, cx);
    }
    pub(super) fn start_plugin_command(
        &mut self,
        plugin: plugin_host::LoadedPluginCommand,
        cx: &mut Context<Self>,
    ) {
        let request = crate::plugin_runtime::requested_capabilities(&plugin);
        let Some(hash) = crate::plugin_runtime::plugin_binary_hash(&plugin) else {
            log::warn!(
                "plugin {} has no hashable binary; refusing to run",
                plugin.qualified_id()
            );
            return;
        };
        let grants = crate::plugin_runtime::grants();
        let record = grants.grants.get(&plugin.plugin_id);
        let plugin_id = plugin.plugin_id.clone();
        let plugin_name = plugin.plugin_name.clone();
        if self.gate_or_prompt_consent(
            &plugin_id,
            &plugin_name,
            request,
            record,
            hash,
            PluginConsentKind::Command(plugin.clone()),
            cx,
        ) {
            self.invoke_plugin_command(plugin, cx);
        }
    }
    /// The single consent gate. Returns true when the request is already
    /// covered by a stored grant; otherwise arms the consent overlay and
    /// returns false. `plugin_grants::check` never denies, so every gap is a
    /// prompt — the caller decides what "approved" means (invoke vs connect).
    pub(super) fn gate_or_prompt_consent(
        &mut self,
        plugin_id: &str,
        plugin_name: &str,
        request: Vec<plugin_protocol::v2::Capability>,
        record: Option<&plugin_grants::GrantRecord>,
        hash: plugin_grants::BinaryHash,
        kind: PluginConsentKind,
        cx: &mut Context<Self>,
    ) -> bool {
        match plugin_grants::check(&request, record, &hash) {
            plugin_grants::Decision::Allowed => true,
            plugin_grants::Decision::NeedsConsent { reason, missing } => {
                let previously: Vec<_> = record
                    .map(|r| r.granted.iter().copied().collect())
                    .unwrap_or_default();
                let tier = record.map(|r| r.tier).unwrap_or(plugin_grants::Tier::Local);
                self.plugin_consent = Some(PluginConsentPending {
                    prompt: crate::plugin_monitor_panel::consent_prompt(
                        plugin_id,
                        plugin_name,
                        tier,
                        reason,
                        &missing,
                        &previously,
                    ),
                    kind,
                    hash,
                    request,
                });
                self.mode.open(OverlayKind::PluginConsent);
                cx.notify();
                false
            }
        }
    }
    pub(super) fn invoke_plugin_command(
        &mut self,
        plugin: plugin_host::LoadedPluginCommand,
        cx: &mut Context<Self>,
    ) {
        // v2 and resident sessions speak the multiplexed dialect; the v1
        // per-invocation path cannot carry Event / Render / Call.
        if plugin.api_version >= plugin_host::PLUGIN_API_VERSION_V2
            || plugin.lifecycle == plugin_host::PluginLifecycle::Resident
        {
            self.invoke_v2_command(plugin, cx);
            return;
        }
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let context = crate::plugin_runtime::build_context(&plugin, &view, cx);
        let allowed = crate::plugin_runtime::allowed_permissions(cx);
        let qualified_id = plugin.qualified_id();
        log::info!(
            "plugin: invoking {qualified_id} (selection={} bytes, cwd={:?})",
            context.selection.as_deref().map(str::len).unwrap_or(0),
            context.cwd,
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { plugin_host::run_command(&plugin, &context, &allowed) },
                )
                .await;
            this.update(cx, |_this, cx| match result {
                Ok(output) => crate::plugin_runtime::apply_output(output, &view, cx),
                Err(err) => log::warn!("plugin {qualified_id} failed: {err}"),
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn invoke_v2_command(
        &mut self,
        plugin: plugin_host::LoadedPluginCommand,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let context = crate::plugin_runtime::build_context(&plugin, &view, cx);
        let Some(loaded) = crate::plugin_runtime::PluginRuntime::plugins(cx)
            .into_iter()
            .find(|p| p.manifest.id == plugin.plugin_id)
        else {
            log::warn!("plugin {} not in catalog", plugin.plugin_id);
            return;
        };
        let grants = crate::plugin_runtime::grants();
        let granted: Vec<plugin_protocol::v2::Capability> = grants
            .grants
            .get(&plugin.plugin_id)
            .map(|r| r.granted.iter().copied().collect())
            .unwrap_or_else(|| crate::plugin_runtime::requested_capabilities(&plugin));
        let spec = crate::plugin_runtime::launch_spec(&loaded, granted);
        let Some(sup) = crate::plugin_runtime::supervisor(cx) else {
            return;
        };
        let command_id = plugin.command.id.clone();
        let qualified_id = plugin.qualified_id();
        let invoke_ctx = plugin_protocol::InvokeContext {
            cwd: context.cwd,
            title: context.title,
            selection: context.selection,
            visible_screen: context.visible_screen,
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { sup.invoke(&spec, &command_id, invoke_ctx) })
                .await;
            this.update(cx, |_this, cx| match result {
                Ok(output) => {
                    let routed = match output {
                        plugin_protocol::Output::Ignore => plugin_host::PluginRunOutput::Ignored,
                        plugin_protocol::Output::Insert { text } => {
                            plugin_host::PluginRunOutput::Insert(text)
                        }
                        plugin_protocol::Output::Copy { text } => {
                            plugin_host::PluginRunOutput::Copy(text)
                        }
                    };
                    crate::plugin_runtime::apply_output(routed, &view, cx);
                }
                Err(err) => log::warn!("plugin {qualified_id} failed: {err}"),
            })
            .ok();
        })
        .detach();
    }
    /// Handshake every granted resident. First-run / binary-change / new-cap
    /// gaps become a consent prompt; nothing is launched without a grant.
    pub(super) fn start_resident_plugins(&mut self, cx: &mut Context<Self>) {
        if self.plugin_consent.is_some() {
            return;
        }
        for plugin in crate::plugin_runtime::PluginRuntime::plugins(cx) {
            if plugin.manifest.lifecycle != plugin_host::PluginLifecycle::Resident {
                continue;
            }
            if crate::plugin_runtime::is_plugin_live(&plugin.manifest.id, cx) {
                continue;
            }
            let request = crate::plugin_runtime::requested_capabilities_for_plugin(&plugin);
            let Some(hash) = crate::plugin_runtime::loaded_plugin_hash(&plugin) else {
                log::warn!(
                    "plugin {} has no hashable binary; refusing to start",
                    plugin.manifest.id
                );
                continue;
            };
            let grants = crate::plugin_runtime::grants();
            let record = grants.grants.get(&plugin.manifest.id);
            if self.gate_or_prompt_consent(
                &plugin.manifest.id,
                &plugin.manifest.name,
                request.clone(),
                record,
                hash,
                PluginConsentKind::Resident(plugin.clone()),
                cx,
            ) {
                let granted = record
                    .map(|r| r.granted.iter().copied().collect())
                    .unwrap_or(request);
                self.connect_resident(plugin, granted, cx);
            } else {
                // One prompt at a time: the rest re-evaluate after approve/deny.
                return;
            }
        }
    }
    pub(super) fn connect_resident(
        &mut self,
        plugin: plugin_host::LoadedPlugin,
        granted: Vec<plugin_protocol::v2::Capability>,
        cx: &mut Context<Self>,
    ) {
        let Some(sup) = crate::plugin_runtime::supervisor(cx) else {
            return;
        };
        let spec = crate::plugin_runtime::launch_spec(&plugin, granted);
        let id = plugin.manifest.id;
        // Claim the launch synchronously. `is_plugin_live` cannot see a connect
        // that has been spawned but not yet completed, so without this a second
        // window (or a settings reload racing construction) starts the same
        // resident process twice.
        if !crate::plugin_runtime::begin_connect(&id, cx) {
            return;
        }
        log::info!("plugin: starting resident {id}");
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { sup.connect(&spec) }).await;
            this.update(cx, |_this, cx| {
                if let Err(err) = result {
                    log::warn!("plugin {id} failed to start: {err}");
                }
                // Release on both paths: a failed launch must stay retryable.
                crate::plugin_runtime::finish_connect(&id, cx);
            })
            .ok();
        })
        .detach();
    }
}

pub(super) fn run_event_to_host(
    event: &RunEvent,
    snapshot: &[run_ledger::Run],
) -> Option<plugin_protocol::v2::HostEvent> {
    use plugin_protocol::v2::HostEvent;
    match event {
        RunEvent::Started { pane, cwd, .. } => {
            let run = snapshot
                .iter()
                .rev()
                .find(|r| r.pane == *pane && r.state == run_ledger::RunState::Running)?;
            Some(HostEvent::RunStarted {
                run_id: run.id,
                pane: run.pane,
                command: run.command.clone(),
                cwd: cwd.clone().or_else(|| run.cwd.clone()),
            })
        }
        RunEvent::Finished { pane, .. } => {
            let run = snapshot.iter().rev().find(|r| {
                r.pane == *pane
                    && r.state.is_finished()
                    && r.state != run_ledger::RunState::Abandoned
            })?;
            Some(HostEvent::RunFinished {
                run_id: run.id,
                pane: run.pane,
                exit_code: run.exit_code,
                duration_ms: run.duration.as_millis() as u64,
            })
        }
        RunEvent::PaneClosed { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_started_host_event_uses_ledger_redacted_command() {
        let mut ledger = run_ledger::Ledger::new(run_ledger::LaunchId::nil());
        let pane = run_ledger::PaneKey::from_u128(1);
        let event = run_ledger::RunEvent::started(
            pane,
            "AWS_SECRET_ACCESS_KEY=supersecret aws s3 ls",
            None,
            10,
        );
        ledger.apply(event.clone());
        let host = run_event_to_host(&event, &ledger.snapshot()).expect("mapped");
        let plugin_protocol::v2::HostEvent::RunStarted { command, .. } = host else {
            panic!("expected RunStarted");
        };
        assert!(
            !command.contains("supersecret"),
            "plugins must never see the raw command line: {command}"
        );
    }
}
