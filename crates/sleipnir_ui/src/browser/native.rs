//! OS WebViews hosted as children of the existing GPUI window.
//! No IPC bridge, local-file protocol, injected scripts or shell access is exposed.
use super::address::allowed_url;
use gpui::{Bounds, Pixels, Window};
use std::{cell::Cell, rc::Rc};
use wry::dpi::{LogicalPosition, LogicalSize};

#[derive(Debug)]
pub(super) enum NativeEvent {
    Loading(bool),
    Title(String),
    Open(String),
    Blocked(String),
}

pub(super) struct NativeHost {
    pub webview: wry::WebView,
    bounds: Cell<Option<(i32, i32, i32, i32)>>,
    visible: Cell<bool>,
    #[cfg(target_os = "windows")]
    parent: windows::Win32::Foundation::HWND,
    // Keep context alive until after the webview drops.
    _context: wry::WebContext,
}

impl NativeHost {
    pub fn new(
        window: &Window,
        tx: async_channel::Sender<NativeEvent>,
    ) -> Result<Rc<Self>, String> {
        #[cfg(target_os = "windows")]
        let parent = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match HasWindowHandle::window_handle(window)
                .map_err(|e| e.to_string())?
                .as_raw()
            {
                RawWindowHandle::Win32(handle) => {
                    windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _)
                }
                _ => return Err("No Win32 window handle".into()),
            }
        };
        let profile = dirs::data_local_dir()
            .ok_or("Cannot locate browser data directory")?
            .join("sleipnir")
            .join("browser");
        std::fs::create_dir_all(&profile).map_err(|e| format!("Browser profile: {e}"))?;
        let mut context = wry::WebContext::new(Some(profile));
        let load_tx = tx.clone();
        let title_tx = tx.clone();
        let open_tx = tx.clone();
        let navigation_tx = tx;
        let built = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_bounds(wry::Rect {
                position: LogicalPosition::new(0., 0.).into(),
                size: LogicalSize::new(1., 1.).into(),
            })
            .with_visible(false)
            .with_focused(false)
            .with_accept_first_mouse(true)
            .with_url("about:blank")
            .with_navigation_handler(move |url| {
                let allowed = allowed_url(&url);
                if !allowed {
                    let _ = navigation_tx.try_send(NativeEvent::Blocked(
                        "Only HTTP/HTTPS navigation is supported".into(),
                    ));
                }
                allowed
            })
            .with_on_page_load_handler(move |event, _url| {
                let _ = load_tx.try_send(NativeEvent::Loading(matches!(
                    event,
                    wry::PageLoadEvent::Started
                )));
            })
            .with_document_title_changed_handler(move |title| {
                let _ = title_tx.try_send(NativeEvent::Title(title));
            })
            .with_new_window_req_handler(move |url, _| {
                if allowed_url(&url) {
                    let _ = open_tx.try_send(NativeEvent::Open(url));
                }
                wry::NewWindowResponse::Deny
            })
            // Downloads are deliberately not implemented in this initial panel.
            .with_download_started_handler(|_, _| false)
            .build_as_child(window)
            .map_err(|e| format!("Cannot create system WebView: {e}"))?;
        Ok(Rc::new(Self {
            webview: built,
            bounds: Cell::new(None),
            visible: Cell::new(false),
            #[cfg(target_os = "windows")]
            parent,
            _context: context,
        }))
    }

    pub fn sync_bounds(&self, bounds: Bounds<Pixels>) -> Result<(), String> {
        let rect = (
            f32::from(bounds.left()).round() as i32,
            f32::from(bounds.top()).round() as i32,
            f32::from(bounds.right()).round() as i32,
            f32::from(bounds.bottom()).round() as i32,
        );
        if self.bounds.get() == Some(rect) {
            return Ok(());
        }
        self.webview
            .set_bounds(wry::Rect {
                position: LogicalPosition::new(f64::from(rect.0), f64::from(rect.1)).into(),
                size: LogicalSize::new(
                    f64::from((rect.2 - rect.0).max(0)),
                    f64::from((rect.3 - rect.1).max(0)),
                )
                .into(),
            })
            .map_err(|e| e.to_string())?;
        self.bounds.set(Some(rect));
        Ok(())
    }
    pub fn set_visible(&self, visible: bool) -> Result<(), String> {
        if self.visible.get() == visible {
            return Ok(());
        }
        if !visible && self.focused() {
            self.reclaim_focus();
        }
        self.webview
            .set_visible(visible)
            .map_err(|e| e.to_string())?;
        self.visible.set(visible);
        Ok(())
    }
    pub fn reclaim_focus(&self) {
        if let Err(error) = self.webview.focus_parent() {
            log::warn!("browser focus parent: {error}");
        }
    }

    #[cfg(target_os = "macos")]
    pub fn focused(&self) -> bool {
        use objc2::{msg_send, runtime::AnyObject};
        use wry::WebViewExtMacOS;
        let wk = self.webview.webview();
        unsafe {
            let window: *mut AnyObject = msg_send![&*wk, window];
            if window.is_null() {
                return false;
            }
            let responder: *mut AnyObject = msg_send![window, firstResponder];
            if responder.is_null() {
                return false;
            }
            let responds: bool =
                msg_send![responder, respondsToSelector: objc2::sel!(isDescendantOf:)];
            responds && msg_send![responder, isDescendantOf: &*wk]
        }
    }
    #[cfg(target_os = "windows")]
    pub fn focused(&self) -> bool {
        use windows::Win32::UI::{Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::IsChild};
        let focused = unsafe { GetFocus() };
        focused != self.parent && unsafe { IsChild(self.parent, focused).as_bool() }
    }
    #[cfg(target_os = "macos")]
    pub fn history(&self) -> (bool, bool) {
        use objc2::msg_send;
        use wry::WebViewExtMacOS;
        let wk = self.webview.webview();
        unsafe { (msg_send![&*wk, canGoBack], msg_send![&*wk, canGoForward]) }
    }
    #[cfg(target_os = "windows")]
    pub fn history(&self) -> (bool, bool) {
        use wry::WebViewExtWindows;
        let mut back = windows::core::BOOL(0);
        let mut forward = windows::core::BOOL(0);
        let webview = self.webview.webview();
        unsafe {
            let _ = webview.CanGoBack(&mut back);
            let _ = webview.CanGoForward(&mut forward);
        }
        (back.as_bool(), forward.as_bool())
    }
    #[cfg(target_os = "macos")]
    pub fn go(&self, back: bool) -> Result<(), String> {
        use objc2::{msg_send, runtime::AnyObject};
        use wry::WebViewExtMacOS;
        let wk = self.webview.webview();
        unsafe {
            let _: *mut AnyObject = if back {
                msg_send![&*wk, goBack]
            } else {
                msg_send![&*wk, goForward]
            };
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    pub fn go(&self, back: bool) -> Result<(), String> {
        use wry::WebViewExtWindows;
        let webview = self.webview.webview();
        unsafe {
            if back {
                webview.GoBack()
            } else {
                webview.GoForward()
            }
        }
        .map_err(|e| e.to_string())
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        let _ = self.set_visible(false);
    }
}
