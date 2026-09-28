//! The kitty graphics protocol: commands, and the store that carries them out.
//!
//! Placements — where an image sits on the grid — live in [`crate::graphics`].
//! Painting is the view's.

use std::collections::HashMap;

/// The most one assembled image may carry, chunks included: 64 MiB.
const MAX_IMAGE: usize = 64 << 20;

/// How many images are kept before the oldest is dropped.
const MAX_IMAGES: usize = 64;

mod animation;
mod command;
mod store;

pub use command::*;
pub use store::*;
