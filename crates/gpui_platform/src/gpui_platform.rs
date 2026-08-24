//! OS-branched application entry for sleipnir (forked/simplified from Zed gpui_platform).

pub use gpui::Platform;

use std::rc::Rc;

pub fn application() -> gpui::Application {
    gpui::Application::with_platform(current_platform(false))
}

#[cfg(any(target_os = "linux", test))]
fn linux_startup_diagnostic(source: &str) -> String {
    format!(
        "{source}\nLinux startup failed. Check WAYLAND_DISPLAY or DISPLAY, \
         install libvulkan1 and mesa-vulkan-drivers, or install the vendor \
         Vulkan driver for your GPU."
    )
}

#[cfg(any(target_os = "linux", test))]
fn linux_display_preflight(
    headless: bool,
    zed_headless: bool,
    wayland_display: Option<&str>,
    x11_display: Option<&str>,
) -> Result<(), String> {
    let has_display = [wayland_display, x11_display]
        .into_iter()
        .flatten()
        .any(|display| !display.is_empty());
    if headless || zed_headless || has_display {
        Ok(())
    } else {
        Err(linux_startup_diagnostic(
            "no usable Linux display was found for graphical startup",
        ))
    }
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

    #[cfg(target_os = "linux")]
    {
        linux_display_preflight(
            headless,
            std::env::var_os("ZED_HEADLESS").is_some(),
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        )
        .unwrap_or_else(|message| panic!("{message}"));

        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gpui_linux::current_platform(headless)
        })) {
            Ok(platform) => platform,
            Err(payload) => {
                let source = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown GPUI Linux platform initialization panic");
                panic!("{}", linux_startup_diagnostic(source));
            }
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = headless;
        compile_error!("sleipnir gpui_platform supports macOS, Windows, and Linux only");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_startup_diagnostic_keeps_source_and_actionable_hints() {
        let message = linux_startup_diagnostic("Failed to initialize X11 client");
        assert!(message.contains("Failed to initialize X11 client"));
        assert!(message.contains("WAYLAND_DISPLAY"));
        assert!(message.contains("DISPLAY"));
        assert!(message.contains("libvulkan1"));
        assert!(message.contains("mesa-vulkan-drivers"));
        assert!(message.contains("vendor Vulkan driver"));
    }

    #[test]
    fn linux_display_preflight_allows_headless_and_requires_a_gui_display() {
        assert!(linux_display_preflight(true, false, None, None).is_ok());
        assert!(linux_display_preflight(false, true, None, None).is_ok());
        assert!(linux_display_preflight(false, false, Some("wayland-0"), None).is_ok());
        assert!(linux_display_preflight(false, false, None, Some(":0")).is_ok());

        let message = linux_display_preflight(false, false, None, None).unwrap_err();
        assert!(message.contains("no usable Linux display"));
        assert!(message.contains("WAYLAND_DISPLAY"));
        assert!(message.contains("DISPLAY"));
        assert!(message.contains("libvulkan1"));
    }

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
            impl_src.contains("gpui_linux::current_platform"),
            "Linux backend must stay wired"
        );
        assert!(
            impl_src.contains("linux_display_preflight("),
            "graphical Linux startup must reject a missing display"
        );
        assert!(
            impl_src.contains("std::env::var_os(\"ZED_HEADLESS\")")
                && impl_src.contains("std::env::var(\"WAYLAND_DISPLAY\")")
                && impl_src.contains("std::env::var(\"DISPLAY\")"),
            "Linux startup must preflight the headless override and both display variables"
        );
        assert!(
            impl_src.contains("target_os = \"linux\""),
            "Linux constructor must be cfg-gated"
        );
        assert!(
            impl_src.contains("macOS, Windows, and Linux only"),
            "unsupported-target diagnostic must name the shipped platforms"
        );
    }
}
