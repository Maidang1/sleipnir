//! Session persistence for tabs, splits, and pane working directories (M8).
//!
//! Snapshot format lives at `~/.config/sleipnir/session.json`. Only structure
//! is restored — not scrollback, running processes, or window geometry.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Schema version for forward-compatible loaders.
pub const SESSION_VERSION: u32 = 1;

/// On-disk session document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SessionFile {
    pub version: u32,
    /// Index of the active tab (clamped on restore).
    pub active_tab: usize,
    pub tabs: Vec<SessionTab>,
}

impl Default for SessionFile {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            active_tab: 0,
            tabs: Vec::new(),
        }
    }
}

/// One restored tab: custom title + pane tree.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SessionTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// Pane id that should receive focus within this tab.
    pub active_pane: u64,
    pub tree: SessionNode,
}

/// Recursive pane layout snapshot (mirrors `PaneNode` without live entities).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionNode {
    Leaf {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Split {
        axis: SessionAxis,
        ratio: f32,
        first: Box<SessionNode>,
        second: Box<SessionNode>,
    },
}

/// Split orientation in the session file.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionAxis {
    /// Left | right (`⌘D`).
    Horizontal,
    /// Top / bottom (`⌘⇧D`).
    Vertical,
}

impl SessionNode {
    /// Maximum pane id in this subtree (used to advance id counters).
    pub fn max_pane_id(&self) -> u64 {
        match self {
            SessionNode::Leaf { id, .. } => *id,
            SessionNode::Split { first, second, .. } => {
                first.max_pane_id().max(second.max_pane_id())
            }
        }
    }

    /// Number of leaf panes.
    pub fn leaf_count(&self) -> usize {
        match self {
            SessionNode::Leaf { .. } => 1,
            SessionNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// First leaf id (fallback active pane).
    pub fn first_leaf_id(&self) -> u64 {
        match self {
            SessionNode::Leaf { id, .. } => *id,
            SessionNode::Split { first, .. } => first.first_leaf_id(),
        }
    }

    /// Whether this subtree contains `id`.
    pub fn contains_pane(&self, id: u64) -> bool {
        match self {
            SessionNode::Leaf { id: leaf, .. } => *leaf == id,
            SessionNode::Split { first, second, .. } => {
                first.contains_pane(id) || second.contains_pane(id)
            }
        }
    }
}

/// Default session file path: `~/.config/sleipnir/session.json`.
pub fn session_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/sleipnir/session.json")
}

/// Load a session from disk. Returns `None` if the file is missing, empty,
/// invalid, or has no tabs.
pub fn load_session(path: &Path) -> Option<SessionFile> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let file: SessionFile = serde_json::from_slice(&bytes).ok()?;
    if file.tabs.is_empty() {
        return None;
    }
    // Unknown future versions still try to load if the shape matches.
    Some(file)
}

/// Persist a session snapshot atomically (write temp + rename).
pub fn save_session(path: &Path, session: &SessionFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut json = serde_json::to_vec_pretty(session)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    json.push(b'\n');
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Normalize a loaded session: clamp active indices, drop empty trees.
pub fn sanitize_session(mut session: SessionFile) -> Option<SessionFile> {
    session.tabs.retain(|t| t.tree.leaf_count() > 0);
    if session.tabs.is_empty() {
        return None;
    }
    for tab in &mut session.tabs {
        if !tab.tree.contains_pane(tab.active_pane) {
            tab.active_pane = tab.tree.first_leaf_id();
        }
        // Clamp split ratios.
        sanitize_ratios(&mut tab.tree);
    }
    if session.active_tab >= session.tabs.len() {
        session.active_tab = session.tabs.len() - 1;
    }
    session.version = SESSION_VERSION;
    Some(session)
}

fn sanitize_ratios(node: &mut SessionNode) {
    match node {
        SessionNode::Leaf { .. } => {}
        SessionNode::Split {
            ratio,
            first,
            second,
            ..
        } => {
            *ratio = ratio.clamp(0.1, 0.9);
            sanitize_ratios(first);
            sanitize_ratios(second);
        }
    }
}

/// Validate that a cwd string points at an existing directory; otherwise `None`.
pub fn resolve_cwd(cwd: Option<&str>) -> Option<PathBuf> {
    let raw = cwd?.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    if path.is_dir() {
        Some(path)
    } else {
        // Parent may still exist if the folder was deleted mid-session.
        path.parent()
            .filter(|p| p.is_dir())
            .map(|p| p.to_path_buf())
            .or_else(dirs::home_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionFile {
        SessionFile {
            version: 1,
            active_tab: 0,
            tabs: vec![SessionTab {
                custom_title: Some("work".into()),
                active_pane: 2,
                tree: SessionNode::Split {
                    axis: SessionAxis::Horizontal,
                    ratio: 0.4,
                    first: Box::new(SessionNode::Leaf {
                        id: 1,
                        cwd: Some("/tmp".into()),
                    }),
                    second: Box::new(SessionNode::Leaf {
                        id: 2,
                        cwd: None,
                    }),
                },
            }],
        }
    }

    #[test]
    fn roundtrip_json() {
        let s = sample();
        let bytes = serde_json::to_vec_pretty(&s).unwrap();
        let back: SessionFile = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn sanitize_clamps_active_tab_and_pane() {
        let mut s = sample();
        s.active_tab = 99;
        s.tabs[0].active_pane = 999;
        let cleaned = sanitize_session(s).unwrap();
        assert_eq!(cleaned.active_tab, 0);
        assert_eq!(cleaned.tabs[0].active_pane, 1); // first leaf fallback
    }

    #[test]
    fn sanitize_drops_empty() {
        let s = SessionFile {
            version: 1,
            active_tab: 0,
            tabs: vec![],
        };
        assert!(sanitize_session(s).is_none());
    }

    #[test]
    fn max_pane_id_walks_tree() {
        let s = sample();
        assert_eq!(s.tabs[0].tree.max_pane_id(), 2);
    }

    #[test]
    fn save_and_load_tmp() {
        let dir = std::env::temp_dir().join(format!("sleipnir-session-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        let s = sample();
        save_session(&path, &s).unwrap();
        let loaded = load_session(&path).unwrap();
        assert_eq!(loaded.tabs[0].custom_title.as_deref(), Some("work"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
