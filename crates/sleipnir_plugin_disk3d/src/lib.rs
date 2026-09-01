//! Disk 3D — a plugin that shows where disk space went, as a real 3D chart.
//!
//! The point is the *data*: each cuboid is a direct child of the working
//! directory, its height is that child's share of the bytes, and the picture is
//! rotatable so the bars can be read from any side. Rendering is a genuine
//! software rasteriser (projection, z-buffer, Lambert shading) whose
//! framebuffer happens to be character cells, because ADR-0004 declines the
//! graphics protocol and ADR-0017 bans images. Split into three testable parts:
//!
//! - [`raster`] — 3D maths and the cell framebuffer. Knows nothing about disks.
//! - [`scan`]   — the bounded filesystem walk that produces the numbers.
//! - [`view`]   — scan + camera → widget tree. Pure, so it is unit testable.

pub mod raster;
pub mod scan;
pub mod view;

pub use raster::{Camera, Canvas, Scene};
pub use scan::{Entry, Scan, human_bytes, scan};
pub use view::{View, render};

/// Yaw step per arrow press, in radians (15°).
pub const YAW_STEP: f32 = std::f32::consts::PI / 12.0;

/// Pitch step per arrow press, in radians (~7°).
pub const PITCH_STEP: f32 = std::f32::consts::PI / 24.0;

/// Multiplicative zoom step.
pub const ZOOM_STEP: f32 = 1.25;

/// Yaw advance per animation frame while spinning.
pub const SPIN_STEP: f32 = 0.14;

/// Frame interval while spinning. ~12 fps: fast enough to read as motion,
/// slow enough that each frame is one bounded `Render` on the host's queue.
pub const SPIN_INTERVAL_MS: u64 = 80;
