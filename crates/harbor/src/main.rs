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
use terminal::{Copy, Paste};

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
