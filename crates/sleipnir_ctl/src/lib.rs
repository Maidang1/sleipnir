//! Control-surface protocol types (ADR-0011). Pure; no socket, no I/O.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// `$SLEIPNIR_CONTROL_SOCKET` if set and non-empty, else `~/.config/sleipnir/control.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("SLEIPNIR_CONTROL_SOCKET") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if cfg!(windows) {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
            .join("control.sock")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir/control.sock")
    }
}

/// `SLEIPNIR_CONTROL=1` (or `true`) turns the surface on regardless of settings.
pub fn env_enabled() -> bool {
    match std::env::var("SLEIPNIR_CONTROL") {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => false,
    }
}

/// Settings key OR env — either is enough. A missing key is off.
pub fn enabled(settings_on: bool) -> bool {
    settings_on || env_enabled()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitUntil {
    Free,
    Failed,
    Attention,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneSnap {
    pub pane: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    Ls,
    Capture {
        pane: Uuid,
    },
    Send {
        pane: Uuid,
        text: String,
        enter: bool,
    },
    Wait {
        pane: Uuid,
        until: WaitUntil,
        timeout_secs: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlResponse {
    Ls { panes: Vec<PaneSnap> },
    Capture { text: String },
    Send,
    Wait,
    Error { message: String },
}

/// Whether a `wait` predicate is satisfied by the current pane/ledger facts.
pub fn wait_matches(
    until: WaitUntil,
    busy: bool,
    has_failed_attention: bool,
    has_attention: bool,
) -> bool {
    match until {
        WaitUntil::Free => !busy,
        WaitUntil::Failed => has_failed_attention,
        WaitUntil::Attention => has_attention,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_matches_when_not_busy() {
        assert!(wait_matches(WaitUntil::Free, false, true, true));
    }

    #[test]
    fn free_does_not_match_when_busy() {
        assert!(!wait_matches(WaitUntil::Free, true, false, false));
    }

    #[test]
    fn failed_matches_failed_attention_only() {
        assert!(wait_matches(WaitUntil::Failed, true, true, false));
        assert!(!wait_matches(WaitUntil::Failed, false, false, true));
    }

    #[test]
    fn attention_matches_attention_only() {
        assert!(wait_matches(WaitUntil::Attention, true, false, true));
        assert!(!wait_matches(WaitUntil::Attention, false, true, false));
    }

    #[test]
    fn settings_off_stays_off_without_env() {
        assert!(!enabled(false));
        assert!(enabled(true));
    }
}
