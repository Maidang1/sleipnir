//! UI-free unified-diff model.
//!
//! Parses `git diff` / `gh pr diff` text into files → hunks → rows, then
//! computes word-level intra-line ranges on equal-length removed/added runs.
//! Heuristics follow ellie/lgtm (MIT); reimplemented here.

use imara_diff::intern::InternedInput;
use imara_diff::Algorithm;
use std::ops::Range;

#[derive(Debug, Default, Clone)]
pub struct PatchDiff {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    Binary,
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: FileStatus,
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
}

impl FileDiff {
    pub fn display_path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("<unknown>")
    }
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub section: String,
    pub rows: Vec<DiffRow>,
}

/// A row is only Context, Added, or Removed. A "modified" line is a removed
/// row immediately followed by an added row.
#[derive(Debug, Clone)]
pub enum DiffRow {
    Context {
        old_no: u32,
        new_no: u32,
        text: String,
    },
    Added {
        new_no: u32,
        text: String,
        intra: Vec<Range<usize>>,
    },
    Removed {
        old_no: u32,
        text: String,
        intra: Vec<Range<usize>>,
    },
}

pub fn parse_patch(patch: &str) -> PatchDiff {
    let mut files = Vec::new();
    let mut lines = patch.lines().peekable();

    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("diff --git ") else {
            continue;
        };
        let (old_guess, new_guess) = parse_git_paths(rest);
        let mut file = FileDiff {
            old_path: old_guess,
            new_path: new_guess,
            status: FileStatus::Modified,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
        };
        let mut is_rename = false;

        while let Some(next) = lines.peek() {
            if next.starts_with("diff --git ") || next.starts_with("@@ ") {
                break;
            }
            let next = lines.next().unwrap();
            if next.starts_with("new file mode") {
                file.status = FileStatus::Added;
            } else if next.starts_with("deleted file mode") {
                file.status = FileStatus::Deleted;
            } else if let Some(path) = next.strip_prefix("rename from ") {
                file.old_path = Some(path.to_string());
                is_rename = true;
            } else if let Some(path) = next.strip_prefix("rename to ") {
                file.new_path = Some(path.to_string());
                is_rename = true;
            } else if next.starts_with("Binary files ") || next == "GIT binary patch" {
                file.status = FileStatus::Binary;
            } else if let Some(path) = next.strip_prefix("--- ") {
                if let Some(path) = parse_marker_path(path) {
                    file.old_path = Some(path);
                }
            } else if let Some(path) = next.strip_prefix("+++ ") {
                if let Some(path) = parse_marker_path(path) {
                    file.new_path = Some(path);
                }
            }
        }
        if is_rename && file.status == FileStatus::Modified {
            file.status = FileStatus::Renamed;
        }
        if file.status == FileStatus::Added {
            file.old_path = None;
        }
        if file.status == FileStatus::Deleted {
            file.new_path = None;
        }

        while let Some(next) = lines.peek() {
            if !next.starts_with("@@ ") {
                break;
            }
            let header = lines.next().unwrap();
            let Some(mut hunk) = parse_hunk_header(header) else {
                break;
            };
            let mut old_no = hunk.old_start;
            let mut new_no = hunk.new_start;
            while let Some(body) = lines.peek() {
                if body.starts_with("diff --git ") || body.starts_with("@@ ") {
                    break;
                }
                let body = lines.next().unwrap();
                if let Some(text) = body.strip_prefix('+') {
                    hunk.rows.push(DiffRow::Added {
                        new_no,
                        text: text.to_string(),
                        intra: Vec::new(),
                    });
                    new_no += 1;
                    file.additions += 1;
                } else if let Some(text) = body.strip_prefix('-') {
                    hunk.rows.push(DiffRow::Removed {
                        old_no,
                        text: text.to_string(),
                        intra: Vec::new(),
                    });
                    old_no += 1;
                    file.deletions += 1;
                } else if body.starts_with('\\') {
                    // "\ No newline at end of file"
                } else {
                    let text = body.strip_prefix(' ').unwrap_or(body);
                    hunk.rows.push(DiffRow::Context {
                        old_no,
                        new_no,
                        text: text.to_string(),
                    });
                    old_no += 1;
                    new_no += 1;
                }
            }
            compute_intra_line(&mut hunk.rows);
            file.hunks.push(hunk);
        }
        files.push(file);
    }

    PatchDiff { files }
}

/// Authoritative line diff of two complete texts. Context lines around nearby
/// changes merge when the gap between them is ≤ `2 * context` (git semantics).
/// Line endings are stripped (`str::lines`); a trailing-newline-only change
/// produces no hunks.
pub fn diff_texts(old: &str, new: &str, context: u32) -> Vec<Hunk> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut input = InternedInput::default();
    input.update_before(old_lines.iter().copied());
    input.update_after(new_lines.iter().copied());

    let mut changes: Vec<(Range<u32>, Range<u32>)> = Vec::new();
    imara_diff::diff(
        Algorithm::Histogram,
        &input,
        |before: Range<u32>, after: Range<u32>| changes.push((before, after)),
    );
    if changes.is_empty() {
        return Vec::new();
    }

    let mut groups: Vec<Range<usize>> = Vec::new();
    let mut start = 0;
    for ix in 1..changes.len() {
        if changes[ix].0.start - changes[ix - 1].0.end > 2 * context {
            groups.push(start..ix);
            start = ix;
        }
    }
    groups.push(start..changes.len());

    let mut hunks = Vec::new();
    for group in groups {
        let first = &changes[group.start];
        let last = &changes[group.end - 1];
        let old_lo = first.0.start.saturating_sub(context);
        let old_hi = (last.0.end + context).min(old_lines.len() as u32);
        let new_lo = first.1.start.saturating_sub(context);
        let new_hi = (last.1.end + context).min(new_lines.len() as u32);

        let mut rows = Vec::new();
        let (mut old_no, mut new_no) = (old_lo, new_lo);
        for (before, after) in &changes[group] {
            while old_no < before.start {
                rows.push(DiffRow::Context {
                    old_no: old_no + 1,
                    new_no: new_no + 1,
                    text: old_lines[old_no as usize].to_string(),
                });
                old_no += 1;
                new_no += 1;
            }
            for no in before.clone() {
                rows.push(DiffRow::Removed {
                    old_no: no + 1,
                    text: old_lines[no as usize].to_string(),
                    intra: Vec::new(),
                });
            }
            for no in after.clone() {
                rows.push(DiffRow::Added {
                    new_no: no + 1,
                    text: new_lines[no as usize].to_string(),
                    intra: Vec::new(),
                });
            }
            old_no = before.end;
            new_no = after.end;
        }
        while old_no < old_hi {
            rows.push(DiffRow::Context {
                old_no: old_no + 1,
                new_no: new_no + 1,
                text: old_lines[old_no as usize].to_string(),
            });
            old_no += 1;
            new_no += 1;
        }
        compute_intra_line(&mut rows);

        let old_count = old_hi - old_lo;
        let new_count = new_hi - new_lo;
        hunks.push(Hunk {
            old_start: if old_count == 0 { old_lo } else { old_lo + 1 },
            old_count,
            new_start: if new_count == 0 { new_lo } else { new_lo + 1 },
            new_count,
            section: String::new(),
            rows,
        });
    }
    hunks
}

/// Hidden shared lines in gap `gap_ix` (0..=hunks.len()) of an upgraded file:
/// `(first_old, first_new, count)`, 1-based. Count 0 means there is no gap.
pub fn gap_span(hunks: &[Hunk], gap_ix: usize, total_new: u32) -> (u32, u32, u32) {
    let (old_lo, new_lo) = if gap_ix == 0 {
        (1, 1)
    } else {
        let h = &hunks[gap_ix - 1];
        let pre_old = if h.old_count == 0 {
            h.old_start
        } else {
            h.old_start - 1
        };
        let pre_new = if h.new_count == 0 {
            h.new_start
        } else {
            h.new_start - 1
        };
        (pre_old + h.old_count + 1, pre_new + h.new_count + 1)
    };
    let new_hi = if gap_ix == hunks.len() {
        total_new
    } else {
        let h = &hunks[gap_ix];
        if h.new_count == 0 {
            h.new_start
        } else {
            h.new_start - 1
        }
    };
    (old_lo, new_lo, (new_hi + 1).saturating_sub(new_lo))
}

/// Shared context lines for an expanded gap: `(old_no, new_no, text)`.
/// Empty when [`gap_span`] reports a zero count or `new_lines` is short.
pub fn expand_gap_lines(
    new_lines: &[String],
    hunks: &[Hunk],
    gap_ix: usize,
) -> Vec<(u32, u32, String)> {
    let (old_lo, new_lo, count) = gap_span(hunks, gap_ix, new_lines.len() as u32);
    if count == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count as usize);
    for j in 0..count {
        let new_no = new_lo + j;
        let Some(text) = new_lines.get((new_no - 1) as usize) else {
            break;
        };
        out.push((old_lo + j, new_no, text.clone()));
    }
    out
}

fn parse_git_paths(rest: &str) -> (Option<String>, Option<String>) {
    if rest.starts_with('"') {
        let mut parts = Vec::new();
        let mut chars = rest.char_indices();
        while let Some((start, ch)) = chars.next() {
            if ch != '"' {
                continue;
            }
            for (end, ch) in chars.by_ref() {
                if ch == '"' {
                    parts.push(&rest[start + 1..end]);
                    break;
                }
            }
        }
        if parts.len() == 2 {
            return (strip_side(parts[0]), strip_side(parts[1]));
        }
    }
    if let Some(pos) = rest.find(" b/") {
        let old = strip_side(&rest[..pos]);
        let new = strip_side(&rest[pos + 1..]);
        return (old, new);
    }
    (None, None)
}

fn strip_side(path: &str) -> Option<String> {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .map(str::to_string)
}

fn parse_marker_path(p: &str) -> Option<String> {
    let p = p.trim_end();
    let p = p.strip_prefix('"').unwrap_or(p);
    let p = p.strip_suffix('"').unwrap_or(p);
    if p == "/dev/null" {
        return None;
    }
    strip_side(p).or_else(|| Some(p.to_string()))
}

fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, rest) = rest.split_once(" @@")?;
    let parse_pair = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = parse_pair(old)?;
    let (new_start, new_count) = parse_pair(new)?;
    Some(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section: rest.trim_start().to_string(),
        rows: Vec::new(),
    })
}

const MAX_PAIR_RUN: usize = 32;
const MAX_LINE_BYTES: usize = 4096;
const MAX_LINE_TOKENS: usize = 512;
const MAX_CHANGED_FRACTION: f32 = 0.7;

fn compute_intra_line(rows: &mut [DiffRow]) {
    let mut i = 0;
    while i < rows.len() {
        if !matches!(rows[i], DiffRow::Removed { .. }) {
            i += 1;
            continue;
        }
        let start = i;
        while i < rows.len() && matches!(rows[i], DiffRow::Removed { .. }) {
            i += 1;
        }
        let mid = i;
        while i < rows.len() && matches!(rows[i], DiffRow::Added { .. }) {
            i += 1;
        }
        let removed = mid - start;
        let added = i - mid;
        if removed != added || removed > MAX_PAIR_RUN {
            continue;
        }
        for pair in 0..removed {
            let old_text = match &rows[start + pair] {
                DiffRow::Removed { text, .. } => text.clone(),
                _ => unreachable!(),
            };
            let new_text = match &rows[mid + pair] {
                DiffRow::Added { text, .. } => text.clone(),
                _ => unreachable!(),
            };
            if old_text.len() > MAX_LINE_BYTES || new_text.len() > MAX_LINE_BYTES {
                continue;
            }
            let (old_ranges, new_ranges) = word_diff(&old_text, &new_text);
            if let DiffRow::Removed { intra, .. } = &mut rows[start + pair] {
                *intra = old_ranges;
            }
            if let DiffRow::Added { intra, .. } = &mut rows[mid + pair] {
                *intra = new_ranges;
            }
        }
    }
}

fn token_ranges(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = s.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let class = char_class(ch);
        let mut end = start + ch.len_utf8();
        if class != CharClass::Punct {
            while let Some(&(next_ix, next_ch)) = chars.peek() {
                if char_class(next_ch) != class {
                    break;
                }
                end = next_ix + next_ch.len_utf8();
                chars.next();
            }
        }
        out.push(start..end);
    }
    out
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CharClass {
    Word,
    Space,
    Punct,
}

fn char_class(ch: char) -> CharClass {
    if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else if ch.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Punct
    }
}

fn word_diff(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let old_tokens = token_ranges(old);
    let new_tokens = token_ranges(new);
    if old_tokens.len() > MAX_LINE_TOKENS || new_tokens.len() > MAX_LINE_TOKENS {
        return (Vec::new(), Vec::new());
    }

    let mut input = InternedInput::default();
    input.update_before(old_tokens.iter().map(|r| &old[r.clone()]));
    input.update_after(new_tokens.iter().map(|r| &new[r.clone()]));

    let mut old_ranges: Vec<Range<usize>> = Vec::new();
    let mut new_ranges: Vec<Range<usize>> = Vec::new();
    imara_diff::diff(
        Algorithm::Histogram,
        &input,
        |before: Range<u32>, after: Range<u32>| {
            if before.start < before.end {
                let start = old_tokens[before.start as usize].start;
                let end = old_tokens[before.end as usize - 1].end;
                old_ranges.push(start..end);
            }
            if after.start < after.end {
                let start = new_tokens[after.start as usize].start;
                let end = new_tokens[after.end as usize - 1].end;
                new_ranges.push(start..end);
            }
        },
    );

    let changed = |ranges: &[Range<usize>], len: usize| {
        len > 0
            && ranges.iter().map(|r| r.len()).sum::<usize>() as f32 / len as f32
                > MAX_CHANGED_FRACTION
    };
    if changed(&old_ranges, old.trim_end().len()) || changed(&new_ranges, new.trim_end().len()) {
        return (Vec::new(), Vec::new());
    }
    (old_ranges, new_ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,4 +1,5 @@ fn main()
 use std::io;
-let foo = 1;
+let bar = 1;
+let baz = 2;
 println!();
@@ -10,2 +11,2 @@
 // tail
-old line
+new line
diff --git a/README.md b/README.md
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/README.md
@@ -0,0 +1,1 @@
+# hello
\\ No newline at end of file
diff --git a/logo.png b/logo.png
index 4444444..5555555 100644
Binary files a/logo.png and b/logo.png differ
diff --git a/old.rs b/renamed.rs
similarity index 90%
rename from old.rs
rename to renamed.rs
index 6666666..7777777 100644
--- a/old.rs
+++ b/renamed.rs
@@ -1,1 +1,1 @@
-x
+y
";

    #[test]
    fn parses_files_hunks_and_rows() {
        let diff = parse_patch(SAMPLE);
        assert_eq!(diff.files.len(), 4);

        let main = &diff.files[0];
        assert_eq!(main.display_path(), "src/main.rs");
        assert_eq!(main.status, FileStatus::Modified);
        assert_eq!(main.hunks.len(), 2);
        assert_eq!((main.additions, main.deletions), (3, 2));
        assert_eq!(main.hunks[0].section, "fn main()");
        assert_eq!(main.hunks[0].rows.len(), 5);
        match &main.hunks[0].rows[1] {
            DiffRow::Removed { old_no, text, .. } => {
                assert_eq!(*old_no, 2);
                assert_eq!(text, "let foo = 1;");
            }
            other => panic!("expected removed row, got {other:?}"),
        }

        let readme = &diff.files[1];
        assert_eq!(readme.status, FileStatus::Added);
        assert_eq!(readme.old_path, None);
        assert_eq!(readme.display_path(), "README.md");

        assert_eq!(diff.files[2].status, FileStatus::Binary);
        assert_eq!(diff.files[3].status, FileStatus::Renamed);
        assert_eq!(diff.files[3].old_path.as_deref(), Some("old.rs"));
        assert_eq!(diff.files[3].display_path(), "renamed.rs");
    }

    #[test]
    fn word_diff_highlights_identifier_only() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-let foo = 1;
+let bar = 1;
";
        let diff = parse_patch(patch);
        match &diff.files[0].hunks[0].rows[0] {
            DiffRow::Removed { text, intra, .. } => {
                assert_eq!(text, "let foo = 1;");
                assert_eq!(intra, &[4..7]);
            }
            other => panic!("{other:?}"),
        }
        match &diff.files[0].hunks[0].rows[1] {
            DiffRow::Added { text, intra, .. } => {
                assert_eq!(text, "let bar = 1;");
                assert_eq!(intra, &[4..7]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn whole_line_rewrite_has_no_intra() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-aaaaaaaa
+bbbbbbbb
";
        let diff = parse_patch(patch);
        match &diff.files[0].hunks[0].rows[0] {
            DiffRow::Removed { intra, .. } => assert!(intra.is_empty(), "{intra:?}"),
            other => panic!("{other:?}"),
        }
        match &diff.files[0].hunks[0].rows[1] {
            DiffRow::Added { intra, .. } => assert!(intra.is_empty(), "{intra:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unequal_runs_are_not_paired() {
        let patch = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,1 @@
-a
-b
+c
";
        let diff = parse_patch(patch);
        for row in &diff.files[0].hunks[0].rows {
            match row {
                DiffRow::Added { intra, .. } | DiffRow::Removed { intra, .. } => {
                    assert!(intra.is_empty(), "{row:?}");
                }
                DiffRow::Context { .. } => {}
            }
        }
    }

    #[test]
    fn empty_patch_is_empty() {
        assert!(parse_patch("").files.is_empty());
        assert!(parse_patch("not a diff\n").files.is_empty());
    }

    #[test]
    fn identical_texts_produce_no_hunks() {
        assert!(diff_texts("a\nb\nc\n", "a\nb\nc\n", 3).is_empty());
        assert!(diff_texts("a\nb", "a\nb\n", 3).is_empty());
    }

    #[test]
    fn mid_file_edit_forms_a_real_gap_and_expands() {
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let mut new = old.clone();
        new = new.replace("line 10\n", "line 10 changed\n");
        let hunks = diff_texts(&old, &new, 3);
        assert_eq!(hunks.len(), 1, "{hunks:?}");
        let h = &hunks[0];
        assert_eq!(h.old_start, 7);
        assert_eq!(h.new_start, 7);
        // 3 context before + changed + 3 after
        assert_eq!(h.old_count, 7);
        assert_eq!(h.new_count, 7);

        let new_lines: Vec<String> = new.lines().map(str::to_string).collect();
        assert_eq!(new_lines.len(), 20);
        let before = gap_span(&hunks, 0, 20);
        assert_eq!(before, (1, 1, 6));
        let after = gap_span(&hunks, 1, 20);
        assert_eq!(after, (14, 14, 7));
        assert_eq!(gap_span(&hunks, 0, 20).2, 6);
        // No extra gap in the middle of a single hunk.
        let expanded = expand_gap_lines(&new_lines, &hunks, 0);
        let texts: Vec<&str> = expanded.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(texts, ["line 1", "line 2", "line 3", "line 4", "line 5", "line 6"]);
        assert_eq!(expanded[0], (1, 1, "line 1".into()));
        assert_eq!(expanded[5], (6, 6, "line 6".into()));
        let tail = expand_gap_lines(&new_lines, &hunks, 1);
        assert_eq!(tail.len(), 7);
        assert_eq!(tail[0], (14, 14, "line 14".into()));
        assert_eq!(tail[6], (20, 20, "line 20".into()));
    }

    #[test]
    fn nearby_changes_merge_when_context_touches() {
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let new = old
            .replace("line 8\n", "line 8x\n")
            .replace("line 12\n", "line 12x\n");
        let hunks = diff_texts(&old, &new, 3);
        assert_eq!(hunks.len(), 1, "gap of 3 lines ≤ 2*context, should merge");
    }

    #[test]
    fn distant_changes_stay_two_hunks() {
        let old: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let new = old
            .replace("line 5\n", "line 5x\n")
            .replace("line 25\n", "line 25x\n");
        let hunks = diff_texts(&old, &new, 3);
        assert_eq!(hunks.len(), 2);
        let mid = gap_span(&hunks, 1, 30);
        assert!(mid.2 > 0, "hidden lines between hunks: {mid:?}");
        let new_lines: Vec<String> = new.lines().map(str::to_string).collect();
        let hidden = expand_gap_lines(&new_lines, &hunks, 1);
        assert_eq!(hidden.len(), mid.2 as usize);
        assert!(hidden.iter().all(|(o, n, _)| o == n));
    }

    #[test]
    fn change_at_start_has_no_leading_gap() {
        let old = "head\nkeep\nkeep\nkeep\nkeep\nkeep\nkeep\n";
        let new = "HEAD\nkeep\nkeep\nkeep\nkeep\nkeep\nkeep\n";
        let hunks = diff_texts(old, new, 3);
        assert_eq!(hunks.len(), 1);
        let (old_lo, new_lo, count) = gap_span(&hunks, 0, 7);
        assert_eq!(count, 0, "leading gap at file start: {old_lo}/{new_lo}/{count}");
        assert!(expand_gap_lines(
            &new.lines().map(str::to_string).collect::<Vec<_>>(),
            &hunks,
            0
        )
        .is_empty());
    }

    #[test]
    fn change_at_end_has_no_trailing_gap() {
        let old = "keep\nkeep\nkeep\nkeep\nkeep\nkeep\ntail\n";
        let new = "keep\nkeep\nkeep\nkeep\nkeep\nkeep\nTAIL\n";
        let hunks = diff_texts(old, new, 3);
        assert_eq!(hunks.len(), 1);
        let (_, _, count) = gap_span(&hunks, 1, 7);
        assert_eq!(count, 0, "trailing gap should be empty");
    }
}
