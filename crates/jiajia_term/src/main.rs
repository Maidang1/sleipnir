//! jiajia-term — standalone terminal (M1: display-only ANSI grid).

use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_platform::application;
use jiajia_settings;
use jiajia_term_ui::TermView;
use release_channel::AppVersion;

fn main() {
    application().run(|cx: &mut App| {
        AppVersion::init(cx);
        jiajia_settings::init(cx);

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
            |window, cx| {
                cx.new(|cx| {
                    let view = TermView::new_display_only(window, cx);
                    view.write_demo_ansi(cx);
                    view
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
