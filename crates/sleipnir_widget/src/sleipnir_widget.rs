//! Widget layout, measurement and hit-testing (ADR-0017).
//!
//! Widgets render in three places — Block (inside scrollback), Panel (a split),
//! Chrome (badges / status). Three implementations would mean three schemas
//! and three sets of bugs, so layout, measurement and hit-testing live here
//! once. Mount points consume the result; they do not re-measure.
//!
//! This crate is the pure, GPUI-free half. It depends on `plugin_protocol`
//! (the schema already defined in v2). It does **not** depend on gpui, does
//! not name `Hsla` or `Pixels`, and does no `f32` layout math. Sizes are
//! integer cells so a Block's height is an integer row count (ADR-0018) and
//! survives font-size changes.
//!
//! The schema is **not** redefined here: [`Widget`], [`Tone`], [`measure`],
//! [`MAX_WIDGET_NODES`] and [`MAX_WIDGET_DEPTH`] come from
//! `plugin_protocol::v2`. Unknown `t` values are an inert visible placeholder.
//! Over-budget trees are truncated with a visible marker, not rejected
//! silently and not rendered in full (constraint 5: external authors will
//! send pathological trees; layout cost is paid on the UI thread).
//!
//! Attribution (ADR-0017, ADR-0016 §7) is drawn by this renderer, not by the
//! plugin: every surface reserves a band naming the plugin, and tree content
//! cannot occupy it.

mod cells;
mod geom;
mod hit;
mod layout;

pub use cells::{
    ATTRIBUTION_ROWS, BAR_COLS, CHIP_PAD, ELLIPSIS, MAX_CODE_LINES, MAX_LEAF_CHARS, SPARK_RAMP,
    UNKNOWN_COLS, cell_cols, fit_cols, spark_glyphs, wrap_text,
};
pub use geom::{CellPos, CellRect};
pub use hit::{Hit, hit_test};
pub use layout::{CodeLine, LaidOut, LaidOutKind, Layout, Walk, layout};
pub use plugin_protocol::v2::{
    MAX_WIDGET_DEPTH, MAX_WIDGET_NODES, Tone, TreeStats, Widget, measure,
};

#[cfg(test)]
mod tests;
