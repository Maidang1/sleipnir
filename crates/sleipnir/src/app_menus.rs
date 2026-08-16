//! Native macOS application menu bar (Shell / Edit / View / Window).
//!
//! Mirrors the discoverability surface of Terminal.app / Kaku / iTerm2, wiring
//! existing GPUI actions so menu items and keybindings share one code path.

use gpui::{actions, Menu, MenuItem, SystemMenuType};
use sleipnir_ui::{
    CheckForUpdates, ClearRunLedger, CloseTab, CycleTheme, DecreaseFontSize, ExportScrollback,
    FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, IncreaseFontSize, JumpNextPrompt,
    JumpPrevPrompt, NewTab, NewWindow, NextTab, OpenQuickTerminal, OpenSettings, PrevTab,
    ReloadSettings, ResetFontSize, SplitDown, SplitRight, ToggleBroadcast, TogglePaneZoom,
    ToggleQuickSelect,
};
use terminal::{Clear, Copy, Paste, PasteText, SelectAll, ToggleViMode};

actions!(
    sleipnir_app,
    [
        /// Quit the application (⌘Q).
        Quit,
        /// Hide Sleipnir (⌘H).
        Hide,
        /// Hide other applications (⌥⌘H).
        HideOthers,
        /// Show all applications.
        ShowAll,
    ]
);

/// Top-level menu titles for this OS.
pub fn app_menu_bar_titles() -> &'static [&'static str] {
    #[cfg(target_os = "linux")]
    {
        &["File", "Edit", "View", "Window"]
    }
    #[cfg(not(target_os = "linux"))]
    {
        app_menu_bar_titles_for(cfg!(windows))
    }
}

/// Top-level menu titles for a macOS-style app-menu layout. macOS only; kept
/// for the shared `macos_menus`/`windows_menus` builders and their tests.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn app_menu_bar_titles_for(windows: bool) -> &'static [&'static str] {
    if windows {
        &["File", "Edit", "View", "Window"]
    } else {
        &["Sleipnir", "Shell", "Edit", "View", "Window"]
    }
}

/// Build the main menu bar. First entry is the application menu on macOS.
pub fn app_menus() -> Vec<Menu> {
    #[cfg(target_os = "linux")]
    let menus = linux_menus();
    #[cfg(not(target_os = "linux"))]
    let menus = if cfg!(windows) {
        windows_menus()
    } else {
        macos_menus()
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

#[cfg_attr(target_os = "linux", allow(dead_code))]
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
        ]),
        edit,
        view,
        window,
    ]
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn windows_menus() -> Vec<Menu> {
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

/// Linux menus: no app menu, no macOS Services/Hide items. Mirrors the Windows
/// layout (File contains shell/quit) since Linux desktops expose no native
/// menu bar; GPUI stores these for key-equivalent validation only.
fn linux_menus() -> Vec<Menu> {
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
            MenuItem::action("Clear Run Ledger", ClearRunLedger),
            MenuItem::separator(),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Quit Sleipnir", Quit),
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
    fn windows_menu_bar_has_file_not_app_menu() {
        assert_eq!(
            app_menu_bar_titles_for(true),
            &["File", "Edit", "View", "Window"]
        );
        assert!(!app_menu_bar_titles_for(true).contains(&"Sleipnir"));
    }

    #[test]
    fn macos_menu_bar_keeps_app_and_shell() {
        assert_eq!(
            app_menu_bar_titles_for(false),
            &["Sleipnir", "Shell", "Edit", "View", "Window"]
        );
    }
}
