//! Kitty graphics commands parsed off the pty stream.
//!
//! The store, placements, and paint land with static images. This module is
//! the command parser the scanner needs to tell a graphics APC from any other.

mod command;

pub use command::{Action, Command, CursorMovement, Format};
