//! Non-macOS caption buttons (min / max / close) for the custom titlebar.
//!
//! GPUI hides the OS title bar when `appears_transparent` is set. Native
//! caption buttons are not drawn; hit-testing via [`WindowControlArea`] is
//! what actually minimizes, maximizes, and closes.

use gpui::{Context, IntoElement, Window, div, px};
#[cfg(any(windows, target_os = "linux"))]
use gpui::{
    InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    WindowControlArea, prelude::FluentBuilder as _, rgb,
};

use crate::app_shell::AppShell;
use crate::chrome::ChromeTokens;

/// Width of one desktop caption button. Three of them are 138px, matching
/// the trailing pad reserved for custom window controls.
pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;

#[cfg_attr(not(test), allow(dead_code))]
pub fn caption_bar_width() -> gpui::Pixels {
    px(CAPTION_BUTTON_WIDTH * 3.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptionButtonStrategy {
    PlatformHitTest,
    ExplicitClick,
}

fn caption_button_strategy_for(linux: bool) -> CaptionButtonStrategy {
    if linux {
        CaptionButtonStrategy::ExplicitClick
    } else {
        CaptionButtonStrategy::PlatformHitTest
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[derive(Clone, Copy)]
enum CaptionButtonAction {
    Minimize,
    MaximizeRestore,
    Close,
}

impl AppShell {
    /// Settings affordance + desktop caption buttons. Empty on macOS.
    pub(crate) fn render_desktop_titlebar_end(
        &self,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        {
            let _ = (tokens, window, cx);
            div()
        }
        #[cfg(any(windows, target_os = "linux"))]
        {
            div()
                .id("desktop-titlebar-end")
                .flex()
                .flex_row()
                .flex_shrink_0()
                .h_full()
                .child(self.render_titlebar_settings_button(tokens, cx))
                .child(self.render_desktop_caption_buttons(tokens, window))
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn render_titlebar_settings_button(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.settings_open;
        div()
            .id("titlebar-settings")
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .h_full()
            .w(px(36.0))
            .flex_shrink_0()
            .occlude()
            .cursor_pointer()
            .text_size(px(13.0))
            .when(open, |el| {
                el.bg(tokens.accent.opacity(0.2)).text_color(tokens.fg)
            })
            .when(!open, |el| {
                el.text_color(tokens.fg_muted)
                    .hover(|style| style.bg(tokens.hover).text_color(tokens.fg))
            })
            .child("⚙")
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_settings(window, cx);
            }))
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn render_desktop_caption_buttons(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
    ) -> impl IntoElement {
        let maximized = window.is_maximized();
        div()
            .id("desktop-window-controls")
            .flex()
            .flex_row()
            .flex_shrink_0()
            .h_full()
            .child(platform_caption_button(
                "caption-min",
                "\u{2013}",
                CaptionButtonAction::Minimize,
                tokens,
                false,
            ))
            .child(platform_caption_button(
                "caption-max",
                if maximized { "\u{2750}" } else { "\u{25A1}" },
                CaptionButtonAction::MaximizeRestore,
                tokens,
                false,
            ))
            .child(platform_caption_button(
                "caption-close",
                "\u{2715}",
                CaptionButtonAction::Close,
                tokens,
                true,
            ))
    }
}

#[cfg(windows)]
fn platform_caption_button(
    id: &'static str,
    glyph: &'static str,
    action: CaptionButtonAction,
    tokens: &ChromeTokens,
    is_close: bool,
) -> impl IntoElement {
    windows_caption_button(id, glyph, action, tokens, is_close)
}

#[cfg(target_os = "linux")]
fn platform_caption_button(
    id: &'static str,
    glyph: &'static str,
    action: CaptionButtonAction,
    tokens: &ChromeTokens,
    is_close: bool,
) -> impl IntoElement {
    linux_caption_button(id, glyph, action, tokens, is_close)
}

#[cfg(windows)]
fn windows_caption_button(
    id: &'static str,
    glyph: &'static str,
    action: CaptionButtonAction,
    tokens: &ChromeTokens,
    is_close: bool,
) -> impl IntoElement {
    caption_button_base(id, glyph, tokens, is_close)
        .window_control_area(action.window_control_area())
}

#[cfg(target_os = "linux")]
fn linux_caption_button(
    id: &'static str,
    glyph: &'static str,
    action: CaptionButtonAction,
    tokens: &ChromeTokens,
    is_close: bool,
) -> impl IntoElement {
    caption_button_base(id, glyph, tokens, is_close).on_click(move |_, window, cx| {
        cx.stop_propagation();
        match action {
            CaptionButtonAction::Minimize => window.minimize_window(),
            CaptionButtonAction::MaximizeRestore => window.zoom_window(),
            CaptionButtonAction::Close => window.remove_window(),
        }
    })
}

#[cfg(windows)]
impl CaptionButtonAction {
    fn window_control_area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::MaximizeRestore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn caption_button_base(
    id: &'static str,
    glyph: &'static str,
    tokens: &ChromeTokens,
    is_close: bool,
) -> gpui::Stateful<gpui::Div> {
    let hover_bg = if is_close {
        rgb(0xe81123).into()
    } else {
        tokens.hover
    };
    let hover_fg = if is_close { gpui::white() } else { tokens.fg };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .h_full()
        .w(px(CAPTION_BUTTON_WIDTH))
        .flex_shrink_0()
        .occlude()
        .text_size(px(11.0))
        .text_color(tokens.fg_muted)
        .hover(|style| style.bg(hover_bg).text_color(hover_fg))
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::geometry::trailing_pad_for;

    #[test]
    fn caption_button_strategy_is_platform_specific() {
        assert_eq!(
            caption_button_strategy_for(false),
            CaptionButtonStrategy::PlatformHitTest
        );
        assert_eq!(
            caption_button_strategy_for(true),
            CaptionButtonStrategy::ExplicitClick
        );
    }

    #[test]
    fn linux_buttons_use_explicit_clicks_and_windows_keep_hit_tests() {
        let src = include_str!("desktop_window_controls.rs");
        let linux = src
            .split("fn linux_caption_button")
            .nth(1)
            .expect("Linux caption-button implementation");
        for method in ["minimize_window()", "zoom_window()", "remove_window()"] {
            assert!(linux.contains(method), "Linux click path missing {method}");
        }
        assert!(linux.contains(".on_click("));

        let windows = src
            .split("fn windows_caption_button")
            .nth(1)
            .expect("Windows caption-button implementation");
        assert!(windows.contains(".window_control_area(action.window_control_area())"));
    }

    #[test]
    fn caption_bar_matches_desktop_trailing_pad() {
        assert_eq!(caption_bar_width(), trailing_pad_for(true));
    }
}
