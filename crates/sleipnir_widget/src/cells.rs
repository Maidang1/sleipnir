//! Cell occupancy, wrapping, and finite-value conversion.
//!
//! Layout math stays in integers. `f32` appears only as schema input (`Bar`,
//! `Spark`) and is reduced to a cell count before anything is positioned.

/// Cap on characters consumed from a single `text` / `code` node.
///
/// Node/depth budgets (ADR-0017 constraint 5) do not bound string size. One
/// unbounded leaf would still freeze the UI thread on wrap.
pub const MAX_LEAF_CHARS: usize = 8192;

/// Cap on `code` lines after the character cap. Wrapped code is forbidden;
/// an enormous file still cannot become an enormous Block.
pub const MAX_CODE_LINES: usize = 256;

/// Visible columns reserved for a `bar` track. Fixed so leftover width does
/// not change a Block's row count.
pub const BAR_COLS: u32 = 10;

/// Padding cells on each side of a badge or button label.
pub const CHIP_PAD: u32 = 1;

/// Placeholder footprint for [`plugin_protocol::v2::Widget::Unknown`]. Never
/// zero: the user must be able to see that something did not render.
pub const UNKNOWN_COLS: u32 = 3;

/// Rows reserved for the renderer-owned plugin marker (ADR-0017 attribution).
pub const ATTRIBUTION_ROWS: u32 = 1;

/// Glyph used when a code line (or a capped string) is cut. One scalar, one
/// cell under the v1 occupancy rule below.
pub const ELLIPSIS: char = '…';

/// Columns occupied by `s` under the v1 rule: one Unicode scalar = one cell,
/// control characters occupy none (`\t` counts as one so tabs stay visible).
///
/// East-Asian width is a later refinement — adding it now would change row
/// counts under CJK without a protocol bump.
pub fn cell_cols(s: &str) -> u32 {
    s.chars().map(char_cols).fold(0, u32::saturating_add)
}

pub fn char_cols(c: char) -> u32 {
    match c {
        '\t' => 1,
        '\n' | '\r' => 0,
        c if c.is_control() => 0,
        _ => 1,
    }
}

/// Prefix of `s` with at most `max` scalars. `true` when anything was cut.
pub fn take_chars(s: &str, max: usize) -> (String, bool) {
    match s.char_indices().nth(max) {
        Some((i, _)) => (s[..i].to_string(), true),
        None => (s.to_string(), false),
    }
}

/// Wrap `s` to `width` columns. Hard `\n` breaks; otherwise greedy scalar wrap.
/// Empty input is one blank row so a `text` node is never zero-size.
pub fn wrap_text(s: &str, width: u32) -> Vec<String> {
    let width = width.max(1);
    let (mut src, cut) = take_chars(s, MAX_LEAF_CHARS);
    if cut {
        src.push(ELLIPSIS);
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0u32;
    for c in src.chars() {
        if c == '\n' {
            lines.push(std::mem::take(&mut current));
            used = 0;
            continue;
        }
        let w = char_cols(c);
        if w == 0 {
            current.push(c);
            continue;
        }
        if used.saturating_add(w) > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(c);
        used = used.saturating_add(w);
    }
    lines.push(current);
    lines
}

/// Fit `s` into `width` columns without wrapping. Overflow is truncated and
/// marked with [`ELLIPSIS`]. Used for `code` (wrapped code is unreadable).
pub fn fit_cols(s: &str, width: u32) -> (String, bool) {
    let width = width.max(1);
    if cell_cols(s) <= width {
        return (s.to_string(), false);
    }
    if width == 1 {
        return (ELLIPSIS.to_string(), true);
    }
    let budget = width - 1;
    let mut used = 0u32;
    let mut end = 0usize;
    for (i, c) in s.char_indices() {
        let w = char_cols(c);
        if w == 0 {
            end = i + c.len_utf8();
            continue;
        }
        if used.saturating_add(w) > budget {
            break;
        }
        used = used.saturating_add(w);
        end = i + c.len_utf8();
    }
    let mut out = s[..end].to_string();
    out.push(ELLIPSIS);
    (out, true)
}

/// Convert a `Bar` value in `0.0..=1.0` to filled cells. Non-finite and
/// negative values collapse to empty; values above 1 fill the track.
pub fn bar_filled(v: f32, width: u32) -> u32 {
    if width == 0 || !v.is_finite() || v <= 0.0 {
        return 0;
    }
    if v >= 1.0 {
        return width;
    }
    let milli = (v * 1000.0).round();
    let milli = (milli as u32).min(1000);
    ((milli as u64 * width as u64) / 1000) as u32
}

/// Quantize spark samples into `0..=8` (block-element levels). The prefix of
/// `vs` that fits in `width` is kept; extra samples are dropped, not
/// downsampled, so the result is deterministic and O(width).
pub fn spark_levels(vs: &[f32], width: u32) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    if vs.is_empty() {
        return vec![0];
    }
    let take = (width as usize).min(vs.len());
    let slice = &vs[..take];
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &v in slice {
        if v.is_finite() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    slice.iter().map(|&v| spark_level(v, min, max)).collect()
}

/// Eighth-block ramp for one spark level (`0..=8`, saturating).
///
/// One Unicode scalar is one cell, which is what [`spark_levels`] and layout
/// reserved. Shared so the Block and Panel painters cannot draw the same
/// sparkline differently — they render one schema (ADR-0017).
pub const SPARK_RAMP: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Render quantized spark levels as ramp glyphs, one cell each.
pub fn spark_glyphs(levels: &[u8]) -> String {
    levels
        .iter()
        .map(|&lv| SPARK_RAMP[(lv as usize).min(SPARK_RAMP.len() - 1)])
        .collect()
}

fn spark_level(v: f32, min: f32, max: f32) -> u8 {
    if !v.is_finite() || !min.is_finite() || !max.is_finite() {
        return 0;
    }
    if max <= min {
        return 4;
    }
    let t = (v - min) / (max - min);
    let t = t.clamp(0.0, 1.0);
    (t * 8.0).round().clamp(0.0, 8.0) as u8
}

/// Host-owned attribution copy. Newlines are stripped so the band stays one
/// row; an empty id still produces a marker (it is non-suppressible).
pub fn attribution_label(plugin_id: &str, width: u32) -> (String, String) {
    let mut id: String = plugin_id
        .chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .collect();
    if id.chars().all(char::is_whitespace) {
        id = "plugin".to_string();
    }
    let raw = format!("plugin:{id}");
    let (label, _) = fit_cols(&raw, width.max(1));
    (id, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_on_width_and_newline() {
        assert_eq!(wrap_text("abcdef", 4), vec!["abcd", "ef"]);
        assert_eq!(wrap_text("abcd", 4), vec!["abcd"]);
        assert_eq!(wrap_text("ab\ncd", 10), vec!["ab", "cd"]);
        assert_eq!(wrap_text("", 4), vec![""]);
    }

    #[test]
    fn fit_cols_marks_overflow() {
        let (s, cut) = fit_cols("abcdefgh", 4);
        assert!(cut);
        assert_eq!(cell_cols(&s), 4);
        assert!(s.ends_with(ELLIPSIS));
        let (s, cut) = fit_cols("ab", 4);
        assert!(!cut);
        assert_eq!(s, "ab");
    }

    #[test]
    fn bar_non_finite_is_empty() {
        assert_eq!(bar_filled(f32::NAN, 10), 0);
        assert_eq!(bar_filled(f32::INFINITY, 10), 0);
        assert_eq!(bar_filled(f32::NEG_INFINITY, 10), 0);
        assert_eq!(bar_filled(-1.0, 10), 0);
        assert_eq!(bar_filled(0.0, 10), 0);
        assert_eq!(bar_filled(1.0, 10), 10);
        assert_eq!(bar_filled(2.0, 10), 10);
        assert_eq!(bar_filled(0.5, 10), 5);
    }

    #[test]
    fn spark_empty_is_one_flat_level() {
        assert_eq!(spark_levels(&[], 8), vec![0]);
        let levels = spark_levels(&[f32::NAN, f32::INFINITY, 1.0, 2.0], 4);
        assert_eq!(levels.len(), 4);
    }
}
