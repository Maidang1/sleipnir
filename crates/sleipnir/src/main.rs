//! Sleipnir — standalone macOS terminal (HIG-aligned window chrome).

mod app_menus;

use app_menus::{Hide, HideOthers, Quit, ShowAll, app_menus};
use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, px, size,
};
use gpui_platform::application;
use release_channel::AppVersion;
use sleipnir_settings::{self, KeyBindingSpec, TerminalSettings};
use sleipnir_ui::CheckForUpdates;
use sleipnir_ui::{
    ActivateTab, AppShell, ChromeGeometry, CloseTab, CycleTheme, Find, FindNext, FindPrev,
    FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, NewTab, NextTab, OpenSettings,
    PrevTab, ReloadSettings, SplitDown, SplitRight, ToggleCommandPalette,
};
use terminal::{
    Clear, Copy, Paste, PasteText, ScrollLineDown, ScrollLineUp, ScrollPageDown, ScrollPageUp,
    ScrollToBottom, ScrollToTop, SelectAll, SendKeystroke, SendText, ShowCharacterPalette,
    ToggleViMode,
};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    application().run(|cx: &mut App| {
        AppVersion::init_with(env!("CARGO_PKG_VERSION"), cx);
        sleipnir_settings::init(cx);

        // App-menu actions (always available so validation enables the items).
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());

        /// Bind an action in both AppShell and Terminal contexts.
        fn bind_both(
            key: &str,
            action: impl gpui::Action + Clone,
        ) -> [KeyBinding; 2] {
            [
                KeyBinding::new(key, action.clone(), Some("AppShell")),
                KeyBinding::new(key, action, Some("Terminal")),
            ]
        }

        cx.bind_keys([
            // Application (menu bar + macOS conventions)
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
            // Clipboard
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("ctrl-shift-c", Copy, Some("Terminal")),
            KeyBinding::new("ctrl-shift-v", Paste, Some("Terminal")),
            KeyBinding::new("ctrl-cmd-v", PasteText, Some("Terminal")),
            // Select / clear / character palette / vi mode
            KeyBinding::new("cmd-a", SelectAll, Some("Terminal")),
            KeyBinding::new("cmd-k", Clear, Some("Terminal")),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("Terminal")),
            KeyBinding::new("ctrl-shift-space", ToggleViMode, Some("Terminal")),
            // Scrollback (disabled for alt-screen apps in the handlers)
            KeyBinding::new("shift-up", ScrollLineUp, Some("Terminal")),
            KeyBinding::new("shift-down", ScrollLineDown, Some("Terminal")),
            KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
            KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
            KeyBinding::new("cmd-up", ScrollPageUp, Some("Terminal")),
            KeyBinding::new("cmd-down", ScrollPageDown, Some("Terminal")),
            KeyBinding::new("shift-home", ScrollToTop, Some("Terminal")),
            KeyBinding::new("shift-end", ScrollToBottom, Some("Terminal")),
            KeyBinding::new("cmd-home", ScrollToTop, Some("Terminal")),
            KeyBinding::new("cmd-end", ScrollToBottom, Some("Terminal")),
            // Shell line editing conveniences (Zed Terminal parity)
            KeyBinding::new(
                "cmd-backspace",
                SendKeystroke("ctrl-u".into()),
                Some("Terminal"),
            ),
            KeyBinding::new(
                "cmd-delete",
                SendKeystroke("ctrl-k".into()),
                Some("Terminal"),
            ),
            KeyBinding::new("cmd-left", SendKeystroke("ctrl-a".into()), Some("Terminal")),
            KeyBinding::new("cmd-right", SendKeystroke("ctrl-e".into()), Some("Terminal")),
            KeyBinding::new("alt-left", SendText("\u{1b}b".into()), Some("Terminal")),
            KeyBinding::new("alt-right", SendText("\u{1b}f".into()), Some("Terminal")),
            KeyBinding::new("alt-b", SendText("\u{1b}b".into()), Some("Terminal")),
            KeyBinding::new("alt-f", SendText("\u{1b}f".into()), Some("Terminal")),
            KeyBinding::new("alt-delete", SendText("\u{1b}d".into()), Some("Terminal")),
            KeyBinding::new(
                "ctrl-delete",
                SendText("\u{1b}[3;5~".into()),
                Some("Terminal"),
            ),
        ]);

        // Tabs, splits, and navigation — bound in both AppShell and Terminal.
        let dual_bindings: Vec<KeyBinding> = [
            bind_both("cmd-t", NewTab),
            bind_both("cmd-w", CloseTab),
            bind_both("cmd-shift-]", NextTab),
            bind_both("cmd-shift-[", PrevTab),
            bind_both("ctrl-tab", NextTab),
            bind_both("ctrl-shift-tab", PrevTab),
            bind_both("cmd-shift-r", ReloadSettings),
            bind_both("cmd-,", OpenSettings),
            bind_both("cmd-shift-p", CycleTheme),
            bind_both("cmd-d", SplitRight),
            bind_both("cmd-shift-d", SplitDown),
            bind_both("cmd-alt-left", FocusPaneLeft),
            bind_both("cmd-alt-right", FocusPaneRight),
            bind_both("cmd-alt-up", FocusPaneUp),
            bind_both("cmd-alt-down", FocusPaneDown),
            bind_both("cmd-shift-u", CheckForUpdates),
            bind_both("cmd-shift-k", ToggleCommandPalette),
            bind_both("cmd-f", Find),
            bind_both("cmd-g", FindNext),
            bind_both("cmd-shift-g", FindPrev),
        ]
        .into_iter()
        .flatten()
        .collect();
        cx.bind_keys(dual_bindings);

        // Jump directly to tab N (⌘1..⌘9); 1-based, out-of-range is a no-op.
        let tab_bindings: Vec<KeyBinding> = (1..=9u8)
            .flat_map(|n| bind_both(&format!("cmd-{n}"), ActivateTab(n as usize)))
            .collect();
        cx.bind_keys(tab_bindings);

        // User key binding overrides from settings.json (M9).
        bind_user_key_bindings(cx);

        // After keybindings so menu items pick up key equivalents from the keymap.
        cx.set_menus(app_menus());

        let geo = ChromeGeometry::standard();
        let bounds = Bounds::centered(None, size(px(1024.0), px(680.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Sleipnir".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(geo.traffic_light_position),
                }),
                app_owns_titlebar_drag: true,
                window_background: WindowBackgroundAppearance::Opaque,
                window_min_size: Some(size(px(360.0), px(240.0))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| AppShell::new(window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}

/// Layer optional user key bindings from `settings.json` on top of builtins.
fn bind_user_key_bindings(cx: &mut App) {
    let bindings = TerminalSettings::get_global(cx).key_bindings.clone();
    if bindings.is_empty() {
        return;
    }
    let mut keys = Vec::new();
    for spec in &bindings {
        let expanded = key_bindings_for_spec(spec);
        if expanded.is_empty() {
            log::warn!(
                "unknown key binding action {:?} for key {:?}",
                spec.action,
                spec.key
            );
        } else {
            keys.extend(expanded);
        }
    }
    if !keys.is_empty() {
        log::info!("applying {} user key binding(s)", keys.len());
        cx.bind_keys(keys);
    }
}

fn key_bindings_for_spec(spec: &KeyBindingSpec) -> Vec<KeyBinding> {
    let contexts: Vec<&str> = match spec.context.as_deref() {
        None | Some("") => vec!["AppShell", "Terminal"],
        Some(c) => vec![c],
    };
    let mut out = Vec::new();
    for ctx in contexts {
        let kb = match spec.action.as_str() {
            "new_tab" => KeyBinding::new(&spec.key, NewTab, Some(ctx)),
            "close_tab" | "close_pane" => KeyBinding::new(&spec.key, CloseTab, Some(ctx)),
            "next_tab" => KeyBinding::new(&spec.key, NextTab, Some(ctx)),
            "prev_tab" => KeyBinding::new(&spec.key, PrevTab, Some(ctx)),
            "split_right" => KeyBinding::new(&spec.key, SplitRight, Some(ctx)),
            "split_down" => KeyBinding::new(&spec.key, SplitDown, Some(ctx)),
            "open_settings" => KeyBinding::new(&spec.key, OpenSettings, Some(ctx)),
            "reload_settings" => KeyBinding::new(&spec.key, ReloadSettings, Some(ctx)),
            "cycle_theme" => KeyBinding::new(&spec.key, CycleTheme, Some(ctx)),
            "check_for_updates" => KeyBinding::new(&spec.key, CheckForUpdates, Some(ctx)),
            "find" | "search" => KeyBinding::new(&spec.key, Find, Some(ctx)),
            "find_next" => KeyBinding::new(&spec.key, FindNext, Some(ctx)),
            "find_prev" => KeyBinding::new(&spec.key, FindPrev, Some(ctx)),
            "toggle_command_palette" | "command_palette" => {
                KeyBinding::new(&spec.key, ToggleCommandPalette, Some(ctx))
            }
            // Terminal actions (scroll, clipboard, send)
            "copy" => KeyBinding::new(&spec.key, Copy, Some(ctx)),
            "paste" => KeyBinding::new(&spec.key, Paste, Some(ctx)),
            "paste_text" => KeyBinding::new(&spec.key, PasteText, Some(ctx)),
            "select_all" => KeyBinding::new(&spec.key, SelectAll, Some(ctx)),
            "clear" => KeyBinding::new(&spec.key, Clear, Some(ctx)),
            "scroll_line_up" => KeyBinding::new(&spec.key, ScrollLineUp, Some(ctx)),
            "scroll_line_down" => KeyBinding::new(&spec.key, ScrollLineDown, Some(ctx)),
            "scroll_page_up" => KeyBinding::new(&spec.key, ScrollPageUp, Some(ctx)),
            "scroll_page_down" => KeyBinding::new(&spec.key, ScrollPageDown, Some(ctx)),
            "scroll_to_top" => KeyBinding::new(&spec.key, ScrollToTop, Some(ctx)),
            "scroll_to_bottom" => KeyBinding::new(&spec.key, ScrollToBottom, Some(ctx)),
            "toggle_vi_mode" => KeyBinding::new(&spec.key, ToggleViMode, Some(ctx)),
            "show_character_palette" => {
                KeyBinding::new(&spec.key, ShowCharacterPalette, Some(ctx))
            }
            _ => continue,
        };
        out.push(kb);
    }
    out
}
