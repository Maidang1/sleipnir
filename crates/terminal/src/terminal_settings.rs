//! Terminal settings surface for sleipnir.
//! Backed by `sleipnir_settings` instead of Zed's full settings stack.

pub use sleipnir_settings::{
    AlternateScroll, CursorShape, TerminalBell, TerminalBlink, TerminalLineHeight, TerminalSettings,
    Toolbar, WorkingDirectory,
};
