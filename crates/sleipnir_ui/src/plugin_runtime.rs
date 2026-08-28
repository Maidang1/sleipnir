//! UI adapter for manifest-based external command plugins.

use gpui::{App, Global};
use plugin_grants::{GrantRecord, GrantsFile, Tier};
use plugin_host::resident::{
    BroadcastReport, ConnectionSnapshot, ProcessLauncher, Supervisor, SupervisorConfig, SystemClock,
};
use plugin_host::{
    LoadedPluginCommand, Permission, PluginCatalog, PluginContext, PluginLifecycle, PluginRunOutput,
};
use plugin_protocol::v2::{Capability, HostEvent};
use sleipnir_settings::TerminalSettings;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::TermView;

pub struct PluginRuntime {
    catalog: PluginCatalog,
    supervisor: Supervisor,
}

impl Global for PluginRuntime {}

impl PluginRuntime {
    pub fn init(cx: &mut App) {
        if !cx.has_global::<PluginRuntime>() {
            cx.set_global(PluginRuntime {
                catalog: PluginCatalog::default(),
                supervisor: Supervisor::new(
                    SupervisorConfig::default(),
                    Arc::new(ProcessLauncher),
                    Arc::new(SystemClock),
                ),
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
            "plugin: enabled={} loaded {} command(s): {:?} (allowed_permissions={:?})",
            settings.plugins.enabled,
            catalog.commands.len(),
            catalog
                .commands
                .iter()
                .map(plugin_host::LoadedPluginCommand::qualified_id)
                .collect::<Vec<_>>(),
            settings.plugins.allowed_permissions,
        );
        cx.global_mut::<PluginRuntime>().catalog = catalog;
    }

    pub fn commands(cx: &App) -> Vec<LoadedPluginCommand> {
        if !cx.has_global::<PluginRuntime>() {
            return Vec::new();
        }
        cx.global::<PluginRuntime>().catalog.commands.clone()
    }
}

pub fn allowed_permissions(cx: &App) -> BTreeSet<Permission> {
    TerminalSettings::get_global(cx)
        .plugins
        .allowed_permissions
        .iter()
        .copied()
        .collect()
}

pub fn build_context(
    plugin: &LoadedPluginCommand,
    view: &gpui::Entity<TermView>,
    cx: &App,
) -> PluginContext {
    let permissions = &plugin.command.permissions;
    let view = view.read(cx);
    PluginContext {
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

pub fn apply_output(output: PluginRunOutput, view: &gpui::Entity<TermView>, cx: &mut App) {
    match output {
        PluginRunOutput::Ignored => {}
        PluginRunOutput::Insert(text) => {
            if !text.is_empty() {
                view.update(cx, |view, cx| view.input_bytes(text.into_bytes(), cx));
            }
        }
        PluginRunOutput::Copy(text) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)),
    }
}

/// Capabilities this command is asking for. `Resident` is implied by the
/// manifest lifecycle, not by the v1 permission set.
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

pub fn plugin_binary_hash(plugin: &LoadedPluginCommand) -> Option<String> {
    let path = plugin_binary_path(plugin)?;
    plugin_grants::hash_binary(&path).ok()
}

fn plugin_binary_path(plugin: &LoadedPluginCommand) -> Option<PathBuf> {
    let bin = Path::new(&plugin.binary);
    let candidate =
        if bin.is_absolute() || plugin.binary.contains('/') || plugin.binary.contains('\\') {
            plugin.directory.join(bin)
        } else {
            let next_to_manifest = plugin.directory.join(&plugin.binary);
            if next_to_manifest.is_file() {
                next_to_manifest
            } else {
                PathBuf::from(&plugin.binary)
            }
        };
    candidate.is_file().then_some(candidate)
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

/// Persist a grant bound to `hash`. An empty hash is refused: a grant without
/// binary identity would let a later binary inherit the approval.
pub fn save_grant(plugin_id: &str, request: &[Capability], hash: &str, tier: Tier) {
    if hash.is_empty() {
        log::warn!("plugin: refusing to persist a grant without a binary hash");
        return;
    }
    let path = plugin_grants::default_grants_path();
    let mut file = plugin_grants::load(&path);
    file.grants.insert(
        plugin_id.to_string(),
        GrantRecord {
            granted: request.iter().copied().collect(),
            binary_hash: hash.to_string(),
            granted_at: rfc3339_now(),
            tier,
        },
    );
    if let Err(err) = plugin_grants::save(&path, &file) {
        log::warn!("plugin: failed to save grants: {err}");
    }
}

fn rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

fn rfc3339_from_unix(secs: u64) -> String {
    let mut days = secs / 86_400;
    let rem = secs % 86_400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let mut year = 1970i32;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let feb = if is_leap(year) { 29 } else { 28 };
    let months = [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for dim in months {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    let day = days as u32 + 1;
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_unix_epoch() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1), "1970-01-01T00:00:01Z");
        assert_eq!(rfc3339_from_unix(1_704_067_200), "2024-01-01T00:00:00Z");
    }
}
