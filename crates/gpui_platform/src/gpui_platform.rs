//! OS-branched platform entry for sleipnir (forked/simplified from Zed gpui_platform).

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

/// Returns the default [`Platform`] for the current OS.
pub fn current_platform(headless: bool) -> Rc<dyn Platform> {
    #[cfg(target_os = "macos")]
    {
        Rc::new(gpui_macos::MacPlatform::new(headless))
    }

    #[cfg(target_os = "windows")]
    {
        Rc::new(
            gpui_windows::WindowsPlatform::new(headless)
                .expect("failed to initialize Windows platform"),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        gpui_linux::current_platform(headless)
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        let _ = headless;
        compile_error!("sleipnir gpui_platform supports macOS, Windows, and Linux only");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_entry_selects_a_backend_per_os() {
        let src = include_str!("gpui_platform.rs");
        assert!(
            src.contains("gpui_macos::MacPlatform"),
            "macos backend must stay wired"
        );
        assert!(
            src.contains("gpui_windows::WindowsPlatform"),
            "windows backend must be constructed on Windows"
        );
        assert!(
            src.contains("target_os = \"windows\""),
            "Windows constructor must be cfg-gated"
        );
        assert!(
            src.contains("target_os = \"macos\""),
            "macOS constructor must be cfg-gated"
        );
        assert!(
            src.contains("gpui_linux::current_platform"),
            "linux backend must stay wired"
        );
        assert!(
            src.contains("target_os = \"linux\""),
            "Linux constructor must be cfg-gated"
        );
    }
}
