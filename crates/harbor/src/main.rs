//! Harbor — standalone macOS terminal (HIG-aligned window chrome).

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, px, size,
};
use gpui_platform::application;
use harbor_settings;
use harbor_ui::{
    ActivateTab, AppShell, ChromeGeometry, CloseTab, CycleTheme, FocusPaneDown, FocusPaneLeft,
    FocusPaneRight, FocusPaneUp, NewTab, NextTab, PrevTab, ReloadSettings, SplitDown, SplitRight,
};
use release_channel::AppVersion;
use terminal::{
    Clear, Copy, Paste, PasteText, ScrollLineDown, ScrollLineUp, ScrollPageDown, ScrollPageUp,
    ScrollToBottom, ScrollToTop, SelectAll, SendKeystroke, SendText, ShowCharacterPalette,
    ToggleViMode,
};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    application().run(|cx: &mut App| {
        AppVersion::init(cx);
        harbor_settings::init(cx);

        cx.bind_keys([
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
            // Theme cycling lives on ⌘⇧P for now; ⌘⇧T is reserved for the
            // conventional "reopen closed tab" (Phase 3 replaces this with a picker).
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
        ]);

        let geo = ChromeGeometry::standard();
        let bounds = Bounds::centered(None, size(px(1024.0), px(680.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Harbor".into()),
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
