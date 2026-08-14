//! OS-specific built-in key bindings and display labels.
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

/// Bindings for the compiling OS.
pub fn builtin_bindings() -> Vec<BuiltinBinding> {
    #[cfg(target_os = "linux")]
    {
        builtin_bindings_for_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        builtin_bindings_for(cfg!(windows))
    }
}

/// Bindings for Linux. Uses Ctrl/Ctrl+Shift like GNOME Terminal / Konsole so
/// plain Ctrl+C/V/D/W stay with the shell (VT conventions).
pub fn builtin_bindings_for_linux() -> Vec<BuiltinBinding> {
    let mut out = linux_static_bindings();
    for n in 1..=9usize {
        out.push(BuiltinBinding {
            key: format!("ctrl-{n}"),
            action: BuiltinAction::ActivateTab(n),
            context: BindingContext::Both,
        });
    }
    for (key, action) in font_zoom_key_bindings_for_linux() {
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

/// Bindings for a given OS family. `windows = true` uses Ctrl/Ctrl+Shift.
pub fn builtin_bindings_for(windows: bool) -> Vec<BuiltinBinding> {
    let mut out = if windows {
        windows_static_bindings()
    } else {
        macos_static_bindings()
    };
    let tab_mod = if windows { "ctrl" } else { "cmd" };
    for n in 1..=9usize {
        out.push(BuiltinBinding {
            key: format!("{tab_mod}-{n}"),
            action: BuiltinAction::ActivateTab(n),
            context: BindingContext::Both,
        });
    }
    for (key, action) in font_zoom_key_bindings_for(windows) {
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
    ]
}

fn windows_static_bindings() -> Vec<BuiltinBinding> {
    use BindingContext::{Both, Global, Terminal};
    vec![
        b("alt-f4", BuiltinAction::Quit, Global),
        b("ctrl-q", BuiltinAction::Quit, Global),
        b("ctrl-shift-c", BuiltinAction::Copy, Terminal),
        b("ctrl-insert", BuiltinAction::Copy, Terminal),
        b("ctrl-shift-v", BuiltinAction::Paste, Terminal),
        b("shift-insert", BuiltinAction::Paste, Terminal),
        b("ctrl-v", BuiltinAction::Paste, Terminal),
        b("ctrl-alt-v", BuiltinAction::PasteText, Terminal),
        b("ctrl-shift-a", BuiltinAction::SelectAll, Terminal),
        b("ctrl-shift-l", BuiltinAction::Clear, Terminal),
        b("ctrl-shift-space", BuiltinAction::ToggleViMode, Terminal),
        b("shift-up", BuiltinAction::ScrollLineUp, Terminal),
        b("shift-down", BuiltinAction::ScrollLineDown, Terminal),
        b("shift-pageup", BuiltinAction::ScrollPageUp, Terminal),
        b("shift-pagedown", BuiltinAction::ScrollPageDown, Terminal),
        b("shift-home", BuiltinAction::ScrollToTop, Terminal),
        b("shift-end", BuiltinAction::ScrollToBottom, Terminal),
        b("alt-left", BuiltinAction::SendText("\u{1b}b"), Terminal),
        b("alt-right", BuiltinAction::SendText("\u{1b}f"), Terminal),
        b("alt-b", BuiltinAction::SendText("\u{1b}b"), Terminal),
        b("alt-f", BuiltinAction::SendText("\u{1b}f"), Terminal),
        b("ctrl-delete", BuiltinAction::SendText("\u{1b}[3;5~"), Terminal),
        b("ctrl-shift-t", BuiltinAction::NewTab, Both),
        b("ctrl-shift-w", BuiltinAction::CloseTab, Both),
        b("ctrl-shift-n", BuiltinAction::NewWindow, Both),
        b("ctrl-tab", BuiltinAction::NextTab, Both),
        b("ctrl-shift-tab", BuiltinAction::PrevTab, Both),
        b("ctrl-shift-r", BuiltinAction::ReloadSettings, Both),
        b("ctrl-,", BuiltinAction::OpenSettings, Both),
        b("ctrl-shift-p", BuiltinAction::ToggleCommandPalette, Both),
        b("ctrl-shift-alt-p", BuiltinAction::CycleTheme, Both),
        b("alt-shift-d", BuiltinAction::SplitRight, Both),
        b("alt-shift--", BuiltinAction::SplitDown, Both),
        b("ctrl-alt-left", BuiltinAction::FocusPaneLeft, Both),
        b("ctrl-alt-right", BuiltinAction::FocusPaneRight, Both),
        b("ctrl-alt-up", BuiltinAction::FocusPaneUp, Both),
        b("ctrl-alt-down", BuiltinAction::FocusPaneDown, Both),
        b("ctrl-shift-u", BuiltinAction::CheckForUpdates, Both),
        b("ctrl-shift-f", BuiltinAction::Find, Both),
        b("ctrl-shift-g", BuiltinAction::FindNext, Both),
        b("ctrl-shift-alt-g", BuiltinAction::FindPrev, Both),
        b("ctrl-shift-enter", BuiltinAction::TogglePaneZoom, Both),
        b("ctrl-shift-b", BuiltinAction::ToggleBroadcast, Both),
        b("ctrl-shift-up", BuiltinAction::JumpPrevPrompt, Both),
        b("ctrl-shift-down", BuiltinAction::JumpNextPrompt, Both),
        b("ctrl-shift-o", BuiltinAction::ToggleQuickSelect, Both),
        b("ctrl-alt-n", BuiltinAction::OpenQuickTerminal, Both),
    ]
}

fn b(key: &'static str, action: BuiltinAction, context: BindingContext) -> BuiltinBinding {
    BuiltinBinding {
        key: key.to_string(),
        action,
        context,
    }
}

/// Linux bindings follow terminal-desktop conventions: app chords use
/// Ctrl+Shift (GNOME Terminal / Konsole) and Ctrl+C/D/W stay unbound so the
/// shell still receives them. Alt-as-Meta handles word navigation.
fn linux_static_bindings() -> Vec<BuiltinBinding> {
    use BindingContext::{Both, Global, Terminal};
    vec![
        b("ctrl-shift-q", BuiltinAction::Quit, Global),
        b("ctrl-shift-c", BuiltinAction::Copy, Terminal),
        b("ctrl-insert", BuiltinAction::Copy, Terminal),
        b("ctrl-shift-v", BuiltinAction::Paste, Terminal),
        b("shift-insert", BuiltinAction::Paste, Terminal),
        b("ctrl-shift-a", BuiltinAction::SelectAll, Terminal),
        b("ctrl-shift-l", BuiltinAction::Clear, Terminal),
        b("ctrl-shift-space", BuiltinAction::ToggleViMode, Terminal),
        b("shift-up", BuiltinAction::ScrollLineUp, Terminal),
        b("shift-down", BuiltinAction::ScrollLineDown, Terminal),
        b("shift-pageup", BuiltinAction::ScrollPageUp, Terminal),
        b("shift-pagedown", BuiltinAction::ScrollPageDown, Terminal),
        b("shift-home", BuiltinAction::ScrollToTop, Terminal),
        b("shift-end", BuiltinAction::ScrollToBottom, Terminal),
        b("alt-left", BuiltinAction::SendText("\u{1b}b"), Terminal),
        b("alt-right", BuiltinAction::SendText("\u{1b}f"), Terminal),
        b("alt-b", BuiltinAction::SendText("\u{1b}b"), Terminal),
        b("alt-f", BuiltinAction::SendText("\u{1b}f"), Terminal),
        b("alt-delete", BuiltinAction::SendText("\u{1b}d"), Terminal),
        b("ctrl-delete", BuiltinAction::SendText("\u{1b}[3;5~"), Terminal),
        b("ctrl-shift-t", BuiltinAction::NewTab, Both),
        b("ctrl-shift-w", BuiltinAction::CloseTab, Both),
        b("ctrl-shift-n", BuiltinAction::NewWindow, Both),
        b("ctrl-tab", BuiltinAction::NextTab, Both),
        b("ctrl-shift-tab", BuiltinAction::PrevTab, Both),
        b("ctrl-shift-r", BuiltinAction::ReloadSettings, Both),
        b("ctrl-,", BuiltinAction::OpenSettings, Both),
        b("ctrl-shift-p", BuiltinAction::ToggleCommandPalette, Both),
        b("ctrl-shift-alt-p", BuiltinAction::CycleTheme, Both),
        b("alt-shift-d", BuiltinAction::SplitRight, Both),
        b("alt-shift--", BuiltinAction::SplitDown, Both),
        b("ctrl-alt-left", BuiltinAction::FocusPaneLeft, Both),
        b("ctrl-alt-right", BuiltinAction::FocusPaneRight, Both),
        b("ctrl-alt-up", BuiltinAction::FocusPaneUp, Both),
        b("ctrl-alt-down", BuiltinAction::FocusPaneDown, Both),
        b("ctrl-shift-u", BuiltinAction::CheckForUpdates, Both),
        b("ctrl-shift-f", BuiltinAction::Find, Both),
        b("ctrl-shift-g", BuiltinAction::FindNext, Both),
        b("ctrl-shift-alt-g", BuiltinAction::FindPrev, Both),
        b("ctrl-shift-enter", BuiltinAction::TogglePaneZoom, Both),
        b("ctrl-shift-b", BuiltinAction::ToggleBroadcast, Both),
        b("ctrl-shift-up", BuiltinAction::JumpPrevPrompt, Both),
        b("ctrl-shift-down", BuiltinAction::JumpNextPrompt, Both),
        b("ctrl-shift-o", BuiltinAction::ToggleQuickSelect, Both),
        b("ctrl-alt-n", BuiltinAction::OpenQuickTerminal, Both),
    ]
}

/// Font-zoom keystroke table for the compiling OS.
pub fn font_zoom_key_bindings() -> &'static [(&'static str, &'static str)] {
    #[cfg(target_os = "linux")]
    {
        font_zoom_key_bindings_for_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        font_zoom_key_bindings_for(cfg!(windows))
    }
}

/// Font-zoom table for Linux (same Ctrl chords as Windows).
pub fn font_zoom_key_bindings_for_linux() -> &'static [(&'static str, &'static str)] {
    &[
        ("ctrl-=", "increase_font_size"),
        ("ctrl-+", "increase_font_size"),
        ("ctrl--", "decrease_font_size"),
        ("ctrl-0", "reset_font_size"),
    ]
}

/// Font-zoom keystroke table. `windows = true` uses `ctrl-*`.
pub fn font_zoom_key_bindings_for(windows: bool) -> &'static [(&'static str, &'static str)] {
    if windows {
        &[
            ("ctrl-=", "increase_font_size"),
            ("ctrl-+", "increase_font_size"),
            ("ctrl--", "decrease_font_size"),
            ("ctrl-0", "reset_font_size"),
        ]
    } else {
        &[
            ("cmd-=", "increase_font_size"),
            ("cmd-+", "increase_font_size"),
            ("cmd--", "decrease_font_size"),
            ("cmd-0", "reset_font_size"),
        ]
    }
}

/// Whether the last window closing should terminate the process.
pub fn last_window_close_quits() -> bool {
    last_window_close_quits_for(cfg!(not(target_os = "macos")))
}

/// `non_macos = true` → quit (Windows). macOS keeps the process for Dock reopen.
pub fn last_window_close_quits_for(non_macos: bool) -> bool {
    non_macos
}

/// Human-readable shortcut for a command id on this OS.
pub fn display_shortcut(id: &str) -> &'static str {
    #[cfg(target_os = "linux")]
    {
        display_shortcut_for(id, true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        display_shortcut_for(id, cfg!(windows))
    }
}

/// Human-readable shortcut for a command id.
pub fn display_shortcut_for(id: &str, windows: bool) -> &'static str {
    match (id, windows) {
        ("new_tab", false) => "⌘T",
        ("new_tab", true) => "Ctrl+Shift+T",
        ("close_tab", false) => "⌘W",
        ("close_tab", true) => "Ctrl+Shift+W",
        ("next_tab", _) => "⌃Tab",
        ("prev_tab", _) => "⌃⇧Tab",
        ("split_right", false) => "⌘D",
        ("split_right", true) => "Alt+Shift+D",
        ("split_down", false) => "⌘⇧D",
        ("split_down", true) => "Alt+Shift+-",
        ("find", false) => "⌘F",
        ("find", true) => "Ctrl+Shift+F",
        ("open_settings", false) => "⌘,",
        ("open_settings", true) => "Ctrl+,",
        ("reload_settings", false) => "⌘⇧R",
        ("reload_settings", true) => "Ctrl+Shift+R",
        ("cycle_theme", false) => "⌘⇧P",
        ("cycle_theme", true) => "Ctrl+Shift+Alt+P",
        ("check_for_updates", false) => "⌘⇧U",
        ("check_for_updates", true) => "Ctrl+Shift+U",
        ("toggle_command_palette", false) => "⌘⇧K",
        ("toggle_command_palette", true) => "Ctrl+Shift+P",
        ("new_window", false) => "⌘N",
        ("new_window", true) => "Ctrl+Shift+N",
        ("increase_font_size", false) => "⌘+",
        ("increase_font_size", true) => "Ctrl++",
        ("decrease_font_size", false) => "⌘-",
        ("decrease_font_size", true) => "Ctrl+-",
        ("reset_font_size", false) => "⌘0",
        ("reset_font_size", true) => "Ctrl+0",
        ("toggle_pane_zoom", false) => "⌘⇧Enter",
        ("toggle_pane_zoom", true) => "Ctrl+Shift+Enter",
        ("toggle_broadcast", false) => "⌘⇧B",
        ("toggle_broadcast", true) => "Ctrl+Shift+B",
        ("jump_prev_prompt", false) => "⌘⇧↑",
        ("jump_prev_prompt", true) => "Ctrl+Shift+↑",
        ("jump_next_prompt", false) => "⌘⇧↓",
        ("jump_next_prompt", true) => "Ctrl+Shift+↓",
        ("toggle_quick_select", false) => "⌘⇧O",
        ("toggle_quick_select", true) => "Ctrl+Shift+O",
        ("open_quick_terminal", false) => "⌘⇧N",
        ("open_quick_terminal", true) => "Ctrl+Alt+N",
        ("secondary_click", false) => "⌘",
        ("secondary_click", true) => "Ctrl",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_for(windows: bool) -> Vec<String> {
        builtin_bindings_for(windows)
            .into_iter()
            .map(|b| b.key)
            .collect()
    }

    fn keys_for_linux() -> Vec<String> {
        builtin_bindings_for_linux()
            .into_iter()
            .map(|b| b.key)
            .collect()
    }

    #[test]
    fn linux_bindings_use_ctrl_shift_and_leave_shell_keys() {
        let keys = keys_for_linux();
        assert!(keys.iter().any(|k| k == "ctrl-shift-t"));
        assert!(keys.iter().any(|k| k == "ctrl-shift-w"));
        assert!(keys.iter().any(|k| k == "ctrl-shift-c"));
        assert!(keys.iter().any(|k| k == "ctrl-shift-v"));
        assert!(
            !keys
                .iter()
                .any(|k| k == "ctrl-c" || k == "ctrl-w" || k == "ctrl-d" || k == "ctrl-v"),
            "Linux table must not steal plain Ctrl+C/W/D/V: {keys:?}"
        );
        assert!(
            !keys.iter().any(|k| k.starts_with("cmd-")),
            "Linux table must not use cmd-: {keys:?}"
        );
    }

    #[test]
    fn windows_bindings_use_ctrl_and_do_not_steal_shell_keys() {
        let keys = keys_for(true);
        assert!(keys.iter().any(|k| k == "ctrl-shift-t"));
        assert!(keys.iter().any(|k| k == "ctrl-shift-w"));
        assert!(keys.iter().any(|k| k == "ctrl-v"));
        assert!(
            !keys.iter().any(|k| k == "ctrl-c" || k == "ctrl-w" || k == "ctrl-d"),
            "must not bind Ctrl+C/W/D as app shortcuts: {keys:?}"
        );
        assert!(
            !keys.iter().any(|k| k.starts_with("cmd-")),
            "Windows table must not use cmd-: {keys:?}"
        );
        let actions: Vec<_> = builtin_bindings_for(true)
            .into_iter()
            .filter(|b| b.key == "ctrl-v")
            .map(|b| b.action)
            .collect();
        assert_eq!(actions, vec![BuiltinAction::Paste]);
    }

    #[test]
    fn macos_bindings_keep_cmd() {
        let keys = keys_for(false);
        assert!(keys.iter().any(|k| k == "cmd-t"));
        assert!(keys.iter().any(|k| k == "cmd-c"));
        assert!(keys.iter().any(|k| k == "cmd-w"));
        assert!(keys.iter().any(|k| k == "cmd-d"));
        assert!(!keys.iter().any(|k| k == "ctrl-v"));
    }

    #[test]
    fn font_zoom_tables_are_os_specific() {
        let win = font_zoom_key_bindings_for(true);
        assert!(
            win.iter()
                .any(|(k, a)| *k == "ctrl-+" && *a == "increase_font_size")
        );
        assert!(
            win.iter()
                .any(|(k, a)| *k == "ctrl--" && *a == "decrease_font_size")
        );
        assert!(win.iter().all(|(k, _)| k.starts_with("ctrl-")));

        let mac = font_zoom_key_bindings_for(false);
        assert!(
            mac.iter()
                .any(|(k, a)| *k == "cmd-+" && *a == "increase_font_size")
        );
        assert!(mac.iter().all(|(k, _)| k.starts_with("cmd-")));
    }

    #[test]
    fn last_window_quits_off_macos() {
        assert!(!last_window_close_quits_for(false));
        assert!(last_window_close_quits_for(true));
        assert_eq!(
            last_window_close_quits(),
            cfg!(not(target_os = "macos"))
        );
    }
}
