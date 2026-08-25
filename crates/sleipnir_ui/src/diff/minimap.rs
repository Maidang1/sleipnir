//! Reduce display rows to coalesced minimap runs. Paint is a thin consumer.

use super::rows::{DisplayRow, LineKind};

const MAX_MINIMAP_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapRow {
    pub kind: MinimapKind,
    pub len_frac: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MinimapKind {
    Context,
    Added,
    Removed,
    SplitPair {
        left_change: bool,
        right_change: bool,
        left_frac: f32,
        right_frac: f32,
    },
    Header,
    Gap,
    Blank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapLane {
    Full,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapColor {
    Added,
    Removed,
    Context,
    Header,
    Gap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MinimapRun {
    pub start: usize,
    pub end: usize,
    pub lane: MinimapLane,
    pub frac: f32,
    pub color: MinimapColor,
    pub tick: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MinimapLayout {
    pub slot_h: f32,
    pub group: usize,
    pub runs: Vec<MinimapRun>,
}

fn line_frac(text: &str) -> f32 {
    text.chars().take(MAX_MINIMAP_CHARS).count() as f32 / MAX_MINIMAP_CHARS as f32
}

/// One minimap row per display row, index-aligned.
pub fn minimap_rows(rows: &[DisplayRow]) -> Vec<MinimapRow> {
    rows.iter()
        .map(|row| match row {
            DisplayRow::Line { kind, text, .. } => MinimapRow {
                kind: match kind {
                    LineKind::Context => MinimapKind::Context,
                    LineKind::Added => MinimapKind::Added,
                    LineKind::Removed => MinimapKind::Removed,
                },
                len_frac: line_frac(text),
            },
            DisplayRow::SplitLine { left, right } => {
                let left_frac = left.as_ref().map(|c| line_frac(&c.text)).unwrap_or(0.0);
                let right_frac = right.as_ref().map(|c| line_frac(&c.text)).unwrap_or(0.0);
                let left_change = left.as_ref().is_some_and(|c| c.kind == LineKind::Removed);
                let right_change = right.as_ref().is_some_and(|c| c.kind == LineKind::Added);
                MinimapRow {
                    kind: MinimapKind::SplitPair {
                        left_change,
                        right_change,
                        left_frac,
                        right_frac,
                    },
                    len_frac: left_frac.max(right_frac),
                }
            }
            DisplayRow::FileHeader { .. } | DisplayRow::HunkHeader { .. } => MinimapRow {
                kind: MinimapKind::Header,
                len_frac: 1.0,
            },
            DisplayRow::Gap { .. } => MinimapRow {
                kind: MinimapKind::Gap,
                len_frac: 0.4,
            },
            DisplayRow::Binary => MinimapRow {
                kind: MinimapKind::Blank,
                len_frac: 0.0,
            },
        })
        .collect()
}

fn scale(total: usize, pane_px: f32) -> (f32, usize) {
    if total == 0 || pane_px <= 0.0 {
        return (1.0, 1);
    }
    if total as f32 <= pane_px {
        return (pane_px / total as f32, 1);
    }
    let group = ((total as f32 / pane_px).ceil() as usize).max(1);
    (1.0, group)
}

fn color_of(kind: MinimapKind) -> Option<MinimapColor> {
    match kind {
        MinimapKind::Added => Some(MinimapColor::Added),
        MinimapKind::Removed => Some(MinimapColor::Removed),
        MinimapKind::Context => Some(MinimapColor::Context),
        MinimapKind::Header => Some(MinimapColor::Header),
        MinimapKind::Gap => Some(MinimapColor::Gap),
        MinimapKind::Blank => None,
        MinimapKind::SplitPair { .. } => None,
    }
}

/// Downsample and coalesce neighboring same-color slots.
pub fn minimap_runs(rows: &[MinimapRow], pane_px: f32) -> MinimapLayout {
    let (slot_h, group) = scale(rows.len(), pane_px);
    let mut runs: Vec<MinimapRun> = Vec::new();
    let push = |runs: &mut Vec<MinimapRun>, run: MinimapRun| {
        if run.frac <= 0.0 {
            return;
        }
        if let Some(prev) = runs.iter_mut().rev().find(|prev| prev.lane == run.lane) {
            if prev.color == run.color && !prev.tick && !run.tick && prev.end == run.start {
                prev.end = run.end;
                prev.frac = prev.frac.max(run.frac);
                return;
            }
        }
        runs.push(run);
    };

    let slots = if rows.is_empty() {
        0
    } else {
        rows.len().div_ceil(group)
    };
    for slot in 0..slots {
        let start = slot * group;
        let end = ((slot + 1) * group).min(rows.len());
        let slice = &rows[start..end];
        let has_split = slice
            .iter()
            .any(|r| matches!(r.kind, MinimapKind::SplitPair { .. }));
        if has_split {
            let mut left_change = false;
            let mut right_change = false;
            let mut left_frac = 0.0f32;
            let mut right_frac = 0.0f32;
            let mut fallback: Option<MinimapColor> = None;
            for row in slice {
                match row.kind {
                    MinimapKind::SplitPair {
                        left_change: lc,
                        right_change: rc,
                        left_frac: lf,
                        right_frac: rf,
                    } => {
                        left_change |= lc;
                        right_change |= rc;
                        left_frac = left_frac.max(lf);
                        right_frac = right_frac.max(rf);
                    }
                    other => {
                        if fallback.is_none() {
                            fallback = color_of(other);
                        }
                    }
                }
            }
            let left_color = if left_change {
                MinimapColor::Removed
            } else {
                fallback.unwrap_or(MinimapColor::Context)
            };
            let right_color = if right_change {
                MinimapColor::Added
            } else {
                fallback.unwrap_or(MinimapColor::Context)
            };
            push(
                &mut runs,
                MinimapRun {
                    start: slot,
                    end: slot + 1,
                    lane: MinimapLane::Left,
                    frac: left_frac,
                    color: left_color,
                    tick: false,
                },
            );
            push(
                &mut runs,
                MinimapRun {
                    start: slot,
                    end: slot + 1,
                    lane: MinimapLane::Right,
                    frac: right_frac,
                    color: right_color,
                    tick: false,
                },
            );
        } else {
            let mut best: Option<(MinimapColor, f32)> = None;
            for row in slice {
                if let Some(color) = color_of(row.kind) {
                    let score = match color {
                        MinimapColor::Added | MinimapColor::Removed => 3,
                        MinimapColor::Header => 2,
                        MinimapColor::Gap => 1,
                        MinimapColor::Context => 0,
                    };
                    let take = match best {
                        None => true,
                        Some((prev, _)) => {
                            let prev_score = match prev {
                                MinimapColor::Added | MinimapColor::Removed => 3,
                                MinimapColor::Header => 2,
                                MinimapColor::Gap => 1,
                                MinimapColor::Context => 0,
                            };
                            score > prev_score
                        }
                    };
                    if take {
                        best = Some((color, row.len_frac));
                    }
                }
            }
            if let Some((color, frac)) = best {
                push(
                    &mut runs,
                    MinimapRun {
                        start: slot,
                        end: slot + 1,
                        lane: MinimapLane::Full,
                        frac,
                        color,
                        tick: matches!(color, MinimapColor::Header | MinimapColor::Gap)
                            && slot_h > 2.0,
                    },
                );
            }
        }
    }
    MinimapLayout {
        slot_h,
        group,
        runs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::rows::{Cell, LineKind};

    fn line(kind: LineKind, text: &str) -> DisplayRow {
        DisplayRow::Line {
            old_no: Some(1),
            new_no: Some(1),
            kind,
            text: text.into(),
            intra: Vec::new(),
            syntax: Vec::new(),
        }
    }

    #[test]
    fn reduce_distinguishes_kinds_and_empty_line_has_no_bar() {
        let rows = vec![
            DisplayRow::FileHeader {
                path: "a.rs".into(),
                status: diff_core::FileStatus::Modified,
                additions: 1,
                deletions: 1,
            },
            line(LineKind::Context, "keep"),
            line(LineKind::Removed, "old"),
            line(LineKind::Added, "new"),
            line(LineKind::Context, ""),
            DisplayRow::Gap {
                file_ix: 0,
                gap_ix: 0,
                hidden: 4,
            },
        ];
        let mini = minimap_rows(&rows);
        assert!(matches!(mini[0].kind, MinimapKind::Header));
        assert!(matches!(mini[1].kind, MinimapKind::Context));
        assert!(matches!(mini[2].kind, MinimapKind::Removed));
        assert!(matches!(mini[3].kind, MinimapKind::Added));
        assert_eq!(mini[4].len_frac, 0.0);
        assert!(matches!(mini[5].kind, MinimapKind::Gap));
        let layout = minimap_runs(&mini, 200.0);
        assert!(
            !layout.runs.iter().any(|r| r.start == 4 && r.frac > 0.0),
            "empty line must not paint a bar: {:?}",
            layout.runs
        );
        let coalesced = layout
            .runs
            .iter()
            .filter(|r| r.color == MinimapColor::Header || r.color == MinimapColor::Removed)
            .count();
        assert!(coalesced >= 2);
    }

    #[test]
    fn split_rows_keep_two_lanes_and_neighbors_merge() {
        let cell = |kind, text: &str| {
            Some(Cell {
                no: 1,
                kind,
                text: text.into(),
                intra: Vec::new(),
                syntax: Vec::new(),
            })
        };
        let rows = vec![
            DisplayRow::SplitLine {
                left: cell(LineKind::Removed, "aaaa"),
                right: cell(LineKind::Added, "bbbb"),
            },
            DisplayRow::SplitLine {
                left: cell(LineKind::Removed, "cccc"),
                right: cell(LineKind::Added, "dddd"),
            },
        ];
        let mini = minimap_rows(&rows);
        assert!(matches!(
            mini[0].kind,
            MinimapKind::SplitPair {
                left_change: true,
                right_change: true,
                ..
            }
        ));
        let layout = minimap_runs(&mini, 200.0);
        let left = layout
            .runs
            .iter()
            .find(|r| r.lane == MinimapLane::Left)
            .unwrap();
        let right = layout
            .runs
            .iter()
            .find(|r| r.lane == MinimapLane::Right)
            .unwrap();
        assert_eq!(left.color, MinimapColor::Removed);
        assert_eq!(right.color, MinimapColor::Added);
        assert_eq!(left.start, 0);
        assert_eq!(left.end, 2, "adjacent same-color slots coalesce: {left:?}");
    }
}
