//! Domain types for the Run Ledger.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

pub type RunId = Uuid;
/// Identifies one Pane across restarts; persisted in `session.json`.
pub type PaneKey = Uuid;
/// Identifies one process launch; jumping is only valid within the current one.
pub type LaunchId = Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Succeeded,
    Failed,
    /// Finished without a usable exit code (no OSC 133 `D` status).
    Unknown,
    /// Process/pane/app went away while the Run was still going.
    /// Never rendered as success.
    Abandoned,
}

impl RunState {
    pub fn is_finished(self) -> bool {
        !matches!(self, RunState::Running)
    }
}

/// One command execution. `command` is already redacted (spec §2: redact-at-capture).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub launch_id: LaunchId,
    pub pane: PaneKey,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Wall-clock start, unix millis — for display only.
    pub started_at_unix_ms: u64,
    /// Monotonic start, millis since process start — for duration math.
    #[serde(skip)]
    started_at_mono_ms: u64,
    #[serde(default)]
    pub duration: Duration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub state: RunState,
    /// True when derived from the busy probe instead of OSC 133.
    #[serde(default)]
    pub inferred: bool,
    /// In-memory only (spec §2): Attention never crosses a restart.
    #[serde(skip)]
    pub seen: bool,
}

/// What the terminal reports. Times are monotonic millis since process start.
#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Started {
        pane: PaneKey,
        command: String,
        cwd: Option<String>,
        at_ms: u64,
        inferred: bool,
    },
    Finished {
        pane: PaneKey,
        exit_code: Option<i32>,
        at_ms: u64,
    },
    PaneClosed {
        pane: PaneKey,
        at_ms: u64,
    },
}

impl RunEvent {
    pub fn started(pane: PaneKey, command: &str, cwd: Option<String>, at_ms: u64) -> Self {
        Self::Started { pane, command: command.into(), cwd, at_ms, inferred: false }
    }

    pub fn started_inferred(pane: PaneKey, command: &str, cwd: Option<String>, at_ms: u64) -> Self {
        Self::Started { pane, command: command.into(), cwd, at_ms, inferred: true }
    }

    pub fn finished(pane: PaneKey, exit_code: Option<i32>, at_ms: u64) -> Self {
        Self::Finished { pane, exit_code, at_ms }
    }
}

impl Run {
    pub(crate) fn start(
        launch_id: LaunchId,
        pane: PaneKey,
        command: String,
        cwd: Option<String>,
        mono_ms: u64,
        unix_ms: u64,
        inferred: bool,
    ) -> Self {
        Self {
            id: RunId::new_v4(),
            launch_id,
            pane,
            command,
            cwd,
            started_at_unix_ms: unix_ms,
            started_at_mono_ms: mono_ms,
            duration: Duration::ZERO,
            exit_code: None,
            state: RunState::Running,
            inferred,
            seen: false,
        }
    }

    pub(crate) fn finish(&mut self, exit_code: Option<i32>, mono_ms: u64) {
        self.duration = Duration::from_millis(mono_ms.saturating_sub(self.started_at_mono_ms));
        self.exit_code = exit_code;
        self.state = match exit_code {
            Some(0) => RunState::Succeeded,
            Some(_) => RunState::Failed,
            None => RunState::Unknown,
        };
    }

    pub(crate) fn abandon(&mut self, mono_ms: u64) {
        self.duration = Duration::from_millis(mono_ms.saturating_sub(self.started_at_mono_ms));
        self.state = RunState::Abandoned;
    }
}
