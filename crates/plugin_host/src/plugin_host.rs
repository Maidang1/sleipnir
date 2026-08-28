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
//! - `resident`: v1 [`run_command`] still launches per invocation. Connection
//!   caching, crash backoff, and teardown live in [`resident`] (ADR-0016).
//!
//! [`resident`] is the v2 supervisor: a wedged plugin must never stall the
//! terminal, which is why stderr is drained, I/O is threaded, and the transport
//! is a trait (so the supervisor is testable without spawning binaries).

use plugin_protocol::{
    Capability, HostMessage, InvokeContext, Lifecycle, Output, PROTOCOL_VERSION, PluginMessage,
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

pub const PLUGIN_API_VERSION: u32 = PROTOCOL_VERSION;
const MANIFEST_FILE: &str = "plugin.json";

/// Permission is the user-facing capability name; it maps 1:1 to the wire
/// `Capability`. Kept as its own type so settings/schema stay in this crate.
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
}

impl Permission {
    fn to_capability(self) -> Capability {
        match self {
            Self::ReadSelection => Capability::ReadSelection,
            Self::ReadVisibleScreen => Capability::ReadVisibleScreen,
            Self::ReadCwd => Capability::ReadCwd,
            Self::ReadTitle => Capability::ReadTitle,
            Self::WriteTerminal => Capability::WriteTerminal,
            Self::Clipboard => Capability::Clipboard,
            Self::Network => Capability::Network,
        }
    }

    /// Map onto the v2 wire capability. The v1 seven are 1:1; v2-only
    /// capabilities are not representable here and must be declared separately.
    pub fn to_v2(self) -> plugin_protocol::v2::Capability {
        match self {
            Self::ReadSelection => plugin_protocol::v2::Capability::ReadSelection,
            Self::ReadVisibleScreen => plugin_protocol::v2::Capability::ReadVisibleScreen,
            Self::ReadCwd => plugin_protocol::v2::Capability::ReadCwd,
            Self::ReadTitle => plugin_protocol::v2::Capability::ReadTitle,
            Self::WriteTerminal => plugin_protocol::v2::Capability::WriteTerminal,
            Self::Clipboard => plugin_protocol::v2::Capability::Clipboard,
            Self::Network => plugin_protocol::v2::Capability::Network,
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

impl PluginLifecycle {
    // Reserved for the resident-connection supervisor (ADR-0015); v1 supervises
    // per-invocation, so the wire lifecycle is not sent yet.
    #[allow(dead_code)]
    fn to_wire(self) -> Lifecycle {
        match self {
            Self::OnDemand => Lifecycle::OnDemand,
            Self::Resident => Lifecycle::Resident,
        }
    }
}

/// `plugin.json` v1 (ADR-0015). Declares the binary, lifecycle, and the
/// commands with the capabilities each needs.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPluginCommand {
    pub plugin_id: String,
    pub plugin_name: String,
    pub plugin_version: String,
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
    let mut seen = HashSet::new();

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
                    for command in &manifest.commands {
                        let qualified = format!("{}.{}", manifest.id, command.id);
                        if !seen.insert(qualified.clone()) {
                            catalog
                                .diagnostics
                                .push(format!("duplicate plugin command: {qualified}"));
                            continue;
                        }
                        catalog.commands.push(LoadedPluginCommand {
                            plugin_id: manifest.id.clone(),
                            plugin_name: manifest.name.clone(),
                            plugin_version: manifest.version.clone(),
                            lifecycle: manifest.lifecycle,
                            binary: manifest.binary.clone(),
                            args: manifest.args.clone(),
                            command: command.clone(),
                            directory: directory.clone(),
                        });
                    }
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
    if manifest.api_version != PLUGIN_API_VERSION {
        return Err(PluginError::InvalidManifest(format!(
            "unsupported api_version {} (expected {PLUGIN_API_VERSION})",
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
    if manifest.commands.is_empty() {
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
/// v1 supervises per-invocation for both lifecycles: launch → Hello → Ready →
/// Invoke → Invoked → Shutdown. The `resident` flag is recorded on the command
/// but connection caching is a later change; correctness does not depend on it.
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
        .map(|p| p.to_capability())
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
}
