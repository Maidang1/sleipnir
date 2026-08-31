//! Official Rust SDK for Sleipnir plugins (ADR-0015, ADR-0016).
//!
//! A plugin author writes plain Rust: implement [`Plugin`] (v1) or
//! [`v2::Plugin`], call [`run`] / [`v2::run`] from `main`. The SDK owns the
//! wire protocol — the handshake, the read/decode/dispatch/encode loop over
//! stdin/stdout, and serialization — so the author never touches JSON or
//! process plumbing.
//!
//! v1 is request/response. [`v2`] is the resident, event-driven, rendering
//! surface (ADR-0016). Both remain supported for the N / N-1 window.
//!
//! ```no_run
//! use sleipnir_plugin::{Plugin, Manifest, CommandSpec, Capability, Invoke, Output, Lifecycle, run};
//!
//! struct Echo;
//! impl Plugin for Echo {
//!     fn manifest(&self) -> Manifest {
//!         Manifest {
//!             id: "echo".into(),
//!             name: "Echo".into(),
//!             version: "0.1.0".into(),
//!             description: "Echo the selection".into(),
//!             lifecycle: Lifecycle::OnDemand,
//!             commands: vec![CommandSpec {
//!                 id: "echo".into(),
//!                 title: "Echo Selection".into(),
//!                 description: String::new(),
//!                 keywords: vec![],
//!                 capabilities: vec![Capability::ReadSelection, Capability::Clipboard],
//!             }],
//!         }
//!     }
//!     fn requests(&self) -> Vec<Capability> {
//!         vec![Capability::ReadSelection, Capability::Clipboard]
//!     }
//!     fn invoke(&mut self, req: Invoke) -> Output {
//!         Output::copy(req.context.selection.unwrap_or_default())
//!     }
//! }
//!
//! fn main() { run(Echo); }
//! ```

pub mod v2;
mod widgets;

use std::io::{BufRead, Write};

pub use plugin_protocol::{
    Capability, CommandSpec, InvokeContext, Lifecycle, Manifest, Output as WireOutput,
    PROTOCOL_VERSION,
};
use plugin_protocol::{HostMessage, Output as ProtoOutput, PluginMessage, versions_compatible};

/// One command invocation delivered to the plugin.
pub struct Invoke {
    pub command_id: String,
    pub context: InvokeContext,
}

/// How the plugin wants its result routed. A thin, ergonomic wrapper over the
/// wire [`WireOutput`].
pub enum Output {
    Ignore,
    Insert(String),
    Copy(String),
}

impl Output {
    pub fn insert(text: impl Into<String>) -> Self {
        Self::Insert(text.into())
    }
    pub fn copy(text: impl Into<String>) -> Self {
        Self::Copy(text.into())
    }

    pub(crate) fn into_wire(self) -> ProtoOutput {
        match self {
            Output::Ignore => ProtoOutput::Ignore,
            Output::Insert(text) => ProtoOutput::Insert { text },
            Output::Copy(text) => ProtoOutput::Copy { text },
        }
    }
}

/// The trait a plugin implements. `manifest` and `requests` are asked once
/// during the handshake; `invoke` runs per command.
pub trait Plugin {
    /// Self-description contributed to the host's command palette.
    fn manifest(&self) -> Manifest;

    /// Capabilities the plugin requests. Defaults to the union declared across
    /// the manifest's commands, which is the right answer for most plugins.
    fn requests(&self) -> Vec<Capability> {
        let mut caps: Vec<Capability> = self
            .manifest()
            .commands
            .iter()
            .flat_map(|c| c.capabilities.iter().copied())
            .collect();
        caps.sort();
        caps.dedup();
        caps
    }

    /// Run one command. Errors are reported to the host via `Failed`; return
    /// `Err(message)` to surface a failure without panicking.
    fn invoke(&mut self, req: Invoke) -> Output;
}

/// Run the plugin: perform the handshake and serve invocations until the host
/// sends `Shutdown` or closes the pipe. Call this from `main`.
pub fn run<P: Plugin>(plugin: P) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(err) = serve(plugin, stdin.lock(), stdout.lock()) {
        eprintln!("sleipnir-plugin: {err}");
        std::process::exit(1);
    }
}

/// Testable core of [`run`]: drives the protocol over any reader/writer.
pub fn serve<P: Plugin>(
    mut plugin: P,
    reader: impl BufRead,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let mut lines = reader.lines();

    // Handshake: expect Hello, reply Ready.
    let Some(first) = lines.next().transpose()? else {
        return Ok(()); // host closed before saying hello
    };
    let hello: HostMessage = serde_json::from_str(&first)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let HostMessage::Hello {
        protocol_version, ..
    } = hello
    else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "expected hello as first message",
        ));
    };
    if !versions_compatible(protocol_version, PROTOCOL_VERSION) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("host speaks protocol {protocol_version}, plugin speaks {PROTOCOL_VERSION}"),
        ));
    }
    write_msg(
        &mut writer,
        &PluginMessage::Ready {
            protocol_version: PROTOCOL_VERSION,
            manifest: plugin.manifest(),
            requests: plugin.requests(),
        },
    )?;

    // Serve loop.
    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: HostMessage = serde_json::from_str(&line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        match msg {
            HostMessage::Invoke {
                command_id,
                context,
            } => {
                let output = plugin
                    .invoke(Invoke {
                        command_id,
                        context,
                    })
                    .into_wire();
                write_msg(&mut writer, &PluginMessage::Invoked { output })?;
            }
            HostMessage::Shutdown => break,
            HostMessage::Hello { .. } => {
                // A second hello is a protocol violation; ignore defensively.
            }
        }
    }
    Ok(())
}

fn write_msg(writer: &mut impl Write, msg: &PluginMessage) -> std::io::Result<()> {
    let line = serde_json::to_string(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CopyCat;
    impl Plugin for CopyCat {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "copycat".into(),
                name: "CopyCat".into(),
                version: "0.1.0".into(),
                description: String::new(),
                lifecycle: Lifecycle::OnDemand,
                commands: vec![CommandSpec {
                    id: "copy".into(),
                    title: "Copy Selection".into(),
                    description: String::new(),
                    keywords: vec![],
                    capabilities: vec![Capability::ReadSelection, Capability::Clipboard],
                }],
            }
        }
        fn invoke(&mut self, req: Invoke) -> Output {
            Output::copy(req.context.selection.unwrap_or_default())
        }
    }

    fn conversation(input: &str) -> Vec<PluginMessage> {
        let mut out = Vec::new();
        serve(CopyCat, std::io::Cursor::new(input), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn handshake_replies_ready_with_manifest() {
        let hello = serde_json::to_string(&HostMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            granted: vec![],
        })
        .unwrap();
        let msgs = conversation(&format!("{hello}\n"));
        let PluginMessage::Ready {
            manifest, requests, ..
        } = &msgs[0]
        else {
            panic!("expected ready");
        };
        assert_eq!(manifest.id, "copycat");
        // requests() defaults to the union of command capabilities.
        assert!(requests.contains(&Capability::Clipboard));
    }

    #[test]
    fn invoke_routes_copy_through_the_trait() {
        let hello = serde_json::to_string(&HostMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            granted: vec![Capability::ReadSelection, Capability::Clipboard],
        })
        .unwrap();
        let invoke = serde_json::to_string(&HostMessage::Invoke {
            command_id: "copy".into(),
            context: InvokeContext {
                selection: Some("hi".into()),
                ..Default::default()
            },
        })
        .unwrap();
        let msgs = conversation(&format!("{hello}\n{invoke}\n"));
        let PluginMessage::Invoked {
            output: ProtoOutput::Copy { text },
        } = &msgs[1]
        else {
            panic!("expected copy invoked, got {:?}", msgs[1]);
        };
        assert_eq!(text, "hi");
    }

    #[test]
    fn version_mismatch_is_an_error() {
        let hello = serde_json::to_string(&HostMessage::Hello {
            protocol_version: 999,
            granted: vec![],
        })
        .unwrap();
        let mut out = Vec::new();
        let err = serve(
            CopyCat,
            std::io::Cursor::new(format!("{hello}\n")),
            &mut out,
        )
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    }

    #[test]
    fn shutdown_ends_the_session_cleanly() {
        let hello = serde_json::to_string(&HostMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            granted: vec![],
        })
        .unwrap();
        let shutdown = serde_json::to_string(&HostMessage::Shutdown).unwrap();
        let extra = serde_json::to_string(&HostMessage::Invoke {
            command_id: "markdown".into(),
            context: InvokeContext::default(),
        })
        .unwrap();
        // Invoke after shutdown must not be served.
        let msgs = conversation(&format!("{hello}\n{shutdown}\n{extra}\n"));
        assert_eq!(
            msgs.len(),
            1,
            "only the Ready reply, nothing after shutdown"
        );
    }
}
