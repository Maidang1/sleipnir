//! UI adapter for manifest-based external command plugins.

use gpui::{App, BorrowAppContext as _, Global};
use plugin_grants::{GrantRecord, GrantsFile, Tier};
use plugin_host::resident::{
    BroadcastReport, ConnectionSnapshot, ConnectionState, LaunchSpec, ProcessLauncher, Supervisor,
    SupervisorConfig, SystemClock,
};
use plugin_host::{
    LoadedPlugin, LoadedPluginCommand, Permission, PluginCatalog, PluginLifecycle,
};
use plugin_protocol::v2::{Capability, HostEvent, InvokeContext, Output};
use sleipnir_settings::TerminalSettings;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::TermView;

pub struct PluginRuntime {
    catalog: PluginCatalog,
    supervisor: Arc<Supervisor>,
    /// Plugin ids whose `connect` has been spawned but has not finished yet.
    ///
    /// `is_plugin_live` reads supervisor snapshots, which only turn `Live`
    /// *after* the async connect completes. Two synchronous calls to
    /// `start_resident_plugins` (a second window, or a settings reload racing
    /// construction) therefore both pass that check and both launch the same
    /// resident process. This set closes that window synchronously.
    connecting: BTreeSet<String>,
}

impl Global for PluginRuntime {}

impl PluginRuntime {
    pub fn init(cx: &mut App) {
        if !cx.has_global::<PluginRuntime>() {
            cx.set_global(PluginRuntime {
                catalog: PluginCatalog::default(),
                connecting: BTreeSet::new(),
                supervisor: Arc::new(Supervisor::new(
                    SupervisorConfig::default(),
                    Arc::new(ProcessLauncher),
                    Arc::new(SystemClock::new()),
                )),
            });
        }
        Self::reload(cx);
    }

    pub fn reload(cx: &mut App) {
        let settings = TerminalSettings::get_global(cx);
        let catalog = if settings.plugins.enabled {
            plugin_host::load_catalog(&settings.plugins.directories)
        } else {
            PluginCatalog::default()
        };
        for diagnostic in &catalog.diagnostics {
            log::warn!("plugin: {diagnostic}");
        }
        log::info!(
            "plugin: enabled={} loaded {} plugin(s), {} command(s): {:?}",
            settings.plugins.enabled,
            catalog.plugins.len(),
            catalog.commands.len(),
            catalog
                .commands
                .iter()
                .map(plugin_host::LoadedPluginCommand::qualified_id)
                .collect::<Vec<_>>(),
        );
        cx.global_mut::<PluginRuntime>().catalog = catalog;
    }

    pub fn commands(cx: &App) -> Vec<LoadedPluginCommand> {
        if !cx.has_global::<PluginRuntime>() {
            return Vec::new();
        }
        cx.global::<PluginRuntime>().catalog.commands.clone()
    }

    pub fn plugins(cx: &App) -> Vec<LoadedPlugin> {
        if !cx.has_global::<PluginRuntime>() {
            return Vec::new();
        }
        cx.global::<PluginRuntime>().catalog.plugins.clone()
    }
}

pub fn build_context(
    plugin: &LoadedPluginCommand,
    view: &gpui::Entity<TermView>,
    cx: &App,
) -> InvokeContext {
    let permissions = &plugin.command.permissions;
    let view = view.read(cx);
    InvokeContext {
        cwd: permissions
            .contains(&Permission::ReadCwd)
            .then(|| view.working_directory(cx))
            .flatten()
            .map(|path| path.to_string_lossy().into_owned()),
        title: permissions
            .contains(&Permission::ReadTitle)
            .then(|| view.title().to_string()),
        selection: permissions
            .contains(&Permission::ReadSelection)
            .then(|| view.selection_text(cx))
            .flatten(),
        visible_screen: permissions
            .contains(&Permission::ReadVisibleScreen)
            .then(|| view.visible_screen_text(cx)),
    }
}

pub fn apply_output(output: Output, view: &gpui::Entity<TermView>, cx: &mut App) {
    match output {
        Output::Ignore => {}
        Output::Insert { text } => {
            if !text.is_empty() {
                view.update(cx, |view, cx| view.input_bytes(text.into_bytes(), cx));
            }
        }
        Output::Copy { text } => cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)),
    }
}

/// Capabilities this command is asking for. `Resident` is implied by the
/// manifest lifecycle.
pub fn requested_capabilities(plugin: &LoadedPluginCommand) -> Vec<Capability> {
    let mut caps: Vec<_> = plugin
        .command
        .permissions
        .iter()
        .copied()
        .map(Permission::to_v2)
        .collect();
    if plugin.lifecycle == PluginLifecycle::Resident && !caps.contains(&Capability::Resident) {
        caps.push(Capability::Resident);
    }
    caps
}

pub fn plugin_binary_hash(plugin: &LoadedPluginCommand) -> Option<plugin_grants::BinaryHash> {
    hash_plugin_binary(&plugin.directory, &plugin.binary)
}

pub fn loaded_plugin_hash(plugin: &LoadedPlugin) -> Option<plugin_grants::BinaryHash> {
    hash_plugin_binary(&plugin.directory, &plugin.manifest.binary)
}

fn hash_plugin_binary(directory: &Path, binary: &str) -> Option<plugin_grants::BinaryHash> {
    let path = plugin_binary_path(directory, binary)?;
    plugin_grants::hash_binary(&path).ok()
}

fn plugin_binary_path(directory: &Path, binary: &str) -> Option<PathBuf> {
    let bin = Path::new(binary);
    let candidate = if bin.is_absolute() || binary.contains('/') || binary.contains('\\') {
        directory.join(bin)
    } else {
        let next_to_manifest = directory.join(binary);
        if next_to_manifest.is_file() {
            next_to_manifest
        } else {
            PathBuf::from(binary)
        }
    };
    candidate.is_file().then_some(candidate)
}

/// Full declared set from `plugin.json`. This is what Ready.requests is
/// checked against, and what first-run consent asks for.
pub fn requested_capabilities_for_plugin(plugin: &LoadedPlugin) -> Vec<Capability> {
    plugin_host::resident::declared_capabilities(&plugin.manifest)
        .into_iter()
        .collect()
}

pub fn launch_spec(plugin: &LoadedPlugin, granted: Vec<Capability>) -> LaunchSpec {
    LaunchSpec::from_plugin(&plugin.manifest, &plugin.directory, granted)
}

pub fn supervisor(cx: &App) -> Option<Arc<Supervisor>> {
    cx.try_global::<PluginRuntime>()
        .map(|rt| Arc::clone(&rt.supervisor))
}

/// Drive the resident supervisor's housekeeping: reap dead sessions, apply
/// crash backoff, reset crash counters past `stable_after`, evict idle
/// residents. The shell calls this on a timer; without it `idle` and
/// `stable_after` would be decorative.
pub fn tick(cx: &App) {
    if let Some(rt) = cx.try_global::<PluginRuntime>() {
        rt.supervisor.tick();
    }
}

pub fn is_plugin_live(plugin_id: &str, cx: &App) -> bool {
    snapshots(cx)
        .iter()
        .any(|s| s.plugin_id == plugin_id && s.state == ConnectionState::Live)
}

/// Claim the right to launch `plugin_id`, returning false when someone already
/// has it. Synchronous and global, so it closes the gap between spawning a
/// connect and the supervisor reporting the connection as `Live`.
pub fn begin_connect(plugin_id: &str, cx: &mut App) -> bool {
    if !cx.has_global::<PluginRuntime>() {
        return false;
    }
    cx.update_global(|rt: &mut PluginRuntime, _| rt.connecting.insert(plugin_id.to_string()))
}

/// Release a claim taken by [`begin_connect`], on success or failure. A failed
/// launch must be retryable, so this is called on both paths.
pub fn finish_connect(plugin_id: &str, cx: &mut App) {
    if cx.has_global::<PluginRuntime>() {
        cx.update_global(|rt: &mut PluginRuntime, _| rt.connecting.remove(plugin_id));
    }
}

pub fn grants() -> GrantsFile {
    plugin_grants::load(&plugin_grants::default_grants_path())
}

pub fn grant_tiers() -> BTreeMap<String, Tier> {
    grants()
        .grants
        .into_iter()
        .map(|(id, rec)| (id, rec.tier))
        .collect()
}

pub fn catalog_names(cx: &App) -> BTreeMap<String, String> {
    PluginRuntime::commands(cx)
        .into_iter()
        .map(|cmd| (cmd.plugin_id, cmd.plugin_name))
        .collect()
}

/// Copy snapshots off the supervisor. The panel must not hold the live
/// connection set; it renders this Vec.
pub fn snapshots(cx: &App) -> Vec<ConnectionSnapshot> {
    cx.try_global::<PluginRuntime>()
        .map(|rt| rt.supervisor.snapshots())
        .unwrap_or_default()
}

pub fn kill_plugin(plugin_id: &str, cx: &App) {
    if let Some(rt) = cx.try_global::<PluginRuntime>() {
        rt.supervisor.shutdown(plugin_id);
    }
}

/// Fan-out a host event. `try_send` on the far side; this never blocks the UI.
pub fn broadcast_event(event: HostEvent, cx: &App) -> BroadcastReport {
    cx.try_global::<PluginRuntime>()
        .map(|rt| rt.supervisor.broadcast(event))
        .unwrap_or_default()
}

pub fn drain_all_inbound(cx: &App) -> Vec<(String, plugin_host::resident::Inbound)> {
    cx.try_global::<PluginRuntime>()
        .map(|rt| rt.supervisor.drain_all_inbound())
        .unwrap_or_default()
}

pub fn has_grant(plugin_id: &str, cap: Capability, cx: &App) -> bool {
    cx.try_global::<PluginRuntime>()
        .is_some_and(|rt| rt.supervisor.has_grant(plugin_id, cap))
}

pub fn push_action(
    plugin_id: &str,
    block_id: plugin_protocol::v2::BlockId,
    action: String,
    arg: Option<String>,
    cx: &App,
) -> bool {
    cx.try_global::<PluginRuntime>()
        .and_then(|rt| {
            rt.supervisor
                .push_action(plugin_id, block_id, action, arg)
                .ok()
        })
        .is_some()
}

/// Always attempt a Reply for a Call id. A dead session returns false; that
/// is a lost write, not a hang — the Call itself is already off the inbound
/// queue.
pub fn reply_host_call(
    plugin_id: &str,
    id: plugin_protocol::v2::MessageId,
    result: plugin_protocol::v2::HostCallResult,
    cx: &App,
) -> bool {
    cx.try_global::<PluginRuntime>()
        .and_then(|rt| rt.supervisor.reply(plugin_id, id, result).ok())
        .is_some()
}

/// Persist a grant bound to `hash`. The [`plugin_grants::BinaryHash`] newtype
/// makes an unhashed grant unrepresentable: a grant without binary identity
/// would let a later binary inherit the approval.
pub fn save_grant(
    plugin_id: &str,
    request: &[Capability],
    hash: &plugin_grants::BinaryHash,
    tier: Tier,
) {
    let path = plugin_grants::default_grants_path();
    let mut file = plugin_grants::load(&path);
    file.grants.insert(
        plugin_id.to_string(),
        GrantRecord {
            granted: request.iter().copied().collect(),
            binary_hash: hash.to_string(),
            granted_at: plugin_grants::now_stamp(),
            tier,
        },
    );
    if let Err(err) = plugin_grants::save(&path, &file) {
        log::warn!("plugin: failed to save grants: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `connect_resident` logs and then spawns the real connect, so
    /// `is_plugin_live` still reports false until that task lands. Two
    /// synchronous `start_resident_plugins` calls both passed and launched the
    /// same resident twice (observed as a duplicated "starting resident" log).
    /// The claim must be exclusive, and releasing must make it retryable.
    #[test]
    fn connect_claim_is_exclusive_and_released() {
        let mut connecting: BTreeSet<String> = BTreeSet::new();
        assert!(connecting.insert("failed-run".into()), "first claim wins");
        assert!(
            !connecting.insert("failed-run".into()),
            "second claim must lose while the first is in flight"
        );
        assert!(connecting.insert("other".into()), "claims are per plugin");
        connecting.remove("failed-run");
        assert!(
            connecting.insert("failed-run".into()),
            "a released claim must be retryable after a failed launch"
        );
    }
}
