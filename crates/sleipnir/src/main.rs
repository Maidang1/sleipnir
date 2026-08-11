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
            // Tabs
            KeyBinding::new("cmd-t", NewTab, Some("AppShell")),
            KeyBinding::new("cmd-w", CloseTab, Some("AppShell")),
            KeyBinding::new("cmd-shift-]", NextTab, Some("AppShell")),
            KeyBinding::new("cmd-shift-[", PrevTab, Some("AppShell")),
            KeyBinding::new("ctrl-tab", NextTab, Some("AppShell")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("AppShell")),
            KeyBinding::new("cmd-t", NewTab, Some("Terminal")),
            KeyBinding::new("cmd-w", CloseTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-]", NextTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-[", PrevTab, Some("Terminal")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Terminal")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Terminal")),
            // Jump directly to tab N (⌘1..⌘9); 1-based, out-of-range is a no-op.
            KeyBinding::new("cmd-1", ActivateTab(1), Some("AppShell")),
            KeyBinding::new("cmd-2", ActivateTab(2), Some("AppShell")),
            KeyBinding::new("cmd-3", ActivateTab(3), Some("AppShell")),
            KeyBinding::new("cmd-4", ActivateTab(4), Some("AppShell")),
            KeyBinding::new("cmd-5", ActivateTab(5), Some("AppShell")),
            KeyBinding::new("cmd-6", ActivateTab(6), Some("AppShell")),
            KeyBinding::new("cmd-7", ActivateTab(7), Some("AppShell")),
            KeyBinding::new("cmd-8", ActivateTab(8), Some("AppShell")),
            KeyBinding::new("cmd-9", ActivateTab(9), Some("AppShell")),
            KeyBinding::new("cmd-1", ActivateTab(1), Some("Terminal")),
            KeyBinding::new("cmd-2", ActivateTab(2), Some("Terminal")),
            KeyBinding::new("cmd-3", ActivateTab(3), Some("Terminal")),
            KeyBinding::new("cmd-4", ActivateTab(4), Some("Terminal")),
            KeyBinding::new("cmd-5", ActivateTab(5), Some("Terminal")),
            KeyBinding::new("cmd-6", ActivateTab(6), Some("Terminal")),
            KeyBinding::new("cmd-7", ActivateTab(7), Some("Terminal")),
            KeyBinding::new("cmd-8", ActivateTab(8), Some("Terminal")),
            KeyBinding::new("cmd-9", ActivateTab(9), Some("Terminal")),
            // Settings / theme
            KeyBinding::new("cmd-shift-r", ReloadSettings, Some("AppShell")),
            KeyBinding::new("cmd-shift-r", ReloadSettings, Some("Terminal")),
            // Settings panel (theme picker); macOS convention is ⌘,.
            KeyBinding::new("cmd-,", OpenSettings, Some("AppShell")),
            KeyBinding::new("cmd-,", OpenSettings, Some("Terminal")),
            // Quick theme cycle (also persists). ⌘⇧T reserved for reopen-closed-tab.
            KeyBinding::new("cmd-shift-p", CycleTheme, Some("AppShell")),
            KeyBinding::new("cmd-shift-p", CycleTheme, Some("Terminal")),
            // Splits (iTerm2 keymap): ⌘D splits right, ⌘⇧D splits down.
            KeyBinding::new("cmd-d", SplitRight, Some("AppShell")),
            KeyBinding::new("cmd-shift-d", SplitDown, Some("AppShell")),
            KeyBinding::new("cmd-d", SplitRight, Some("Terminal")),
            KeyBinding::new("cmd-shift-d", SplitDown, Some("Terminal")),
            // Pane focus navigation on ⌘⌥+arrows (⌘+arrows are scroll/line binds).
            KeyBinding::new("cmd-alt-left", FocusPaneLeft, Some("AppShell")),
            KeyBinding::new("cmd-alt-right", FocusPaneRight, Some("AppShell")),
            KeyBinding::new("cmd-alt-up", FocusPaneUp, Some("AppShell")),
            KeyBinding::new("cmd-alt-down", FocusPaneDown, Some("AppShell")),
            KeyBinding::new("cmd-alt-left", FocusPaneLeft, Some("Terminal")),
            KeyBinding::new("cmd-alt-right", FocusPaneRight, Some("Terminal")),
            KeyBinding::new("cmd-alt-up", FocusPaneUp, Some("Terminal")),
            KeyBinding::new("cmd-alt-down", FocusPaneDown, Some("Terminal")),
            // Check for updates (menu + shortcut).
            KeyBinding::new("cmd-shift-u", CheckForUpdates, Some("AppShell")),
            KeyBinding::new("cmd-shift-u", CheckForUpdates, Some("Terminal")),
            // Command palette (M9). ⌘⇧P is theme cycle; palette uses ⌘⇧K.
            KeyBinding::new("cmd-shift-k", ToggleCommandPalette, Some("AppShell")),
            KeyBinding::new("cmd-shift-k", ToggleCommandPalette, Some("Terminal")),
            // Find in scrollback (M10).
            KeyBinding::new("cmd-f", Find, Some("AppShell")),
            KeyBinding::new("cmd-f", Find, Some("Terminal")),
            KeyBinding::new("cmd-g", FindNext, Some("AppShell")),
            KeyBinding::new("cmd-g", FindNext, Some("Terminal")),
            KeyBinding::new("cmd-shift-g", FindPrev, Some("AppShell")),
            KeyBinding::new("cmd-shift-g", FindPrev, Some("Terminal")),
        ]);

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
            _ => continue,
        };
        out.push(kb);
    }
    out
}
