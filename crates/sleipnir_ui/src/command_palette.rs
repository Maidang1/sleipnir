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
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    NewWindow,
    TogglePaneZoom,
    ToggleBroadcast,
    JumpPrevPrompt,
    JumpNextPrompt,
    ToggleQuickSelect,
    OpenQuickTerminal,
    ExportScrollback,
    ClearRunLedger,
    ToggleRunLedger,
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
            CommandId::IncreaseFontSize => "increase_font_size",
            CommandId::DecreaseFontSize => "decrease_font_size",
            CommandId::ResetFontSize => "reset_font_size",
            CommandId::NewWindow => "new_window",
            CommandId::TogglePaneZoom => "toggle_pane_zoom",
            CommandId::ToggleBroadcast => "toggle_broadcast",
            CommandId::JumpPrevPrompt => "jump_prev_prompt",
            CommandId::JumpNextPrompt => "jump_next_prompt",
            CommandId::ToggleQuickSelect => "toggle_quick_select",
            CommandId::OpenQuickTerminal => "open_quick_terminal",
            CommandId::ExportScrollback => "export_scrollback",
            CommandId::ClearRunLedger => "clear_run_ledger",
            CommandId::ToggleRunLedger => "toggle_run_ledger",
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
            "increase_font_size" | "font_size_up" => Some(CommandId::IncreaseFontSize),
            "decrease_font_size" | "font_size_down" => Some(CommandId::DecreaseFontSize),
            "reset_font_size" | "font_size_reset" => Some(CommandId::ResetFontSize),
            "new_window" => Some(CommandId::NewWindow),
            "toggle_pane_zoom" | "pane_zoom" => Some(CommandId::TogglePaneZoom),
            "toggle_broadcast" | "broadcast" => Some(CommandId::ToggleBroadcast),
            "jump_prev_prompt" | "prev_prompt" => Some(CommandId::JumpPrevPrompt),
            "jump_next_prompt" | "next_prompt" => Some(CommandId::JumpNextPrompt),
            "toggle_quick_select" | "quick_select" => Some(CommandId::ToggleQuickSelect),
            "open_quick_terminal" | "quick_terminal" => Some(CommandId::OpenQuickTerminal),
            "export_scrollback" | "export_scrollback_to_file" => Some(CommandId::ExportScrollback),
            "clear_run_ledger" => Some(CommandId::ClearRunLedger),
            "toggle_run_ledger" => Some(CommandId::ToggleRunLedger),
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

/// Built-in command catalog for this OS.
pub fn commands() -> Vec<CommandItem> {
    commands_for(cfg!(windows))
}

/// Built-in command catalog. `windows = true` shows Ctrl labels.
pub fn commands_for(windows: bool) -> Vec<CommandItem> {
    use crate::keymap::display_shortcut_for;
    vec![
        CommandItem {
            id: CommandId::NewTab,
            title: "New Tab".into(),
            shortcut: display_shortcut_for("new_tab", windows).into(),
            keywords: "tab new open",
        },
        CommandItem {
            id: CommandId::ClosePane,
            title: "Close Pane / Tab".into(),
            shortcut: display_shortcut_for("close_tab", windows).into(),
            keywords: "close pane tab",
        },
        CommandItem {
            id: CommandId::NextTab,
            title: "Next Tab".into(),
            shortcut: display_shortcut_for("next_tab", windows).into(),
            keywords: "tab next",
        },
        CommandItem {
            id: CommandId::PrevTab,
            title: "Previous Tab".into(),
            shortcut: display_shortcut_for("prev_tab", windows).into(),
            keywords: "tab previous prev",
        },
        CommandItem {
            id: CommandId::SplitRight,
            title: "Split Pane Right".into(),
            shortcut: display_shortcut_for("split_right", windows).into(),
            keywords: "split right vertical",
        },
        CommandItem {
            id: CommandId::SplitDown,
            title: "Split Pane Down".into(),
            shortcut: display_shortcut_for("split_down", windows).into(),
            keywords: "split down horizontal",
        },
        CommandItem {
            id: CommandId::Find,
            title: "Find in Scrollback".into(),
            shortcut: display_shortcut_for("find", windows).into(),
            keywords: "find search scrollback",
        },
        CommandItem {
            id: CommandId::OpenSettings,
            title: "Open Settings".into(),
            shortcut: display_shortcut_for("open_settings", windows).into(),
            keywords: "settings preferences theme",
        },
        CommandItem {
            id: CommandId::ReloadSettings,
            title: "Reload Settings".into(),
            shortcut: display_shortcut_for("reload_settings", windows).into(),
            keywords: "reload settings config",
        },
        CommandItem {
            id: CommandId::CycleTheme,
            title: "Cycle Theme".into(),
            shortcut: display_shortcut_for("cycle_theme", windows).into(),
            keywords: "theme cycle appearance",
        },
        CommandItem {
            id: CommandId::CheckForUpdates,
            title: "Check for Updates".into(),
            shortcut: display_shortcut_for("check_for_updates", windows).into(),
            keywords: "update upgrade release",
        },
        CommandItem {
            id: CommandId::ToggleCommandPalette,
            title: "Toggle Command Palette".into(),
            shortcut: display_shortcut_for("toggle_command_palette", windows).into(),
            keywords: "command palette actions",
        },
        CommandItem {
            id: CommandId::NewWindow,
            title: "New Window".into(),
            shortcut: display_shortcut_for("new_window", windows).into(),
            keywords: "window new open",
        },
        CommandItem {
            id: CommandId::IncreaseFontSize,
            title: "Increase Font Size".into(),
            shortcut: display_shortcut_for("increase_font_size", windows).into(),
            keywords: "font zoom larger bigger size",
        },
        CommandItem {
            id: CommandId::DecreaseFontSize,
            title: "Decrease Font Size".into(),
            shortcut: display_shortcut_for("decrease_font_size", windows).into(),
            keywords: "font zoom smaller size",
        },
        CommandItem {
            id: CommandId::ResetFontSize,
            title: "Reset Font Size".into(),
            shortcut: display_shortcut_for("reset_font_size", windows).into(),
            keywords: "font zoom reset default size",
        },
        CommandItem {
            id: CommandId::TogglePaneZoom,
            title: "Toggle Pane Zoom".into(),
            shortcut: display_shortcut_for("toggle_pane_zoom", windows).into(),
            keywords: "zoom maximize pane split",
        },
        CommandItem {
            id: CommandId::ToggleBroadcast,
            title: "Toggle Broadcast Input".into(),
            shortcut: display_shortcut_for("toggle_broadcast", windows).into(),
            keywords: "broadcast all panes input",
        },
        CommandItem {
            id: CommandId::JumpPrevPrompt,
            title: "Jump to Previous Prompt".into(),
            shortcut: display_shortcut_for("jump_prev_prompt", windows).into(),
            keywords: "prompt shell osc133 jump previous",
        },
        CommandItem {
            id: CommandId::JumpNextPrompt,
            title: "Jump to Next Prompt".into(),
            shortcut: display_shortcut_for("jump_next_prompt", windows).into(),
            keywords: "prompt shell osc133 jump next",
        },
        CommandItem {
            id: CommandId::ToggleQuickSelect,
            title: "Toggle Quick Select".into(),
            shortcut: display_shortcut_for("toggle_quick_select", windows).into(),
            keywords: "quick select labels links",
        },
        CommandItem {
            id: CommandId::OpenQuickTerminal,
            title: "Open Quick Terminal".into(),
            shortcut: display_shortcut_for("open_quick_terminal", windows).into(),
            keywords: "quick terminal dropdown window",
        },
        CommandItem {
            id: CommandId::ExportScrollback,
            title: "Export Scrollback to File".into(),
            shortcut: display_shortcut_for("export_scrollback", windows).into(),
            keywords: "export scrollback save file editor dump",
        },
        CommandItem {
            id: CommandId::ClearRunLedger,
            title: "Clear Run Ledger".into(),
            shortcut: "".into(),
            keywords: "clear run ledger history runs delete",
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

    #[test]
    fn windows_palette_copy_uses_ctrl() {
        let items = commands_for(true);
        let new_tab = items
            .iter()
            .find(|i| i.id == CommandId::NewTab)
            .expect("new tab");
        assert_eq!(new_tab.shortcut.as_ref(), "Ctrl+Shift+T");
        let close = items
            .iter()
            .find(|i| i.id == CommandId::ClosePane)
            .expect("close");
        assert_eq!(close.shortcut.as_ref(), "Ctrl+Shift+W");
        assert!(
            items.iter().all(|i| !i.shortcut.to_string().contains('⌘')),
            "Windows palette must not show ⌘"
        );
    }

    #[test]
    fn macos_palette_copy_keeps_cmd() {
        let items = commands_for(false);
        let new_tab = items
            .iter()
            .find(|i| i.id == CommandId::NewTab)
            .expect("new tab");
        assert_eq!(new_tab.shortcut.as_ref(), "⌘T");
    }

    #[test]
    fn clear_run_ledger_is_a_known_action_name() {
        assert_eq!(
            CommandId::from_str("clear_run_ledger"),
            Some(CommandId::ClearRunLedger)
        );
        assert_eq!(
            CommandId::from_str("toggle_run_ledger"),
            Some(CommandId::ToggleRunLedger)
        );
        assert!(
            commands()
                .iter()
                .any(|i| i.id == CommandId::ClearRunLedger),
            "Clear Run Ledger must appear in the palette"
        );
    }
}
