//! Light/dark switching: what the user asked for, what the OS reports, and the
//! repaint that follows a change in either.
//!
//! Three pieces:
//!
//! 1. [`AppearanceMode`] — follow the OS, or pin light or dark.
//! 2. [`AppearanceState`] — a gpui global holding that choice beside the last
//!    appearance the OS reported, so [`resolve`] can combine them.
//! 3. [`observe_window`] — subscribes to the platform appearance notification
//!    and re-applies. Settings hot-reload calls [`apply`] the same way; this
//!    module does not install a palette at process boot.
//!
//! # Why `refresh_windows` and not `notify`
//!
//! Chrome and terminal colors are read imperatively at paint time. No view
//! holds a reactive binding, so `notify()` on one entity repaints that entity
//! and leaves every other window on the cached palette. [`App::refresh_windows`]
//! marks every window dirty and disables gpui's per-view prepaint cache for
//! the frame, which is what forces already-laid-out elements to paint again.

use gpui::{App, Global, Subscription, Window, WindowAppearance};
use serde::{Deserialize, Serialize};
use sleipnir_settings::{Appearance, TerminalSettings};

/// The user's appearance preference.
///
/// Serde-serializable so a settings file can store it. This module never
/// touches disk; hot-reload is the caller's job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppearanceMode {
    /// Follow the OS. The default.
    #[default]
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];
}

/// What the user chose, and what the OS last said.
///
/// Kept separate so flipping the OS appearance while the user has pinned Light
/// still records the new system value, and takes effect the moment they switch
/// back to [`AppearanceMode::System`].
pub struct AppearanceState {
    pub mode: AppearanceMode,
    pub system: Appearance,
}

impl Global for AppearanceState {}

/// Combine the user's choice with the OS state.
pub fn resolve(mode: AppearanceMode, system: Appearance) -> Appearance {
    match mode {
        AppearanceMode::System => system,
        AppearanceMode::Light => Appearance::Light,
        AppearanceMode::Dark => Appearance::Dark,
    }
}

/// Whether what a window reports is the OS's own answer.
///
/// A pinned mode can make the platform report that override back. Only
/// [`AppearanceMode::System`] has nothing in the way, so only then is the
/// report stored.
pub fn reports_the_os(mode: AppearanceMode) -> bool {
    matches!(mode, AppearanceMode::System)
}

pub(crate) fn from_window(appearance: WindowAppearance) -> Appearance {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

/// The mode currently in effect (defaults to [`AppearanceMode::System`]).
pub fn mode(cx: &App) -> AppearanceMode {
    cx.try_global::<AppearanceState>()
        .map(|state| state.mode)
        .unwrap_or_default()
}

/// Install the global from the window that is opening and resolve the palette.
///
/// Call once per window open, after settings have loaded. Later changes go
/// through [`set_mode`], [`sync_system`], or [`apply`] from settings hot-reload.
pub fn boot(system: Appearance, cx: &mut App) {
    cx.set_global(AppearanceState {
        mode: AppearanceMode::System,
        system,
    });
    apply(cx);
}

/// Change the user's preference and repaint when that changes the palette.
/// Persisting the choice is the caller's job.
pub fn set_mode(mode: AppearanceMode, cx: &mut App) {
    if !cx.has_global::<AppearanceState>() {
        return;
    }
    let state = cx.global_mut::<AppearanceState>();
    if state.mode == mode {
        return;
    }
    state.mode = mode;
    // Coming back to System, ask the OS before resolving. While a mode was
    // pinned, [`sync_system`] declined to record window reports, so `system`
    // is as stale as the moment the mode was pinned.
    if mode == AppearanceMode::System {
        let system = from_window(cx.window_appearance());
        cx.global_mut::<AppearanceState>().system = system;
    }
    apply(cx);
}

/// Subscribe a window to OS appearance changes.
///
/// The notification is per window, but the appearance it reports is a system
/// setting, so any one window is enough. Re-applying is idempotent.
///
/// Reconcile against the window's own appearance before subscribing. Asking
/// the app before a window exists reads a value that is not reliably populated
/// that early on macOS, and a wrong guess sticks until some later event.
pub fn observe_window(window: &mut Window, cx: &mut App) -> Subscription {
    sync_system(from_window(window.appearance()), cx);
    window.observe_window_appearance(|window, cx| {
        sync_system(from_window(window.appearance()), cx);
    })
}

/// Record an OS appearance and re-apply if it moved.
///
/// Only what [`reports_the_os`] will vouch for. Recording an override read
/// back would overwrite the OS value with the mode just left, and the trip
/// back to System would resolve to that mode.
pub fn sync_system(system: Appearance, cx: &mut App) {
    if !cx.has_global::<AppearanceState>() {
        boot(system, cx);
        return;
    }
    let state = cx.global_mut::<AppearanceState>();
    if !reports_the_os(state.mode) || state.system == system {
        return;
    }
    state.system = system;
    apply(cx);
}

/// Re-resolve the active palette from [`AppearanceState`] and force a full
/// repaint. Settings hot-reload calls this after the file has been read, so a
/// theme change and an appearance change share one path.
///
/// Always refreshes, even when the resolved appearance did not move: a theme
/// edit with the same light/dark mode still has to bust the prepaint cache.
pub fn apply(cx: &mut App) {
    let Some(state) = cx.try_global::<AppearanceState>() else {
        return;
    };
    let wanted = resolve(state.mode, state.system);
    TerminalSettings::set_appearance(wanted, cx);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_mode_follows_the_os() {
        assert_eq!(
            resolve(AppearanceMode::System, Appearance::Light),
            Appearance::Light
        );
        assert_eq!(
            resolve(AppearanceMode::System, Appearance::Dark),
            Appearance::Dark
        );
    }

    #[test]
    fn pinned_modes_ignore_the_os() {
        for system in [Appearance::Light, Appearance::Dark] {
            assert_eq!(resolve(AppearanceMode::Light, system), Appearance::Light);
            assert_eq!(resolve(AppearanceMode::Dark, system), Appearance::Dark);
        }
    }

    #[test]
    fn default_mode_is_system() {
        assert_eq!(AppearanceMode::default(), AppearanceMode::System);
    }

    #[test]
    fn mode_serialises_stably() {
        for (mode, json) in [
            (AppearanceMode::System, "\"system\""),
            (AppearanceMode::Light, "\"light\""),
            (AppearanceMode::Dark, "\"dark\""),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), json);
            assert_eq!(serde_json::from_str::<AppearanceMode>(json).unwrap(), mode);
        }
    }

    #[test]
    fn only_system_mode_hears_the_os() {
        assert!(reports_the_os(AppearanceMode::System));
        assert!(!reports_the_os(AppearanceMode::Light));
        assert!(!reports_the_os(AppearanceMode::Dark));
    }
}
