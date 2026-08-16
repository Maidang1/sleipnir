//! Agent identity from a foreground process name. Marks we own — not vendor logos.

use std::path::Path;

use gpui::{App, Hsla, hsla};

use crate::app_shell::Tab;

/// A known coding-agent process, identified by argv / command name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentKind {
    pub id: &'static str,
    pub mark: &'static str,
    pub color: Hsla,
}

const CLAUDE: AgentKind = AgentKind {
    id: "claude",
    mark: "C",
    color: hsla(0.10, 0.85, 0.55, 1.0),
};
const CODEX: AgentKind = AgentKind {
    id: "codex",
    mark: "X",
    color: hsla(0.38, 0.55, 0.48, 1.0),
};
const GEMINI: AgentKind = AgentKind {
    id: "gemini",
    mark: "G",
    color: hsla(0.58, 0.70, 0.52, 1.0),
};
const AIDER: AgentKind = AgentKind {
    id: "aider",
    mark: "A",
    color: hsla(0.78, 0.45, 0.55, 1.0),
};
const OPENCODE: AgentKind = AgentKind {
    id: "opencode",
    mark: "O",
    color: hsla(0.48, 0.50, 0.45, 1.0),
};
const GOOSE: AgentKind = AgentKind {
    id: "goose",
    mark: "G",
    color: hsla(0.08, 0.70, 0.50, 1.0),
};
const CRUSH: AgentKind = AgentKind {
    id: "crush",
    mark: "K",
    color: hsla(0.92, 0.65, 0.52, 1.0),
};
const CURSOR: AgentKind = AgentKind {
    id: "cursor",
    mark: "C",
    color: hsla(0.55, 0.15, 0.70, 1.0),
};
const AMP: AgentKind = AgentKind {
    id: "amp",
    mark: "A",
    color: hsla(0.02, 0.75, 0.52, 1.0),
};
const PI: AgentKind = AgentKind {
    id: "pi",
    mark: "π",
    color: hsla(0.72, 0.55, 0.58, 1.0),
};
const COPILOT: AgentKind = AgentKind {
    id: "copilot",
    mark: "P",
    color: hsla(0.62, 0.40, 0.55, 1.0),
};
const GROK: AgentKind = AgentKind {
    id: "grok",
    mark: "G",
    color: hsla(0.00, 0.00, 0.75, 1.0),
};

/// `(normalized command name, kind)` — first match wins.
const CATALOG: &[(&str, AgentKind)] = &[
    ("claude", CLAUDE),
    ("claude-code", CLAUDE),
    ("codex", CODEX),
    ("gemini", GEMINI),
    ("gemini-cli", GEMINI),
    ("aider", AIDER),
    ("opencode", OPENCODE),
    ("goose", GOOSE),
    ("crush", CRUSH),
    ("cursor-agent", CURSOR),
    ("cursor", CURSOR),
    ("amp", AMP),
    ("pi", PI),
    ("copilot", COPILOT),
    ("grok", GROK),
];

/// Match a foreground process / script name against the built-in catalog.
pub fn identify(command: &str) -> Option<AgentKind> {
    let name = normalize_command(command);
    if name.is_empty() {
        return None;
    }
    CATALOG
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, kind)| *kind)
}

/// Active pane's agent, else the first known agent running in any pane of the tab.
pub fn identify_tab(tab: &Tab, cx: &App) -> Option<AgentKind> {
    let mut leaves = Vec::new();
    tab.tree.leaves(&mut leaves);
    let active = leaves
        .iter()
        .find(|(id, _)| *id == tab.active_pane_id())
        .map(|(_, view)| *view);
    if let Some(view) = active {
        if let Some(kind) = view
            .read(cx)
            .foreground_process_command_name(cx)
            .as_deref()
            .and_then(identify)
        {
            return Some(kind);
        }
    }
    for (_, view) in &leaves {
        if let Some(kind) = view
            .read(cx)
            .foreground_process_command_name(cx)
            .as_deref()
            .and_then(identify)
        {
            return Some(kind);
        }
    }
    None
}

fn normalize_command(command: &str) -> String {
    Path::new(command.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_known_agents() {
        assert_eq!(identify("claude").map(|k| k.id), Some("claude"));
        assert_eq!(identify("claude-code").map(|k| k.mark), Some("C"));
        assert_eq!(
            identify("/usr/local/bin/codex").map(|k| k.id),
            Some("codex")
        );
        assert_eq!(identify("GEMINI").map(|k| k.id), Some("gemini"));
        assert_eq!(identify("cursor-agent").map(|k| k.id), Some("cursor"));
        assert_eq!(identify("pi").map(|k| k.mark), Some("π"));
        assert_eq!(identify("grok").map(|k| k.id), Some("grok"));
    }

    #[test]
    fn shells_and_unknown_are_not_agents() {
        assert_eq!(identify("zsh"), None);
        assert_eq!(identify("bash"), None);
        assert_eq!(identify("node"), None);
        assert_eq!(identify("npm"), None);
        assert_eq!(identify(""), None);
        assert_eq!(identify("sleep"), None);
    }
}
