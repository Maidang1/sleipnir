//! Minimal release_channel stub for sleipnir (not the full Zed crate).

use gpui::{App, Global};
use std::fmt;

#[derive(Clone, Debug)]
pub struct AppVersion(pub String);

impl Global for AppVersion {}

impl Default for AppVersion {
    fn default() -> Self {
        Self(env!("CARGO_PKG_VERSION").to_string())
    }
}

impl fmt::Display for AppVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AppVersion {
    pub fn global(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Initialize with an explicit version string.
    ///
    /// Callers should pass their own `env!("CARGO_PKG_VERSION")` so the reported
    /// version matches the released binary (not this helper crate's version).
    pub fn init_with(version: impl Into<String>, cx: &mut App) {
        cx.set_global(Self(version.into()));
    }
}
