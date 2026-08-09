//! jiajia-term — standalone terminal (M2: local PTY + input).

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
};
use gpui_platform::application;
use jiajia_settings;
use jiajia_term_ui::TermView;
use release_channel::AppVersion;
use terminal::{Copy, Paste};

fn main() {
    application().run(|cx: &mut App| {
        AppVersion::init(cx);
        jiajia_settings::init(cx);

        // Clipboard bindings in Terminal key context.
        cx.bind_keys([
            KeyBinding::new("cmd-c", Copy, Some("Terminal")),
            KeyBinding::new("cmd-v", Paste, Some("Terminal")),
            KeyBinding::new("ctrl-shift-c", Copy, Some("Terminal")),
            KeyBinding::new("ctrl-shift-v", Paste, Some("Terminal")),
        ]);

        let bounds = Bounds::centered(None, size(px(960.0), px(640.0)), cx);
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
            |window, cx| cx.new(|cx| TermView::new_local(window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
