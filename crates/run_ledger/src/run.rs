//! Domain types for the Run Ledger.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

pub type RunId = Uuid;
/// Identifies one Pane across restarts; persisted in `session.json`.
pub type PaneKey = Uuid;
/// Identifies one process launch; jumping is only valid within the current one.
pub type LaunchId = Uuid;

/// Scrollback position of a Run. Process-local — never written to `runs.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anchor {
    /// Absolute line (`cursor.line + history_size` when the OSC 133 C fired).
    pub line: i32,
    pub column: usize,
}

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
    /// In-memory only (spec §2): scrollback is gone after a restart.
    #[serde(skip)]
    pub anchor: Option<Anchor>,
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
        anchor: Option<Anchor>,
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
        Self::Started {
            pane,
            command: command.into(),
            cwd,
            at_ms,
            inferred: false,
            anchor: None,
        }
    }

    pub fn started_inferred(pane: PaneKey, command: &str, cwd: Option<String>, at_ms: u64) -> Self {
        Self::Started {
            pane,
            command: command.into(),
            cwd,
            at_ms,
            inferred: true,
            anchor: None,
        }
    }

    pub fn started_at(
        pane: PaneKey,
        command: &str,
        cwd: Option<String>,
        at_ms: u64,
        inferred: bool,
        anchor: Option<Anchor>,
    ) -> Self {
        Self::Started {
            pane,
            command: command.into(),
            cwd,
            at_ms,
            inferred,
            anchor,
        }
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
            anchor: None,
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

    pub(crate) fn started_at_mono_ms(&self) -> u64 {
        self.started_at_mono_ms
    }

    /// Build a Run for chrome helpers and tests. Not a live capture path.
    pub fn for_display(
        launch_id: LaunchId,
        pane: PaneKey,
        command: &str,
        state: RunState,
        exit_code: Option<i32>,
        started_at_unix_ms: u64,
        seen: bool,
    ) -> Self {
        Self {
            id: RunId::new_v4(),
            launch_id,
            pane,
            command: command.into(),
            cwd: None,
            started_at_unix_ms,
            started_at_mono_ms: 0,
            duration: Duration::from_millis(1200),
            exit_code,
            state,
            inferred: false,
            seen,
            anchor: None,
        }
    }

    /// Elapsed millis for a still-running Run, using the caller's monotonic clock.
    pub fn elapsed_ms(&self, now_mono_ms: u64) -> u64 {
        now_mono_ms.saturating_sub(self.started_at_mono_ms)
    }
}
