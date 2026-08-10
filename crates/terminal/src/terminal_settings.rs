//! Terminal settings surface for harbor.
//! Backed by `harbor_settings` instead of Zed's full settings stack.

pub use harbor_settings::{
    AlternateScroll, CursorShape, TerminalBell, TerminalBlink, TerminalLineHeight, TerminalSettings,
    Toolbar, WorkingDirectory,
};
