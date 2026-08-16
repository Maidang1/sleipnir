//! macOS menu-bar Attention item and Dock badge (failed Attention count).

use crate::run_ledger_global::RunLedgerGlobal;
use gpui::App;
use sleipnir_settings::TerminalSettings;

/// Dock tile label: failed Attention count, or none when the count is zero.
pub fn dock_badge_label(failed: usize) -> Option<String> {
    if failed == 0 {
        None
    } else {
        Some(failed.to_string())
    }
}

/// Menu-bar title: failed first, then any Attention, else a quiet mark.
pub fn tray_title(failed: usize, attention: usize) -> String {
    if failed > 0 {
        format!("✗{failed}")
    } else if attention > 0 {
        format!("●{attention}")
    } else {
        "S".into()
    }
}

pub fn refresh(cx: &mut App) {
    let show_tray = TerminalSettings::get_global(cx).show_tray_icon;
    let (failed, attention) = if cx.has_global::<RunLedgerGlobal>() {
        let g = cx.global::<RunLedgerGlobal>();
        (g.failed_attention_count(), g.attention_count())
    } else {
        (0, 0)
    };
    set_dock_badge(dock_badge_label(failed).as_deref());
    set_tray(show_tray, &tray_title(failed, attention));
}

#[cfg(target_os = "macos")]
fn set_dock_badge(label: Option<&str>) {
    use objc::{class, msg_send, sel, sel_impl};
    use objc::runtime::Object;
    use std::ffi::CString;

    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let tile: *mut Object = msg_send![app, dockTile];
        match label {
            Some(text) => {
                let c = CString::new(text).unwrap_or_default();
                let ns: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()];
                let _: () = msg_send![tile, setBadgeLabel: ns];
            }
            None => {
                let nil: *mut Object = std::ptr::null_mut();
                let _: () = msg_send![tile, setBadgeLabel: nil];
            }
        }
        let _: () = msg_send![tile, display];
    }
}

#[cfg(not(target_os = "macos"))]
fn set_dock_badge(_label: Option<&str>) {}

#[cfg(target_os = "macos")]
fn set_tray(show: bool, title: &str) {
    use objc::{class, msg_send, sel, sel_impl};
    use objc::runtime::Object;
    use std::ffi::CString;
    use std::sync::Mutex;

    struct Tray(*mut Object);
    unsafe impl Send for Tray {}

    static TRAY: Mutex<Option<Tray>> = Mutex::new(None);

    let mut slot = match TRAY.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    if !show {
        if let Some(Tray(item)) = slot.take() {
            unsafe {
                let bar: *mut Object = msg_send![class!(NSStatusBar), systemStatusBar];
                let _: () = msg_send![bar, removeStatusItem: item];
                let _: () = msg_send![item, release];
            }
        }
        return;
    }

    unsafe {
        if slot.is_none() {
            let bar: *mut Object = msg_send![class!(NSStatusBar), systemStatusBar];
            // NSVariableStatusItemLength == -1.0
            let item: *mut Object = msg_send![bar, statusItemWithLength: -1.0f64];
            let item: *mut Object = msg_send![item, retain];

            let menu: *mut Object = msg_send![class!(NSMenu), new];
            let title_s = CString::new("Show Sleipnir").unwrap_or_default();
            let empty = CString::new("").unwrap_or_default();
            let ns_title: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: title_s.as_ptr()];
            let ns_empty: *mut Object =
                msg_send![class!(NSString), stringWithUTF8String: empty.as_ptr()];
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let mi: *mut Object = msg_send![class!(NSMenuItem), alloc];
            let mi: *mut Object = msg_send![
                mi,
                initWithTitle: ns_title
                action: sel!(activateIgnoringOtherApps:)
                keyEquivalent: ns_empty
            ];
            let _: () = msg_send![mi, setTarget: app];
            let _: () = msg_send![menu, addItem: mi];
            let _: () = msg_send![item, setMenu: menu];
            *slot = Some(Tray(item));
        }

        if let Some(Tray(item)) = slot.as_ref() {
            let button: *mut Object = msg_send![*item, button];
            if !button.is_null() {
                let c = CString::new(title).unwrap_or_default();
                let ns: *mut Object =
                    msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()];
                let _: () = msg_send![button, setTitle: ns];
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_tray(_show: bool, _title: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_badge_is_the_failed_count() {
        assert_eq!(dock_badge_label(0), None);
        assert_eq!(dock_badge_label(3).as_deref(), Some("3"));
    }

    #[test]
    fn tray_prefers_failed_then_attention() {
        assert_eq!(tray_title(2, 5), "✗2");
        assert_eq!(tray_title(0, 4), "●4");
        assert_eq!(tray_title(0, 0), "S");
    }
}
