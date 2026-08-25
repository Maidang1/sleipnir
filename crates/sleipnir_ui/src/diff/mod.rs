//! Git diff inspector: parse + flatten + overlay. Not a Pane.

mod fetch;
mod minimap;
mod render;
mod rows;
pub(crate) mod upgrade;

pub(crate) use fetch::{FetchOutcome, fetch_worktree_diff};
pub(crate) use rows::{
    Cell, DisplayRow, FileUpgrade, LineKind, TreeEntry, ViewMode, file_index_at,
};

use gpui::{ListAlignment, ListOffset, ListState, Pixels, px};
use std::path::PathBuf;
use std::time::Instant;

/// How long a fetched session may be reused without hitting git again.
pub(crate) const SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(3);

/// Window-level inspector state. Lives on AppShell; never on a Pane.
pub(crate) enum DiffView {
    Loading {
        title: String,
        #[allow(dead_code)]
        generation: u64,
    },
    Ready(DiffSession),
    Message {
        title: String,
        body: String,
    },
}

pub(crate) struct DiffSession {
    pub root: PathBuf,
    pub title: String,
    pub additions: u32,
    pub deletions: u32,
    pub parsed: diff_core::PatchDiff,
    pub mode: ViewMode,
    pub rows: Vec<DisplayRow>,
    pub file_rows: Vec<usize>,
    pub hunk_rows: Vec<usize>,
    pub tree: Vec<TreeEntry>,
    pub upgrades: std::collections::HashMap<usize, FileUpgrade>,
    pub patch: String,
    pub scroll: ListState,
    pub cursor: usize,
    pub fetched_at: Instant,
    pub minimap_visible: bool,
}

impl DiffSession {
    pub fn from_ready(ready: fetch::ReadyDiff, mode: ViewMode) -> Self {
        let (rows, file_rows, hunk_rows, tree) = rows::build_rows(&ready.parsed, mode);
        let row_count = rows.len();
        Self {
            root: ready.root,
            title: ready.title,
            additions: ready.additions,
            deletions: ready.deletions,
            parsed: ready.parsed,
            mode,
            rows,
            file_rows,
            hunk_rows,
            tree,
            upgrades: std::collections::HashMap::new(),
            patch: ready.patch,
            scroll: ListState::new(row_count, ListAlignment::Top, px(80.0)),
            cursor: 0,
            fetched_at: Instant::now(),
            minimap_visible: true,
        }
    }

    fn replace_rows(&mut self) {
        let (rows, file_rows, hunk_rows, tree) =
            rows::build_rows_with(&self.parsed, self.mode, &self.upgrades);
        self.rows = rows;
        self.file_rows = file_rows;
        self.hunk_rows = hunk_rows;
        self.tree = tree;
    }

    fn rebuild_rows(&mut self) {
        self.replace_rows();
        self.scroll.reset(self.rows.len());
    }

    fn scroll_row_to_top(&self, ix: usize) {
        self.scroll.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(0.0),
        });
    }

    pub fn set_mode(&mut self, mode: ViewMode) {
        if self.mode == mode {
            return;
        }
        let file_ix = file_index_at(&self.file_rows, self.cursor).unwrap_or(0);
        self.mode = mode;
        self.rebuild_rows();
        self.cursor = self.file_rows.get(file_ix).copied().unwrap_or(0);
        self.scroll_row_to_top(self.cursor);
    }

    pub fn apply_upgrades(&mut self, files: Vec<upgrade::UpgradedFile>) {
        for file in files {
            if let Some(target) = self.parsed.files.get_mut(file.file_ix) {
                target.hunks = file.hunks;
                target.additions = file.additions;
                target.deletions = file.deletions;
            }
            self.upgrades.insert(file.file_ix, file.upgrade);
        }
        (self.additions, self.deletions) = self
            .parsed
            .files
            .iter()
            .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
        let file_ix = file_index_at(&self.file_rows, self.cursor).unwrap_or(0);
        self.rebuild_rows();
        self.cursor = self.file_rows.get(file_ix).copied().unwrap_or(0);
    }

    /// Expand one hidden gap. Returns the display-row index of the gap and how
    /// many rows were inserted (for scroll compensation).
    pub fn expand_gap(&mut self, file_ix: usize, gap_ix: usize) -> Option<(usize, usize)> {
        let upgrade = self.upgrades.get_mut(&file_ix)?;
        if !upgrade.expanded.insert(gap_ix) {
            return None;
        }
        let gap_row = self.rows.iter().position(|row| {
            matches!(
                row,
                DisplayRow::Gap {
                    file_ix: f,
                    gap_ix: g,
                    ..
                } if *f == file_ix && *g == gap_ix
            )
        })?;
        let old_len = self.rows.len();
        self.replace_rows();
        let inserted = self.rows.len().saturating_sub(old_len);
        self.scroll.splice(gap_row..gap_row + 1, inserted + 1);
        Some((gap_row, inserted))
    }

    pub fn jump_to_row(&mut self, row: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.cursor = row.min(self.rows.len() - 1);
        self.scroll_row_to_top(self.cursor);
    }

    pub fn still_fresh(&self, root: &std::path::Path) -> bool {
        self.root == root && self.fetched_at.elapsed() < SESSION_TTL
    }

    pub fn jump_next(&mut self, targets: &[usize]) {
        let next = targets.iter().copied().find(|&ix| ix > self.cursor);
        if let Some(ix) = next.or_else(|| targets.first().copied()) {
            self.cursor = ix;
            self.scroll_row_to_top(ix);
        }
    }

    pub fn jump_prev(&mut self, targets: &[usize]) {
        let prev = targets.iter().rev().copied().find(|&ix| ix < self.cursor);
        if let Some(ix) = prev.or_else(|| targets.last().copied()) {
            self.cursor = ix;
            self.scroll_row_to_top(ix);
        }
    }

    pub fn jump_home(&mut self) {
        self.cursor = 0;
        self.scroll_row_to_top(0);
    }

    pub fn jump_end(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.cursor = self.rows.len() - 1;
        self.scroll.scroll_to_end();
    }
}

pub(crate) fn row_height(font_px: Pixels) -> Pixels {
    px((f32::from(font_px) * 1.7).round())
}
