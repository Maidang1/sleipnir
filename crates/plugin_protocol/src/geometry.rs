//! Process-local geometry types.
//!
//! These are host-internal, never crossing the wire: they are not
//! `Serialize`/`Deserialize` and are deliberately kept out of the versioned
//! [`crate::v2`] dialect surface, even though `v2` re-exports [`Anchor`] for
//! consumers that historically imported it from there.

/// Scrollback position of a Run or Block. Process-local — never persisted:
/// a restored anchor would claim a scrollback line that no longer means
/// anything (ADR-0018 lifecycle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Anchor {
    /// Absolute line (`cursor.line + history_size` when the Run was recorded).
    pub line: i32,
    pub column: usize,
}
