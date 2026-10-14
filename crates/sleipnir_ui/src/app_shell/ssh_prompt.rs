//! SSH askpass modal.
//!
//! Shown when `ssh` in one of this window's panes wants a password or a key
//! passphrase. The prompt text comes verbatim from `ssh`, the typed secret is
//! masked, and Enter hands it back over the askpass socket so `ssh` never has
//! to read from the raw terminal.

use gpui::{
    Context, FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    Styled as _, deferred, div, px,
};

use super::{AppShell, ChromeTokens};
use crate::app_shell::query::QuerySurface;

impl AppShell {
    /// Render the askpass modal for the pending request, if any.
    pub(super) fn render_ssh_prompt(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let border_w = crate::chrome::pixel::PIXEL_BORDER;
        let Some(state) = self.input.ssh_prompt() else {
            return deferred(div());
        };

        let masked = "•".repeat(state.secret.chars().count());
        let prompt = state.prompt.clone();

        deferred(
            div()
                .id("ssh-prompt-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .justify_center()
                .pt(px(140.0))
                .child(
                    div()
                        .id("ssh-prompt-panel")
                        .w(px(400.0))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .shadow(crate::chrome::pixel::hard_shadow())
                        .child(crate::chrome::pixel::pixel_panel_bg(
                            tokens.content_bg,
                            tokens.border,
                        ))
                        // Keep clicks inside the panel from reaching the backdrop.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .px_4()
                                .pt_4()
                                .pb_2()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tokens.fg)
                                .child("SSH Authentication"),
                        )
                        .child(
                            div()
                                .px_4()
                                .pb_3()
                                .text_size(px(12.0))
                                .text_color(tokens.fg_muted)
                                .child(prompt),
                        )
                        .child(
                            div()
                                .relative()
                                .mx_4()
                                .mb_2()
                                .px_3()
                                .py_2()
                                .min_h(px(30.0))
                                .border(border_w)
                                .border_color(tokens.accent)
                                .text_size(px(13.0))
                                .text_color(tokens.fg)
                                .child(if masked.is_empty() {
                                    String::new()
                                } else {
                                    masked
                                })
                                .child(self.query_input_canvas(QuerySurface::SshPrompt, cx)),
                        )
                        .child(
                            div()
                                .px_4()
                                .pb_4()
                                .flex()
                                .gap(px(12.0))
                                .text_size(px(11.0))
                                .text_color(tokens.fg_muted)
                                .child("Enter to send")
                                .child("Esc to cancel"),
                        ),
                ),
        )
    }
}
