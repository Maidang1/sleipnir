//! Terminal settings surface for jiajia-term.
//! Backed by `jiajia_settings` instead of Zed's full settings stack.

pub use jiajia_settings::{
    AlternateScroll, CursorShape, TerminalBell, TerminalBlink, TerminalLineHeight, TerminalSettings,
    Toolbar, WorkingDirectory,
};
