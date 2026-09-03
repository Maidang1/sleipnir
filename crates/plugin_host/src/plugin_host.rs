//! Out-of-process plugin supervisor (ADR-0015).
//!
//! Plugins are independent binaries. The host discovers them from `plugin.json`
//! manifests, launches the declared binary as a child process, completes the
//! RPC handshake (`plugin_protocol`), and invokes commands over line-delimited
//! JSON. A plugin runs in its own process, so a crash or panic never unwinds
//! the terminal.
//!
//! `plugin.json` is retained (ADR-0015 decision) so a user can audit a plugin's
//! declared binary, lifecycle, and capabilities *without running it*. The host
//! trusts `plugin.json` for launch and permission decisions; the plugin's own
//! `Ready` manifest is cross-checked but cannot exceed what the manifest and the
//! user's allowlist permit.
//!
//! Lifecycle is declared by the plugin (per manifest):
//! - `on_demand`: launched per invocation, shut down after. Default, strictest.
//! - `resident`: kept connected across invocations. Connection caching, crash
//!   backoff, and teardown live in [`resident`] (ADR-0016).
//!
//! [`resident`] is the v2 supervisor: a wedged plugin must never stall the
//! terminal, which is why stderr is drained, I/O is threaded, and the transport
//! is a trait (so the supervisor is testable without spawning binaries).

use plugin_protocol::{
    Capability, HostMessage, InvokeContext, Output, PROTOCOL_VERSION, PluginMessage,
    versions_compatible,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub mod resident;

/// Default `plugin.json` `api_version` when the field is omitted: v1.
///
/// This is **not** `plugin_protocol::PROTOCOL_VERSION` aliased blindly. The
/// host speaks protocol v2; ADR-0016 §8 keeps v1 manifests loadable for the
/// N / N-1 window. A missing field must keep meaning "v1", otherwise every
/// existing `plugin.json` would silently become a v2 manifest.
pub const PLUGIN_API_VERSION: u32 = 1;
/// Current manifest / protocol version (ADR-0016).
pub const PLUGIN_API_VERSION_V2: u32 = plugin_protocol::v2::PROTOCOL_VERSION;
const MANIFEST_FILE: &str = "plugin.json";

/// True when `plugin.json` `api_version` is in the N / N-1 window.
pub fn api_version_supported(version: u32) -> bool {
    version == PLUGIN_API_VERSION || version == PLUGIN_API_VERSION_V2
}

/// Permission is the user-facing capability name; it maps 1:1 to the v2 wire
/// `Capability`. Kept as its own type so settings/schema stay in this crate.
///
/// The v1 seven are snapshots taken when the user asked. The v2 additions are
/// categorically stronger (ADR-0016 §4) and exist so `plugin.json` can declare
/// them for pre-launch audit — the whole point of keeping the file
/// (ADR-0015). They are never implied by the v1 set.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadSelection,
    ReadVisibleScreen,
    ReadCwd,
    ReadTitle,
    WriteTerminal,
    Clipboard,
    Network,
    /// Process stays up between invocations.
    Resident,
    /// Continuous observation; narrowable with `EventFilter`.
    SubscribeEvents,
    RenderBlock,
    RenderPanel,
    RenderStatus,
    HostCallNotify,
    HostCallReadScreen,
    HostCallListPanes,
    HostCallOpenPane,
    HostCallDrawScene,
}

impl Permission {
    /// Map onto the v1 wire capability. v2-only permissions have no v1
    /// equivalent; `run_command` (the per-invocation path) cannot grant them.
    fn to_capability(self) -> Option<Capability> {
        match self {
            Self::ReadSelection => Some(Capability::ReadSelection),
            Self::ReadVisibleScreen => Some(Capability::ReadVisibleScreen),
            Self::ReadCwd => Some(Capability::ReadCwd),
            Self::ReadTitle => Some(Capability::ReadTitle),
            Self::WriteTerminal => Some(Capability::WriteTerminal),
            Self::Clipboard => Some(Capability::Clipboard),
            Self::Network => Some(Capability::Network),
            Self::Resident
            | Self::SubscribeEvents
            | Self::RenderBlock
            | Self::RenderPanel
            | Self::RenderStatus
            | Self::HostCallNotify
            | Self::HostCallReadScreen
            | Self::HostCallListPanes
            | Self::HostCallOpenPane
            | Self::HostCallDrawScene => None,
        }
    }

    /// Map onto the v2 wire capability. The v1 seven are derived from
    /// [`Permission::to_capability`] plus the shared v1 → v2 conversion in
    /// `plugin_protocol`; the v2-only arms are listed here.
    pub fn to_v2(self) -> plugin_protocol::v2::Capability {
        if let Some(v1) = self.to_capability() {
            return v1.into();
        }
        match self {
            Self::Resident => plugin_protocol::v2::Capability::Resident,
            Self::SubscribeEvents => plugin_protocol::v2::Capability::SubscribeEvents,
            Self::RenderBlock => plugin_protocol::v2::Capability::RenderBlock,
            Self::RenderPanel => plugin_protocol::v2::Capability::RenderPanel,
            Self::RenderStatus => plugin_protocol::v2::Capability::RenderStatus,
            Self::HostCallNotify => plugin_protocol::v2::Capability::HostCallNotify,
            Self::HostCallReadScreen => plugin_protocol::v2::Capability::HostCallReadScreen,
            Self::HostCallListPanes => plugin_protocol::v2::Capability::HostCallListPanes,
            Self::HostCallOpenPane => plugin_protocol::v2::Capability::HostCallOpenPane,
            Self::HostCallDrawScene => plugin_protocol::v2::Capability::HostCallDrawScene,
            v1 => unreachable!("{v1:?} has a v1 capability"),
        }
    }
}

/// The declared lifecycle in `plugin.json`. Distinct type from the wire enum so
/// the manifest schema lives here.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycle {
    #[default]
    OnDemand,
    Resident,
}

/// `plugin.json` (ADR-0015, extended by ADR-0016). Declares the binary,
/// lifecycle, and the capabilities the plugin may request. The host trusts this
/// file for launch and permission decisions; `Ready.requests` cannot exceed it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default = "api_version")]
    pub api_version: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub lifecycle: PluginLifecycle,
    /// Path to the plugin executable. Relative paths resolve from the plugin
    /// directory; bare names resolve via PATH.
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Plugin-level capabilities (ADR-0016). A resident that never contributes
    /// a palette command still has to declare `subscribe_events` / `render_*`
    /// here so a user can audit them without running the binary.
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PluginCommand {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Capabilities this command needs; every one must be in the user allowlist.
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Context the host offers a command. Fields are populated by the caller only
/// when the command holds the matching permission (enforced in the UI adapter);
/// this struct is the transport into `run_command`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginContext {
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub selection: Option<String>,
    pub visible_screen: Option<String>,
}

impl PluginContext {
    fn to_wire(&self) -> InvokeContext {
        InvokeContext {
            cwd: self.cwd.clone(),
            title: self.title.clone(),
            selection: self.selection.clone(),
            visible_screen: self.visible_screen.clone(),
        }
    }
}

/// One discovered plugin, whether or not it contributes palette commands.
///
/// v2 residents can be command-less (they live on events and `Render`). The
/// catalog has to retain them or they are invisible to auto-start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub directory: PathBuf,
}

impl LoadedPlugin {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPluginCommand {
    pub plugin_id: String,
    pub plugin_name: String,
    pub plugin_version: String,
    pub api_version: u32,
    pub lifecycle: PluginLifecycle,
    pub binary: String,
    pub args: Vec<String>,
    pub command: PluginCommand,
    pub directory: PathBuf,
}

impl LoadedPluginCommand {
    pub fn qualified_id(&self) -> String {
        format!("{}.{}", self.plugin_id, self.command.id)
    }
}

#[derive(Clone, Debug, Default)]
pub struct PluginCatalog {
    pub plugins: Vec<LoadedPlugin>,
    pub commands: Vec<LoadedPluginCommand>,
    pub diagnostics: Vec<String>,
}

/// Result of an invocation, routed by the UI adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginRunOutput {
    Ignored,
    Insert(String),
    Copy(String),
}

#[derive(Debug)]
pub enum PluginError {
    InvalidManifest(String),
    PermissionDenied(Permission),
    Io(std::io::Error),
    Protocol(String),
    VersionMismatch { plugin: u32 },
    PluginFailed(String),
    Timeout(Duration),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(m) => write!(f, "invalid plugin manifest: {m}"),
            Self::PermissionDenied(p) => write!(f, "plugin permission denied: {p:?}"),
            Self::Io(e) => write!(f, "plugin I/O failed: {e}"),
            Self::Protocol(m) => write!(f, "plugin protocol error: {m}"),
            Self::VersionMismatch { plugin } => write!(
                f,
                "plugin speaks protocol {plugin}, host speaks {PROTOCOL_VERSION}"
            ),
            Self::PluginFailed(m) => write!(f, "plugin reported failure: {m}"),
            Self::Timeout(d) => write!(f, "plugin timed out after {}s", d.as_secs()),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<std::io::Error> for PluginError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn api_version() -> u32 {
    PLUGIN_API_VERSION
}
fn default_true() -> bool {
    true
}

pub fn default_plugin_dir() -> PathBuf {
    default_plugin_dir_for(cfg!(windows))
}

/// The built-in plugin discovery root for a given OS family. This must mirror
/// `sleipnir_settings::config_dir_for`: on macOS/Unix the config lives under
/// `~/.config/sleipnir` (NOT `dirs::config_dir()`, which is
/// `~/Library/Application Support` on macOS) so plugins are found next to
/// `settings.json`.
pub fn default_plugin_dir_for(windows: bool) -> PathBuf {
    let base = if windows {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir")
    };
    base.join("plugins")
}

pub fn load_catalog(extra_dirs: &[PathBuf]) -> PluginCatalog {
    let mut roots = vec![default_plugin_dir()];
    roots.extend(extra_dirs.iter().cloned());
    load_catalog_from_roots(&roots)
}

pub fn load_catalog_from_roots(roots: &[PathBuf]) -> PluginCatalog {
    let mut catalog = PluginCatalog::default();
    let mut seen_plugins = HashSet::new();
    let mut seen_commands = HashSet::new();

    for root in roots {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                catalog
                    .diagnostics
                    .push(format!("{}: {err}", root.display()));
                continue;
            }
        };
        let mut dirs = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        dirs.sort();

        for directory in dirs {
            let manifest_path = directory.join(MANIFEST_FILE);
            if !manifest_path.is_file() {
                continue;
            }
            match load_manifest(&manifest_path) {
                Ok(manifest) if !manifest.enabled => {}
                Ok(manifest) => {
                    if !seen_plugins.insert(manifest.id.clone()) {
                        catalog
                            .diagnostics
                            .push(format!("duplicate plugin id: {}", manifest.id));
                        continue;
                    }
                    for command in &manifest.commands {
                        let qualified = format!("{}.{}", manifest.id, command.id);
                        if !seen_commands.insert(qualified.clone()) {
                            catalog
                                .diagnostics
                                .push(format!("duplicate plugin command: {qualified}"));
                            continue;
                        }
                        catalog.commands.push(LoadedPluginCommand {
                            plugin_id: manifest.id.clone(),
                            plugin_name: manifest.name.clone(),
                            plugin_version: manifest.version.clone(),
                            api_version: manifest.api_version,
                            lifecycle: manifest.lifecycle,
                            binary: manifest.binary.clone(),
                            args: manifest.args.clone(),
                            command: command.clone(),
                            directory: directory.clone(),
                        });
                    }
                    catalog.plugins.push(LoadedPlugin {
                        manifest,
                        directory: directory.clone(),
                    });
                }
                Err(err) => catalog
                    .diagnostics
                    .push(format!("{}: {err}", manifest_path.display())),
            }
        }
    }
    catalog
}

fn load_manifest(path: &Path) -> Result<PluginManifest, PluginError> {
    let raw = fs::read(path)?;
    let manifest: PluginManifest = serde_json::from_slice(&raw)
        .map_err(|err| PluginError::InvalidManifest(err.to_string()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), PluginError> {
    if !api_version_supported(manifest.api_version) {
        return Err(PluginError::InvalidManifest(format!(
            "unsupported api_version {} (supported: {PLUGIN_API_VERSION}, {PLUGIN_API_VERSION_V2})",
            manifest.api_version
        )));
    }
    validate_id("plugin id", &manifest.id)?;
    if manifest.name.trim().is_empty() || manifest.version.trim().is_empty() {
        return Err(PluginError::InvalidManifest(
            "name and version must not be empty".into(),
        ));
    }
    if manifest.binary.trim().is_empty() {
        return Err(PluginError::InvalidManifest(
            "binary must not be empty".into(),
        ));
    }
    // v1 plugins are palette commands. A v2 resident may have none: it lives
    // on events and Render, and plugin-level `permissions` is the audit.
    if manifest.commands.is_empty()
        && !(manifest.api_version == PLUGIN_API_VERSION_V2
            && manifest.lifecycle == PluginLifecycle::Resident)
    {
        return Err(PluginError::InvalidManifest(
            "at least one command is required".into(),
        ));
    }
    let mut ids = HashSet::new();
    for command in &manifest.commands {
        validate_id("command id", &command.id)?;
        if !ids.insert(command.id.as_str()) {
            return Err(PluginError::InvalidManifest(format!(
                "duplicate command id {}",
                command.id
            )));
        }
        if command.title.trim().is_empty() {
            return Err(PluginError::InvalidManifest(format!(
                "command {} requires a title",
                command.id
            )));
        }
    }
    Ok(())
}

fn validate_id(label: &str, id: &str) -> Result<(), PluginError> {
    let valid = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(PluginError::InvalidManifest(format!(
            "{label} must use lowercase ASCII letters, digits, '-' or '_': {id}"
        )))
    }
}

pub(crate) fn resolve_binary(directory: &Path, binary: &str) -> OsString {
    let path = Path::new(binary);
    if path.is_absolute() || binary.contains('/') || binary.contains('\\') {
        directory.join(path).into_os_string()
    } else {
        OsString::from(binary)
    }
}

/// Launch the plugin, handshake, invoke one command, and route the result.
///
/// This is the v1 per-invocation path: launch → Hello → Ready → Invoke →
/// Invoked → Shutdown. `resident` plugins and v2 sessions are supervised by
/// [`resident`] instead; the UI adapter routes those away before calling this.
pub fn run_command(
    plugin: &LoadedPluginCommand,
    context: &PluginContext,
    allowed_permissions: &BTreeSet<Permission>,
) -> Result<PluginRunOutput, PluginError> {
    // Permission gate: every capability the command declares must be allowed.
    for permission in &plugin.command.permissions {
        if !allowed_permissions.contains(permission) {
            return Err(PluginError::PermissionDenied(*permission));
        }
    }

    let granted: Vec<Capability> = plugin
        .command
        .permissions
        .iter()
        .copied()
        .filter_map(Permission::to_capability)
        .collect();

    let program = resolve_binary(&plugin.directory, &plugin.binary);
    let working_directory = context
        .cwd
        .as_deref()
        .map(Path::new)
        .filter(|p| p.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| plugin.directory.clone());

    let mut child = Command::new(program)
        .args(&plugin.args)
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("SLEIPNIR_PLUGIN_ID", &plugin.plugin_id)
        .env("SLEIPNIR_PLUGIN_COMMAND", &plugin.command.id)
        .env(
            "SLEIPNIR_PLUGIN_API_VERSION",
            PLUGIN_API_VERSION.to_string(),
        )
        .spawn()?;

    let timeout = Duration::from_secs(plugin.command.timeout_secs.unwrap_or(30).clamp(1, 300));

    // The whole RPC session runs on a worker thread so a wedged plugin cannot
    // block the caller past the timeout; on timeout we kill and reap.
    let result = run_session(&mut child, plugin, context, &granted, timeout);

    // Ensure the child is gone regardless of outcome.
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn run_session(
    child: &mut Child,
    plugin: &LoadedPluginCommand,
    context: &PluginContext,
    granted: &[Capability],
    timeout: Duration,
) -> Result<PluginRunOutput, PluginError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| PluginError::Protocol("plugin has no stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PluginError::Protocol("plugin has no stdout".into()))?;

    // Drive the plugin from a worker thread; enforce the timeout on the caller.
    let plugin_id = plugin.plugin_id.clone();
    let command_id = plugin.command.id.clone();
    let context_wire = context.to_wire();
    let granted = granted.to_vec();

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let outcome = drive(
            &mut stdin,
            stdout,
            &plugin_id,
            &command_id,
            context_wire,
            granted,
        );
        let _ = tx.send(outcome);
        // Best-effort shutdown; ignore errors (plugin may already be gone).
        let _ = write_line(&mut stdin, &HostMessage::Shutdown);
    });

    let outcome = match rx.recv_timeout(timeout) {
        Ok(outcome) => outcome,
        Err(_) => {
            // Timeout: kill so the worker's blocking read returns.
            let _ = child.kill();
            let _ = worker.join();
            return Err(PluginError::Timeout(timeout));
        }
    };
    let _ = worker.join();
    outcome
}

/// One synchronous handshake + invoke exchange over the plugin's pipes.
fn drive(
    stdin: &mut std::process::ChildStdin,
    stdout: std::process::ChildStdout,
    _plugin_id: &str,
    command_id: &str,
    context: InvokeContext,
    granted: Vec<Capability>,
) -> Result<PluginRunOutput, PluginError> {
    let mut reader = BufReader::new(stdout);

    write_line(
        stdin,
        &HostMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            granted,
        },
    )?;

    // Expect Ready.
    let ready = read_msg(&mut reader)?;
    match ready {
        PluginMessage::Ready {
            protocol_version, ..
        } => {
            if !versions_compatible(PROTOCOL_VERSION, protocol_version) {
                return Err(PluginError::VersionMismatch {
                    plugin: protocol_version,
                });
            }
        }
        other => {
            return Err(PluginError::Protocol(format!(
                "expected ready, got {other:?}"
            )));
        }
    }

    write_line(
        stdin,
        &HostMessage::Invoke {
            command_id: command_id.to_string(),
            context,
        },
    )?;

    match read_msg(&mut reader)? {
        PluginMessage::Invoked { output } => route(output),
        PluginMessage::Failed { message } => Err(PluginError::PluginFailed(message)),
        other => Err(PluginError::Protocol(format!(
            "expected invoked, got {other:?}"
        ))),
    }
}

fn route(output: Output) -> Result<PluginRunOutput, PluginError> {
    Ok(match output {
        Output::Ignore => PluginRunOutput::Ignored,
        Output::Insert { text } => PluginRunOutput::Insert(text),
        Output::Copy { text } => PluginRunOutput::Copy(text),
    })
}

fn write_line(w: &mut impl Write, msg: &HostMessage) -> Result<(), PluginError> {
    let line = serde_json::to_string(msg).map_err(|e| PluginError::Protocol(e.to_string()))?;
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}

fn read_msg(r: &mut impl BufRead) -> Result<PluginMessage, PluginError> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            return Err(PluginError::Protocol("plugin closed the pipe".into()));
        }
        if line.trim().is_empty() {
            continue;
        }
        return serde_json::from_str(line.trim())
            .map_err(|e| PluginError::Protocol(format!("bad message: {e}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(root: &Path, dir: &str, manifest: &str) {
        let directory = root.join(dir);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(MANIFEST_FILE), manifest).unwrap();
    }

    #[test]
    fn default_plugin_dir_matches_settings_config_dir() {
        // Regression: plugins must be discovered next to settings.json. On
        // macOS/Unix that is ~/.config/sleipnir/plugins, NOT dirs::config_dir()
        // (~/Library/Application Support on macOS). A mismatch here silently
        // loads zero plugins even when settings enable them.
        let unix = default_plugin_dir_for(false);
        assert!(
            unix.ends_with(".config/sleipnir/plugins"),
            "unix plugin dir should be under ~/.config/sleipnir: {unix:?}"
        );
        let win = default_plugin_dir_for(true);
        assert!(win.ends_with("sleipnir/plugins"), "{win:?}");
    }

    #[test]
    fn discovers_commands_in_stable_order() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "b",
            r#"{"id":"beta","name":"Beta","version":"1","binary":"x","commands":[{"id":"run","title":"Run"}]}"#,
        );
        write_plugin(
            temp.path(),
            "a",
            r#"{"id":"alpha","name":"Alpha","version":"1","binary":"x","commands":[{"id":"run","title":"Run"}]}"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        assert_eq!(
            catalog
                .commands
                .iter()
                .map(LoadedPluginCommand::qualified_id)
                .collect::<Vec<_>>(),
            ["alpha.run", "beta.run"]
        );
    }

    #[test]
    fn manifest_requires_a_binary() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "nob",
            r#"{"id":"nob","name":"NoBinary","version":"1","binary":"","commands":[{"id":"run","title":"Run"}]}"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.commands.is_empty());
        assert!(catalog.diagnostics[0].contains("binary"));
    }

    #[test]
    fn lifecycle_defaults_to_on_demand_and_parses_resident() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "res",
            r#"{"id":"res","name":"Res","version":"1","binary":"x","lifecycle":"resident","commands":[{"id":"run","title":"Run"}]}"#,
        );
        write_plugin(
            temp.path(),
            "def",
            r#"{"id":"def","name":"Def","version":"1","binary":"x","commands":[{"id":"run","title":"Run"}]}"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        let res = catalog
            .commands
            .iter()
            .find(|c| c.plugin_id == "res")
            .unwrap();
        let def = catalog
            .commands
            .iter()
            .find(|c| c.plugin_id == "def")
            .unwrap();
        assert_eq!(res.lifecycle, PluginLifecycle::Resident);
        assert_eq!(def.lifecycle, PluginLifecycle::OnDemand);
    }

    #[test]
    fn permission_denied_before_launch() {
        // Binary that does not exist: if the permission gate did not fire first,
        // we would get an Io error instead of PermissionDenied.
        let command = LoadedPluginCommand {
            plugin_id: "x".into(),
            plugin_name: "X".into(),
            plugin_version: "1".into(),
            api_version: PLUGIN_API_VERSION,
            lifecycle: PluginLifecycle::OnDemand,
            binary: "/nonexistent/plugin-binary".into(),
            args: vec![],
            directory: PathBuf::from("."),
            command: PluginCommand {
                id: "run".into(),
                title: "Run".into(),
                description: String::new(),
                keywords: vec![],
                permissions: BTreeSet::from([Permission::Network]),
                timeout_secs: None,
            },
        };
        let err = run_command(&command, &PluginContext::default(), &BTreeSet::new()).unwrap_err();
        assert!(matches!(
            err,
            PluginError::PermissionDenied(Permission::Network)
        ));
    }

    #[test]
    fn v1_manifest_without_api_version_still_loads() {
        // ADR-0016 §8: omitting api_version must keep meaning v1, not silently
        // become v2 when the host's current protocol moves on.
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "alpha",
            r#"{"id":"alpha","name":"Alpha","version":"1","binary":"x","commands":[{"id":"run","title":"Run"}]}"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        assert_eq!(catalog.plugins.len(), 1);
        assert_eq!(catalog.plugins[0].manifest.api_version, PLUGIN_API_VERSION);
        assert_eq!(catalog.commands[0].api_version, PLUGIN_API_VERSION);
    }

    #[test]
    fn v2_manifest_loads_with_new_permissions_and_no_commands() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "failed-run",
            r#"{
                "id":"failed-run",
                "name":"Failed Run",
                "version":"0.1.0",
                "api_version":2,
                "lifecycle":"resident",
                "binary":"./sleipnir-plugin-failed-run",
                "permissions":["subscribe_events","render_block","read_cwd"]
            }"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        assert_eq!(catalog.plugins.len(), 1);
        assert!(
            catalog.commands.is_empty(),
            "a command-less resident has no palette entries"
        );
        let plugin = &catalog.plugins[0];
        assert_eq!(plugin.manifest.api_version, PLUGIN_API_VERSION_V2);
        assert_eq!(plugin.manifest.lifecycle, PluginLifecycle::Resident);
        assert!(
            plugin
                .manifest
                .permissions
                .contains(&Permission::SubscribeEvents)
        );
        assert!(
            plugin
                .manifest
                .permissions
                .contains(&Permission::RenderBlock)
        );
        assert!(plugin.manifest.permissions.contains(&Permission::ReadCwd));
        let caps = crate::resident::declared_capabilities(&plugin.manifest);
        assert!(caps.contains(&plugin_protocol::v2::Capability::SubscribeEvents));
        assert!(caps.contains(&plugin_protocol::v2::Capability::RenderBlock));
        assert!(caps.contains(&plugin_protocol::v2::Capability::ReadCwd));
        assert!(caps.contains(&plugin_protocol::v2::Capability::Resident));
    }

    #[test]
    fn v2_host_call_permissions_parse() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "calls",
            r#"{
                "id":"calls",
                "name":"Calls",
                "version":"1",
                "api_version":2,
                "lifecycle":"resident",
                "binary":"x",
                "permissions":[
                    "resident",
                    "render_panel",
                    "render_status",
                    "host_call_notify",
                    "host_call_read_screen",
                    "host_call_list_panes",
                    "host_call_open_pane"
                ]
            }"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
        let perms = &catalog.plugins[0].manifest.permissions;
        assert!(perms.contains(&Permission::HostCallNotify));
        assert!(perms.contains(&Permission::HostCallOpenPane));
        assert!(perms.contains(&Permission::RenderPanel));
        assert!(perms.contains(&Permission::RenderStatus));
    }

    #[test]
    fn unsupported_api_version_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "future",
            r#"{"id":"future","name":"Future","version":"1","api_version":3,"binary":"x","commands":[{"id":"run","title":"Run"}]}"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.plugins.is_empty());
        assert!(
            catalog.diagnostics[0].contains("unsupported api_version 3"),
            "{:?}",
            catalog.diagnostics
        );
    }

    #[test]
    fn v1_manifest_still_requires_a_command() {
        let temp = tempfile::TempDir::new().unwrap();
        write_plugin(
            temp.path(),
            "empty",
            r#"{"id":"empty","name":"Empty","version":"1","binary":"x"}"#,
        );
        let catalog = load_catalog_from_roots(&[temp.path().to_path_buf()]);
        assert!(catalog.plugins.is_empty());
        assert!(
            catalog.diagnostics[0].contains("at least one command"),
            "{:?}",
            catalog.diagnostics
        );
    }
}
