//! Minimal task types used by the terminal crate (forked surface from Zed `task`).

use collections::HashMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use util::shell::{Shell, ShellKind};
pub use util::shell_builder::ShellBuilder;

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub struct TaskId(pub String);

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HideStrategy {
    #[default]
    Never,
    Always,
    OnSuccess,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealStrategy {
    #[default]
    Always,
    NoFocus,
    Never,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SaveStrategy {
    All,
    Current,
    #[default]
    None,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevealTarget {
    #[default]
    Dock,
    Center,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct SpawnInTerminal {
    pub id: TaskId,
    pub full_label: String,
    pub label: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub command_label: String,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub use_new_terminal: bool,
    pub allow_concurrent_runs: bool,
    pub reveal: RevealStrategy,
    pub reveal_target: RevealTarget,
    pub hide: HideStrategy,
    pub shell: Shell,
    pub show_summary: bool,
    pub show_command: bool,
    pub show_rerun: bool,
    pub save: SaveStrategy,
}
