//! Flatten a parsed patch into unified or split display rows.

use std::ops::Range;

use diff_core::{DiffRow, FileStatus, PatchDiff};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    Unified,
    #[default]
    Split,
}

impl ViewMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Unified => Self::Split,
            Self::Split => Self::Unified,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unified => "Unified",
            Self::Split => "Split",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

/// One side of a split row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub no: u32,
    pub kind: LineKind,
    pub text: String,
    pub intra: Vec<Range<usize>>,
    pub syntax: Vec<(Range<usize>, syntax::Token)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayRow {
    FileHeader {
        path: String,
        status: FileStatus,
        additions: u32,
        deletions: u32,
    },
    HunkHeader {
        label: String,
    },
    Binary,
    Gap {
        file_ix: usize,
        gap_ix: usize,
        hidden: u32,
    },
    Line {
        old_no: Option<u32>,
        new_no: Option<u32>,
        kind: LineKind,
        text: String,
        intra: Vec<Range<usize>>,
        syntax: Vec<(Range<usize>, syntax::Token)>,
    },
    SplitLine {
        left: Option<Cell>,
        right: Option<Cell>,
    },
}

/// Whole-file highlight tables after a successful Phase-3 upgrade.
#[derive(Clone, Debug)]
pub struct FileUpgrade {
    pub new_lines: Vec<String>,
    pub old_spans: Vec<Vec<(Range<usize>, syntax::Token)>>,
    pub new_spans: Vec<Vec<(Range<usize>, syntax::Token)>>,
    pub expanded: std::collections::HashSet<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub row: usize,
}

const MAX_HUNK_SOURCE_BYTES: usize = 100 * 1024;
const MAX_SYNTAX_LINE_BYTES: usize = 4096;

pub fn build_rows(
    diff: &PatchDiff,
    mode: ViewMode,
) -> (Vec<DisplayRow>, Vec<usize>, Vec<usize>, Vec<TreeEntry>) {
    build_rows_with(diff, mode, &std::collections::HashMap::new())
}

pub fn build_rows_with(
    diff: &PatchDiff,
    mode: ViewMode,
    upgrades: &std::collections::HashMap<usize, FileUpgrade>,
) -> (Vec<DisplayRow>, Vec<usize>, Vec<usize>, Vec<TreeEntry>) {
    let mut rows = Vec::new();
    let mut file_rows = Vec::new();
    let mut hunk_rows = Vec::new();
    let mut tree = Vec::new();

    for (file_ix, file) in diff.files.iter().enumerate() {
        file_rows.push(rows.len());
        tree.push(TreeEntry {
            path: file.display_path().to_string(),
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            row: rows.len(),
        });
        rows.push(DisplayRow::FileHeader {
            path: file.display_path().to_string(),
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
        });
        if file.status == FileStatus::Binary {
            rows.push(DisplayRow::Binary);
            continue;
        }
        let upgrade = upgrades.get(&file_ix);
        let lang = syntax::language_for_path(file.display_path());
        for (hunk_ix, hunk) in file.hunks.iter().enumerate() {
            if let Some(upgrade) = upgrade {
                push_gap(&mut rows, upgrade, &file.hunks, file_ix, hunk_ix, mode);
            }
            let syntax_spans = match upgrade {
                Some(upgrade) => hunk
                    .rows
                    .iter()
                    .map(|row| upgrade_row_spans(upgrade, row))
                    .collect(),
                None => hunk_syntax(lang, &hunk.rows),
            };
            hunk_rows.push(rows.len());
            let mut label = format!(
                "@@ -{},{} +{},{} @@",
                hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
            );
            if !hunk.section.is_empty() {
                label.push(' ');
                label.push_str(&hunk.section);
            }
            rows.push(DisplayRow::HunkHeader { label });
            match mode {
                ViewMode::Unified => push_unified(&mut rows, &hunk.rows, &syntax_spans),
                ViewMode::Split => push_split(&mut rows, &hunk.rows, &syntax_spans),
            }
        }
        if let Some(upgrade) = upgrade {
            push_gap(
                &mut rows,
                upgrade,
                &file.hunks,
                file_ix,
                file.hunks.len(),
                mode,
            );
        }
    }

    (rows, file_rows, hunk_rows, tree)
}

fn push_unified(
    rows: &mut Vec<DisplayRow>,
    hunk_rows: &[DiffRow],
    syntax_spans: &[Vec<(Range<usize>, syntax::Token)>],
) {
    for (ix, row) in hunk_rows.iter().enumerate() {
        let syntax = syntax_spans.get(ix).cloned().unwrap_or_default();
        rows.push(match row {
            DiffRow::Context {
                old_no,
                new_no,
                text,
            } => DisplayRow::Line {
                old_no: Some(*old_no),
                new_no: Some(*new_no),
                kind: LineKind::Context,
                text: text.clone(),
                intra: Vec::new(),
                syntax,
            },
            DiffRow::Added {
                new_no,
                text,
                intra,
            } => DisplayRow::Line {
                old_no: None,
                new_no: Some(*new_no),
                kind: LineKind::Added,
                text: text.clone(),
                intra: intra.clone(),
                syntax,
            },
            DiffRow::Removed {
                old_no,
                text,
                intra,
            } => DisplayRow::Line {
                old_no: Some(*old_no),
                new_no: None,
                kind: LineKind::Removed,
                text: text.clone(),
                intra: intra.clone(),
                syntax,
            },
        });
    }
}

fn push_split(
    rows: &mut Vec<DisplayRow>,
    hunk_rows: &[DiffRow],
    syntax_spans: &[Vec<(Range<usize>, syntax::Token)>],
) {
    let mut i = 0;
    while i < hunk_rows.len() {
        match &hunk_rows[i] {
            DiffRow::Context {
                old_no,
                new_no,
                text,
            } => {
                let text = text.clone();
                let syntax = syntax_spans.get(i).cloned().unwrap_or_default();
                rows.push(DisplayRow::SplitLine {
                    left: Some(Cell {
                        no: *old_no,
                        kind: LineKind::Context,
                        text: text.clone(),
                        intra: Vec::new(),
                        syntax: syntax.clone(),
                    }),
                    right: Some(Cell {
                        no: *new_no,
                        kind: LineKind::Context,
                        text,
                        intra: Vec::new(),
                        syntax,
                    }),
                });
                i += 1;
            }
            DiffRow::Added {
                new_no,
                text,
                intra,
            } => {
                rows.push(DisplayRow::SplitLine {
                    left: None,
                    right: Some(Cell {
                        no: *new_no,
                        kind: LineKind::Added,
                        text: text.clone(),
                        intra: intra.clone(),
                        syntax: syntax_spans.get(i).cloned().unwrap_or_default(),
                    }),
                });
                i += 1;
            }
            DiffRow::Removed { .. } => {
                let start = i;
                while i < hunk_rows.len() && matches!(hunk_rows[i], DiffRow::Removed { .. }) {
                    i += 1;
                }
                let mid = i;
                while i < hunk_rows.len() && matches!(hunk_rows[i], DiffRow::Added { .. }) {
                    i += 1;
                }
                let removed = mid - start;
                let added = i - mid;
                let pairs = removed.max(added);
                for pair in 0..pairs {
                    let left = if pair < removed {
                        match &hunk_rows[start + pair] {
                            DiffRow::Removed {
                                old_no,
                                text,
                                intra,
                            } => Some(Cell {
                                no: *old_no,
                                kind: LineKind::Removed,
                                text: text.clone(),
                                intra: intra.clone(),
                                syntax: syntax_spans
                                    .get(start + pair)
                                    .cloned()
                                    .unwrap_or_default(),
                            }),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let right = if pair < added {
                        match &hunk_rows[mid + pair] {
                            DiffRow::Added {
                                new_no,
                                text,
                                intra,
                            } => Some(Cell {
                                no: *new_no,
                                kind: LineKind::Added,
                                text: text.clone(),
                                intra: intra.clone(),
                                syntax: syntax_spans.get(mid + pair).cloned().unwrap_or_default(),
                            }),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    rows.push(DisplayRow::SplitLine { left, right });
                }
            }
        }
    }
}

fn push_gap(
    rows: &mut Vec<DisplayRow>,
    upgrade: &FileUpgrade,
    hunks: &[diff_core::Hunk],
    file_ix: usize,
    gap_ix: usize,
    mode: ViewMode,
) {
    let hidden = diff_core::expand_gap_lines(&upgrade.new_lines, hunks, gap_ix);
    if hidden.is_empty() {
        return;
    }
    if !upgrade.expanded.contains(&gap_ix) {
        rows.push(DisplayRow::Gap {
            file_ix,
            gap_ix,
            hidden: hidden.len() as u32,
        });
        return;
    }
    for (old_no, new_no, text) in hidden {
        let syntax = if text.len() > MAX_SYNTAX_LINE_BYTES {
            Vec::new()
        } else {
            upgrade
                .new_spans
                .get((new_no - 1) as usize)
                .cloned()
                .unwrap_or_default()
        };
        rows.push(match mode {
            ViewMode::Unified => DisplayRow::Line {
                old_no: Some(old_no),
                new_no: Some(new_no),
                kind: LineKind::Context,
                text: text.clone(),
                intra: Vec::new(),
                syntax: syntax.clone(),
            },
            ViewMode::Split => DisplayRow::SplitLine {
                left: Some(Cell {
                    no: old_no,
                    kind: LineKind::Context,
                    text: text.clone(),
                    intra: Vec::new(),
                    syntax: syntax.clone(),
                }),
                right: Some(Cell {
                    no: new_no,
                    kind: LineKind::Context,
                    text,
                    intra: Vec::new(),
                    syntax,
                }),
            },
        });
    }
}

fn upgrade_row_spans(
    upgrade: &FileUpgrade,
    row: &DiffRow,
) -> Vec<(Range<usize>, syntax::Token)> {
    let (table, no, text) = match row {
        DiffRow::Context { new_no, text, .. } | DiffRow::Added { new_no, text, .. } => {
            (&upgrade.new_spans, *new_no, text)
        }
        DiffRow::Removed { old_no, text, .. } => (&upgrade.old_spans, *old_no, text),
    };
    if text.len() > MAX_SYNTAX_LINE_BYTES {
        return Vec::new();
    }
    table.get(no as usize - 1).cloned().unwrap_or_default()
}

fn hunk_syntax(
    lang: Option<&'static syntax::Language>,
    rows: &[DiffRow],
) -> Vec<Vec<(Range<usize>, syntax::Token)>> {
    let Some(lang) = lang else {
        return vec![Vec::new(); rows.len()];
    };
    let mut old_source = String::new();
    let mut new_source = String::new();
    let mut side_lines = Vec::with_capacity(rows.len());
    let (mut old_line, mut new_line) = (0usize, 0usize);
    for row in rows {
        match row {
            DiffRow::Context { text, .. } => {
                old_source.push_str(text);
                old_source.push('\n');
                old_line += 1;
                new_source.push_str(text);
                new_source.push('\n');
                side_lines.push((false, new_line));
                new_line += 1;
            }
            DiffRow::Added { text, .. } => {
                new_source.push_str(text);
                new_source.push('\n');
                side_lines.push((false, new_line));
                new_line += 1;
            }
            DiffRow::Removed { text, .. } => {
                old_source.push_str(text);
                old_source.push('\n');
                side_lines.push((true, old_line));
                old_line += 1;
            }
        }
    }
    let highlight = |source: &str| {
        if source.is_empty() || source.len() > MAX_HUNK_SOURCE_BYTES {
            Vec::new()
        } else {
            syntax::highlight_lines(lang, source)
        }
    };
    let old_spans = highlight(&old_source);
    let new_spans = highlight(&new_source);
    rows.iter()
        .zip(side_lines)
        .map(|(row, (from_old, line))| {
            let text = match row {
                DiffRow::Context { text, .. }
                | DiffRow::Added { text, .. }
                | DiffRow::Removed { text, .. } => text,
            };
            if text.len() > MAX_SYNTAX_LINE_BYTES {
                return Vec::new();
            }
            let side = if from_old { &old_spans } else { &new_spans };
            side.get(line).cloned().unwrap_or_default()
        })
        .collect()
}

/// Index of the file whose header is at or above `cursor`.
pub fn file_index_at(file_rows: &[usize], cursor: usize) -> Option<usize> {
    file_rows.iter().rposition(|&row| row <= cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diff_core::{parse_patch, DiffRow};

    #[test]
    fn empty_diff_is_empty() {
        let (rows, files, hunks, tree) = build_rows(&parse_patch(""), ViewMode::Unified);
        assert!(rows.is_empty());
        assert!(files.is_empty());
        assert!(hunks.is_empty());
        assert!(tree.is_empty());
    }

    #[test]
    fn binary_file_gets_a_placeholder_row() {
        let patch = "\
diff --git a/logo.png b/logo.png
index 1..2 100644
Binary files a/logo.png and b/logo.png differ
";
        let (rows, files, hunks, tree) = build_rows(&parse_patch(patch), ViewMode::Unified);
        assert_eq!(files, vec![0]);
        assert!(hunks.is_empty());
        assert!(matches!(rows[0], DisplayRow::FileHeader { status: FileStatus::Binary, .. }));
        assert!(matches!(rows[1], DisplayRow::Binary));
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].path, "logo.png");
    }

    #[test]
    fn unified_order_is_header_hunk_then_lines() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
 context
-old
+new
";
        let (rows, files, hunks, _) = build_rows(&parse_patch(patch), ViewMode::Unified);
        assert_eq!(files, vec![0]);
        assert_eq!(hunks, vec![1]);
        assert!(matches!(rows[0], DisplayRow::FileHeader { .. }));
        assert!(matches!(rows[1], DisplayRow::HunkHeader { .. }));
        match &rows[2] {
            DisplayRow::Line {
                kind: LineKind::Context,
                text,
                ..
            } => assert_eq!(text, "context"),
            other => panic!("{other:?}"),
        }
        match &rows[3] {
            DisplayRow::Line {
                kind: LineKind::Removed,
                old_no,
                new_no,
                ..
            } => {
                assert_eq!(*old_no, Some(2));
                assert_eq!(*new_no, None);
            }
            other => panic!("{other:?}"),
        }
        match &rows[4] {
            DisplayRow::Line {
                kind: LineKind::Added,
                old_no,
                new_no,
                ..
            } => {
                assert_eq!(*old_no, None);
                assert_eq!(*new_no, Some(2));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn split_pairs_removed_then_added() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
 context
-old
+new
";
        let (rows, _, _, _) = build_rows(&parse_patch(patch), ViewMode::Split);
        match &rows[2] {
            DisplayRow::SplitLine {
                left: Some(left),
                right: Some(right),
            } => {
                assert_eq!(left.kind, LineKind::Context);
                assert_eq!(right.kind, LineKind::Context);
                assert_eq!(left.text, "context");
            }
            other => panic!("{other:?}"),
        }
        match &rows[3] {
            DisplayRow::SplitLine {
                left: Some(left),
                right: Some(right),
            } => {
                assert_eq!(left.kind, LineKind::Removed);
                assert_eq!(left.text, "old");
                assert_eq!(right.kind, LineKind::Added);
                assert_eq!(right.text, "new");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn split_unequal_run_leaves_a_void_cell() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,2 @@
-old
+new
+extra
";
        let (rows, _, _, _) = build_rows(&parse_patch(patch), ViewMode::Split);
        match &rows[2] {
            DisplayRow::SplitLine {
                left: Some(left),
                right: Some(right),
            } => {
                assert_eq!(left.text, "old");
                assert_eq!(right.text, "new");
            }
            other => panic!("{other:?}"),
        }
        match &rows[3] {
            DisplayRow::SplitLine {
                left: None,
                right: Some(right),
            } => assert_eq!(right.text, "extra"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn file_index_at_tracks_the_header_above_cursor() {
        assert_eq!(file_index_at(&[0, 10, 20], 15), Some(1));
        assert_eq!(file_index_at(&[0, 10, 20], 0), Some(0));
        assert_eq!(file_index_at(&[0, 10, 20], 25), Some(2));
        assert_eq!(file_index_at(&[], 0), None);
    }

    #[test]
    fn upgraded_file_shows_a_gap_then_expands_to_hidden_lines() {
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 10\n", "line 10 changed\n");
        let hunks = diff_core::diff_texts(&old, &new, 3);
        let mut file = diff_core::FileDiff {
            old_path: Some("a.rs".into()),
            new_path: Some("a.rs".into()),
            status: FileStatus::Modified,
            hunks,
            additions: 1,
            deletions: 1,
        };
        file.additions = file
            .hunks
            .iter()
            .flat_map(|h| &h.rows)
            .filter(|r| matches!(r, DiffRow::Added { .. }))
            .count() as u32;
        file.deletions = file
            .hunks
            .iter()
            .flat_map(|h| &h.rows)
            .filter(|r| matches!(r, DiffRow::Removed { .. }))
            .count() as u32;
        let parsed = PatchDiff {
            files: vec![file],
        };
        let upgrade = FileUpgrade {
            new_lines: new.lines().map(str::to_string).collect(),
            old_spans: Vec::new(),
            new_spans: Vec::new(),
            expanded: std::collections::HashSet::new(),
        };
        let mut upgrades = std::collections::HashMap::new();
        upgrades.insert(0, upgrade);
        let (rows, _, _, _) = build_rows_with(&parsed, ViewMode::Unified, &upgrades);
        assert!(
            rows.iter()
                .any(|r| matches!(r, DisplayRow::Gap { hidden: 6, .. })),
            "{rows:?}"
        );
        upgrades.get_mut(&0).unwrap().expanded.insert(0);
        let (rows, _, _, _) = build_rows_with(&parsed, ViewMode::Unified, &upgrades);
        assert!(!rows.iter().any(|r| matches!(r, DisplayRow::Gap { hidden: 6, .. })));
        let texts: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                DisplayRow::Line { text, kind: LineKind::Context, old_no: Some(1), .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(texts.first().copied(), Some("line 1"));
    }

    #[test]
    fn rust_hunk_gets_keyword_syntax_spans() {
        let patch = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-fn old() {}
+fn new() {}
";
        let (rows, _, _, _) = build_rows(&parse_patch(patch), ViewMode::Unified);
        let added = rows.iter().find_map(|row| match row {
            DisplayRow::Line {
                kind: LineKind::Added,
                syntax,
                text,
                ..
            } => Some((text.as_str(), syntax)),
            _ => None,
        });
        let (text, syntax) = added.expect("added line");
        assert_eq!(text, "fn new() {}");
        assert!(
            syntax.iter().any(|(r, t)| *t == syntax::Token::Keyword && &text[r.clone()] == "fn"),
            "{syntax:?}"
        );
    }
}
