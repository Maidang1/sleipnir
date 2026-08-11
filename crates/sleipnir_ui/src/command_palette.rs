//! Lightweight command palette (M9).
//!
//! Fixed catalog of actions with fuzzy substring filter. Key bindings remain
//! in `main.rs`; the palette is a discoverability surface, not a keymap editor.

use gpui::SharedString;

/// Stable command identifiers used by the palette and optional key_bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    NewTab,
    ClosePane,
    NextTab,
    PrevTab,
    SplitRight,
    SplitDown,
    OpenSettings,
    ReloadSettings,
    CycleTheme,
    CheckForUpdates,
    Find,
    ToggleCommandPalette,
}

impl CommandId {
    pub fn as_str(self) -> &'static str {
        match self {
            CommandId::NewTab => "new_tab",
            CommandId::ClosePane => "close_tab",
            CommandId::NextTab => "next_tab",
            CommandId::PrevTab => "prev_tab",
            CommandId::SplitRight => "split_right",
            CommandId::SplitDown => "split_down",
            CommandId::OpenSettings => "open_settings",
            CommandId::ReloadSettings => "reload_settings",
            CommandId::CycleTheme => "cycle_theme",
            CommandId::CheckForUpdates => "check_for_updates",
            CommandId::Find => "find",
            CommandId::ToggleCommandPalette => "toggle_command_palette",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "new_tab" => Some(CommandId::NewTab),
            "close_tab" | "close_pane" => Some(CommandId::ClosePane),
            "next_tab" => Some(CommandId::NextTab),
            "prev_tab" => Some(CommandId::PrevTab),
            "split_right" => Some(CommandId::SplitRight),
            "split_down" => Some(CommandId::SplitDown),
            "open_settings" => Some(CommandId::OpenSettings),
            "reload_settings" => Some(CommandId::ReloadSettings),
            "cycle_theme" => Some(CommandId::CycleTheme),
            "check_for_updates" => Some(CommandId::CheckForUpdates),
            "find" | "search" => Some(CommandId::Find),
            "toggle_command_palette" | "command_palette" => Some(CommandId::ToggleCommandPalette),
            _ => None,
        }
    }
}

/// One palette row.
#[derive(Clone, Debug)]
pub struct CommandItem {
    pub id: CommandId,
    pub title: SharedString,
    pub shortcut: SharedString,
    pub keywords: &'static str,
}

/// Built-in command catalog.
pub fn commands() -> Vec<CommandItem> {
    vec![
        CommandItem {
            id: CommandId::NewTab,
            title: "New Tab".into(),
            shortcut: "⌘T".into(),
            keywords: "tab new open",
        },
        CommandItem {
            id: CommandId::ClosePane,
            title: "Close Pane / Tab".into(),
            shortcut: "⌘W".into(),
            keywords: "close pane tab",
        },
        CommandItem {
            id: CommandId::NextTab,
            title: "Next Tab".into(),
            shortcut: "⌃Tab".into(),
            keywords: "tab next",
        },
        CommandItem {
            id: CommandId::PrevTab,
            title: "Previous Tab".into(),
            shortcut: "⌃⇧Tab".into(),
            keywords: "tab previous prev",
        },
        CommandItem {
            id: CommandId::SplitRight,
            title: "Split Pane Right".into(),
            shortcut: "⌘D".into(),
            keywords: "split right vertical",
        },
        CommandItem {
            id: CommandId::SplitDown,
            title: "Split Pane Down".into(),
            shortcut: "⌘⇧D".into(),
            keywords: "split down horizontal",
        },
        CommandItem {
            id: CommandId::Find,
            title: "Find in Scrollback".into(),
            shortcut: "⌘F".into(),
            keywords: "find search scrollback",
        },
        CommandItem {
            id: CommandId::OpenSettings,
            title: "Open Settings".into(),
            shortcut: "⌘,".into(),
            keywords: "settings preferences theme",
        },
        CommandItem {
            id: CommandId::ReloadSettings,
            title: "Reload Settings".into(),
            shortcut: "⌘⇧R".into(),
            keywords: "reload settings config",
        },
        CommandItem {
            id: CommandId::CycleTheme,
            title: "Cycle Theme".into(),
            shortcut: "⌘⇧P".into(),
            keywords: "theme cycle appearance",
        },
        CommandItem {
            id: CommandId::CheckForUpdates,
            title: "Check for Updates".into(),
            shortcut: "⌘⇧U".into(),
            keywords: "update upgrade release",
        },
        CommandItem {
            id: CommandId::ToggleCommandPalette,
            title: "Toggle Command Palette".into(),
            shortcut: "⌘⇧K".into(),
            keywords: "command palette actions",
        },
    ]
}

/// Case-insensitive substring filter over title + keywords.
/// Returns indices into `items` in catalog order.
pub fn filter_commands(items: &[CommandItem], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    let mut scored: Vec<(usize, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let title = item.title.to_lowercase();
            let keys = item.keywords.to_lowercase();
            let hay = format!("{title} {keys}");
            if !hay.contains(&q) {
                return None;
            }
            // Prefer title prefix, then title contains, then keywords.
            let score = if title.starts_with(&q) {
                0
            } else if title.contains(&q) {
                1
            } else {
                2
            };
            Some((score, i))
        })
        .collect();
    scored.sort_by_key(|(score, i)| (*score, *i));
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_empty_returns_all() {
        let items = commands();
        assert_eq!(filter_commands(&items, "").len(), items.len());
    }

    #[test]
    fn filter_finds_split() {
        let items = commands();
        let hits = filter_commands(&items, "split");
        assert!(hits.len() >= 2);
        assert!(hits
            .iter()
            .any(|&i| items[i].id == CommandId::SplitRight));
    }

    #[test]
    fn command_id_roundtrip() {
        for item in commands() {
            assert_eq!(CommandId::from_str(item.id.as_str()), Some(item.id));
        }
    }
}
