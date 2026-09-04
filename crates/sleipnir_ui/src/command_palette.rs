//! Lightweight command palette (M9).
//!
//! Fixed catalog of actions with fuzzy subsequence filtering. Key bindings remain
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
    MarkTabSeen,
    TogglePaneFacts,
    SendSelection,
    PipeSelection,
    SendGitDiff,
    ToggleHistorySearch,
    ToggleDiff,
    TogglePluginMonitor,
    /// Runtime-discovered external plugin command index.
    Plugin(usize),
    /// Dynamic chrome contribution (RenderStatus Btn). Index into
    /// [`crate::plugin_chrome::ChromeRegistry::palette_entries`]. Never a
    /// built-in id, even when the plugin's title copies one.
    PluginContribution(usize),
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
            CommandId::MarkTabSeen => "mark_tab_seen",
            CommandId::TogglePaneFacts => "toggle_pane_facts",
            CommandId::SendSelection => "send_selection",
            CommandId::PipeSelection => "pipe_selection",
            CommandId::SendGitDiff => "send_git_diff",
            CommandId::ToggleHistorySearch => "toggle_history_search",
            CommandId::ToggleDiff => "toggle_diff",
            CommandId::TogglePluginMonitor => "toggle_plugin_monitor",
            CommandId::Plugin(_) => "plugin",
            CommandId::PluginContribution(_) => "plugin_contribution",
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
            "mark_tab_seen" | "mark_as_seen" => Some(CommandId::MarkTabSeen),
            "toggle_pane_facts" | "pane_facts" => Some(CommandId::TogglePaneFacts),
            "send_selection" => Some(CommandId::SendSelection),
            "pipe_selection" => Some(CommandId::PipeSelection),
            "send_git_diff" => Some(CommandId::SendGitDiff),
            "toggle_history_search" | "history_search" => Some(CommandId::ToggleHistorySearch),
            "toggle_diff" => Some(CommandId::ToggleDiff),
            "toggle_plugin_monitor" | "plugin_monitor" => Some(CommandId::TogglePluginMonitor),
            _ if s.trim().starts_with("plugin.") => None,
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
    pub keywords: SharedString,
}

/// Built-in command catalog.
pub fn commands() -> Vec<CommandItem> {
    use crate::keymap::display_shortcut;
    vec![
        CommandItem {
            id: CommandId::NewTab,
            title: "New Tab".into(),
            shortcut: display_shortcut("new_tab").into(),
            keywords: "tab new open".into(),
        },
        CommandItem {
            id: CommandId::ClosePane,
            title: "Close Pane / Tab".into(),
            shortcut: display_shortcut("close_tab").into(),
            keywords: "close pane tab".into(),
        },
        CommandItem {
            id: CommandId::NextTab,
            title: "Next Tab".into(),
            shortcut: display_shortcut("next_tab").into(),
            keywords: "tab next".into(),
        },
        CommandItem {
            id: CommandId::PrevTab,
            title: "Previous Tab".into(),
            shortcut: display_shortcut("prev_tab").into(),
            keywords: "tab previous prev".into(),
        },
        CommandItem {
            id: CommandId::SplitRight,
            title: "Split Pane Right".into(),
            shortcut: display_shortcut("split_right").into(),
            keywords: "split right vertical".into(),
        },
        CommandItem {
            id: CommandId::SplitDown,
            title: "Split Pane Down".into(),
            shortcut: display_shortcut("split_down").into(),
            keywords: "split down horizontal".into(),
        },
        CommandItem {
            id: CommandId::Find,
            title: "Find in Scrollback".into(),
            shortcut: display_shortcut("find").into(),
            keywords: "find search scrollback".into(),
        },
        CommandItem {
            id: CommandId::OpenSettings,
            title: "Open Settings".into(),
            shortcut: display_shortcut("open_settings").into(),
            keywords: "settings preferences theme".into(),
        },
        CommandItem {
            id: CommandId::ReloadSettings,
            title: "Reload Settings".into(),
            shortcut: display_shortcut("reload_settings").into(),
            keywords: "reload settings config".into(),
        },
        CommandItem {
            id: CommandId::CycleTheme,
            title: "Cycle Theme".into(),
            shortcut: display_shortcut("cycle_theme").into(),
            keywords: "theme cycle appearance".into(),
        },
        CommandItem {
            id: CommandId::CheckForUpdates,
            title: "Check for Updates".into(),
            shortcut: display_shortcut("check_for_updates").into(),
            keywords: "update upgrade release".into(),
        },
        CommandItem {
            id: CommandId::ToggleCommandPalette,
            title: "Toggle Command Palette".into(),
            shortcut: display_shortcut("toggle_command_palette").into(),
            keywords: "command palette actions".into(),
        },
        CommandItem {
            id: CommandId::NewWindow,
            title: "New Window".into(),
            shortcut: display_shortcut("new_window").into(),
            keywords: "window new open".into(),
        },
        CommandItem {
            id: CommandId::IncreaseFontSize,
            title: "Increase Font Size".into(),
            shortcut: display_shortcut("increase_font_size").into(),
            keywords: "font zoom larger bigger size".into(),
        },
        CommandItem {
            id: CommandId::DecreaseFontSize,
            title: "Decrease Font Size".into(),
            shortcut: display_shortcut("decrease_font_size").into(),
            keywords: "font zoom smaller size".into(),
        },
        CommandItem {
            id: CommandId::ResetFontSize,
            title: "Reset Font Size".into(),
            shortcut: display_shortcut("reset_font_size").into(),
            keywords: "font zoom reset default size".into(),
        },
        CommandItem {
            id: CommandId::TogglePaneZoom,
            title: "Toggle Pane Zoom".into(),
            shortcut: display_shortcut("toggle_pane_zoom").into(),
            keywords: "zoom maximize pane split".into(),
        },
        CommandItem {
            id: CommandId::ToggleBroadcast,
            title: "Toggle Broadcast Input".into(),
            shortcut: display_shortcut("toggle_broadcast").into(),
            keywords: "broadcast all panes input".into(),
        },
        CommandItem {
            id: CommandId::JumpPrevPrompt,
            title: "Jump to Previous Prompt".into(),
            shortcut: display_shortcut("jump_prev_prompt").into(),
            keywords: "prompt shell osc133 jump previous".into(),
        },
        CommandItem {
            id: CommandId::JumpNextPrompt,
            title: "Jump to Next Prompt".into(),
            shortcut: display_shortcut("jump_next_prompt").into(),
            keywords: "prompt shell osc133 jump next".into(),
        },
        CommandItem {
            id: CommandId::ToggleQuickSelect,
            title: "Toggle Quick Select".into(),
            shortcut: display_shortcut("toggle_quick_select").into(),
            keywords: "quick select labels links".into(),
        },
        CommandItem {
            id: CommandId::OpenQuickTerminal,
            title: "Open Quick Terminal".into(),
            shortcut: display_shortcut("open_quick_terminal").into(),
            keywords: "quick terminal dropdown window".into(),
        },
        CommandItem {
            id: CommandId::ExportScrollback,
            title: "Export Scrollback to File".into(),
            shortcut: display_shortcut("export_scrollback").into(),
            keywords: "export scrollback save file editor dump".into(),
        },
        CommandItem {
            id: CommandId::ClearRunLedger,
            title: "Clear Run Ledger".into(),
            shortcut: "".into(),
            keywords: "clear run ledger history runs delete".into(),
        },
        CommandItem {
            id: CommandId::MarkTabSeen,
            title: "Mark Tab as Seen".into(),
            shortcut: "".into(),
            keywords: "mark seen attention unread tab badge".into(),
        },
        CommandItem {
            id: CommandId::TogglePaneFacts,
            title: "Toggle Pane Facts".into(),
            shortcut: "".into(),
            keywords: "pane facts cwd process tree ports info".into(),
        },
        CommandItem {
            id: CommandId::ToggleRunLedger,
            title: "Toggle Run Ledger".into(),
            shortcut: display_shortcut("toggle_run_ledger").into(),
            keywords: "run ledger panel history attention".into(),
        },
        CommandItem {
            id: CommandId::SendSelection,
            title: "Send Selection to Pane".into(),
            shortcut: "".into(),
            keywords: "send selection paste pty agent".into(),
        },
        CommandItem {
            id: CommandId::PipeSelection,
            title: "Pipe Selection to Command".into(),
            shortcut: "".into(),
            keywords: "pipe selection command external".into(),
        },
        CommandItem {
            id: CommandId::SendGitDiff,
            title: "Send Git Diff to Pane".into(),
            shortcut: "".into(),
            keywords: "git diff send review pane".into(),
        },
        CommandItem {
            id: CommandId::ToggleDiff,
            title: "Toggle Diff Inspector".into(),
            shortcut: display_shortcut("toggle_diff").into(),
            keywords: "git diff inspector review overlay patch".into(),
        },
        CommandItem {
            id: CommandId::ToggleHistorySearch,
            title: "Search Shell History".into(),
            shortcut: display_shortcut("toggle_history_search").into(),
            keywords: "history fuzzy search histfile".into(),
        },
        CommandItem {
            id: CommandId::TogglePluginMonitor,
            title: "Toggle Plugin Monitor".into(),
            shortcut: display_shortcut("toggle_plugin_monitor").into(),
            keywords: "plugin monitor process kill".into(),
        },
    ]
}

pub fn plugin_items(commands: &[plugin_host::LoadedPluginCommand]) -> Vec<CommandItem> {
    commands
        .iter()
        .enumerate()
        .map(|(index, plugin)| {
            let mut keywords = plugin.command.keywords.join(" ");
            if !plugin.command.description.is_empty() {
                keywords.push(' ');
                keywords.push_str(&plugin.command.description);
            }
            keywords.push(' ');
            keywords.push_str(&plugin.plugin_name);
            CommandItem {
                id: CommandId::Plugin(index),
                title: plugin.command.title.clone().into(),
                shortcut: "Plugin".into(),
                keywords: keywords.into(),
            }
        })
        .collect()
}

/// Dynamic RenderStatus Btn entries. Title is already attributed by
/// [`crate::plugin_chrome::attributed_title`]. Shortcut column repeats the
/// plugin id so a copied built-in title cannot pass as host chrome.
pub fn contribution_items(
    entries: &[crate::plugin_chrome::PaletteContribution],
) -> Vec<CommandItem> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| CommandItem {
            id: CommandId::PluginContribution(index),
            title: entry.title.clone().into(),
            shortcut: entry.plugin_id.clone().into(),
            keywords: format!(
                "{} {} {} plugin",
                entry.title, entry.action, entry.plugin_id
            )
            .into(),
        })
        .collect()
}

/// Case-insensitive fuzzy filter over title + keywords.
/// Returns indices ranked by match tier, preserving catalog order within tiers.
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
            let score = if title.starts_with(&q) {
                0
            } else if subsequence(&title, &q) {
                1
            } else if keys.contains(&q) {
                2
            } else if subsequence(&keys, &q) {
                3
            } else {
                return None;
            };
            Some((score, i))
        })
        .collect();
    scored.sort_by_key(|(score, i)| (*score, *i));
    scored.into_iter().map(|(_, i)| i).collect()
}

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut chars = hay.chars();
    needle
        .chars()
        .all(|needle_char| chars.any(|hay_char| hay_char == needle_char))
}

pub(crate) fn record_recent(recents: &mut Vec<CommandId>, id: CommandId) {
    recents.retain(|recent| *recent != id);
    recents.insert(0, id);
    recents.truncate(8);
}

pub(crate) fn prioritize_recents(
    items: &[CommandItem],
    indices: &mut [usize],
    recents: &[CommandId],
) {
    indices.sort_by_key(|&index| {
        recents
            .iter()
            .position(|id| *id == items[index].id)
            .map_or((1, index), |recent| (0, recent))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::display_shortcut;

    #[test]
    fn filter_empty_returns_all() {
        let items = commands();
        assert_eq!(filter_commands(&items, "").len(), items.len());
    }

    #[test]
    fn filter_finds_title_subsequences_case_insensitively() {
        let items = commands();
        let hits = filter_commands(&items, "SpRt");
        assert!(hits.iter().any(|&i| items[i].id == CommandId::SplitRight));
    }

    #[test]
    fn filter_ranks_match_tiers_stably() {
        let item = |id, title: &'static str, keywords: &'static str| CommandItem {
            id,
            title: title.into(),
            shortcut: "".into(),
            keywords: keywords.into(),
        };
        let items = vec![
            item(CommandId::NewTab, "Alpha One", "none"),
            item(CommandId::ClosePane, "X Alpha", "none"),
            item(CommandId::NextTab, "Other", "alpha exact"),
            item(CommandId::PrevTab, "Other Two", "a-l-p-h-a"),
            item(CommandId::SplitRight, "Alpha Two", "none"),
        ];

        assert_eq!(filter_commands(&items, "alpha"), vec![0, 4, 1, 2, 3]);
    }

    #[test]
    fn recents_are_prioritized_in_recency_order_then_catalog_order() {
        let items = commands();
        let mut indices: Vec<_> = (0..items.len()).collect();
        prioritize_recents(&items, &mut indices, &[CommandId::Find, CommandId::NewTab]);

        assert_eq!(items[indices[0]].id, CommandId::Find);
        assert_eq!(items[indices[1]].id, CommandId::NewTab);
        assert_eq!(items[indices[2]].id, CommandId::ClosePane);
    }

    #[test]
    fn recording_recent_commands_deduplicates_and_caps_at_eight() {
        let mut recents = vec![
            CommandId::NewTab,
            CommandId::ClosePane,
            CommandId::NextTab,
            CommandId::PrevTab,
            CommandId::SplitRight,
            CommandId::SplitDown,
            CommandId::OpenSettings,
            CommandId::ReloadSettings,
        ];

        record_recent(&mut recents, CommandId::NextTab);
        assert_eq!(recents[0], CommandId::NextTab);
        assert_eq!(recents.len(), 8);
        assert_eq!(
            recents
                .iter()
                .filter(|id| **id == CommandId::NextTab)
                .count(),
            1
        );

        record_recent(&mut recents, CommandId::CycleTheme);
        assert_eq!(recents[0], CommandId::CycleTheme);
        assert_eq!(recents.len(), 8);
        assert!(!recents.contains(&CommandId::ReloadSettings));
    }

    #[test]
    fn command_id_roundtrip() {
        for item in commands() {
            assert_eq!(CommandId::from_str(item.id.as_str()), Some(item.id));
        }
    }

    #[test]
    fn macos_palette_copy_keeps_cmd() {
        let items = commands();
        let new_tab = items
            .iter()
            .find(|i| i.id == CommandId::NewTab)
            .expect("new tab");
        assert_eq!(new_tab.shortcut.as_ref(), display_shortcut("new_tab"));
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
            commands().iter().any(|i| i.id == CommandId::ClearRunLedger),
            "Clear Run Ledger must appear in the palette"
        );
        assert_eq!(
            CommandId::from_str("mark_tab_seen"),
            Some(CommandId::MarkTabSeen)
        );
        assert!(
            commands().iter().any(|i| i.id == CommandId::MarkTabSeen),
            "Mark Tab as Seen must appear in the palette"
        );
        assert_eq!(
            CommandId::from_str("toggle_pane_facts"),
            Some(CommandId::TogglePaneFacts)
        );
        assert!(
            commands()
                .iter()
                .any(|i| i.id == CommandId::TogglePaneFacts),
            "Toggle Pane Facts must appear in the palette"
        );
        assert_eq!(
            CommandId::from_str("toggle_diff"),
            Some(CommandId::ToggleDiff)
        );
        assert!(
            commands().iter().any(|i| i.id == CommandId::ToggleDiff),
            "Toggle Diff Inspector must appear in the palette"
        );
        assert_eq!(
            CommandId::from_str("toggle_plugin_monitor"),
            Some(CommandId::TogglePluginMonitor)
        );
        assert_eq!(
            CommandId::from_str("plugin_monitor"),
            Some(CommandId::TogglePluginMonitor)
        );
        assert!(
            commands()
                .iter()
                .any(|i| i.id == CommandId::TogglePluginMonitor
                    && i.keywords.as_ref().contains("plugin monitor process kill")),
            "Plugin Monitor must appear in the palette with the required keywords"
        );
    }

    #[test]
    fn contribution_items_cannot_impersonate_a_builtin_command_id() {
        let entries = [crate::plugin_chrome::PaletteContribution {
            plugin_id: "demo".into(),
            title: "demo: Reload Settings".into(),
            action: "reload_settings".into(),
            arg: None,
            surface_id: uuid::Uuid::nil(),
        }];
        let items = contribution_items(&entries);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, CommandId::PluginContribution(0));
        assert_ne!(items[0].id, CommandId::ReloadSettings);
        assert_eq!(
            CommandId::from_str("reload_settings"),
            Some(CommandId::ReloadSettings)
        );
        assert_eq!(items[0].id.as_str(), "plugin_contribution");
        assert_eq!(CommandId::from_str("plugin_contribution"), None);
        assert!(items[0].title.as_ref().starts_with("demo:"));
        let builtins = commands();
        let merged = {
            let mut v = builtins;
            v.extend(items);
            v
        };
        let hits = filter_commands(&merged, "reload");
        assert!(
            hits.iter()
                .any(|&i| merged[i].id == CommandId::ReloadSettings)
        );
        assert!(
            hits.iter()
                .any(|&i| merged[i].id == CommandId::PluginContribution(0))
        );
        assert!(hits[0] < hits[1] || merged[hits[0]].id == CommandId::ReloadSettings);
    }
}
