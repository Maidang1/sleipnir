//! macOS Finder Services: "New Sleipnir Tab Here" / "New Sleipnir Window Here".

use gpui::App;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use crate::app_shell::{AppShell, open_sleipnir_window_at_cwd};
#[cfg(target_os = "macos")]
use gpui::WindowHandle;

/// Which Finder service was invoked.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinderOpenKind {
    Tab,
    Window,
}

/// Register the AppKit services provider. No-op off macOS.
pub fn install_finder_services(cx: &mut App) {
    #[cfg(target_os = "macos")]
    macos::install(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

/// Turn Finder/pasteboard input into a working directory.
///
/// File URLs are decoded; trailing slashes are stripped except for `/`.
/// Directories are used as-is; files use their parent; missing paths are ignored.
pub fn cwd_from_service_input(raw: &str) -> Option<PathBuf> {
    cwd_from_parsed_path(&parse_service_path(raw))
}

/// Parse a pasteboard string into a filesystem path. Does not consult the disk.
pub fn parse_service_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("file://") {
        let path = if rest.starts_with('/') {
            rest.to_string()
        } else if let Some((_, path)) = rest.split_once('/') {
            format!("/{path}")
        } else {
            rest.to_string()
        };
        PathBuf::from(percent_decode(&path))
    } else {
        PathBuf::from(trimmed)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2]))
        {
            out.push((high << 4) | low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn strip_trailing_slashes(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "/" {
        return PathBuf::from("/");
    }
    PathBuf::from(s.trim_end_matches('/'))
}

fn cwd_from_parsed_path(path: &Path) -> Option<PathBuf> {
    let path = strip_trailing_slashes(path);
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_dir() {
        Some(path)
    } else if path.is_file() {
        match path.parent() {
            Some(parent) if parent.as_os_str().is_empty() => Some(PathBuf::from("/")),
            Some(parent) => Some(parent.to_path_buf()),
            None => None,
        }
    } else {
        None
    }
}

fn unique_cwds(raws: &[String]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in raws {
        let Some(cwd) = cwd_from_service_input(raw) else {
            continue;
        };
        if seen.insert(cwd.clone()) {
            out.push(cwd);
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn frontmost_shell(cx: &App) -> Option<WindowHandle<AppShell>> {
    if let Some(stack) = cx.window_stack() {
        for handle in stack {
            if let Some(shell) = handle.downcast::<AppShell>() {
                return Some(shell);
            }
        }
    }
    cx.active_window()
        .and_then(|handle| handle.downcast::<AppShell>())
        .or_else(|| {
            cx.windows()
                .into_iter()
                .find_map(|handle| handle.downcast::<AppShell>())
        })
}

/// Open tabs or a window at the resolved Finder paths.
#[cfg(target_os = "macos")]
pub fn handle_finder_open(kind: FinderOpenKind, paths: &[String], cx: &mut App) {
    let cwds = unique_cwds(paths);
    if cwds.is_empty() {
        log::warn!("Finder service {kind:?} received no usable directory");
        return;
    }
    match kind {
        FinderOpenKind::Tab => open_tabs_at(cwds, cx),
        FinderOpenKind::Window => open_window_at(cwds, cx),
    }
    cx.activate(true);
}

#[cfg(target_os = "macos")]
fn open_tabs_at(cwds: Vec<PathBuf>, cx: &mut App) {
    if let Some(handle) = frontmost_shell(cx) {
        for cwd in cwds {
            if handle
                .update(cx, |shell, window, cx| {
                    shell.add_tab_at(Some(cwd), window, cx);
                    window.activate_window();
                })
                .is_err()
            {
                log::warn!("Finder New Tab Here: frontmost window closed during open");
                break;
            }
        }
        return;
    }
    open_window_at(cwds, cx);
}

#[cfg(target_os = "macos")]
fn open_window_at(cwds: Vec<PathBuf>, cx: &mut App) {
    let mut cwds = cwds.into_iter();
    let Some(first) = cwds.next() else {
        return;
    };
    let Some(handle) = open_sleipnir_window_at_cwd(first, cx) else {
        return;
    };
    for cwd in cwds {
        let _ = handle.update(cx, |shell, window, cx| {
            shell.add_tab_at(Some(cwd), window, cx);
        });
    }
    let _ = handle.update(cx, |_, window, _| window.activate_window());
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{FinderOpenKind, handle_finder_open};
    use gpui::App;
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::{CStr, CString};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};

    struct Request {
        kind: FinderOpenKind,
        paths: Vec<String>,
    }

    /// AppKit retains the services provider; we also keep a process-lifetime
    /// reference so the object cannot be collected if AppKit drops it.
    struct Provider(#[allow(dead_code)] *mut Object);
    unsafe impl Send for Provider {}
    unsafe impl Sync for Provider {}

    static TX: Mutex<Option<async_channel::Sender<Request>>> = Mutex::new(None);
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static PROVIDER: OnceLock<Provider> = OnceLock::new();

    unsafe extern "C" {
        fn NSUpdateDynamicServices();
    }

    pub fn install(cx: &mut App) {
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        let (tx, rx) = async_channel::unbounded();
        *TX.lock().unwrap_or_else(|err| err.into_inner()) = Some(tx);
        register_provider();
        cx.spawn(async move |cx| {
            while let Ok(req) = rx.recv().await {
                cx.update(|cx| handle_finder_open(req.kind, &req.paths, cx));
            }
        })
        .detach();
    }

    fn register_provider() {
        unsafe {
            let cls = service_class();
            let provider: *mut Object = msg_send![cls, new];
            let _: *mut Object = msg_send![provider, retain];
            let _ = PROVIDER.set(Provider(provider));
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setServicesProvider: provider];
            NSUpdateDynamicServices();
        }
    }

    fn service_class() -> &'static Class {
        static CLASS: OnceLock<&'static Class> = OnceLock::new();
        CLASS.get_or_init(|| {
            if let Some(existing) = Class::get("SleipnirFinderServices") {
                return existing;
            }
            let mut decl = ClassDecl::new("SleipnirFinderServices", class!(NSObject))
                .expect("SleipnirFinderServices class");
            unsafe {
                // AppKit's error argument is NSString**; objc 0.2 cannot Encode
                // `*mut *mut Object`, and the ABI of a single pointer is the same.
                decl.add_method(
                    sel!(openTab:userData:error:),
                    open_tab as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object),
                );
                decl.add_method(
                    sel!(openWindow:userData:error:),
                    open_window
                        as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object),
                );
            }
            decl.register()
        })
    }

    extern "C" fn open_tab(
        _this: &Object,
        _sel: Sel,
        pboard: *mut Object,
        _user_data: *mut Object,
        _error: *mut Object,
    ) {
        dispatch(FinderOpenKind::Tab, pboard);
    }

    extern "C" fn open_window(
        _this: &Object,
        _sel: Sel,
        pboard: *mut Object,
        _user_data: *mut Object,
        _error: *mut Object,
    ) {
        dispatch(FinderOpenKind::Window, pboard);
    }

    fn dispatch(kind: FinderOpenKind, pboard: *mut Object) {
        let paths = unsafe { paths_from_pasteboard(pboard) };
        if paths.is_empty() {
            log::warn!("Finder service {kind:?}: pasteboard had no file paths");
            return;
        }
        let tx = TX.lock().unwrap_or_else(|err| err.into_inner());
        match tx.as_ref() {
            Some(tx) => {
                if let Err(err) = tx.try_send(Request { kind, paths }) {
                    log::error!("Finder service {kind:?} dropped: {err}");
                }
            }
            None => log::error!("Finder service {kind:?} arrived before provider was bound"),
        }
    }

    unsafe fn paths_from_pasteboard(pboard: *mut Object) -> Vec<String> {
        if pboard.is_null() {
            return Vec::new();
        }

        let filenames_type = nsstring("NSFilenamesPboardType");
        let list: *mut Object = msg_send![pboard, propertyListForType: filenames_type];
        if !list.is_null() {
            let names = unsafe { nsarray_strings(list) };
            if !names.is_empty() {
                return names;
            }
        }

        for ty in [
            "public.file-url",
            "public.utf8-plain-text",
            "NSStringPboardType",
        ] {
            let t = nsstring(ty);
            let value: *mut Object = msg_send![pboard, stringForType: t];
            if value.is_null() {
                continue;
            }
            let Some(text) = rust_string(value) else {
                continue;
            };
            let parsed: Vec<String> = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            if !parsed.is_empty() {
                return parsed;
            }
        }
        Vec::new()
    }

    fn nsstring(s: &str) -> *mut Object {
        let c = CString::new(s).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
    }

    fn rust_string(ns: *mut Object) -> Option<String> {
        if ns.is_null() {
            return None;
        }
        unsafe {
            let utf8: *const i8 = msg_send![ns, UTF8String];
            if utf8.is_null() {
                return None;
            }
            Some(CStr::from_ptr(utf8).to_string_lossy().into_owned())
        }
    }

    unsafe fn nsarray_strings(arr: *mut Object) -> Vec<String> {
        let count: usize = msg_send![arr, count];
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let item: *mut Object = msg_send![arr, objectAtIndex: i];
            if let Some(s) = rust_string(item) {
                out.push(s);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_plain_path() {
        assert_eq!(parse_service_path(" /tmp/foo "), PathBuf::from("/tmp/foo"));
    }

    #[test]
    fn parse_file_url() {
        assert_eq!(
            parse_service_path("file:///Users/me/src"),
            PathBuf::from("/Users/me/src")
        );
    }

    #[test]
    fn parse_file_url_with_host() {
        assert_eq!(
            parse_service_path("file://localhost/Users/me/src"),
            PathBuf::from("/Users/me/src")
        );
    }

    #[test]
    fn parse_percent_encoded_file_url() {
        assert_eq!(
            parse_service_path("file:///tmp/my%20dir"),
            PathBuf::from("/tmp/my dir")
        );
    }

    #[test]
    fn directory_is_used_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = cwd_from_service_input(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(cwd, dir.path());
    }

    #[test]
    fn trailing_slash_on_directory_is_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let raw = format!("{}/", dir.path().display());
        let cwd = cwd_from_service_input(&raw).unwrap();
        assert_eq!(cwd, dir.path());
        assert!(
            !cwd.to_string_lossy().ends_with('/'),
            "cwd must not keep a trailing slash"
        );
    }

    #[test]
    fn file_uses_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("readme.txt");
        fs::write(&file, "x").unwrap();
        let cwd = cwd_from_service_input(file.to_str().unwrap()).unwrap();
        assert_eq!(cwd, dir.path());
    }

    #[test]
    fn missing_path_is_ignored() {
        assert_eq!(
            cwd_from_service_input("/definitely/not/a/sleipnir/finder/path"),
            None
        );
    }

    #[test]
    fn unique_cwds_dedupes_and_skips_missing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        let paths = vec![
            dir.path().to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
            "/no/such/sleipnir/path".into(),
            dir.path().to_string_lossy().into_owned(),
        ];
        assert_eq!(unique_cwds(&paths), vec![dir.path().to_path_buf()]);
    }

    #[test]
    fn info_plist_declares_finder_services() {
        let plist = include_str!("../../../resources/Info.plist");
        assert!(plist.contains("<key>NSServices</key>"));
        assert!(plist.contains("New Sleipnir Tab Here"));
        assert!(plist.contains("New Sleipnir Window Here"));
        assert!(plist.contains("<string>openTab</string>"));
        assert!(plist.contains("<string>openWindow</string>"));
        assert!(plist.contains("NSFilenamesPboardType"));
    }
}
