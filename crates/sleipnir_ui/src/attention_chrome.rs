//! macOS Dock badge (failed Attention count).

use crate::run_ledger_global::RunLedgerGlobal;
use gpui::App;

/// Dock tile label: failed Attention count, or none when the count is zero.
pub fn dock_badge_label(failed: usize) -> Option<String> {
    if failed == 0 {
        None
    } else {
        Some(failed.to_string())
    }
}

pub fn refresh(cx: &mut App) {
    let failed = if cx.has_global::<RunLedgerGlobal>() {
        cx.global::<RunLedgerGlobal>().failed_attention_count()
    } else {
        0
    };
    set_dock_badge(dock_badge_label(failed).as_deref());
}

#[cfg(target_os = "macos")]
fn set_dock_badge(label: Option<&str>) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_badge_is_the_failed_count() {
        assert_eq!(dock_badge_label(0), None);
        assert_eq!(dock_badge_label(3).as_deref(), Some("3"));
    }
}
