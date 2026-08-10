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

    pub fn init(cx: &mut App) {
        cx.set_global(Self::default());
    }
}
