//! Built-in macOS key bindings and display labels.
//!
//! Tables are data: `main.rs` turns them into GPUI `KeyBinding`s. Tests inspect
//! the same tables instead of dumping the whole keymap.

/// Which GPUI context a builtin binding applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingContext {
    /// No context filter (app-wide).
    Global,
    /// Terminal view only.
    Terminal,
    /// Both `AppShell` and `Terminal`.
    Both,
}

/// Action carried by a builtin binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinAction {
    Quit,
    Hide,
    HideOthers,
    Copy,
    Paste,
    PasteText,
    SelectAll,
    Clear,
    ShowCharacterPalette,
    ToggleViMode,
    ScrollLineUp,
    ScrollLineDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,
    NewTab,
    CloseTab,
    NewWindow,
    NextTab,
    PrevTab,
    ReloadSettings,
    OpenSettings,
    CycleTheme,
    SplitRight,
    SplitDown,
    FocusPaneLeft,
    FocusPaneRight,
    FocusPaneUp,
    FocusPaneDown,
    CheckForUpdates,
    ToggleCommandPalette,
    Find,
    FindNext,
    FindPrev,
    TogglePaneZoom,
    ToggleBroadcast,
    JumpPrevPrompt,
    JumpNextPrompt,
    ToggleQuickSelect,
    OpenQuickTerminal,
    ToggleRunLedger,
    ToggleHistorySearch,
    ToggleDiff,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    ActivateTab(usize),
    SendKeystroke(&'static str),
    SendText(&'static str),
}

/// One shipped key binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltinBinding {
    pub key: String,
    pub action: BuiltinAction,
    pub context: BindingContext,
}

/// Extra chords for `keybinding_preset: tmux`. Unbound prefix keys are not
/// consumed here — the prefix itself is `ctrl-b` chords listed below.
pub fn tmux_preset_bindings() -> Vec<BuiltinBinding> {
    use BindingContext::Both;
    vec![
        b("ctrl-b c", BuiltinAction::NewTab, Both),
        b("ctrl-b n", BuiltinAction::NextTab, Both),
        b("ctrl-b p", BuiltinAction::PrevTab, Both),
        b("ctrl-b %", BuiltinAction::SplitRight, Both),
        b("ctrl-b shift-'", BuiltinAction::SplitDown, Both),
        b("ctrl-b left", BuiltinAction::FocusPaneLeft, Both),
        b("ctrl-b right", BuiltinAction::FocusPaneRight, Both),
        b("ctrl-b up", BuiltinAction::FocusPaneUp, Both),
        b("ctrl-b down", BuiltinAction::FocusPaneDown, Both),
        b("ctrl-b z", BuiltinAction::TogglePaneZoom, Both),
    ]
}

/// Bindings for macOS.
pub fn builtin_bindings() -> Vec<BuiltinBinding> {
    let mut out = macos_static_bindings();
    for n in 1..=9usize {
        out.push(BuiltinBinding {
            key: format!("cmd-{n}"),
            action: BuiltinAction::ActivateTab(n),
            context: BindingContext::Both,
        });
    }
    for (key, action) in font_zoom_key_bindings() {
        let action = match *action {
            "increase_font_size" => BuiltinAction::IncreaseFontSize,
            "decrease_font_size" => BuiltinAction::DecreaseFontSize,
            "reset_font_size" => BuiltinAction::ResetFontSize,
            _ => continue,
        };
        out.push(BuiltinBinding {
            key: (*key).to_string(),
            action,
            context: BindingContext::Both,
        });
    }
    out
}

fn macos_static_bindings() -> Vec<BuiltinBinding> {
    use BindingContext::{Both, Global, Terminal};
    vec![
        b("cmd-q", BuiltinAction::Quit, Global),
        b("cmd-h", BuiltinAction::Hide, Global),
        b("cmd-alt-h", BuiltinAction::HideOthers, Global),
        b("cmd-c", BuiltinAction::Copy, Terminal),
        b("cmd-v", BuiltinAction::Paste, Terminal),
        b("ctrl-shift-c", BuiltinAction::Copy, Terminal),
        b("ctrl-shift-v", BuiltinAction::Paste, Terminal),
        b("ctrl-cmd-v", BuiltinAction::PasteText, Terminal),
        b("cmd-a", BuiltinAction::SelectAll, Terminal),
        b("cmd-k", BuiltinAction::Clear, Terminal),
        b("ctrl-cmd-space", BuiltinAction::ShowCharacterPalette, Terminal),
        b("ctrl-shift-space", BuiltinAction::ToggleViMode, Terminal),
        b("shift-up", BuiltinAction::ScrollLineUp, Terminal),
        b("shift-down", BuiltinAction::ScrollLineDown, Terminal),
        b("shift-pageup", BuiltinAction::ScrollPageUp, Terminal),
        b("shift-pagedown", BuiltinAction::ScrollPageDown, Terminal),
        b("cmd-up", BuiltinAction::ScrollPageUp, Terminal),
        b("cmd-down", BuiltinAction::ScrollPageDown, Terminal),
        b("shift-home", BuiltinAction::ScrollToTop, Terminal),
        b("shift-end", BuiltinAction::ScrollToBottom, Terminal),
        b("cmd-home", BuiltinAction::ScrollToTop, Terminal),
        b("cmd-end", BuiltinAction::ScrollToBottom, Terminal),
        b(
            "cmd-backspace",
            BuiltinAction::SendKeystroke("ctrl-u"),
            Terminal,
        ),
        b("cmd-delete", BuiltinAction::SendKeystroke("ctrl-k"), Terminal),
        b("cmd-left", BuiltinAction::SendKeystroke("ctrl-a"), Terminal),
        b("cmd-right", BuiltinAction::SendKeystroke("ctrl-e"), Terminal),
        b("alt-left", BuiltinAction::SendText("\u{1b}b"), Terminal),
        b("alt-right", BuiltinAction::SendText("\u{1b}f"), Terminal),
        b("alt-b", BuiltinAction::SendText("\u{1b}b"), Terminal),
        b("alt-f", BuiltinAction::SendText("\u{1b}f"), Terminal),
        b("alt-delete", BuiltinAction::SendText("\u{1b}d"), Terminal),
        b("ctrl-delete", BuiltinAction::SendText("\u{1b}[3;5~"), Terminal),
        b("cmd-t", BuiltinAction::NewTab, Both),
        b("cmd-w", BuiltinAction::CloseTab, Both),
        b("cmd-n", BuiltinAction::NewWindow, Both),
        b("cmd-shift-]", BuiltinAction::NextTab, Both),
        b("cmd-shift-[", BuiltinAction::PrevTab, Both),
        b("ctrl-tab", BuiltinAction::NextTab, Both),
        b("ctrl-shift-tab", BuiltinAction::PrevTab, Both),
        b("cmd-shift-r", BuiltinAction::ReloadSettings, Both),
        b("cmd-,", BuiltinAction::OpenSettings, Both),
        b("cmd-shift-p", BuiltinAction::CycleTheme, Both),
        b("cmd-d", BuiltinAction::SplitRight, Both),
        b("cmd-shift-d", BuiltinAction::SplitDown, Both),
        b("cmd-alt-left", BuiltinAction::FocusPaneLeft, Both),
        b("cmd-alt-right", BuiltinAction::FocusPaneRight, Both),
        b("cmd-alt-up", BuiltinAction::FocusPaneUp, Both),
        b("cmd-alt-down", BuiltinAction::FocusPaneDown, Both),
        b("cmd-shift-u", BuiltinAction::CheckForUpdates, Both),
        b("cmd-shift-k", BuiltinAction::ToggleCommandPalette, Both),
        b("cmd-f", BuiltinAction::Find, Both),
        b("cmd-g", BuiltinAction::FindNext, Both),
        b("cmd-shift-g", BuiltinAction::FindPrev, Both),
        b("cmd-shift-enter", BuiltinAction::TogglePaneZoom, Both),
        b("cmd-shift-b", BuiltinAction::ToggleBroadcast, Both),
        b("cmd-shift-up", BuiltinAction::JumpPrevPrompt, Both),
        b("cmd-shift-down", BuiltinAction::JumpNextPrompt, Both),
        b("cmd-shift-o", BuiltinAction::ToggleQuickSelect, Both),
        b("cmd-shift-n", BuiltinAction::OpenQuickTerminal, Both),
        b("cmd-shift-l", BuiltinAction::ToggleRunLedger, Both),
        b("cmd-shift-;", BuiltinAction::ToggleHistorySearch, Both),
        b("cmd-alt-g", BuiltinAction::ToggleDiff, Both),
    ]
}

fn b(key: &'static str, action: BuiltinAction, context: BindingContext) -> BuiltinBinding {
    BuiltinBinding {
        key: key.to_string(),
        action,
        context,
    }
}

/// Font-zoom keystroke table.
pub fn font_zoom_key_bindings() -> &'static [(&'static str, &'static str)] {
    &[
        ("cmd-=", "increase_font_size"),
        ("cmd-+", "increase_font_size"),
        ("cmd--", "decrease_font_size"),
        ("cmd-0", "reset_font_size"),
    ]
}

/// Human-readable shortcut for a command id.
pub fn display_shortcut(id: &str) -> &'static str {
    match id {
        "new_tab" => "⌘T",
        "close_tab" => "⌘W",
        "next_tab" => "⌃Tab",
        "prev_tab" => "⌃⇧Tab",
        "split_right" => "⌘D",
        "split_down" => "⌘⇧D",
        "find" => "⌘F",
        "open_settings" => "⌘,",
        "reload_settings" => "⌘⇧R",
        "cycle_theme" => "⌘⇧P",
        "check_for_updates" => "⌘⇧U",
        "toggle_command_palette" => "⌘⇧K",
        "new_window" => "⌘N",
        "increase_font_size" => "⌘+",
        "decrease_font_size" => "⌘-",
        "reset_font_size" => "⌘0",
        "toggle_pane_zoom" => "⌘⇧Enter",
        "toggle_broadcast" => "⌘⇧B",
        "jump_prev_prompt" => "⌘⇧↑",
        "jump_next_prompt" => "⌘⇧↓",
        "toggle_quick_select" => "⌘⇧O",
        "open_quick_terminal" => "⌘⇧N",
        "toggle_diff" => "⌥⌘G",
        "secondary_click" => "⌘",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        builtin_bindings().into_iter().map(|b| b.key).collect()
    }

    #[test]
    fn macos_bindings_keep_cmd() {
        let keys = keys();
        assert!(keys.iter().any(|k| k == "cmd-t"));
        assert!(keys.iter().any(|k| k == "cmd-c"));
        assert!(keys.iter().any(|k| k == "cmd-w"));
        assert!(keys.iter().any(|k| k == "cmd-d"));
        assert!(keys.iter().any(|k| k == "cmd-alt-g"));
        assert!(!keys.iter().any(|k| k == "ctrl-v"));
    }

    #[test]
    fn font_zoom_table_uses_cmd() {
        let mac = font_zoom_key_bindings();
        assert!(
            mac.iter()
                .any(|(k, a)| *k == "cmd-+" && *a == "increase_font_size")
        );
        assert!(mac.iter().all(|(k, _)| k.starts_with("cmd-")));
    }
}
