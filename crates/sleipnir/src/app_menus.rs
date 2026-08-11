//! Native macOS application menu bar (Shell / Edit / View / Window).
//!
//! Mirrors the discoverability surface of Terminal.app / Kaku / iTerm2, wiring
//! existing GPUI actions so menu items and keybindings share one code path.

use gpui::{Menu, MenuItem, SystemMenuType, actions};
use sleipnir_ui::{
    CheckForUpdates, CloseTab, CycleTheme, FocusPaneDown, FocusPaneLeft, FocusPaneRight,
    FocusPaneUp, NewTab, NextTab, OpenSettings, PrevTab, ReloadSettings, SplitDown, SplitRight,
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

/// Build the main menu bar. First entry is the application menu on macOS.
pub fn app_menus() -> Vec<Menu> {
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
        ]),
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
            MenuItem::action("Toggle Vi Mode", ToggleViMode),
        ]),
        // Name must be exactly "Window" so GPUI registers it as the system
        // Windows menu (Minimize / Zoom / Bring All to Front are added by AppKit).
        Menu::new("Window").items([
            MenuItem::action("Next Tab", NextTab),
            MenuItem::action("Previous Tab", PrevTab),
        ]),
    ]
}
