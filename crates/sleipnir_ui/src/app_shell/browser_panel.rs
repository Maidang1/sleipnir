//! Core browser integration. Hidden behind modal UI; never an input mode or PTY.
use super::{AppShell, ToggleBrowser};
use crate::{chrome::ChromeTokens, ui_mode::InputMode};
use gpui::{prelude::*, *};

impl AppShell {
    pub(super) fn on_toggle_browser(
        &mut self,
        _: &ToggleBrowser,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_browser(window, cx);
    }
    pub(super) fn toggle_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            use crate::browser::{BrowserEvent, BrowserView};
            self.browser_open = !self.browser_open;
            if self.browser_open {
                if self.browser.is_none() {
                    let browser = cx.new(|cx| BrowserView::new(window, cx));
                    cx.subscribe_in(&browser, window, |this, _, event, window, cx| {
                        match event {
                            BrowserEvent::Close => {
                                this.browser_open = false;
                                this.sync_browser_presentation(cx);
                            }
                            BrowserEvent::ReturnToTerminal => this.reclaim_browser_focus(cx),
                        }
                        this.focus_active(window, cx);
                        cx.notify();
                    })
                    .detach();
                    self.browser = Some(browser);
                }
                self.set_input(InputMode::Terminal, cx);
                if let Some(browser) = &self.browser {
                    browser.update(cx, |browser, cx| browser.focus_address(window, cx));
                }
            } else {
                self.sync_browser_presentation(cx);
                self.focus_active(window, cx);
            }
            cx.notify();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (window, cx);
            log::info!("Browser panel is available on macOS and Windows only");
        }
    }

    pub(super) fn sync_browser_presentation(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(browser) = &self.browser {
            let blocked =
                browser_is_blocked(&self.input, self.mode.quick_select_open, self.broadcast);
            browser.update(cx, |browser, cx| {
                browser.set_presentation(self.browser_open, blocked, cx)
            });
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = cx;
    }
    pub(super) fn reclaim_browser_focus(&self, cx: &App) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(browser) = &self.browser {
            browser.read(cx).reclaim_focus();
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = cx;
    }
    pub(super) fn poll_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(browser) = &self.browser {
            browser.update(cx, |browser, cx| browser.poll(window, cx));
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (window, cx);
    }
    pub(super) fn render_browser_toggle(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("toggle-browser")
            .px_2()
            .h(px(26.))
            .flex()
            .items_center()
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(if self.browser_open {
                tokens.accent
            } else {
                tokens.fg_muted
            })
            .hover(|el| el.bg(tokens.hover))
            .child("Browser")
            .on_click(cx.listener(|this, _, window, cx| this.toggle_browser(window, cx)))
    }
    pub(super) fn render_browser_panel(
        &self,
        tokens: &ChromeTokens,
        window: &Window,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = (f32::from(window.viewport_size().width) * 0.45).clamp(180., 720.);
        let panel = div()
            .id("browser-sidebar")
            .w(px(width))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .border_l_1()
            .border_color(tokens.border);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let panel = panel.children(self.browser.clone());
        panel.into_any_element()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn browser_is_blocked(input: &InputMode, quick_select: bool, broadcast: bool) -> bool {
    !matches!(input, InputMode::Terminal | InputMode::Find) || quick_select || broadcast
}

#[cfg(test)]
mod tests {
    use super::browser_is_blocked;
    use crate::ui_mode::{InputMode, OverlayKind};
    #[test]
    fn browser_is_hidden_for_every_application_overlay() {
        for overlay in [
            OverlayKind::Settings,
            OverlayKind::Update,
            OverlayKind::Palette,
            OverlayKind::PaneFacts,
            OverlayKind::History,
            OverlayKind::Diff,
            OverlayKind::PluginMonitor,
        ] {
            assert!(browser_is_blocked(
                &InputMode::Overlay(overlay),
                false,
                false
            ));
        }
        assert!(!browser_is_blocked(&InputMode::Terminal, false, false));
        assert!(!browser_is_blocked(&InputMode::Find, false, false));
        assert!(browser_is_blocked(&InputMode::Terminal, true, false));
        assert!(browser_is_blocked(&InputMode::Terminal, false, true));
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl AppShell {
    pub(crate) fn browser_control_request(
        &mut self,
        request: sleipnir_browser_control::Request,
        reply: std::sync::mpsc::Sender<sleipnir_browser_control::Response>,
        deadline: std::time::Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use sleipnir_browser_control::{BrowserStatus, Request, Response};
        self.sync_browser_presentation(cx);
        if let Some(browser) = &self.browser {
            browser.update(cx, |browser, cx| {
                browser.control(request, reply, deadline, window, cx)
            });
        } else {
            let response = if matches!(request, Request::List) {
                Response::Windows {
                    windows: vec![BrowserStatus {
                        window: window.window_handle().window_id().as_u64(),
                        open: false,
                        authorized: false,
                        blocked: false,
                        loading: false,
                        url: None,
                        title: None,
                    }],
                }
            } else {
                Response::error(
                    "permission_denied",
                    "Open Browser in this window and enable Agent access first",
                )
            };
            let _ = reply.send(response);
        }
    }
}
