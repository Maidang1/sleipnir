//! jiajia-term — standalone terminal (M0: empty window).

use gpui::{
    App, Bounds, Context, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct RootView {
    title: SharedString,
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .size_full()
            .justify_center()
            .items_center()
            .text_color(rgb(0xcdd6f4))
            .text_xl()
            .child(self.title.clone())
            .child(
                div()
                    .mt_4()
                    .text_sm()
                    .text_color(rgb(0x6c7086))
                    .child("M0: GPUI window — terminal arrives in M1+"),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
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
            |_, cx| {
                cx.new(|_| RootView {
                    title: "jiajia-term".into(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
