//! jiajia-term — standalone terminal (M3: multi-tab + http(s) open).

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
};
use gpui_platform::application;
use jiajia_settings;
use jiajia_term_ui::{AppShell, CloseTab, NewTab, NextTab, PrevTab};
use release_channel::AppVersion;
use terminal::{Copy, Paste};

fn main() {
    application().run(|cx: &mut App| {
        AppVersion::init(cx);
        jiajia_settings::init(cx);

        cx.bind_keys([
            // Clipboard (Terminal context)
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("ctrl-shift-c", Copy, Some("Terminal")),
            KeyBinding::new("ctrl-shift-v", Paste, Some("Terminal")),
            // Tabs (AppShell context; also work while focused in Terminal if bubble — bind both)
            KeyBinding::new("cmd-t", NewTab, Some("AppShell")),
            KeyBinding::new("cmd-w", CloseTab, Some("AppShell")),
            KeyBinding::new("cmd-shift-]", NextTab, Some("AppShell")),
            KeyBinding::new("cmd-shift-[", PrevTab, Some("AppShell")),
            KeyBinding::new("ctrl-tab", NextTab, Some("AppShell")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("AppShell")),
            // Also bind on Terminal so shortcuts work while typing in the grid.
            KeyBinding::new("cmd-t", NewTab, Some("Terminal")),
            KeyBinding::new("cmd-w", CloseTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-]", NextTab, Some("Terminal")),
            KeyBinding::new("cmd-shift-[", PrevTab, Some("Terminal")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Terminal")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Terminal")),
        ]);

        let bounds = Bounds::centered(None, size(px(1024.0), px(680.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("jiajia-term".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| AppShell::new(window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
