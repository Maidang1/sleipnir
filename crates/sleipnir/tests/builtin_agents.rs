//! Real shipped-executable smoke tests, without creating a GPUI window or
//! touching the user's settings, plugins, grants, socket, or agent processes.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use agent_coordination::{Request, Response, WireRequest, call};
use plugin_protocol::v2::{Capability, HostCall, HostCallResult, HostMessage, PluginMessage};
use uuid::Uuid;

const BIN: &str = env!("CARGO_BIN_EXE_sleipnir");

struct Plugin {
    child: Child,
    messages: Receiver<PluginMessage>,
    root: tempfile::TempDir,
    socket: std::path::PathBuf,
}

impl Plugin {
    fn start(granted: Vec<Capability>) -> Self {
        // Keep below macOS's Unix socket path limit even when TMPDIR is long.
        let root = tempfile::Builder::new()
            .prefix("sleipnir-builtin-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = root.path().join(".config/sleipnir/agent-control.sock");
        let mut child = Command::new(BIN)
            .arg("--builtin-agents")
            .env("HOME", root.path())
            .env("XDG_CONFIG_HOME", root.path())
            .env("PATH", "")
            .env_remove("SLEIPNIR_AGENT_CONTROL_SOCKET")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let message =
                    serde_json::from_str(&line).expect("protocol-only stdout; no GUI logs");
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let mut plugin = Self {
            child,
            messages,
            root,
            socket,
        };
        plugin.send(HostMessage::Hello {
            protocol_version: 2,
            granted,
            plugin_instance_id: Uuid::new_v4(),
        });
        let PluginMessage::Ready {
            manifest, requests, ..
        } = plugin.next()
        else {
            panic!("expected Ready");
        };
        assert_eq!(manifest.id, "agents");
        assert!(requests.contains(&Capability::HostCallSendText));
        // Sent in on_hello before bind. The next Invoke is handled after bind.
        plugin.send(HostMessage::Invoke {
            id: 900,
            command_id: "open".into(),
            context: Default::default(),
        });
        while !matches!(plugin.next(), PluginMessage::Invoked { id: 900, .. }) {}
        plugin
    }

    fn send(&mut self, message: HostMessage) {
        writeln!(
            self.child.stdin.as_mut().unwrap(),
            "{}",
            serde_json::to_string(&message).unwrap()
        )
        .unwrap();
    }

    fn next(&self) -> PluginMessage {
        self.messages
            .recv_timeout(Duration::from_secs(5))
            .expect("plugin reply deadline")
    }

    fn next_call(&self) -> (u64, HostCall) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "host call deadline");
            if let PluginMessage::Call { id, call } = self.next() {
                return (id, call);
            }
        }
    }

    fn request(&self, body: Request) -> Response {
        call(&self.socket, &WireRequest { id: 1, body })
            .unwrap()
            .body
    }

    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(BIN)
            .arg("agentctl")
            .args(args)
            .env("HOME", self.root.path())
            .env("PATH", "")
            .env_remove("SLEIPNIR_AGENT_CONTROL_SOCKET")
            .output()
            .unwrap()
    }

    fn shutdown(&mut self) {
        self.send(HostMessage::Shutdown);
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "shutdown deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!self.socket.exists(), "normal shutdown unlinks the socket");
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn full_grants() -> Vec<Capability> {
    serde_json::from_str::<serde_json::Value>(include_str!(
        "../../sleipnir_plugin_agents/plugin.json"
    ))
    .unwrap()["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| serde_json::from_value(value.clone()).unwrap())
        .collect()
}

#[test]
fn builtin_child_and_client_work_with_no_installation_or_configuration() {
    let mut plugin = Plugin::start(full_grants());
    assert!(plugin.socket.exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&plugin.socket)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let output = plugin.cli(&["list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let list: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(list["op"], "agents");

    // Exercise the real adapter, replying as a fake host. No external agent is
    // launched: the test only checks the requested pane operations.
    let Response::LaunchAccepted { session, task } = plugin.request(Request::Launch {
        kind: agent_coordination::AgentKind::Codex,
        cwd: plugin.root.path().to_string_lossy().into_owned(),
        name: Some("worker".into()),
        args: vec![],
    }) else {
        panic!("expected accepted launch")
    };
    let (id, host_call) = plugin.next_call();
    assert!(matches!(host_call, HostCall::OpenPaneArgv { ref program, .. } if program == "codex"));
    let pane = Uuid::new_v4();
    plugin.send(HostMessage::Reply {
        id,
        result: HostCallResult::Pane { pane },
    });
    plugin.send(HostMessage::Event {
        id: 1,
        event: plugin_protocol::v2::HostEvent::ForegroundChanged {
            pane,
            agent: Some("codex".into()),
        },
    });
    let output = plugin.cli(&["wait", &task.as_uuid().to_string(), "--timeout-ms", "2000"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let Response::PromptAccepted { task } = plugin.request(Request::Prompt {
        session,
        text: "write tests".into(),
    }) else {
        panic!("expected accepted prompt")
    };
    let (id, host_call) = plugin.next_call();
    let HostCall::SendText {
        text, enter: true, ..
    } = host_call
    else {
        panic!("expected SendText")
    };
    assert!(text.contains(" agentctl report-result "));
    assert!(text.contains(BIN));
    assert!(!text.contains("sleipnir-agentctl"));
    plugin.send(HostMessage::Reply {
        id,
        result: HostCallResult::Ok,
    });
    let output = plugin.cli(&["report-result", &task.as_uuid().to_string(), "done"]);
    assert!(output.status.success());
    let output = plugin.cli(&["wait", &task.as_uuid().to_string(), "--timeout-ms", "2000"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("done"));
    assert!(
        !plugin
            .root
            .path()
            .join(".config/sleipnir/plugin-grants.json")
            .exists()
    );
    plugin.shutdown();
}

#[test]
fn builtin_service_still_refuses_to_bind_without_all_delivery_grants() {
    let mut grants = full_grants();
    grants.retain(|cap| *cap != Capability::HostCallSendKey);
    let mut plugin = Plugin::start(grants);
    assert!(!plugin.socket.exists());
    assert!(!plugin.cli(&["list"]).status.success());
    plugin.shutdown();
}

#[test]
fn builtin_client_help_is_headless() {
    let output = Command::new(BIN)
        .args(["agentctl", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("launch-wait"));
}
