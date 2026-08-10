//! macOS-only platform entry for harbor (forked/simplified from Zed gpui_platform).

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
