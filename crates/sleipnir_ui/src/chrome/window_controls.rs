//! Windows caption buttons (min / max / close) for the custom titlebar.
//!
//! GPUI hides the OS title bar when `appears_transparent` is set. Native
//! caption buttons are not drawn; hit-testing via [`WindowControlArea`] is
//! what actually minimizes, maximizes, and closes.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, WindowControlArea, div,
    prelude::FluentBuilder as _, px, rgb,
};

use crate::app_shell::AppShell;
use crate::chrome::ChromeTokens;

/// Width of one Windows caption button. Three of them are 138px, matching
/// the original trailing pad reserved for OS chrome.
pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;

#[cfg_attr(not(test), allow(dead_code))]
pub fn caption_bar_width() -> gpui::Pixels {
    px(CAPTION_BUTTON_WIDTH * 3.0)
}

impl AppShell {
    /// Settings affordance + Windows caption buttons. Empty on macOS.
    pub(crate) fn render_windows_titlebar_end(
        &self,
        tokens: &ChromeTokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        #[cfg(not(windows))]
        {
            let _ = (tokens, window, cx);
            div()
        }
        #[cfg(windows)]
        {
            div()
                .id("windows-titlebar-end")
                .flex()
                .flex_row()
                .flex_shrink_0()
                .h_full()
                .child(self.render_titlebar_settings_button(tokens, cx))
                .child(self.render_windows_caption_buttons(tokens, window))
        }
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
    fn render_windows_caption_buttons(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
    ) -> impl IntoElement {
        let maximized = window.is_maximized();
        div()
            .id("windows-window-controls")
            .flex()
            .flex_row()
            .flex_shrink_0()
            .h_full()
            .child(caption_button(
                "caption-min",
                "\u{2013}",
                WindowControlArea::Min,
                tokens,
                false,
            ))
            .child(caption_button(
                "caption-max",
                if maximized { "\u{2750}" } else { "\u{25A1}" },
                WindowControlArea::Max,
                tokens,
                false,
            ))
            .child(caption_button(
                "caption-close",
                "\u{2715}",
                WindowControlArea::Close,
                tokens,
                true,
            ))
    }
}

#[cfg(windows)]
fn caption_button(
    id: &'static str,
    glyph: &'static str,
    area: WindowControlArea,
    tokens: &ChromeTokens,
    is_close: bool,
) -> impl IntoElement {
    let hover_bg = if is_close {
        rgb(0xe81123).into()
    } else {
        tokens.hover
    };
    let hover_fg = if is_close {
        gpui::white()
    } else {
        tokens.fg
    };
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
        .window_control_area(area)
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::geometry::trailing_pad_for;

    #[test]
    fn caption_bar_matches_windows_trailing_pad() {
        assert_eq!(caption_bar_width(), trailing_pad_for(true));
    }
}
