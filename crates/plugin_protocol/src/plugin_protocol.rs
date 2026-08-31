//! Wire protocol for out-of-process plugins (ADR-0015).
//!
//! Pure types. No I/O, no process handling, no host or SDK logic. Both the host
//! (`plugin_host`) and the author-facing SDK (`sleipnir-plugin`) depend on this
//! crate so the contract is defined in exactly one place.
//!
//! Transport is line-delimited JSON, one object per line, matching the house
//! IPC style of the control surface (ADR-0011, `sleipnir_ctl`): tagged unions,
//! `snake_case`, forward-compatible with `#[serde(default)]` on additions.
//!
//! ## Session shape (v1)
//!
//! ```text
//! host  ── Hello ──▶ plugin          (protocol version the host speaks)
//! plugin ── Ready ──▶ host           (manifest + capabilities requested)
//! host  ── Invoke ──▶ plugin         (a command run, with granted context)
//! plugin ── Invoked ──▶ host         (the result: how to route stdout)
//!            … more Invoke/Invoked …
//! host  ── Shutdown ──▶ plugin       (session ends; plugin exits)
//! ```
//!
//! v1 is request/response only. There is no plugin-initiated call and no event
//! push; those are reserved behind capability flags for later widening
//! (ADR-0015 "strict first, widen later").

use serde::{Deserialize, Serialize};

/// Protocol v2 (ADR-0016): bidirectional, multiplexed messaging with
/// correlation ids, event push, widget rendering and plugin-initiated host
/// calls. v1 below remains the on-demand contract; the resident supervisor
/// and the SDK's `v2` module speak this dialect.
pub mod v2;

/// Protocol version the host and SDK negotiate. Bump on any wire change.
pub const PROTOCOL_VERSION: u32 = 1;

/// How a plugin wants to be run, declared by the plugin itself (both in its
/// `plugin.json` for pre-launch auditing and in its `Ready` handshake).
///
/// - `OnDemand`: launched per invocation, then shut down. Cheapest and
///   strictest — no process lingers. Right for stateless transformers.
/// - `Resident`: launched once and kept connected across invocations. For
///   plugins that hold state or pay a heavy startup cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    OnDemand,
    Resident,
}

/// A capability a plugin may request in its `Ready` and the host may grant in
/// its `Hello` reply path. v1 mirrors ADR-0013's permissions; reserved variants
/// are intentionally absent until the protocol widens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ReadSelection,
    ReadVisibleScreen,
    ReadCwd,
    ReadTitle,
    WriteTerminal,
    Clipboard,
    Network,
}

/// How a plugin wants its `Invoked` payload routed by the host. Mirrors the
/// output routes established by ADR-0013.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "route", rename_all = "snake_case")]
pub enum Output {
    /// Discard.
    Ignore,
    /// Insert into the active pane (types into the PTY). Requires WriteTerminal.
    Insert { text: String },
    /// Copy to the clipboard. Requires Clipboard.
    Copy { text: String },
}

/// One command a plugin contributes, declared in its `Ready` manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Capabilities this command needs. Must be a subset of the plugin's
    /// requested set and, ultimately, of the user's allowlist.
    #[serde(default)]
    pub capabilities: Vec<Capability>,
}

/// The plugin's self-description, sent once in `Ready`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// How the plugin wants to be supervised. Mirrors the `plugin.json` value;
    /// the host trusts `plugin.json` for launch decisions and cross-checks this.
    #[serde(default)]
    pub lifecycle: Lifecycle,
    pub commands: Vec<CommandSpec>,
}

/// Context handed to the plugin with an `Invoke`. A field is present only when
/// the command holds the matching capability *and* the user granted it, so the
/// plugin never receives data it was not authorized to read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_screen: Option<String>,
}

/// Host → plugin messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum HostMessage {
    /// First message. Announces the protocol version the host speaks and the
    /// capabilities the user's allowlist permits at all.
    Hello {
        protocol_version: u32,
        granted: Vec<Capability>,
    },
    /// Run one of the plugin's commands.
    Invoke {
        command_id: String,
        context: InvokeContext,
    },
    /// End the session; the plugin should exit cleanly.
    Shutdown,
}

/// Plugin → host messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum PluginMessage {
    /// Reply to `Hello`: the manifest and the capabilities the plugin wants.
    Ready {
        protocol_version: u32,
        manifest: Manifest,
        requests: Vec<Capability>,
    },
    /// Reply to `Invoke`: how to route the result.
    Invoked { output: Output },
    /// A command failed; the host logs this and does not crash.
    Failed { message: String },
}

/// True when the plugin and host agree on a protocol version. v1 requires exact
/// equality; a later host may accept a range.
pub fn versions_compatible(host: u32, plugin: u32) -> bool {
    host == plugin
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_messages_round_trip_as_tagged_json() {
        let hello = HostMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            granted: vec![Capability::ReadSelection],
        };
        let line = serde_json::to_string(&hello).unwrap();
        assert!(line.contains(r#""msg":"hello""#));
        assert!(line.contains(r#""read_selection""#));
        assert_eq!(serde_json::from_str::<HostMessage>(&line).unwrap(), hello);
    }

    #[test]
    fn plugin_ready_carries_manifest_and_requests() {
        let ready = PluginMessage::Ready {
            protocol_version: PROTOCOL_VERSION,
            manifest: Manifest {
                id: "beautify".into(),
                name: "Beautify".into(),
                version: "0.1.0".into(),
                description: String::new(),
                lifecycle: Lifecycle::OnDemand,
                commands: vec![CommandSpec {
                    id: "markdown".into(),
                    title: "Preview as Markdown".into(),
                    description: String::new(),
                    keywords: vec!["md".into()],
                    capabilities: vec![Capability::ReadSelection],
                }],
            },
            requests: vec![Capability::ReadSelection],
        };
        let line = serde_json::to_string(&ready).unwrap();
        assert_eq!(serde_json::from_str::<PluginMessage>(&line).unwrap(), ready);
    }

    #[test]
    fn version_equality_is_the_v1_rule() {
        assert!(versions_compatible(PROTOCOL_VERSION, PROTOCOL_VERSION));
        assert!(!versions_compatible(1, 2));
    }
}
