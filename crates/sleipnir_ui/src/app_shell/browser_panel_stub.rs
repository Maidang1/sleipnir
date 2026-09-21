//! No-op browser panel for platforms without a hosted WebView (Linux).
//!
//! `browser_panel` is the macOS/Windows implementation; this module keeps the
//! same surface so call sites never branch on the target OS.

use super::{AppShell, ToggleBrowser};
use crate::chrome::ChromeTokens;
use gpui::{App, Context, IntoElement, Window, div};

impl AppShell {
    pub(super) fn on_toggle_browser(
        &mut self,
        _: &ToggleBrowser,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        log::info!("Browser panel is available on macOS and Windows only");
    }
    pub(super) fn toggle_browser(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        log::info!("Browser panel is available on macOS and Windows only");
    }
    pub(super) fn sync_browser_presentation(&mut self, _cx: &mut Context<Self>) {}
    pub(super) fn reclaim_browser_focus(&self, _cx: &App) {}
    pub(super) fn poll_browser(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
    pub(super) fn browser_panel_is_open(&self) -> bool {
        false
    }
    pub(super) fn render_browser_toggle(
        &self,
        _tokens: &ChromeTokens,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
    }
    pub(super) fn render_browser_panel(
        &self,
        _tokens: &ChromeTokens,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div().into_any_element()
    }
    pub(crate) fn browser_control_request(
        &mut self,
        _request: sleipnir_browser_control::Request,
        reply: std::sync::mpsc::Sender<sleipnir_browser_control::Response>,
        _deadline: std::time::Instant,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        use sleipnir_browser_control::{BrowserStatus, Request, Response};
        let response = if matches!(_request, Request::List) {
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
