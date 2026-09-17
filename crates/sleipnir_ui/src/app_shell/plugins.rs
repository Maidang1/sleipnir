//! Plugin orchestration for the shell (ADR-0015/0016/0017/0018): event
//! polling, render/call application, consent gating, and resident lifecycle.
//!
//! This is a child module of `app_shell` so it can drive `AppShell` internals
//! while they stay private to the shell, matching `command_dispatch.rs` and
//! `panels.rs`.

use super::*;
use plugin_protocol::v2::HostCallResult;

/// Enough to finish a launch after the user approves. The dialog itself
/// renders [`crate::plugin_monitor_panel::ConsentPrompt`] only.
pub(crate) struct PluginConsentPending {
    pub(super) prompt: crate::plugin_monitor_panel::ConsentPrompt,
    kind: PluginConsentKind,
    hash: plugin_grants::BinaryHash,
    request: Vec<plugin_protocol::v2::Capability>,
    supervisor: std::sync::Arc<plugin_host::resident::Supervisor>,
}
pub(super) enum PluginConsentKind {
    Command(plugin_host::LoadedPluginCommand),
    Resident(plugin_host::LoadedPlugin),
}

/// Validate that a cwd string points at an existing directory; otherwise fall
/// back to its parent, then home. `None` when even home is unavailable.
fn resolve_cwd(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    if path.is_dir() {
        Some(path)
    } else {
        path.parent()
            .filter(|p| p.is_dir())
            .map(|p| p.to_path_buf())
            .or_else(dirs::home_dir)
    }
}

impl AppShell {
    pub(crate) fn poll_plugin_events(&mut self, cx: &mut Context<Self>) {
        use crate::plugin_event_watch::PaneUiFacts;
        if !self
            .plugin_watch
            .due(std::time::Instant::now(), std::time::Duration::from_secs(1))
        {
            return;
        }
        if crate::plugin_runtime::snapshots(cx)
            .iter()
            .all(|snapshot| snapshot.state != plugin_host::resident::ConnectionState::Live)
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
        // The built-in Agents observer does not subscribe to port events.
        if !TerminalSettings::get_global(cx).plugins.enabled {
            return;
        }
        if self.plugin_watch.ports_inflight {
            return;
        }
        self.plugin_watch.ports_inflight = true;
        cx.spawn(async move |this, cx| {
            // One machine-level scan per poll: the process and listen tables are
            // the same for every pane, so capture them once off-thread and derive
            // each pane's ports from the shared snapshot instead of rescanning the
            // whole system once per pane.
            let found = cx
                .background_spawn(async move {
                    let snapshot = crate::chrome::pane_facts::MachineSnapshot::capture();
                    port_jobs
                        .into_iter()
                        .map(|(pane, pid)| (pane, snapshot.derive(None, None, pid).ports))
                        .collect::<Vec<_>>()
                })
                .await;
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
    pub(crate) fn apply_plugin_inbound(
        &mut self,
        plugin_id: &str,
        instance_id: uuid::Uuid,
        message: plugin_host::resident::Inbound,
        live_panes: &[(PaneKey, Entity<TermView>)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use plugin_host::resident::Inbound;
        use plugin_protocol::v2::RenderTarget;
        match message {
            Inbound::Render {
                target: RenderTarget::Panel { pane },
                tree,
                ..
            } => self.apply_panel_render(plugin_id, instance_id, pane, tree, window, cx),
            Inbound::Render {
                target: RenderTarget::Status,
                tree,
                ..
            } => self.apply_chrome_status(plugin_id, instance_id, tree, cx),
            Inbound::Render {
                target: RenderTarget::Block { anchor },
                tree,
                ..
            } => self.apply_block_render(plugin_id, instance_id, anchor, tree, cx),
            Inbound::Call { id, call } => {
                self.handle_host_call(plugin_id, instance_id, id, call, live_panes, window, cx)
            }
        }
    }
    pub(crate) fn sync_plugin_surfaces(&mut self, cx: &mut Context<Self>) {
        use plugin_host::resident::ConnectionState;
        let snapshots = crate::plugin_runtime::snapshots(cx);
        let live: std::collections::BTreeSet<uuid::Uuid> = snapshots
            .into_iter()
            .filter(|snap| snap.state == ConnectionState::Live)
            .map(|snap| snap.instance_id)
            .collect();
        // Walk all panel leaves and mark those whose owner is gone stale.
        for tab in &mut self.tabs {
            tab.tree.for_each_panel_mut(&mut |surface| {
                if !live.contains(&surface.owner_instance_id) {
                    surface.stale = true;
                }
            });
        }
        self.mark_missing_blocks_stale(&live, cx);
        if self.plugin_chrome.sync_live(&live) {
            self.rebuild_palette_items();
        }
    }
    pub(super) fn apply_panel_render(
        &mut self,
        plugin_id: &str,
        instance_id: uuid::Uuid,
        pane: PaneKey,
        tree: plugin_protocol::v2::Widget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_panel::{ApplyPanel, PanelSurface, decide_panel_render};
        use plugin_protocol::v2::Capability;
        let granted =
            crate::plugin_runtime::has_grant_for_instance(instance_id, Capability::RenderPanel, cx);
        let (terminals, _panels) = self.terminal_and_panel_keys();
        let is_terminal = terminals.contains(&pane);
        // Borrow the existing surface (if any) from the tree — no clone.
        let existing = self.tabs.iter().find_map(|tab| tab.tree.find_panel(pane));
        let decision = decide_panel_render(existing, plugin_id, instance_id, is_terminal, granted);
        let surface_id = match decision {
            ApplyPanel::Create { surface_id } | ApplyPanel::Replace { surface_id } => surface_id,
            ApplyPanel::DeniedGrant => {
                log::warn!("plugin {plugin_id} RenderPanel denied (no grant)");
                return;
            }
            ApplyPanel::DeniedTerminal => {
                log::warn!("plugin {plugin_id} tried to draw into a terminal pane");
                return;
            }
            ApplyPanel::DeniedOccupied => {
                log::warn!("plugin {plugin_id} tried to take another plugin's panel");
                return;
            }
            ApplyPanel::DeniedOwnerInstance => {
                log::warn!(
                    "plugin {plugin_id} instance {instance_id} tried to take a live panel owned by another instance"
                );
                return;
            }
        };
        let surface = PanelSurface {
            plugin_id: plugin_id.to_string(),
            owner_instance_id: instance_id,
            pane_key: pane,
            surface_id,
            tree,
            stale: false,
        };
        match decision {
            ApplyPanel::Replace { .. } => {
                for tab in &mut self.tabs {
                    if tab.tree.find_panel(pane).is_some() {
                        tab.tree.update_panel_surface(pane, surface);
                        break;
                    }
                }
                cx.notify();
            }
            ApplyPanel::Create { .. } => {
                self.insert_panel_leaf(pane, surface, window, cx);
            }
            _ => unreachable!("denials returned above"),
        }
    }
    pub(super) fn apply_block_render(
        &mut self,
        plugin_id: &str,
        instance_id: uuid::Uuid,
        run_id: plugin_protocol::v2::RunId,
        tree: plugin_protocol::v2::Widget,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_block::ApplyBlock;
        use plugin_protocol::v2::Capability;
        let granted =
            crate::plugin_runtime::has_grant_for_instance(instance_id, Capability::RenderBlock, cx);
        let (pane, ledger_anchor, existing) = if cx.has_global::<RunLedgerGlobal>() {
            let snap = cx.global::<RunLedgerGlobal>().snapshot();
            snap.into_iter()
                .find(|r| r.id == run_id)
                .map(|r| {
                    let existing = self.view_for_pane(r.pane).and_then(|v| {
                        v.read(cx)
                            .blocks()
                            .iter()
                            .find(|s| {
                                s.run_id == run_id
                                    && s.plugin_id == plugin_id
                                    && s.owner_instance_id == instance_id
                            })
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
                instance_id,
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
        live: &std::collections::BTreeSet<uuid::Uuid>,
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
        instance_id: uuid::Uuid,
        tree: plugin_protocol::v2::Widget,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_chrome::ApplyChrome;
        use plugin_protocol::v2::Capability;
        let granted = crate::plugin_runtime::has_grant_for_instance(
            instance_id,
            Capability::RenderStatus,
            cx,
        );
        let hint = self.active_pane_key();
        match self
            .plugin_chrome
            .apply_status(plugin_id, instance_id, tree, granted, hint)
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
        surface: crate::plugin_panel::PanelSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let new_id = self.next_pane_id;
        let content = crate::LeafContent::Panel(surface);
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        let target = tab.active_pane;
        if !tab
            .tree
            .split_content(target, SplitAxis::Horizontal, new_id, pane_key, content)
        {
            return false;
        }
        self.next_pane_id += 1;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.active_pane = new_id;
        }
        self.commit_workspace(window, cx);
        true
    }
    pub(super) fn handle_host_call(
        &mut self,
        plugin_id: &str,
        instance_id: uuid::Uuid,
        id: plugin_protocol::v2::MessageId,
        call: plugin_protocol::v2::HostCall,
        live_panes: &[(PaneKey, Entity<TermView>)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::plugin_host_calls::CallPlan;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let plan = crate::plugin_runtime::authorize_and_plan_host_call(
            plugin_id,
            instance_id,
            &call,
            now_ms,
            cx,
        );
        // Notify is a host side effect, not a workspace verb, so it never goes
        // through `WorkspaceIo`. Everything else is one plan → execute → reply.
        let result = match plan {
            CallPlan::Notify { title, body } => {
                crate::notify_message(&title, &body);
                HostCallResult::Ok
            }
            plan => {
                let mut io = ShellWorkspaceIo {
                    shell: self,
                    live_panes,
                    window,
                    cx,
                };
                plan.execute(&mut io)
            }
        };
        if !crate::plugin_runtime::reply_host_call_to_instance(instance_id, id, result, cx) {
            log::debug!("plugin {plugin_id} Call {id} reply dropped (session gone)");
        }
    }
    pub(crate) fn terminal_and_panel_keys(
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
        use plugin_protocol::v2::HostCallResult;
        let cwd = match cwd.as_deref() {
            None => None,
            Some(raw) => {
                let resolved = resolve_cwd(raw);
                if resolved.is_none() {
                    return error_result(format!("cwd not found: {raw}"));
                }
                resolved
            }
        };
        let argv = command.map(crate::plugin_host_calls::spawn_argv);
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
        self.palette.plugin_commands = crate::plugin_runtime::PluginRuntime::commands(cx);
        self.rebuild_palette_items();
        self.palette.selected = 0;
        self.start_resident_plugins(cx);
    }
    pub(super) fn run_plugin_contribution(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.plugin_chrome.palette_entries().get(index).cloned() else {
            return;
        };
        crate::plugin_runtime::push_action(
            entry.owner_instance_id,
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
        self.toggle_overlay(OverlayKind::PluginMonitor, cx);
        cx.notify();
    }
    pub(super) fn close_plugin_monitor(&mut self, cx: &mut Context<Self>) {
        self.close_overlay(OverlayKind::PluginMonitor, cx);
        cx.notify();
    }
    pub(super) fn deny_plugin_consent(&mut self, cx: &mut Context<Self>) {
        // Deny writes nothing: a dismissed prompt must not become a grant.
        self.dismiss_consent_input(cx);
        cx.notify();
    }
    pub(super) fn approve_plugin_consent(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.take_consent_input(cx) else {
            return;
        };
        if !crate::plugin_runtime::is_current(&pending.supervisor, cx) {
            cx.notify();
            return;
        }
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
        let Some(plugin) = self.palette.plugin_commands.get(index).cloned() else {
            return;
        };
        self.start_plugin_command(plugin, cx);
    }
    pub(super) fn start_plugin_command(
        &mut self,
        plugin: plugin_host::LoadedPluginCommand,
        cx: &mut Context<Self>,
    ) {
        if !crate::plugin_runtime::PluginRuntime::commands(cx).contains(&plugin) {
            return;
        }
        if plugin.source == plugin_host::PluginSource::BuiltInAgents {
            self.invoke_plugin_command(plugin, cx);
            return;
        }
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
                self.set_input(
                    crate::ui_mode::InputMode::Consent(PluginConsentPending {
                        supervisor: crate::plugin_runtime::supervisor(cx)
                            .expect("plugin runtime initialized"),
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
                    }),
                    cx,
                );
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
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let invoke_ctx = crate::plugin_runtime::build_context(&plugin, &view, cx);
        let Some(loaded) = crate::plugin_runtime::PluginRuntime::plugins(cx)
            .into_iter()
            .find(|p| p.manifest.id == plugin.plugin_id)
        else {
            log::warn!("plugin {} not in catalog", plugin.plugin_id);
            return;
        };
        let grants = crate::plugin_runtime::grants();
        let granted: Vec<plugin_protocol::v2::Capability> =
            crate::plugin_runtime::builtin_grants(&loaded).unwrap_or_else(|| {
                grants
                    .grants
                    .get(&plugin.plugin_id)
                    .map(|r| r.granted.iter().copied().collect())
                    .unwrap_or_else(|| crate::plugin_runtime::requested_capabilities(&plugin))
            });
        let spec = crate::plugin_runtime::launch_spec(&loaded, granted);
        let Some(sup) = crate::plugin_runtime::supervisor(cx) else {
            return;
        };
        let command_id = plugin.command.id.clone();
        let qualified_id = plugin.qualified_id();
        let granted = spec.granted.clone();
        let retirement = crate::plugin_runtime::retirement(cx);
        cx.spawn(async move |this, cx| {
            retirement.await;
            let invocation_supervisor = std::sync::Arc::clone(&sup);
            let result = cx
                .background_spawn(async move {
                    invocation_supervisor.invoke(&spec, &command_id, invoke_ctx)
                })
                .await;
            this.update(cx, |_this, cx| {
                if !crate::plugin_runtime::is_current(&sup, cx) {
                    return;
                }
                match result {
                    Ok(output) => crate::plugin_runtime::apply_output(output, &granted, &view, cx),
                    Err(err) => log::warn!("plugin {qualified_id} failed: {err}"),
                }
            })
            .ok();
        })
        .detach();
    }
    /// Handshake every resident. Built-ins use their compiled capability set;
    /// external first-run / binary-change / new-cap gaps need explicit consent.
    pub(super) fn start_resident_plugins(&mut self, cx: &mut Context<Self>) {
        if self.input.consent().is_some() {
            return;
        }
        for plugin in crate::plugin_runtime::PluginRuntime::plugins(cx) {
            if plugin.manifest.lifecycle != plugin_host::PluginLifecycle::Resident {
                continue;
            }
            if crate::plugin_runtime::is_plugin_live(&plugin.manifest.id, cx) {
                continue;
            }
            if let Some(granted) = crate::plugin_runtime::builtin_grants(&plugin) {
                self.connect_resident(plugin, granted, cx);
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
        let retirement = crate::plugin_runtime::retirement(cx);
        cx.spawn(async move |_this, cx| {
            retirement.await;
            let connecting_supervisor = std::sync::Arc::clone(&sup);
            let result = cx
                .background_spawn(async move { connecting_supervisor.connect(&spec) })
                .await;
            cx.update(|cx| {
                if let Err(err) = result {
                    log::warn!("plugin {id} failed to start: {err}");
                }
                // Release on both paths: a failed launch must stay retryable.
                crate::plugin_runtime::finish_connect(&id, &sup, cx);
            });
        })
        .detach();
    }
}

/// [`WorkspaceIo`] backed by the live shell. Borrows the shell plus the frame's
/// `window`/`cx` so `CallPlan::execute` runs the same verbs the control surface
/// does. `live_panes` is the shared terminal walk (`live_terminal_panes`) so a
/// host call and `sleipnir-ctl ls` cannot drift into two enumerations.
struct ShellWorkspaceIo<'a, 'b> {
    shell: &'a mut AppShell,
    live_panes: &'a [(PaneKey, Entity<TermView>)],
    window: &'a mut Window,
    cx: &'a mut Context<'b, AppShell>,
}

impl ShellWorkspaceIo<'_, '_> {
    /// Resolve a terminal pane, denying panels and missing panes with the same
    /// rules as the control surface. Prefers the shared `live_panes` walk.
    fn terminal_view(&self, pane: PaneKey) -> Result<Entity<TermView>, String> {
        let (terminals, panels) = self.shell.terminal_and_panel_keys();
        crate::plugin_host_calls::read_screen_access(pane, &terminals, &panels)?;
        self.live_panes
            .iter()
            .find(|(key, _)| *key == pane)
            .map(|(_, view)| view.clone())
            .or_else(|| self.shell.view_for_pane(pane))
            .ok_or_else(|| format!("pane {pane} not found"))
    }
}

impl crate::plugin_host_calls::WorkspaceIo for ShellWorkspaceIo<'_, '_> {
    fn list_terminal_panes(&mut self) -> Vec<plugin_protocol::v2::PaneInfo> {
        // `live_panes` already excludes plugin Panel leaves, so no second
        // identity filter is needed. Shares PaneInfo construction with ctl.
        crate::control_surface::pane_infos(self.live_panes, self.cx)
    }

    fn read_screen(&mut self, pane: PaneKey) -> Result<String, String> {
        let view = self.terminal_view(pane)?;
        Ok(view.read(self.cx).visible_screen_text(self.cx))
    }

    fn open_pane(
        &mut self,
        cwd: Option<String>,
        command: Option<crate::plugin_host_calls::OpenCommand>,
    ) -> HostCallResult {
        self.shell
            .execute_open_pane(cwd, command, self.window, self.cx)
    }

    fn scroll_to_run(&mut self, run_id: plugin_protocol::v2::RunId) -> Result<(), String> {
        let pane = if self.cx.has_global::<RunLedgerGlobal>() {
            self.cx
                .global::<RunLedgerGlobal>()
                .snapshot()
                .into_iter()
                .find(|run| run.id == run_id)
                .map(|run| run.pane)
        } else {
            None
        };
        match pane {
            // Same semantics as the ledger-panel row jump: an inferred run has
            // no anchor, so the pane is only focused.
            Some(pane) => {
                self.shell
                    .jump_to_ledger_row(pane, Some(run_id), self.window, self.cx);
                Ok(())
            }
            None => Err(format!("run {run_id} not found")),
        }
    }

    fn focus_pane(&mut self, pane: PaneKey) -> Result<(), String> {
        let (terminals, panels) = self.shell.terminal_and_panel_keys();
        crate::plugin_host_calls::read_screen_access(pane, &terminals, &panels)?;
        self.shell
            .jump_to_ledger_row(pane, None, self.window, self.cx);
        Ok(())
    }

    fn send_text(&mut self, pane: PaneKey, text: String, enter: bool) -> Result<(), String> {
        let view = self.terminal_view(pane)?;
        let delivered = view.update(self.cx, |view, cx| view.insert_text(&text, enter, cx));
        if delivered {
            Ok(())
        } else {
            Err(crate::plugin_host_calls::SEND_TEXT_NO_TERMINAL.into())
        }
    }

    fn send_key(
        &mut self,
        pane: PaneKey,
        key: crate::plugin_host_calls::LogicalKey,
    ) -> Result<(), String> {
        let view = self.terminal_view(pane)?;
        let has_terminal = view.read(self.cx).terminal_entity().is_some();
        let vi_mode = view.read(self.cx).vi_mode_enabled(self.cx);
        crate::plugin_host_calls::send_key_ready(has_terminal, vi_mode)?;
        let delivered = view.update(self.cx, |view, cx| {
            view.send_named_keystroke(key.keystroke_str(), cx)
        });
        if delivered {
            Ok(())
        } else {
            Err(format!("key {} was not delivered", key.keystroke_str()))
        }
    }

    fn request_close_pane(&mut self, pane: PaneKey) -> Result<(), String> {
        let (terminals, panels) = self.shell.terminal_and_panel_keys();
        crate::plugin_host_calls::read_screen_access(pane, &terminals, &panels)?;
        self.shell
            .request_close_terminal_pane(pane, self.window, self.cx)
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
                inferred: run.inferred,
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
        RunEvent::PaneClosed { pane, .. } => Some(HostEvent::PaneClosed { pane: *pane }),
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
        let plugin_protocol::v2::HostEvent::RunStarted {
            command, inferred, ..
        } = host
        else {
            panic!("expected RunStarted");
        };
        assert!(
            !command.contains("supersecret"),
            "plugins must never see the raw command line: {command}"
        );
        assert!(!inferred, "an OSC 133 run is precise, not inferred");
    }

    #[test]
    fn inferred_run_started_event_marks_inferred() {
        let mut ledger = run_ledger::Ledger::new(run_ledger::LaunchId::nil());
        let pane = run_ledger::PaneKey::from_u128(2);
        let event = run_ledger::RunEvent::started_inferred(pane, "make", None, 10);
        ledger.apply(event.clone());
        let host = run_event_to_host(&event, &ledger.snapshot()).expect("mapped");
        assert!(
            matches!(
                host,
                plugin_protocol::v2::HostEvent::RunStarted { inferred: true, .. }
            ),
            "a busy-probe guess must stay distinguishable: {host:?}"
        );
    }

    #[test]
    fn pane_closed_maps_to_a_host_event() {
        let ledger = run_ledger::Ledger::new(run_ledger::LaunchId::nil());
        let pane = run_ledger::PaneKey::from_u128(3);
        let event = run_ledger::RunEvent::PaneClosed { pane, at_ms: 10 };
        let host = run_event_to_host(&event, &ledger.snapshot()).expect("mapped");
        assert_eq!(host, plugin_protocol::v2::HostEvent::PaneClosed { pane });
    }
}
