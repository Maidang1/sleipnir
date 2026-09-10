//! UI adapter for manifest-based external command plugins.

use gpui::{App, BorrowAppContext as _, Global, Task};
use plugin_grants::{GrantRecord, GrantsFile, Tier};
use plugin_host::resident::{
    BroadcastReport, ConnectionSnapshot, ConnectionState, LaunchSpec, ProcessLauncher, Supervisor,
    SupervisorConfig, SystemClock,
};
use plugin_host::{LoadedPlugin, LoadedPluginCommand, Permission, PluginCatalog, PluginLifecycle};
use plugin_protocol::v2::{Capability, HostEvent, InvokeContext, Output};
use sleipnir_settings::TerminalSettings;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::TermView;

type CatalogRevision = Vec<(
    LoadedPlugin,
    Option<plugin_grants::BinaryHash>,
    Option<GrantRecord>,
)>;

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
    revision: CatalogRevision,
    calls: crate::plugin_host_calls::HostCallLimiter,
    _pump: Task<()>,
    _housekeeping: Task<()>,
}

impl Global for PluginRuntime {}

impl PluginRuntime {
    pub fn init(cx: &mut App) {
        if !cx.has_global::<PluginRuntime>() {
            cx.set_global(PluginRuntime {
                catalog: PluginCatalog::default(),
                connecting: BTreeSet::new(),
                revision: Vec::new(),
                calls: crate::plugin_host_calls::HostCallLimiter::new(),
                _pump: Task::ready(()),
                _housekeeping: Task::ready(()),
                supervisor: Arc::new(Supervisor::new(
                    SupervisorConfig::default(),
                    Arc::new(ProcessLauncher),
                    Arc::new(SystemClock::new()),
                )),
            });
            Self::reload(cx);
            let pump = cx.spawn(async move |cx| {
                let mut dispatcher = crate::plugin_dispatch::PluginDispatcher::default();
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    cx.update(|cx| dispatcher.pump(cx));
                }
            });
            let housekeeping = cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if let Some(supervisor) = cx.update(|cx| supervisor(cx)) {
                        cx.background_executor()
                            .spawn(async move { supervisor.tick() })
                            .await;
                    }
                }
            });
            let runtime = cx.global_mut::<PluginRuntime>();
            runtime._pump = pump;
            runtime._housekeeping = housekeeping;
        }
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
        let grants = grants();
        let revision = catalog
            .plugins
            .iter()
            .map(|plugin| {
                (
                    plugin.clone(),
                    loaded_plugin_hash(plugin),
                    grants.grants.get(plugin.id()).cloned(),
                )
            })
            .collect();
        let retired = cx
            .global_mut::<PluginRuntime>()
            .replace_catalog(catalog, revision);
        if let Some(retired) = retired {
            cx.background_executor()
                .spawn(async move { retired.shutdown_all() })
                .detach();
        }
    }

    fn replace_catalog(
        &mut self,
        catalog: PluginCatalog,
        revision: CatalogRevision,
    ) -> Option<Arc<Supervisor>> {
        self.catalog = catalog;
        if self.revision == revision {
            return None;
        }
        self.revision = revision;
        self.connecting.clear();
        self.calls = crate::plugin_host_calls::HostCallLimiter::new();
        let retired = std::mem::replace(
            &mut self.supervisor,
            Arc::new(Supervisor::new(
                SupervisorConfig::default(),
                Arc::new(ProcessLauncher),
                Arc::new(SystemClock::new()),
            )),
        );
        retired.close();
        Some(retired)
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

pub fn apply_output(
    output: Output,
    granted: &[Capability],
    view: &gpui::Entity<TermView>,
    cx: &mut App,
) {
    if output
        .required_capability()
        .is_some_and(|capability| !granted.contains(&capability))
    {
        log::warn!("plugin output denied: missing grant");
        return;
    }
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
    hash_plugin_binary(&plugin.resolved_binary)
}

pub fn loaded_plugin_hash(plugin: &LoadedPlugin) -> Option<plugin_grants::BinaryHash> {
    hash_plugin_binary(&plugin.resolved_binary)
}

fn hash_plugin_binary(path: &Path) -> Option<plugin_grants::BinaryHash> {
    plugin_grants::hash_binary(path).ok()
}

/// Full declared set from `plugin.json`. This is what Ready.requests is
/// checked against, and what first-run consent asks for.
pub fn requested_capabilities_for_plugin(plugin: &LoadedPlugin) -> Vec<Capability> {
    plugin_host::resident::declared_capabilities(&plugin.manifest)
        .into_iter()
        .collect()
}

pub fn launch_spec(plugin: &LoadedPlugin, granted: Vec<Capability>) -> LaunchSpec {
    LaunchSpec::from_plugin(
        &plugin.manifest,
        &plugin.resolved_binary,
        &plugin.directory,
        granted,
    )
}

pub fn supervisor(cx: &App) -> Option<Arc<Supervisor>> {
    cx.try_global::<PluginRuntime>()
        .map(|rt| Arc::clone(&rt.supervisor))
}

pub fn is_current(supervisor: &Arc<Supervisor>, cx: &App) -> bool {
    cx.try_global::<PluginRuntime>()
        .is_some_and(|runtime| Arc::ptr_eq(&runtime.supervisor, supervisor))
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
pub fn finish_connect(plugin_id: &str, supervisor: &Arc<Supervisor>, cx: &mut App) {
    if is_current(supervisor, cx) {
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
        let sessions = rt.supervisor.disconnect(plugin_id);
        if !sessions.is_empty() {
            let grace = rt.supervisor.config().shutdown_grace;
            cx.background_executor()
                .spawn(async move {
                    for session in sessions {
                        session.teardown(grace);
                    }
                })
                .detach();
        }
    }
}

/// Fan-out a host event. `try_send` on the far side; this never blocks the UI.
pub fn broadcast_event(event: HostEvent, cx: &App) -> BroadcastReport {
    cx.try_global::<PluginRuntime>()
        .map(|rt| rt.supervisor.broadcast(event))
        .unwrap_or_default()
}

pub fn has_grant_for_instance(instance_id: uuid::Uuid, cap: Capability, cx: &App) -> bool {
    cx.try_global::<PluginRuntime>()
        .is_some_and(|rt| rt.supervisor.has_grant_for_instance(instance_id, cap))
}

pub fn plan_host_call(
    plugin_id: &str,
    call: &plugin_protocol::v2::HostCall,
    granted: &[Capability],
    now_ms: u64,
    cx: &mut App,
) -> crate::plugin_host_calls::CallPlan {
    crate::plugin_host_calls::plan_call(
        plugin_id,
        call,
        granted,
        &mut cx.global_mut::<PluginRuntime>().calls,
        now_ms,
    )
}

pub fn dropped_calls(cx: &App) -> BTreeMap<String, u64> {
    cx.global::<PluginRuntime>().calls.dropped_counts().clone()
}

pub fn push_action(
    instance_id: uuid::Uuid,
    block_id: plugin_protocol::v2::BlockId,
    action: String,
    arg: Option<String>,
    cx: &App,
) -> bool {
    cx.try_global::<PluginRuntime>()
        .and_then(|rt| {
            rt.supervisor
                .push_action_to_instance(instance_id, block_id, action, arg)
                .ok()
        })
        .is_some()
}

pub fn reply_host_call_to_instance(
    instance_id: uuid::Uuid,
    id: plugin_protocol::v2::MessageId,
    result: plugin_protocol::v2::HostCallResult,
    cx: &App,
) -> bool {
    cx.try_global::<PluginRuntime>()
        .and_then(|rt| {
            rt.supervisor
                .reply_to_instance(instance_id, id, result.clone())
                .ok()
        })
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
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn runtime_with_plugin() -> PluginRuntime {
        let resolved_binary = PathBuf::from("/resolved/demo");
        let plugin = LoadedPlugin {
            manifest: serde_json::from_str(r#"{"id":"demo","name":"Demo","version":"1","api_version":2,"lifecycle":"resident","binary":"./demo"}"#).unwrap(),
            directory: PathBuf::from("demo"),
            resolved_binary,
        };
        PluginRuntime {
            catalog: PluginCatalog {
                plugins: vec![plugin.clone()],
                ..PluginCatalog::default()
            },
            supervisor: Arc::new(Supervisor::new(
                SupervisorConfig::for_tests(),
                Arc::new(ProcessLauncher),
                Arc::new(SystemClock::new()),
            )),
            connecting: BTreeSet::from(["demo".into()]),
            revision: vec![(
                plugin,
                Some(plugin_grants::BinaryHash::from_raw(
                    "sha256:original".into(),
                )),
                None,
            )],
            calls: crate::plugin_host_calls::HostCallLimiter::new(),
            _pump: Task::ready(()),
            _housekeeping: Task::ready(()),
        }
    }

    #[test]
    fn launch_spec_uses_resolved_binary_path() {
        let runtime = runtime_with_plugin();
        let launch = launch_spec(&runtime.catalog.plugins[0], vec![Capability::Resident]);
        assert_eq!(launch.binary, OsString::from("/resolved/demo"));
    }

    #[test]
    fn disabling_plugins_retires_the_supervisor_and_pending_launches() {
        let mut runtime = runtime_with_plugin();
        let launch = launch_spec(&runtime.catalog.plugins[0], vec![]);
        let previous = Arc::clone(&runtime.supervisor);
        let retired = runtime
            .replace_catalog(PluginCatalog::default(), Vec::new())
            .unwrap();
        assert!(Arc::ptr_eq(&previous, &retired));
        assert!(!Arc::ptr_eq(&previous, &runtime.supervisor));
        assert!(runtime.connecting.is_empty());
        assert!(runtime.catalog.plugins.is_empty());
        assert!(matches!(
            previous.connect(&launch),
            Err(plugin_host::resident::SessionError::Disconnected)
        ));
    }

    #[test]
    fn unchanged_reload_preserves_sessions_but_binary_and_grant_changes_retire_them() {
        let mut runtime = runtime_with_plugin();
        let catalog = runtime.catalog.clone();
        let revision = runtime.revision.clone();
        let previous = Arc::clone(&runtime.supervisor);
        assert!(
            runtime
                .replace_catalog(catalog.clone(), revision.clone())
                .is_none()
        );
        assert!(Arc::ptr_eq(&previous, &runtime.supervisor));
        let mut changed = revision;
        changed[0].1 = Some(plugin_grants::BinaryHash::from_raw("sha256:updated".into()));
        assert!(
            runtime
                .replace_catalog(catalog.clone(), changed.clone())
                .is_some()
        );
        changed[0].2 = Some(GrantRecord {
            granted: BTreeSet::from([Capability::Resident]),
            binary_hash: "sha256:updated".into(),
            granted_at: String::new(),
            tier: Tier::Local,
        });
        assert!(runtime.replace_catalog(catalog, changed).is_some());
    }

    /// Regression: `connect_resident` logs and then spawns the real connect, so
    /// `is_plugin_live` still reports false until that task lands. Two
    /// synchronous `start_resident_plugins` calls both passed and launched the
    /// same resident twice (observed as a duplicated "starting resident" log).
    /// The claim must be exclusive, and releasing must make it retryable.
    #[test]
    fn connect_claim_is_exclusive_and_released() {
        let mut connecting: BTreeSet<String> = BTreeSet::new();
        assert!(connecting.insert("demo".into()), "first claim wins");
        assert!(
            !connecting.insert("demo".into()),
            "second claim must lose while the first is in flight"
        );
        assert!(connecting.insert("other".into()), "claims are per plugin");
        connecting.remove("demo");
        assert!(
            connecting.insert("demo".into()),
            "a released claim must be retryable after a failed launch"
        );
    }
}
