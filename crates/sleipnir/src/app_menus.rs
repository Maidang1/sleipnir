//! Native application menu bar (Shell / File / Edit / View / Window).
//!
//! Mirrors the discoverability surface of Terminal.app / Windows Terminal,
//! wiring existing GPUI actions so menu items and keybindings share one path.

use gpui::{Menu, MenuItem, SystemMenuType, actions};
use sleipnir_settings::Language;
use sleipnir_ui::{
    CheckForUpdates, CloseTab, CycleTheme, DecreaseFontSize, ExportScrollback, FocusPaneDown,
    FocusPaneLeft, FocusPaneRight, FocusPaneUp, IncreaseFontSize, JumpNextPrompt, JumpPrevPrompt,
    MarkTabSeen, NewTab, NewWindow, NextTab, OpenQuickTerminal, OpenSettings, PipeSelection,
    PrevTab, ReloadSettings, ResetFontSize, SendGitDiff, SendSelection, SplitDown, SplitRight,
    ToggleBroadcast, ToggleBrowser, ToggleDiff, ToggleHistorySearch, TogglePaneFacts,
    TogglePaneZoom, TogglePluginMonitor, ToggleQuickSelect,
};
use terminal::{Clear, Copy, Paste, PasteText, ToggleViMode};

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
pub fn app_menus(language: Language) -> Vec<Menu> {
    let menus = if cfg!(target_os = "macos") {
        macos_menus(language)
    } else {
        desktop_menus(language)
    };
    debug_assert_eq!(menus.len(), app_menu_bar_titles().len());
    menus
}

fn tr(language: Language, key: &'static str) -> &'static str {
    language.text(key)
}

fn shared_edit_view_window(language: Language) -> [Menu; 3] {
    [
        Menu::new(tr(language, "menu.edit")).items([
            MenuItem::action(tr(language, "menu.copy"), Copy),
            MenuItem::action(tr(language, "menu.paste"), Paste),
            MenuItem::action(tr(language, "menu.paste_text_only"), PasteText),
        ]),
        Menu::new(tr(language, "menu.view")).items([
            MenuItem::action(tr(language, "menu.settings"), OpenSettings),
            MenuItem::action(tr(language, "menu.reload_settings"), ReloadSettings),
            MenuItem::action(tr(language, "menu.cycle_theme"), CycleTheme),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.increase_font_size"), IncreaseFontSize),
            MenuItem::action(tr(language, "menu.decrease_font_size"), DecreaseFontSize),
            MenuItem::action(tr(language, "menu.reset_font_size"), ResetFontSize),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.toggle_pane_zoom"), TogglePaneZoom),
            MenuItem::action(tr(language, "menu.toggle_broadcast"), ToggleBroadcast),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.previous_prompt"), JumpPrevPrompt),
            MenuItem::action(tr(language, "menu.next_prompt"), JumpNextPrompt),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.quick_select"), ToggleQuickSelect),
            MenuItem::action(tr(language, "menu.quick_terminal"), OpenQuickTerminal),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.pane_facts"), TogglePaneFacts),
            MenuItem::action(tr(language, "menu.diff_inspector"), ToggleDiff),
            MenuItem::action(tr(language, "menu.browser_panel"), ToggleBrowser),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.toggle_vi_mode"), ToggleViMode),
        ]),
        // Name must be exactly "Window" so GPUI registers it as the system
        // Windows menu (Minimize / Zoom / Bring All to Front are added by AppKit).
        Menu::new("Window").items([
            MenuItem::action(tr(language, "menu.new_window"), NewWindow),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.next_tab"), NextTab),
            MenuItem::action(tr(language, "menu.previous_tab"), PrevTab),
        ]),
    ]
}

fn macos_menus(language: Language) -> Vec<Menu> {
    let [edit, view, window] = shared_edit_view_window(language);
    vec![
        Menu::new("Sleipnir").items([
            MenuItem::action(tr(language, "menu.settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.check_updates"), CheckForUpdates),
            MenuItem::separator(),
            MenuItem::os_submenu(tr(language, "menu.services"), SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.hide_sleipnir"), Hide),
            MenuItem::action(tr(language, "menu.hide_others"), HideOthers),
            MenuItem::action(tr(language, "menu.show_all"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.quit_sleipnir"), Quit),
        ]),
        Menu::new(tr(language, "menu.shell")).items([
            MenuItem::action(tr(language, "menu.new_window"), NewWindow),
            MenuItem::action(tr(language, "menu.new_tab"), NewTab),
            MenuItem::action(tr(language, "menu.close"), CloseTab),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.split_right"), SplitRight),
            MenuItem::action(tr(language, "menu.split_down"), SplitDown),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new(tr(language, "menu.focus_pane")).items([
                MenuItem::action(tr(language, "menu.left"), FocusPaneLeft),
                MenuItem::action(tr(language, "menu.right"), FocusPaneRight),
                MenuItem::action(tr(language, "menu.up"), FocusPaneUp),
                MenuItem::action(tr(language, "menu.down"), FocusPaneDown),
            ])),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.clear"), Clear),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.export_scrollback"), ExportScrollback),
            MenuItem::action(tr(language, "menu.mark_tab_seen"), MarkTabSeen),
            MenuItem::action(tr(language, "menu.plugin_monitor"), TogglePluginMonitor),
            MenuItem::action(tr(language, "menu.send_selection"), SendSelection),
            MenuItem::action(tr(language, "menu.pipe_selection"), PipeSelection),
            MenuItem::action(tr(language, "menu.send_git_diff"), SendGitDiff),
            MenuItem::action(tr(language, "menu.search_history"), ToggleHistorySearch),
        ]),
        edit,
        view,
        window,
    ]
}

fn desktop_menus(language: Language) -> Vec<Menu> {
    let [edit, view, window] = shared_edit_view_window(language);
    vec![
        Menu::new(tr(language, "menu.file")).items([
            MenuItem::action(tr(language, "menu.new_window"), NewWindow),
            MenuItem::action(tr(language, "menu.new_tab"), NewTab),
            MenuItem::action(tr(language, "menu.close"), CloseTab),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.split_right"), SplitRight),
            MenuItem::action(tr(language, "menu.split_down"), SplitDown),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new(tr(language, "menu.focus_pane")).items([
                MenuItem::action(tr(language, "menu.left"), FocusPaneLeft),
                MenuItem::action(tr(language, "menu.right"), FocusPaneRight),
                MenuItem::action(tr(language, "menu.up"), FocusPaneUp),
                MenuItem::action(tr(language, "menu.down"), FocusPaneDown),
            ])),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.clear"), Clear),
            MenuItem::action(tr(language, "menu.export_scrollback"), ExportScrollback),
            MenuItem::action(tr(language, "menu.mark_tab_seen"), MarkTabSeen),
            MenuItem::action(tr(language, "menu.plugin_monitor"), TogglePluginMonitor),
            MenuItem::action(tr(language, "menu.send_selection"), SendSelection),
            MenuItem::action(tr(language, "menu.pipe_selection"), PipeSelection),
            MenuItem::action(tr(language, "menu.send_git_diff"), SendGitDiff),
            MenuItem::action(tr(language, "menu.search_history"), ToggleHistorySearch),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.check_updates"), CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action(tr(language, "menu.exit"), Quit),
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

    #[test]
    fn chinese_labels_are_available() {
        assert_eq!(tr(Language::ZhCn, "menu.file"), "文件");
        assert_eq!(tr(Language::En, "menu.file"), "File");
    }

    #[test]
    fn non_macos_menus_never_expose_hide_actions() {
        // gpui's Windows `hide()` is a no-op and `hide_other_apps()` /
        // `unhide_other_apps()` are `unimplemented!()`, so the non-macOS
        // menu builder must never reference Hide / HideOthers / ShowAll.
        let src = include_str!("app_menus.rs");
        let desktop = src
            .split("fn desktop_menus(")
            .nth(1)
            .expect("desktop_menus body");
        let desktop = desktop
            .split("#[cfg(test)]")
            .next()
            .expect("desktop_menus body before tests");
        for token in ["Hide", "ShowAll"] {
            assert!(
                !desktop.contains(token),
                "non-macOS menu must not expose {token}"
            );
        }
    }
}
