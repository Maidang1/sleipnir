//! OS-branched application entry for sleipnir (forked/simplified from Zed gpui_platform).

pub use gpui::Platform;

use std::rc::Rc;

pub fn application() -> gpui::Application {
    gpui::Application::with_platform(current_platform(false))
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

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = headless;
        compile_error!("sleipnir gpui_platform supports macOS and Windows only");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_entry_selects_a_backend_per_os() {
        let src = include_str!("gpui_platform.rs");
        let impl_src = src.split("#[cfg(test)]").next().expect("impl before tests");
        assert!(
            impl_src.contains("gpui_macos::MacPlatform"),
            "macos backend must stay wired"
        );
        assert!(
            impl_src.contains("gpui_windows::WindowsPlatform"),
            "windows backend must be constructed on Windows"
        );
        assert!(
            impl_src.contains("target_os = \"windows\""),
            "Windows constructor must be cfg-gated"
        );
        assert!(
            impl_src.contains("target_os = \"macos\""),
            "macOS constructor must be cfg-gated"
        );
        assert!(
            !impl_src.contains("gpui_linux::"),
            "Linux backend must stay removed"
        );
    }
}
