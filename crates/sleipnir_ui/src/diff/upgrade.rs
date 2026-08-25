//! Phase-3 full-file upgrade: fetch both sides, re-diff, highlight.

use std::path::Path;
use std::process::Command;

use super::rows::FileUpgrade;
use diff_core::{DiffRow, FileStatus, Hunk, diff_texts};

/// Per-side blob cap. Larger or non-UTF-8 files stay patch-derived.
pub const MAX_UPGRADE_BLOB_BYTES: usize = 1024 * 1024;
const UPGRADE_CONTEXT: u32 = 3;

pub struct UpgradeJob {
    pub file_ix: usize,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: FileStatus,
}

pub struct UpgradedFile {
    pub file_ix: usize,
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
    pub upgrade: FileUpgrade,
}

/// Accept a blob for upgrade. Size and UTF-8 are the documented gates.
pub fn accept_blob(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_UPGRADE_BLOB_BYTES {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// Contents of `path` at HEAD, or None if missing / binary / too large.
pub fn file_at_head(root: &Path, path: &str) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "show",
            &format!("HEAD:{path}"),
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    accept_blob(&output.stdout)
}

/// Working-tree bytes at `root/path`.
pub fn file_in_worktree(root: &Path, path: &str) -> Option<String> {
    let bytes = std::fs::read(root.join(path)).ok()?;
    accept_blob(&bytes)
}

/// Re-diff one file. `None` keeps the patch-derived hunks.
pub fn upgrade_file(root: &Path, job: &UpgradeJob) -> Option<UpgradedFile> {
    let old_text = match (job.status, job.old_path.as_deref()) {
        (FileStatus::Added, _) => String::new(),
        (_, Some(path)) => file_at_head(root, path)?,
        (_, None) => return None,
    };
    let new_text = match (job.status, job.new_path.as_deref()) {
        (FileStatus::Deleted, _) => String::new(),
        (_, Some(path)) => file_in_worktree(root, path)?,
        (_, None) => return None,
    };
    let normalize = |text: String| {
        if text.contains('\r') {
            text.replace("\r\n", "\n")
        } else {
            text
        }
    };
    let old_text = normalize(old_text);
    let new_text = normalize(new_text);
    let hunks = diff_texts(&old_text, &new_text, UPGRADE_CONTEXT);
    let (additions, deletions) = hunks
        .iter()
        .flat_map(|h| &h.rows)
        .fold((0, 0), |(a, d), row| match row {
            DiffRow::Added { .. } => (a + 1, d),
            DiffRow::Removed { .. } => (a, d + 1),
            DiffRow::Context { .. } => (a, d),
        });
    let path = job.new_path.as_deref().or(job.old_path.as_deref())?;
    let lang = syntax::language_for_path(path);
    let spans = |text: &str| match lang {
        Some(lang) if !text.is_empty() => syntax::highlight_lines(lang, text),
        _ => Vec::new(),
    };
    Some(UpgradedFile {
        file_ix: job.file_ix,
        hunks,
        additions,
        deletions,
        upgrade: FileUpgrade {
            old_spans: spans(&old_text),
            new_spans: spans(&new_text),
            new_lines: new_text.lines().map(str::to_string).collect(),
            expanded: std::collections::HashSet::new(),
        },
    })
}

pub fn run_upgrade(root: &Path, jobs: Vec<UpgradeJob>) -> Vec<UpgradedFile> {
    jobs.into_iter()
        .filter_map(|job| upgrade_file(root, &job))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn accept_blob_rejects_huge_and_non_utf8() {
        assert!(accept_blob(b"ok\n").is_some());
        assert!(accept_blob(&[0xff, 0xfe]).is_none());
        let huge = vec![b'a'; MAX_UPGRADE_BLOB_BYTES + 1];
        assert!(accept_blob(&huge).is_none());
    }

    #[test]
    fn upgrade_path_upgrades_text_and_skips_binary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        if !git(root, &["init", "-q", "-b", "main"]) {
            return;
        }
        let mut old = String::new();
        for n in 1..=20 {
            old.push_str(&format!("line {n}\n"));
        }
        fs::write(root.join("note.rs"), &old).unwrap();
        fs::write(root.join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
        assert!(git(root, &["add", "."]));
        assert!(git(root, &["commit", "-qm", "init"]));
        let new = old.replace("line 10\n", "line 10 changed\n");
        fs::write(root.join("note.rs"), &new).unwrap();

        let text = upgrade_file(
            root,
            &UpgradeJob {
                file_ix: 0,
                old_path: Some("note.rs".into()),
                new_path: Some("note.rs".into()),
                status: FileStatus::Modified,
            },
        )
        .expect("text file should upgrade");
        assert_eq!(text.file_ix, 0);
        assert_eq!(text.hunks.len(), 1);
        assert!(text.upgrade.new_lines.len() >= 20);
        let (_, _, hidden) =
            diff_core::gap_span(&text.hunks, 0, text.upgrade.new_lines.len() as u32);
        assert!(hidden > 0, "mid-file edit must leave a leading gap");

        let binary = upgrade_file(
            root,
            &UpgradeJob {
                file_ix: 1,
                old_path: Some("blob.bin".into()),
                new_path: Some("blob.bin".into()),
                status: FileStatus::Modified,
            },
        );
        assert!(binary.is_none(), "binary must stay patch-derived");
    }
}
