//! Terminal settings surface for sleipnir.
//! Backed by `sleipnir_settings` instead of Zed's full settings stack.

pub use sleipnir_settings::{
    AlternateScroll, ConfirmClose, CursorShape, TerminalBell, TerminalBlink, TerminalLineHeight,
    TerminalSettings,
};
