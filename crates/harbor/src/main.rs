//! Harbor — standalone macOS terminal (HIG-aligned window chrome).

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, px, size,
};
use gpui_platform::application;
use harbor_settings;
use harbor_ui::{
    AppShell, ChromeGeometry, CloseTab, CycleTheme, NewTab, NextTab, PrevTab, ReloadSettings,
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
            // Settings / theme
            KeyBinding::new("cmd-shift-r", ReloadSettings, Some("AppShell")),
            KeyBinding::new("cmd-shift-r", ReloadSettings, Some("Terminal")),
            KeyBinding::new("cmd-shift-t", CycleTheme, Some("AppShell")),
            KeyBinding::new("cmd-shift-t", CycleTheme, Some("Terminal")),
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
