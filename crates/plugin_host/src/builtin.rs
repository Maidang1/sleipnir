//! First-party plugins compiled into the terminal executable. No user files,
//! PATH lookup, binary copies, or persisted grants participate in their trust.

use std::path::Path;

use crate::{LoadedPlugin, LoadedPluginCommand, PluginCatalog, PluginManifest, PluginSource};

pub const AGENTS_ID: &str = "agents";
pub const AGENTS_ARGUMENT: &str = "--builtin-agents";

/// Load built-ins first and optionally discover external plugins. The Agents
/// id is reserved even when disabled so a local manifest cannot impersonate
/// the built-in or silently undo the opt-out.
pub fn load_catalog(
    executable: &Path,
    agents_enabled: bool,
    external_roots: Option<&[std::path::PathBuf]>,
) -> PluginCatalog {
    let mut catalog = PluginCatalog::default();
    if agents_enabled {
        match agents_plugin(executable) {
            Ok(plugin) => {
                catalog
                    .commands
                    .extend(
                        plugin
                            .manifest
                            .commands
                            .iter()
                            .map(|command| LoadedPluginCommand {
                                source: plugin.source,
                                plugin_id: plugin.manifest.id.clone(),
                                plugin_name: plugin.manifest.name.clone(),
                                plugin_version: plugin.manifest.version.clone(),
                                lifecycle: plugin.manifest.lifecycle,
                                binary: plugin.manifest.binary.clone(),
                                resolved_binary: plugin.resolved_binary.clone(),
                                args: plugin.manifest.args.clone(),
                                command: command.clone(),
                                directory: plugin.directory.clone(),
                            }),
                    );
                catalog.plugins.push(plugin);
            }
            Err(err) => catalog.diagnostics.push(format!("built-in Agents: {err}")),
        }
    }
    if let Some(roots) = external_roots {
        merge_external(&mut catalog, crate::load_catalog(roots));
    }
    catalog
}

fn merge_external(catalog: &mut PluginCatalog, mut external: PluginCatalog) {
    if external
        .plugins
        .iter()
        .any(|plugin| plugin.id() == AGENTS_ID)
    {
        catalog
            .diagnostics
            .push("ignoring external plugin 'agents': id reserved for built-in Agents".into());
    }
    external.plugins.retain(|plugin| plugin.id() != AGENTS_ID);
    external
        .commands
        .retain(|command| command.plugin_id != AGENTS_ID);
    catalog.plugins.extend(external.plugins);
    catalog.commands.extend(external.commands);
    catalog.diagnostics.extend(external.diagnostics);
}

fn agents_plugin(executable: &Path) -> Result<LoadedPlugin, crate::PluginError> {
    if !executable.is_absolute() {
        return Err(crate::PluginError::InvalidManifest(
            "terminal executable must be absolute".into(),
        ));
    }
    // The manifest is part of the application, not read from its installation
    // directory. Keep the audit set identical to the standalone plugin.
    let mut manifest: PluginManifest =
        serde_json::from_str(include_str!("../../sleipnir_plugin_agents/plugin.json"))
            .map_err(|err| crate::PluginError::InvalidManifest(err.to_string()))?;
    manifest.binary = executable.to_string_lossy().into_owned();
    manifest.args = vec![AGENTS_ARGUMENT.into()];
    crate::validate_manifest(&manifest)?;
    Ok(LoadedPlugin {
        source: PluginSource::BuiltInAgents,
        manifest,
        directory: executable.parent().unwrap_or(executable).to_path_buf(),
        resolved_binary: executable.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resident::declared_capabilities;
    use plugin_protocol::v2::Capability;

    #[test]
    fn builtins_need_no_directory_manifest_or_path_lookup() {
        let executable = std::env::current_exe().unwrap();
        let catalog = load_catalog(&executable, true, None);
        assert!(catalog.diagnostics.is_empty());
        assert_eq!(catalog.plugins.len(), 1);
        let plugin = &catalog.plugins[0];
        assert_eq!(plugin.source, PluginSource::BuiltInAgents);
        assert_eq!(plugin.resolved_binary, executable);
        assert_eq!(plugin.manifest.args, [AGENTS_ARGUMENT]);
        assert_eq!(catalog.commands.len(), 1);
        assert_eq!(catalog.commands[0].qualified_id(), "agents.open");
        let caps = declared_capabilities(&plugin.manifest);
        for cap in [
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::HostCallOpenPane,
            Capability::HostCallSendText,
            Capability::HostCallSendKey,
            Capability::HostCallFocusPane,
            Capability::HostCallRequestClosePane,
        ] {
            assert!(caps.contains(&cap));
        }
        assert!(!caps.contains(&Capability::Network));
        assert!(!caps.contains(&Capability::HostCallReadScreen));
    }

    #[test]
    fn builtin_opt_out_is_independent_of_external_discovery() {
        let executable = std::env::current_exe().unwrap();
        let catalog = load_catalog(&executable, false, None);
        assert!(catalog.plugins.is_empty());
        assert!(catalog.commands.is_empty());
    }

    #[test]
    fn local_manifests_cannot_replace_or_reenable_the_builtin() {
        let root = tempfile::tempdir().unwrap();
        for id in ["agents", "demo"] {
            let dir = root.path().join(id);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("plugin.json"), format!(
                r#"{{"id":"{id}","name":"Local","version":"1","api_version":2,"lifecycle":"resident","binary":"./plugin","commands":[{{"id":"open","title":"Open"}}]}}"#,
            )).unwrap();
            std::fs::copy(std::env::current_exe().unwrap(), dir.join("plugin")).unwrap();
        }
        let external = crate::load_catalog_from_roots(&[root.path().to_path_buf()]);
        assert_eq!(external.plugins.len(), 2);
        assert!(
            external
                .plugins
                .iter()
                .all(|p| p.source == PluginSource::External)
        );
        for enabled in [true, false] {
            let mut catalog = load_catalog(&std::env::current_exe().unwrap(), enabled, None);
            merge_external(&mut catalog, external.clone());
            assert_eq!(catalog.plugins.len(), if enabled { 2 } else { 1 });
            assert_eq!(
                catalog
                    .commands
                    .iter()
                    .filter(|c| c.plugin_id == AGENTS_ID)
                    .count(),
                usize::from(enabled)
            );
            assert!(
                catalog
                    .plugins
                    .iter()
                    .filter(|p| p.id() == AGENTS_ID)
                    .all(|p| p.source == PluginSource::BuiltInAgents)
            );
            assert!(!catalog.diagnostics.is_empty());
        }
    }

    #[test]
    fn provenance_cannot_be_declared_in_json() {
        assert!(
            serde_json::from_str::<PluginManifest>(
                r#"{
            "id":"agents","name":"Agents","version":"1","api_version":2,
            "lifecycle":"resident","binary":"evil","source":"BuiltInAgents"
        }"#
            )
            .is_err()
        );
    }
}
