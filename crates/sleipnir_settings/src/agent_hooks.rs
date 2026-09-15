//! One-click installation of coding agent hooks that report status
//! back to the sleipnir coordination socket.
//!
//! Each supported agent (Claude Code, Codex) gets a shell script hook
//! installed into its config directory. The hook sends JSON-lines events
//! over `$SLEIPNIR_AGENT_CONTROL_SOCKET` (or the default socket path)
//! so the built-in Agents plugin can track agent running state.

use std::path::PathBuf;

const HOOK_MARKER: &str = "# sleipnir-agent-hook";

pub struct AgentHookTarget {
    pub id: &'static str,
    pub label: &'static str,
    pub hooks_dir: PathBuf,
    pub hook_filename: &'static str,
    /// For agents that use a settings.json hooks config (Claude Code).
    pub settings_path: Option<PathBuf>,
}

pub fn supported_targets() -> Vec<AgentHookTarget> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    vec![
        AgentHookTarget {
            id: "claude",
            label: "Claude Code",
            hooks_dir: home.join(".claude/hooks"),
            hook_filename: "sleipnir-agent-state.sh",
            settings_path: Some(home.join(".claude/settings.json")),
        },
        AgentHookTarget {
            id: "codex",
            label: "Codex",
            hooks_dir: home.join(".codex/hooks"),
            hook_filename: "sleipnir-agent-state.sh",
            settings_path: None,
        },
    ]
}

pub enum HookStatus {
    Installed,
    NotInstalled,
    Outdated,
    DirMissing,
}

pub fn check_hook_status(target: &AgentHookTarget) -> HookStatus {
    if !target.hooks_dir.exists() {
        return HookStatus::DirMissing;
    }
    let path = target.hooks_dir.join(target.hook_filename);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            if content.contains(HOOK_MARKER) {
                if content.contains(&current_hook_version()) {
                    HookStatus::Installed
                } else {
                    HookStatus::Outdated
                }
            } else {
                HookStatus::NotInstalled
            }
        }
        Err(_) => HookStatus::NotInstalled,
    }
}

fn current_hook_version() -> String {
    format!("SLEIPNIR_HOOK_VERSION={}", env!("CARGO_PKG_VERSION"))
}

fn default_socket_path() -> String {
    sleipnir_paths::agent_control_socket_path()
        .display()
        .to_string()
}

fn generate_hook_script(agent_id: &str) -> String {
    let version = current_hook_version();
    let socket_default = default_socket_path();

    format!(
        r##"#!/bin/sh
{HOOK_MARKER}
# {version}
# Installed by sleipnir. Reports agent lifecycle events to the
# sleipnir coordination socket so the terminal can track status.
# Safe to delete — sleipnir will offer to reinstall from Settings.

set -eu

SOCKET="${{SLEIPNIR_AGENT_CONTROL_SOCKET:-{socket_default}}}"
[ -S "$SOCKET" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

hook_input="$(cat 2>/dev/null || true)"
[ -n "$hook_input" ] || exit 0

python3 - "$SOCKET" "{agent_id}" <<'PYEOF'
import json, os, socket, sys, time, random

sock_path = sys.argv[1]
agent_id = sys.argv[2]

try:
    data = json.loads(sys.stdin.read())
except Exception:
    sys.exit(0)

event = data.get("hook_event_name") or data.get("event") or ""
session_id = data.get("session_id") or ""
cwd = data.get("cwd") or ""

status_map = {{
    "SessionStart": "running",
    "UserPromptSubmit": "running",
    "PreToolUse": "running",
    "PostToolUse": "running",
    "PermissionRequest": "awaiting_human",
    "Stop": "idle",
    "SessionEnd": "exited",
}}

status = status_map.get(event)
if not status:
    sys.exit(0)

request_id = int(time.time() * 1000) * 1000 + random.randrange(1000)
report = {{
    "id": request_id,
    "op": "report_hook_event",
    "agent": agent_id,
    "session_id": session_id,
    "event": event,
    "status": status,
    "cwd": cwd,
}}

try:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(0.5)
    client.connect(sock_path)
    client.sendall((json.dumps(report) + "\n").encode())
    try:
        client.recv(256)
    except Exception:
        pass
    client.close()
except Exception:
    pass
PYEOF
"##
    )
}

fn install_hook_file(target: &AgentHookTarget) -> Result<(), String> {
    std::fs::create_dir_all(&target.hooks_dir)
        .map_err(|e| format!("failed to create {}: {e}", target.hooks_dir.display()))?;

    let path = target.hooks_dir.join(target.hook_filename);
    let script = generate_hook_script(target.id);

    std::fs::write(&path, &script)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to chmod {}: {e}", path.display()))?;
    }

    Ok(())
}

fn register_claude_settings_hook(target: &AgentHookTarget) -> Result<(), String> {
    let settings_path = match &target.settings_path {
        Some(p) => p,
        None => return Ok(()),
    };

    let hook_script_path = target.hooks_dir.join(target.hook_filename);
    let hook_cmd = format!("bash '{}' \"$1\"", hook_script_path.display());

    let mut settings: serde_json::Value = match std::fs::read_to_string(settings_path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse {}: {e}", settings_path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(format!("failed to read {}: {e}", settings_path.display())),
    };

    let hooks = settings
        .as_object_mut()
        .ok_or("settings root is not an object")?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().ok_or("hooks is not an object")?;

    for event in &[
        "SessionStart",
        "Stop",
        "UserPromptSubmit",
        "PermissionRequest",
        "SessionEnd",
    ] {
        let entries = hooks_obj
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));
        let arr = entries
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} is not an array"))?;

        let already = arr.iter().any(|entry| {
            entry
                .get("hooks")
                .and_then(|h| h.as_array())
                .map(|hooks| {
                    hooks.iter().any(|h| {
                        h.get("command")
                            .and_then(|c| c.as_str())
                            .map_or(false, |c| c.contains("sleipnir-agent-state"))
                    })
                })
                .unwrap_or(false)
        });

        if !already {
            arr.push(serde_json::json!({
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": hook_cmd,
                    "timeout": 5
                }]
            }));
        }
    }

    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    std::fs::write(settings_path, format!("{json}\n"))
        .map_err(|e| format!("failed to write {}: {e}", settings_path.display()))?;

    Ok(())
}

pub fn install_hooks(target: &AgentHookTarget) -> Result<(), String> {
    install_hook_file(target)?;
    register_claude_settings_hook(target)?;
    Ok(())
}

pub fn install_all_hooks() -> Vec<(String, Result<(), String>)> {
    supported_targets()
        .iter()
        .map(|t| (t.label.to_string(), install_hooks(t)))
        .collect()
}

pub fn uninstall_hook(target: &AgentHookTarget) -> Result<(), String> {
    let path = target.hooks_dir.join(target.hook_filename);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_script_contains_marker_and_version() {
        let script = generate_hook_script("claude");
        assert!(script.contains(HOOK_MARKER));
        assert!(script.contains(&current_hook_version()));
        assert!(script.contains("claude"));
    }

    #[test]
    fn supported_targets_are_nonempty() {
        let targets = supported_targets();
        assert!(targets.len() >= 2);
        assert!(targets.iter().any(|t| t.id == "claude"));
        assert!(targets.iter().any(|t| t.id == "codex"));
    }
}
