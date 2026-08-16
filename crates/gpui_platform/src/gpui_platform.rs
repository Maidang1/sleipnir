//! macOS application entry for sleipnir (forked/simplified from Zed gpui_platform).

#[cfg(not(target_os = "macos"))]
compile_error!("Sleipnir is macOS-only");

pub use gpui::Platform;

use std::rc::Rc;

/// Returns a background executor for the current platform.
pub fn background_executor() -> gpui::BackgroundExecutor {
    current_platform(true).background_executor()
}

pub fn application() -> gpui::Application {
    gpui::Application::with_platform(current_platform(false))
}

pub fn headless() -> gpui::Application {
    gpui::Application::with_platform(current_platform(true))
}

/// Returns the default [`Platform`] for macOS.
pub fn current_platform(headless: bool) -> Rc<dyn Platform> {
    Rc::new(gpui_macos::MacPlatform::new(headless))
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_entry_selects_macos_backend() {
        let src = include_str!("gpui_platform.rs");
        let impl_src = src.split("#[cfg(test)]").next().expect("impl before tests");
        assert!(
            impl_src.contains("gpui_macos::MacPlatform"),
            "macos backend must stay wired"
        );
        assert!(
            !impl_src.contains("WindowsPlatform"),
            "Windows backend must stay removed"
        );
        assert!(
            !impl_src.contains("gpui_linux::"),
            "Linux backend must stay removed"
        );
    }
}
