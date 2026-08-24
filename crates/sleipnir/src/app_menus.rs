//! Native application menu bar (Shell / File / Edit / View / Window).
//!
//! Mirrors the discoverability surface of Terminal.app / Windows Terminal,
//! wiring existing GPUI actions so menu items and keybindings share one path.

use gpui::{Menu, MenuItem, SystemMenuType, actions};
use sleipnir_ui::{
    CheckForUpdates, ClearRunLedger, CloseTab, CycleTheme, DecreaseFontSize, ExportScrollback,
    FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, IncreaseFontSize, JumpNextPrompt,
    JumpPrevPrompt, MarkTabSeen, NewTab, NewWindow, NextTab, OpenQuickTerminal, OpenSettings,
    PipeSelection, PrevTab, ReloadSettings, ResetFontSize, SendGitDiff, SendSelection, SplitDown,
    SplitRight, ToggleBroadcast, ToggleDiff, ToggleHistorySearch, TogglePaneFacts, TogglePaneZoom,
    ToggleQuickSelect, ToggleRunLedger,
};
use terminal::{Clear, Copy, Paste, PasteText, SelectAll, ToggleViMode};

actions!(
    sleipnir_app,
    [
        /// Quit the application (⌘Q / Alt+F4).
        Quit,
        /// Hide Sleipnir (⌘H). macOS only.
        Hide,
        /// Hide other applications (⌥⌘H). macOS only.
        HideOthers,
        /// Show all applications.
        ShowAll,
    ]
);

/// Top-level menu titles for this OS.
pub fn app_menu_bar_titles() -> &'static [&'static str] {
    app_menu_bar_titles_for(cfg!(target_os = "macos"))
}

pub fn app_menu_bar_titles_for(macos: bool) -> &'static [&'static str] {
    if macos {
        &["Sleipnir", "Shell", "Edit", "View", "Window"]
    } else {
        &["File", "Edit", "View", "Window"]
    }
}

/// Build the main menu bar. First entry is the application menu on macOS.
pub fn app_menus() -> Vec<Menu> {
    let menus = if cfg!(target_os = "macos") {
        macos_menus()
    } else {
        desktop_menus()
    };
    debug_assert_eq!(menus.len(), app_menu_bar_titles().len());
    menus
}

fn shared_edit_view_window() -> [Menu; 3] {
    [
        Menu::new("Edit").items([
            MenuItem::action("Copy", Copy),
            MenuItem::action("Paste", Paste),
            MenuItem::action("Paste Text Only", PasteText),
            MenuItem::separator(),
            MenuItem::action("Select All", SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Reload Settings", ReloadSettings),
            MenuItem::action("Cycle Theme", CycleTheme),
            MenuItem::separator(),
            MenuItem::action("Increase Font Size", IncreaseFontSize),
            MenuItem::action("Decrease Font Size", DecreaseFontSize),
            MenuItem::action("Reset Font Size", ResetFontSize),
            MenuItem::separator(),
            MenuItem::action("Toggle Pane Zoom", TogglePaneZoom),
            MenuItem::action("Toggle Broadcast Input", ToggleBroadcast),
            MenuItem::separator(),
            MenuItem::action("Previous Prompt", JumpPrevPrompt),
            MenuItem::action("Next Prompt", JumpNextPrompt),
            MenuItem::separator(),
            MenuItem::action("Quick Select", ToggleQuickSelect),
            MenuItem::action("Quick Terminal", OpenQuickTerminal),
            MenuItem::separator(),
            MenuItem::action("Pane Facts", TogglePaneFacts),
            MenuItem::action("Diff Inspector", ToggleDiff),
            MenuItem::separator(),
            MenuItem::action("Toggle Vi Mode", ToggleViMode),
        ]),
        // Name must be exactly "Window" so GPUI registers it as the system
        // Windows menu (Minimize / Zoom / Bring All to Front are added by AppKit).
        Menu::new("Window").items([
            MenuItem::action("New Window", NewWindow),
            MenuItem::separator(),
            MenuItem::action("Next Tab", NextTab),
            MenuItem::action("Previous Tab", PrevTab),
        ]),
    ]
}

fn macos_menus() -> Vec<Menu> {
    let [edit, view, window] = shared_edit_view_window();
    vec![
        Menu::new("Sleipnir").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Sleipnir", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Sleipnir", Quit),
        ]),
        Menu::new("Shell").items([
            MenuItem::action("New Window", NewWindow),
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("Close", CloseTab),
            MenuItem::separator(),
            MenuItem::action("Split Right", SplitRight),
            MenuItem::action("Split Down", SplitDown),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Focus Pane").items([
                MenuItem::action("Left", FocusPaneLeft),
                MenuItem::action("Right", FocusPaneRight),
                MenuItem::action("Up", FocusPaneUp),
                MenuItem::action("Down", FocusPaneDown),
            ])),
            MenuItem::separator(),
            MenuItem::action("Clear", Clear),
            MenuItem::separator(),
            MenuItem::action("Export Scrollback…", ExportScrollback),
            MenuItem::action("Clear Run Ledger", ClearRunLedger),
            MenuItem::action("Mark Tab as Seen", MarkTabSeen),
            MenuItem::action("Run Ledger", ToggleRunLedger),
            MenuItem::action("Send Selection to Pane", SendSelection),
            MenuItem::action("Pipe Selection to Command", PipeSelection),
            MenuItem::action("Send Git Diff to Pane", SendGitDiff),
            MenuItem::action("Search Shell History", ToggleHistorySearch),
        ]),
        edit,
        view,
        window,
    ]
}

fn desktop_menus() -> Vec<Menu> {
    let [edit, view, window] = shared_edit_view_window();
    vec![
        Menu::new("File").items([
            MenuItem::action("New Window", NewWindow),
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("Close", CloseTab),
            MenuItem::separator(),
            MenuItem::action("Split Right", SplitRight),
            MenuItem::action("Split Down", SplitDown),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Focus Pane").items([
                MenuItem::action("Left", FocusPaneLeft),
                MenuItem::action("Right", FocusPaneRight),
                MenuItem::action("Up", FocusPaneUp),
                MenuItem::action("Down", FocusPaneDown),
            ])),
            MenuItem::separator(),
            MenuItem::action("Clear", Clear),
            MenuItem::action("Export Scrollback…", ExportScrollback),
            MenuItem::action("Clear Run Ledger", ClearRunLedger),
            MenuItem::action("Mark Tab as Seen", MarkTabSeen),
            MenuItem::action("Run Ledger", ToggleRunLedger),
            MenuItem::action("Send Selection to Pane", SendSelection),
            MenuItem::action("Pipe Selection to Command", PipeSelection),
            MenuItem::action("Send Git Diff to Pane", SendGitDiff),
            MenuItem::action("Search Shell History", ToggleHistorySearch),
            MenuItem::separator(),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Exit", Quit),
        ]),
        edit,
        view,
        window,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_menu_bar_uses_file_layout() {
        assert_eq!(
            app_menu_bar_titles_for(false),
            &["File", "Edit", "View", "Window"]
        );
    }

    #[test]
    fn windows_menu_bar_has_file_not_app_menu() {
        assert_eq!(
            app_menu_bar_titles_for(false),
            &["File", "Edit", "View", "Window"]
        );
        assert!(!app_menu_bar_titles_for(false).contains(&"Sleipnir"));
    }

    #[test]
    fn macos_menu_bar_keeps_app_and_shell() {
        assert_eq!(
            app_menu_bar_titles_for(true),
            &["Sleipnir", "Shell", "Edit", "View", "Window"]
        );
    }
}
