//! The data behind the picture: how disk space is distributed in a directory.
//!
//! Bounded on purpose. A plugin that walks `$HOME` unbounded would spin for
//! minutes and produce a chart nobody can read, so the walk is capped by depth,
//! entry count and wall clock, and reports whether it hit a cap. Symlinks are
//! never followed — a cycle would otherwise make the scan unbounded regardless.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Directory recursion cap for a single child's subtree.
pub const MAX_DEPTH: usize = 6;

/// Entry cap across the whole scan.
pub const MAX_ENTRIES: usize = 40_000;

/// Wall-clock cap. Hitting it yields partial results, clearly marked.
pub const TIME_BUDGET: Duration = Duration::from_millis(1_500);

/// Bars in the chart. Beyond this, the smallest are folded into one "other"
/// bar: more bars than this cannot be labelled legibly in a split.
pub const MAX_BARS: usize = 12;

/// One bar: a direct child of the scanned directory, or the "other" fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: u64,
    pub is_dir: bool,
    /// True for the synthetic "other" aggregate, which has no path to open.
    pub aggregated: bool,
}

/// Result of one scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub root: PathBuf,
    /// Descending by size, at most [`MAX_BARS`].
    pub entries: Vec<Entry>,
    pub total_bytes: u64,
    /// Set when a cap cut the walk: the numbers are a lower bound.
    pub partial: bool,
    /// Entries skipped for permissions. Surfaced rather than hidden.
    pub unreadable: usize,
}

impl Scan {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn largest_bytes(&self) -> u64 {
        self.entries.first().map(|e| e.bytes).unwrap_or(0)
    }
}

struct Budget {
    entries_left: usize,
    deadline: Instant,
    unreadable: usize,
    exhausted: bool,
}

impl Budget {
    fn spend(&mut self) -> bool {
        if self.entries_left == 0 {
            self.exhausted = true;
            return false;
        }
        // Check the clock rarely: `Instant::now` per entry dominates a walk of
        // small files.
        if self.entries_left.is_multiple_of(512) && Instant::now() >= self.deadline {
            self.exhausted = true;
            return false;
        }
        self.entries_left -= 1;
        true
    }
}

/// Scan the direct children of `root`, sizing each subtree.
pub fn scan(root: &Path) -> Scan {
    let mut budget = Budget {
        entries_left: MAX_ENTRIES,
        deadline: Instant::now() + TIME_BUDGET,
        unreadable: 0,
        exhausted: false,
    };
    let mut entries = Vec::new();
    match std::fs::read_dir(root) {
        Ok(dir) => {
            for item in dir.flatten() {
                if !budget.spend() {
                    break;
                }
                let name = item.file_name().to_string_lossy().to_string();
                // `fs::symlink_metadata` (not `DirEntry::metadata`, which
                // follows links): never traverse a link, and size the link
                // itself rather than its target.
                let Ok(meta) = std::fs::symlink_metadata(item.path()) else {
                    budget.unreadable += 1;
                    continue;
                };
                if meta.is_dir() {
                    let bytes = dir_size(&item.path(), 1, &mut budget);
                    entries.push(Entry {
                        name,
                        bytes,
                        is_dir: true,
                        aggregated: false,
                    });
                } else {
                    // Symlinks land here: sized as the link itself, never
                    // traversed.
                    entries.push(Entry {
                        name,
                        bytes: meta.len(),
                        is_dir: false,
                        aggregated: false,
                    });
                }
            }
        }
        Err(_) => budget.unreadable += 1,
    }

    entries.retain(|e| e.bytes > 0);
    // Name is the tiebreak so equal sizes cannot reorder between scans and make
    // the chart flicker.
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    let total_bytes = entries
        .iter()
        .map(|e| e.bytes)
        .fold(0u64, u64::saturating_add);
    let entries = fold_tail(entries);

    Scan {
        root: root.to_path_buf(),
        entries,
        total_bytes,
        partial: budget.exhausted,
        unreadable: budget.unreadable,
    }
}

/// Keep the largest bars; sum the rest into one labelled aggregate so the total
/// stays honest.
fn fold_tail(mut entries: Vec<Entry>) -> Vec<Entry> {
    if entries.len() <= MAX_BARS {
        return entries;
    }
    let tail: Vec<Entry> = entries.split_off(MAX_BARS - 1);
    let bytes = tail.iter().map(|e| e.bytes).fold(0u64, u64::saturating_add);
    if bytes > 0 {
        entries.push(Entry {
            name: format!("other ({})", tail.len()),
            bytes,
            is_dir: false,
            aggregated: true,
        });
    }
    entries
}

fn dir_size(path: &Path, depth: usize, budget: &mut Budget) -> u64 {
    if depth > MAX_DEPTH {
        budget.exhausted = true;
        return 0;
    }
    let Ok(dir) = std::fs::read_dir(path) else {
        budget.unreadable += 1;
        return 0;
    };
    let mut total = 0u64;
    for item in dir.flatten() {
        if !budget.spend() {
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(item.path()) else {
            budget.unreadable += 1;
            continue;
        };
        if meta.file_type().is_symlink() {
            total = total.saturating_add(meta.len());
        } else if meta.is_dir() {
            total = total.saturating_add(dir_size(&item.path(), depth + 1, budget));
        } else {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

/// Binary-prefix size, 3 significant digits. Used in the legend, where every
/// extra column costs a label character.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if bytes < 1024 {
        return format!("{bytes}B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.2}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("disk3d-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn entries_are_sorted_by_size_descending() {
        let root = tmp("sorted");
        write(&root.join("small.txt"), 100);
        write(&root.join("big.txt"), 5000);
        write(&root.join("mid.txt"), 1000);
        let scan = scan(&root);
        let names: Vec<&str> = scan.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["big.txt", "mid.txt", "small.txt"]);
        assert_eq!(scan.total_bytes, 6100);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_size_is_the_sum_of_its_subtree() {
        let root = tmp("subtree");
        write(&root.join("pkg/a.txt"), 400);
        write(&root.join("pkg/nested/b.txt"), 600);
        let scan = scan(&root);
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].name, "pkg");
        assert!(scan.entries[0].is_dir);
        assert_eq!(scan.entries[0].bytes, 1000);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_and_zero_byte_entries_are_dropped() {
        let root = tmp("empty");
        write(&root.join("zero.txt"), 0);
        fs::create_dir_all(root.join("emptydir")).unwrap();
        let scan = scan(&root);
        assert!(scan.is_empty());
        assert_eq!(scan.total_bytes, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_directory_is_reported_not_panicked() {
        let scan = scan(Path::new("/definitely/does/not/exist/anywhere"));
        assert!(scan.is_empty());
        assert_eq!(scan.unreadable, 1);
    }

    #[test]
    fn tail_is_folded_and_the_total_is_preserved() {
        let root = tmp("fold");
        // 20 distinct sizes → more bars than MAX_BARS.
        for i in 0..20 {
            write(&root.join(format!("f{i:02}.txt")), 100 * (i + 1));
        }
        let scan = scan(&root);
        assert_eq!(scan.entries.len(), MAX_BARS);
        let folded = scan.entries.last().unwrap();
        assert!(folded.aggregated);
        let charted: u64 = scan.entries.iter().map(|e| e.bytes).sum();
        assert_eq!(
            charted, scan.total_bytes,
            "folding must not lose or invent bytes"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn symlinks_are_sized_not_followed() {
        let root = tmp("symlink");
        write(&root.join("real/data.bin"), 2048);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
            let scan = scan(&root);
            let link = scan.entries.iter().find(|e| e.name == "link").unwrap();
            assert!(!link.is_dir, "a followed symlink would double-count");
            assert!(link.bytes < 2048);
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn human_bytes_is_compact_and_monotonic() {
        assert_eq!(human_bytes(0), "0B");
        assert_eq!(human_bytes(999), "999B");
        assert_eq!(human_bytes(1024), "1.00K");
        assert_eq!(human_bytes(1536), "1.50K");
        assert_eq!(human_bytes(15 * 1024), "15.0K");
        assert_eq!(human_bytes(150 * 1024), "150K");
        assert_eq!(human_bytes(1024 * 1024), "1.00M");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.00G");
        // Never longer than 6 cells: the legend budget depends on it.
        for b in [0u64, 1, 1023, 1024, u64::MAX / 2, u64::MAX] {
            assert!(human_bytes(b).chars().count() <= 6, "{b}");
        }
    }
}
