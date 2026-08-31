//! [`RowGeometry`]: the step-function mapping both paint and hit-testing use.
//!
//! Internally this is integer cell-row space. Conversion to pixels is one
//! multiply so a long scrollback cannot accumulate float error. `y_for` and
//! `hit` are inverses on cell rows by construction.

use crate::Px;
use plugin_protocol::v2::{BlockId, RunId};

/// Scrollback position of a Block. Same shape as `run_ledger::run::Anchor`,
/// and process-local for the same reason: a restored Block would claim a line
/// that no longer means anything (ADR-0018 lifecycle). Never persisted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Anchor {
    /// Absolute line (`cursor.line + history_size` when the Run was recorded).
    pub line: i32,
    pub column: usize,
}

/// One Block placed in scrollback, anchored to a Run.
///
/// Height is an integer cell-row count from `sleipnir_widget::layout`. The
/// Block occupies that many rows at [`Self::anchor`], replacing the single
/// character row the grid still stores there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Block {
    pub id: BlockId,
    pub run_id: RunId,
    pub anchor: Anchor,
    pub height: u16,
}

/// Result of mapping a pixel y to a grid line or a Block.
///
/// `hit` takes only y: column is an x-axis concern the mouse mapper still
/// owns. `local_y` is pixels from the top of the hit Block, not of the
/// stacked group at that line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitTarget {
    Cell { line: i32 },
    Block { id: BlockId, local_y: Px },
}

/// One mapping, both directions (ADR-0018 decision 1).
///
/// Host-side. Knows nothing about the grid's internals; the grid knows
/// nothing about it. The sorted span list is an implementation detail so
/// `y_for` / `hit` can binary-search; callers upsert Blocks and query.
#[derive(Clone, Debug)]
pub struct RowGeometry {
    line_height: Px,
    line_count: i32,
    alt_screen: bool,
    frozen: bool,
    spans: Vec<Span>,
}

/// Prepared Block: the caller's [`Block`] plus the prefix state `y_for`/`hit`
/// binary-search over. Rebuilt on every mutation; never on the hot path.
#[derive(Clone, Debug)]
struct Span {
    block: Block,
    /// Extra cell-rows from every block-line strictly before this one.
    cum_extra: u64,
    /// Extra cell-rows this grid line contributes (`sum(heights) - 1`, or 0
    /// when the line's blocks have total height 0 and the cell remains).
    line_extra: u64,
    /// Cell-row index of this Block's top. Stacked siblings at the same
    /// line have increasing `start_rows`.
    start_rows: i64,
}

impl RowGeometry {
    /// Empty document, no Blocks, linear mapping.
    ///
    /// A non-finite or non-positive `line_height` is kept as given for
    /// inspection but treated as zero by every query so we never panic or
    /// invert the axis.
    pub fn new(line_height: Px) -> Self {
        Self {
            line_height,
            line_count: 0,
            alt_screen: false,
            frozen: false,
            spans: Vec::new(),
        }
    }

    pub fn line_height(&self) -> Px {
        self.line_height
    }

    pub fn set_line_height(&mut self, line_height: Px) {
        self.line_height = line_height;
    }

    /// Number of grid lines in the document (`history + screen`). Used only
    /// for [`Self::total_height`]; queries on other lines still work so an
    /// out-of-range anchor cannot panic.
    pub fn line_count(&self) -> i32 {
        self.line_count
    }

    pub fn set_line_count(&mut self, line_count: i32) {
        self.line_count = line_count.max(0);
    }

    /// ADR-0018 decision 4: while the alt screen is active, geometry reports
    /// no Blocks and collapses to `line * line_height`. The Blocks themselves
    /// stay; they reappear on return to the primary screen.
    pub fn set_alt_screen(&mut self, alt_screen: bool) {
        self.alt_screen = alt_screen;
    }

    pub fn alt_screen(&self) -> bool {
        self.alt_screen
    }

    /// ADR-0018 decision 3: pin Block heights to their last computed values
    /// so a divider drag does not re-layout every visible tree each frame.
    /// Anchors still rebase (history shrink / eviction); only height updates
    /// are ignored.
    pub fn set_frozen(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Insert or replace a Block. Unsorted and overlapping input is
    /// normalised: spans sort by `(anchor.line, id)`, and several Blocks at
    /// the same line stack in that order. While frozen, an existing Block
    /// keeps its pinned height.
    pub fn upsert(&mut self, mut block: Block) {
        if let Some(idx) = self.spans.iter().position(|s| s.block.id == block.id) {
            if self.frozen {
                block.height = self.spans[idx].block.height;
            }
            self.spans[idx].block = block;
        } else {
            self.spans.push(Span::from_block(block));
        }
        self.rebuild();
    }

    pub fn remove(&mut self, id: BlockId) -> Option<Block> {
        let idx = self.spans.iter().position(|s| s.block.id == id)?;
        let span = self.spans.remove(idx);
        self.rebuild();
        Some(span.block)
    }

    pub fn get(&self, id: BlockId) -> Option<&Block> {
        self.spans.iter().map(|s| &s.block).find(|b| b.id == id)
    }

    /// Blocks in ascending anchor order, including those hidden by alt-screen.
    /// Alt-screen suppresses them from `y_for`/`hit`, not from the store.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.spans.iter().map(|s| &s.block)
    }

    pub fn clear_blocks(&mut self) {
        self.spans.clear();
    }

    /// History shrink (`clear`, `ED 3`). Same rule as
    /// `rebase_markers_after_history_shrink` in `osc133.rs`: survivors shift
    /// down by `removed`; a Block whose anchor fell inside the removed
    /// region is dropped.
    pub fn rebase_after_history_shrink(&mut self, removed: i32) {
        if removed <= 0 {
            return;
        }
        self.spans.retain(|s| s.block.anchor.line >= removed);
        for span in &mut self.spans {
            span.block.anchor.line -= removed;
        }
        self.rebuild();
    }

    /// Scrollback eviction: drop every Block whose anchor line is `< line`.
    /// The grid has forgotten those rows; the Block goes with them.
    pub fn evict_before(&mut self, line: i32) {
        let before = self.spans.len();
        self.spans.retain(|s| s.block.anchor.line >= line);
        if self.spans.len() != before {
            self.rebuild();
        }
    }

    /// Pixel y of the top of `abs_line`. A step function: between Blocks it
    /// advances by one `line_height`; across a Block it advances by that
    /// Block's integer row count.
    pub fn y_for(&self, abs_line: i32) -> Px {
        if !usable_line_height(self.line_height) {
            return 0.0;
        }
        if self.ignores_blocks() {
            return (abs_line as Px) * self.line_height;
        }
        let extra = self.extra_before(abs_line);
        let rows = (abs_line as i64).saturating_add(i64_from_u64(extra));
        self.rows_to_px(rows)
    }

    /// Pixel → row or Block. Replaces `(pos.y / line_height) as i32`.
    ///
    /// Inverse of [`Self::y_for`] on cell rows: `hit(y_for(line))` identifies
    /// that same line, as a [`HitTarget::Cell`] or as a [`HitTarget::Block`]
    /// at that anchor with `local_y == 0`.
    pub fn hit(&self, y: Px) -> HitTarget {
        let loc = self.locate(y);
        match loc.block {
            Some((id, local_y)) => HitTarget::Block { id, local_y },
            None => HitTarget::Cell { line: loc.line },
        }
    }

    /// Total document height, for scroll extent. Empty documents are 0.
    pub fn total_height(&self) -> Px {
        if self.line_count <= 0 {
            return 0.0;
        }
        self.y_for(self.line_count)
    }

    /// Pixel height of one grid line, including stacked Blocks at that line.
    /// Always one `line_height` under alt-screen, and for lines with no Block.
    pub fn height_of(&self, abs_line: i32) -> Px {
        if !usable_line_height(self.line_height) {
            return 0.0;
        }
        (self.row_count_of(abs_line) as Px) * self.line_height
    }
}

struct Locate {
    line: i32,
    block: Option<(BlockId, Px)>,
}

impl RowGeometry {
    fn ignores_blocks(&self) -> bool {
        self.alt_screen || self.spans.is_empty()
    }

    fn extra_before(&self, line: i32) -> u64 {
        let i = self.spans.partition_point(|s| s.block.anchor.line < line);
        if i == 0 {
            0
        } else {
            let prev = &self.spans[i - 1];
            prev.cum_extra.saturating_add(prev.line_extra)
        }
    }

    fn row_count_of(&self, abs_line: i32) -> u64 {
        if self.ignores_blocks() {
            return 1;
        }
        let i = self
            .spans
            .partition_point(|s| s.block.anchor.line < abs_line);
        if i >= self.spans.len() || self.spans[i].block.anchor.line != abs_line {
            return 1;
        }
        // `line_extra = sum(heights).saturating_sub(1)`. When every Block at
        // this line has height 0, extra is 0 and the character row remains.
        self.spans[i].line_extra.saturating_add(1)
    }

    fn rows_to_px(&self, rows: i64) -> Px {
        if !usable_line_height(self.line_height) {
            0.0
        } else {
            (rows as Px) * self.line_height
        }
    }

    /// Inverse of [`Self::rows_to_px`]. Snaps so `rows_to_px(n) <= y <
    /// rows_to_px(n+1)`, which is what makes `y_for`/`hit` exact inverses
    /// even when `line_height` is not a power of two.
    fn rows_from_px(&self, y: Px) -> i64 {
        if !usable_line_height(self.line_height) || !y.is_finite() {
            return 0;
        }
        let h = self.line_height;
        let mut n = (y / h).floor() as i64;
        for _ in 0..16 {
            let start = self.rows_to_px(n);
            if start > y {
                n = n.saturating_sub(1);
                continue;
            }
            let end = self.rows_to_px(n.saturating_add(1));
            if end <= y {
                n = n.saturating_add(1);
                continue;
            }
            break;
        }
        n
    }

    fn locate(&self, y: Px) -> Locate {
        if !y.is_finite() || !usable_line_height(self.line_height) {
            return Locate {
                line: 0,
                block: None,
            };
        }
        if self.ignores_blocks() {
            return Locate {
                line: i32_from_i64(self.rows_from_px(y)),
                block: None,
            };
        }
        let rows = self.rows_from_px(y);
        let i = self.spans.partition_point(|s| s.start_rows <= rows);
        if i == 0 {
            return Locate {
                line: i32_from_i64(rows),
                block: None,
            };
        }
        let span = &self.spans[i - 1];
        let end = span.start_rows.saturating_add(i64::from(span.block.height));
        if rows < end {
            let local_y = (y - self.rows_to_px(span.start_rows)).max(0.0);
            return Locate {
                line: span.block.anchor.line,
                block: Some((span.block.id, local_y)),
            };
        }
        let extra_through = span.cum_extra.saturating_add(span.line_extra);
        let line = rows.saturating_sub(i64_from_u64(extra_through));
        Locate {
            line: i32_from_i64(line),
            block: None,
        }
    }

    fn rebuild(&mut self) {
        self.spans
            .sort_by_key(|s| (s.block.anchor.line, s.block.id));
        let mut i = 0;
        let mut cum = 0u64;
        while i < self.spans.len() {
            let line = self.spans[i].block.anchor.line;
            let mut j = i + 1;
            while j < self.spans.len() && self.spans[j].block.anchor.line == line {
                j += 1;
            }
            let mut sum_h = 0u64;
            for span in &self.spans[i..j] {
                sum_h = sum_h.saturating_add(u64::from(span.block.height));
            }
            let line_extra = sum_h.saturating_sub(1);
            let mut start = (line as i64).saturating_add(i64_from_u64(cum));
            for span in &mut self.spans[i..j] {
                span.cum_extra = cum;
                span.line_extra = line_extra;
                span.start_rows = start;
                start = start.saturating_add(i64::from(span.block.height));
            }
            cum = cum.saturating_add(line_extra);
            i = j;
        }
    }
}

impl Span {
    fn from_block(block: Block) -> Self {
        Self {
            block,
            cum_extra: 0,
            line_extra: 0,
            start_rows: 0,
        }
    }
}

fn usable_line_height(h: Px) -> bool {
    h.is_finite() && h > 0.0
}

pub(crate) fn i32_from_i64(n: i64) -> i32 {
    i32::try_from(n).unwrap_or(if n < 0 { i32::MIN } else { i32::MAX })
}

pub(crate) fn i32_from_usize(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

pub(crate) fn usize_from_i32(n: i32) -> usize {
    usize::try_from(n).unwrap_or(0)
}

fn i64_from_u64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}
