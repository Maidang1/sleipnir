//! Layout of a [`Widget`] tree onto a cell grid.
//!
//! Given a tree and an available width in columns, this produces positioned
//! items with integer cell rects and an exact total height in rows. ADR-0018
//! uses that height as a Block row count, so it must be deterministic: the
//! same tree and width always yield the same layout.
//!
//! Over-budget trees are truncated with a visible marker (ADR-0017 constraint
//! 5) — never rejected silently, never laid out in full. Attribution is
//! appended in a reserved band the tree cannot occupy.

use crate::cells::{
    ATTRIBUTION_ROWS, BAR_COLS, CHIP_PAD, MAX_CODE_LINES, MAX_LEAF_CHARS, UNKNOWN_COLS,
    attribution_label, bar_filled, cell_cols, fit_cols, spark_levels, take_chars, wrap_text,
};
use crate::geom::{CellPos, CellRect};
use plugin_protocol::v2::{MAX_WIDGET_DEPTH, MAX_WIDGET_NODES, Tone, TreeStats, Widget, measure};

/// A laid-out widget surface: the plugin tree plus the renderer-owned
/// attribution band beneath it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Widget tree, positioned from (0, 0). Never overlaps [`Self::attribution`].
    pub root: LaidOut,
    /// Renderer-drawn plugin marker. The tree cannot produce this node.
    pub attribution: LaidOut,
    /// Surface width in columns (the available width, at least 1).
    pub width: u32,
    /// `root.height + attribution.height`. The Block row count for ADR-0018.
    pub height: u32,
    /// [`measure`] of the *input* tree, before truncation.
    pub stats: TreeStats,
    /// True when the node/depth budget cut the tree.
    pub truncated: bool,
}

impl Layout {
    /// Height of the plugin tree, excluding the attribution band.
    pub fn content_height(&self) -> u32 {
        self.root.rect.height
    }

    /// Depth-first walk of the plugin tree (attribution is not included).
    pub fn walk(&self) -> Walk<'_> {
        Walk {
            stack: vec![&self.root],
        }
    }
}

/// Pre-order iterator over laid-out nodes.
pub struct Walk<'a> {
    stack: Vec<&'a LaidOut>,
}

impl<'a> Iterator for Walk<'a> {
    type Item = &'a LaidOut;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        for child in node.children.iter().rev() {
            self.stack.push(child);
        }
        Some(node)
    }
}

/// One positioned node. Containers keep their children; leaves have none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidOut {
    pub rect: CellRect,
    pub kind: LaidOutKind,
    pub children: Vec<LaidOut>,
}

/// What to paint in a [`LaidOut`] rect. Mount points map tones to
/// `ChromeTokens`; this crate stops at the schema-level description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaidOutKind {
    Col,
    Row,
    Text {
        lines: Vec<String>,
        tone: Tone,
        bold: bool,
    },
    Code {
        lines: Vec<CodeLine>,
    },
    Badge {
        text: String,
        tone: Tone,
    },
    Bar {
        filled: u32,
        width: u32,
    },
    Spark {
        levels: Vec<u8>,
    },
    Sep,
    Btn {
        text: String,
        action: String,
        arg: Option<String>,
    },
    /// Inert placeholder for an unknown `t`. Never zero-size.
    Unknown,
    /// Visible cut mark. Inserted by the renderer when the budget is hit.
    Truncated,
    /// Renderer-owned attribution. Not a [`Widget`] variant; a crafted tree
    /// cannot emit this kind.
    Attribution {
        plugin_id: String,
        label: String,
    },
}

/// One display line of a `code` node. Lines are never wrapped: overflow is
/// truncated and marked, because wrapped code is unreadable (line-based
/// syntax and diffs break).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeLine {
    pub text: String,
    pub truncated: bool,
}

struct Budget {
    nodes_left: usize,
    truncated: bool,
}

impl Budget {
    fn new() -> Self {
        Self {
            nodes_left: MAX_WIDGET_NODES,
            truncated: false,
        }
    }

    fn enter(&mut self, depth_left: usize) -> bool {
        if self.nodes_left == 0 || depth_left == 0 {
            self.truncated = true;
            false
        } else {
            self.nodes_left -= 1;
            true
        }
    }
}

/// Lay out `tree` into a cell grid `available_cols` wide.
///
/// `plugin_id` is the host's name for the plugin that produced the tree. It is
/// drawn by this function — not by the plugin — in a reserved band the tree
/// cannot occupy (ADR-0017 attribution).
///
/// A zero available width is treated as one column: a surface with no cells
/// cannot be shown, and leaves must stay visible.
pub fn layout(tree: &Widget, available_cols: u16, plugin_id: &str) -> Layout {
    let width = u32::from(available_cols).max(1);
    let stats = measure(tree);
    let mut budget = Budget::new();
    let root = layout_node(tree, CellPos::ORIGIN, width, MAX_WIDGET_DEPTH, &mut budget);
    let (plugin_id, label) = attribution_label(plugin_id, width);
    let attribution = LaidOut {
        rect: CellRect::new(0, root.rect.height, width, ATTRIBUTION_ROWS),
        kind: LaidOutKind::Attribution { plugin_id, label },
        children: Vec::new(),
    };
    let height = root.rect.height.saturating_add(attribution.rect.height);
    Layout {
        root,
        attribution,
        width,
        height,
        stats,
        truncated: budget.truncated,
    }
}

fn layout_node(
    widget: &Widget,
    origin: CellPos,
    avail: u32,
    depth_left: usize,
    budget: &mut Budget,
) -> LaidOut {
    let avail = avail.max(1);
    if !budget.enter(depth_left) {
        return truncated_marker(origin, avail);
    }
    match widget {
        Widget::Col { gap, children } => {
            layout_col(*gap, children, origin, avail, depth_left, budget)
        }
        Widget::Row { gap, children } => {
            layout_row(*gap, children, origin, avail, depth_left, budget)
        }
        Widget::Text { s, fg, bold } => layout_text(s, *fg, *bold, origin, avail),
        Widget::Code { s, .. } => layout_code(s, origin, avail),
        Widget::Badge { s, tone } => layout_badge(s, *tone, origin, avail),
        Widget::Bar { v } => layout_bar(*v, origin, avail),
        Widget::Spark { vs } => layout_spark(vs, origin, avail),
        Widget::Sep => layout_sep(origin, avail),
        Widget::Btn { s, action, arg } => layout_btn(s, action, arg.as_deref(), origin, avail),
        Widget::Unknown => layout_unknown(origin, avail),
    }
}

fn truncated_marker(origin: CellPos, avail: u32) -> LaidOut {
    LaidOut {
        rect: CellRect::at(origin, avail.max(1), 1),
        kind: LaidOutKind::Truncated,
        children: Vec::new(),
    }
}

fn layout_col(
    gap: u16,
    children: &[Widget],
    origin: CellPos,
    avail: u32,
    depth_left: usize,
    budget: &mut Budget,
) -> LaidOut {
    let gap = u32::from(gap);
    let child_depth = depth_left.saturating_sub(1);
    let mut kids = Vec::new();
    let mut y = origin.row;
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            y = y.saturating_add(gap);
        }
        let kid = layout_node(
            child,
            CellPos::new(origin.col, y),
            avail,
            child_depth,
            budget,
        );
        let cut = matches!(kid.kind, LaidOutKind::Truncated);
        if !cut {
            y = kid.rect.bottom();
        }
        kids.push(kid);
        if cut {
            break;
        }
    }
    let height = y.saturating_sub(origin.row);
    LaidOut {
        rect: CellRect::at(origin, avail, height),
        kind: LaidOutKind::Col,
        children: kids,
    }
}

fn layout_row(
    gap: u16,
    children: &[Widget],
    origin: CellPos,
    avail: u32,
    depth_left: usize,
    budget: &mut Budget,
) -> LaidOut {
    let gap = u32::from(gap);
    let child_depth = depth_left.saturating_sub(1);
    let widths = allocate_row_widths(children, avail, gap);
    let dropped = widths.len() < children.len();
    let mut kids = Vec::new();
    let mut x = origin.col;
    let mut height = 0u32;
    for (i, child) in children.iter().enumerate() {
        if i >= widths.len() {
            break;
        }
        if i > 0 {
            x = x.saturating_add(gap);
        }
        let kid = layout_node(
            child,
            CellPos::new(x, origin.row),
            widths[i].max(1),
            child_depth,
            budget,
        );
        if matches!(kid.kind, LaidOutKind::Truncated) {
            push_truncated(&mut kids, origin, x, avail, budget);
            break;
        }
        x = kid.rect.right();
        height = height.max(kid.rect.height);
        kids.push(kid);
    }
    if dropped && !matches!(kids.last().map(|k| &k.kind), Some(LaidOutKind::Truncated)) {
        push_truncated(&mut kids, origin, x, avail, budget);
    }
    for kid in &mut kids {
        if matches!(kid.kind, LaidOutKind::Sep) {
            kid.rect.height = height.max(1);
        }
    }
    if height == 0 && !kids.is_empty() {
        height = 1;
    }
    LaidOut {
        rect: CellRect::at(origin, avail, height),
        kind: LaidOutKind::Row,
        children: kids,
    }
}

fn push_truncated(
    kids: &mut Vec<LaidOut>,
    origin: CellPos,
    x: u32,
    avail: u32,
    budget: &mut Budget,
) {
    budget.truncated = true;
    let remain = avail.saturating_sub(x.saturating_sub(origin.col));
    if remain > 0 {
        kids.push(truncated_marker(CellPos::new(x, origin.row), remain));
    }
}

fn is_flex(widget: &Widget) -> bool {
    matches!(
        widget,
        Widget::Col { .. } | Widget::Row { .. } | Widget::Text { .. } | Widget::Code { .. }
    )
}

fn intrinsic_width(widget: &Widget, avail: u32) -> u32 {
    let avail = avail.max(1);
    match widget {
        Widget::Badge { s, .. } | Widget::Btn { s, .. } => chip_width(s, avail),
        Widget::Sep => 1,
        Widget::Spark { vs } => (vs.len() as u32).clamp(1, avail),
        Widget::Bar { .. } => BAR_COLS.min(avail).max(1),
        Widget::Unknown => UNKNOWN_COLS.min(avail).max(1),
        Widget::Text { s, .. } => cell_cols(s).clamp(1, avail),
        Widget::Code { s, .. } => s
            .split('\n')
            .map(cell_cols)
            .max()
            .unwrap_or(1)
            .clamp(1, avail),
        Widget::Col { .. } | Widget::Row { .. } => avail,
    }
}

fn chip_width(s: &str, avail: u32) -> u32 {
    let inner = cell_cols(s);
    let w = inner.saturating_add(CHIP_PAD.saturating_mul(2));
    w.clamp(1, avail.max(1))
}

fn allocate_row_widths(children: &[Widget], avail: u32, gap: u32) -> Vec<u32> {
    let n = children.len();
    if n == 0 {
        return Vec::new();
    }
    let mut widths: Vec<u32> = children
        .iter()
        .map(|c| {
            if is_flex(c) {
                1
            } else {
                intrinsic_width(c, avail).max(1)
            }
        })
        .collect();

    fit_widths(&mut widths, avail, gap);

    let inner = avail.saturating_sub(gap.saturating_mul((widths.len().saturating_sub(1)) as u32));
    let used: u32 = widths.iter().copied().fold(0, u32::saturating_add);
    let extra = inner.saturating_sub(used);
    if extra > 0 {
        let flex: Vec<usize> = children
            .iter()
            .enumerate()
            .take(widths.len())
            .filter(|(_, c)| is_flex(c))
            .map(|(i, _)| i)
            .collect();
        if !flex.is_empty() {
            let each = extra / flex.len() as u32;
            let rem = extra % flex.len() as u32;
            for (k, &i) in flex.iter().enumerate() {
                widths[i] = widths[i].saturating_add(each);
                if (k as u32) < rem {
                    widths[i] = widths[i].saturating_add(1);
                }
            }
        }
    }
    widths
}

fn fit_widths(widths: &mut Vec<u32>, avail: u32, gap: u32) {
    loop {
        let n = widths.len();
        if n == 0 {
            return;
        }
        let gaps = gap.saturating_mul((n - 1) as u32);
        let sum = widths.iter().copied().fold(0, u32::saturating_add);
        let mut excess = gaps.saturating_add(sum).saturating_sub(avail);
        if excess == 0 {
            return;
        }
        // Shrink from the right, taking as much as possible from each column
        // before moving left, so the order matches one-cell-at-a-time cuts.
        for w in widths.iter_mut().rev() {
            if excess == 0 {
                break;
            }
            let cut = w.saturating_sub(1).min(excess);
            *w -= cut;
            excess -= cut;
        }
        if excess == 0 {
            return;
        }
        if n > 1 {
            widths.pop();
            continue;
        }
        widths[0] = avail.max(1);
        return;
    }
}

fn layout_text(s: &str, fg: Tone, bold: bool, origin: CellPos, avail: u32) -> LaidOut {
    let lines = wrap_text(s, avail);
    let height = (lines.len() as u32).max(1);
    LaidOut {
        rect: CellRect::at(origin, avail, height),
        kind: LaidOutKind::Text {
            lines,
            tone: fg,
            bold,
        },
        children: Vec::new(),
    }
}

fn layout_code(s: &str, origin: CellPos, avail: u32) -> LaidOut {
    // Code is never wrapped: wrapping destroys line-oriented syntax and makes
    // snippets unreadable. Overflow is truncated and marked instead.
    let (source, source_cut) = take_chars(s, MAX_LEAF_CHARS);
    let mut raw_lines: Vec<&str> = source.split('\n').collect();
    let line_cut = raw_lines.len() > MAX_CODE_LINES;
    if line_cut {
        raw_lines.truncate(MAX_CODE_LINES);
    }
    if raw_lines.is_empty() {
        raw_lines.push("");
    }

    let mut lines = Vec::with_capacity(raw_lines.len());
    for (i, raw) in raw_lines.iter().enumerate() {
        let (text, width_cut) = fit_cols(raw, avail);
        let truncated = width_cut
            || (line_cut && i + 1 == raw_lines.len())
            || (source_cut && i + 1 == raw_lines.len());
        lines.push(CodeLine { text, truncated });
    }
    let height = (lines.len() as u32).max(1);
    LaidOut {
        rect: CellRect::at(origin, avail, height),
        kind: LaidOutKind::Code { lines },
        children: Vec::new(),
    }
}

fn layout_badge(s: &str, tone: Tone, origin: CellPos, avail: u32) -> LaidOut {
    let width = chip_width(s, avail);
    let (text, _) = fit_cols(s, width.saturating_sub(CHIP_PAD.saturating_mul(2)).max(1));
    LaidOut {
        rect: CellRect::at(origin, width, 1),
        kind: LaidOutKind::Badge { text, tone },
        children: Vec::new(),
    }
}

fn layout_bar(v: f32, origin: CellPos, avail: u32) -> LaidOut {
    let width = BAR_COLS.min(avail).max(1);
    let filled = bar_filled(v, width);
    LaidOut {
        rect: CellRect::at(origin, width, 1),
        kind: LaidOutKind::Bar { filled, width },
        children: Vec::new(),
    }
}

fn layout_spark(vs: &[f32], origin: CellPos, avail: u32) -> LaidOut {
    let width = if vs.is_empty() {
        1
    } else {
        (vs.len() as u32).min(avail).max(1)
    };
    let levels = spark_levels(vs, width);
    LaidOut {
        rect: CellRect::at(origin, width, 1),
        kind: LaidOutKind::Spark { levels },
        children: Vec::new(),
    }
}

fn layout_sep(origin: CellPos, avail: u32) -> LaidOut {
    // In a column this is a full-width rule; in a row, [`layout_row`] stretches
    // the height to the row after siblings are measured.
    LaidOut {
        rect: CellRect::at(origin, avail.max(1), 1),
        kind: LaidOutKind::Sep,
        children: Vec::new(),
    }
}

fn layout_btn(s: &str, action: &str, arg: Option<&str>, origin: CellPos, avail: u32) -> LaidOut {
    let width = chip_width(s, avail);
    let (text, _) = fit_cols(s, width.saturating_sub(CHIP_PAD.saturating_mul(2)).max(1));
    LaidOut {
        rect: CellRect::at(origin, width, 1),
        kind: LaidOutKind::Btn {
            text,
            action: action.to_string(),
            arg: arg.map(str::to_string),
        },
        children: Vec::new(),
    }
}

fn layout_unknown(origin: CellPos, avail: u32) -> LaidOut {
    let width = UNKNOWN_COLS.min(avail).max(1);
    LaidOut {
        rect: CellRect::at(origin, width, 1),
        kind: LaidOutKind::Unknown,
        children: Vec::new(),
    }
}
