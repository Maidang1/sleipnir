//! Fetch a work-tree unified patch off the UI thread.

use crate::chrome::git_status;
use crate::chrome::workspace::git_root;
use diff_core::PatchDiff;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Refuse to flatten a patch larger than this. File headers still show.
pub const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub struct ReadyDiff {
    pub root: PathBuf,
    pub title: String,
    pub additions: u32,
    pub deletions: u32,
    pub parsed: PatchDiff,
    pub patch: String,
}

#[derive(Debug)]
pub enum FetchOutcome {
    Ready(ReadyDiff),
    Clean { title: String },
    Failed { title: String, message: String },
}

/// Blocking. Call from a background executor.
pub fn fetch_worktree_diff(cwd: &Path) -> FetchOutcome {
    let root = git_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let branch = git_status::git_snapshot_in(&root, &git_status::RealFs)
        .map(|snap| snap.branch)
        .unwrap_or_else(|| "HEAD".into());
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| root.display().to_string());
    let title = format!("{name} · {branch}");

    let output = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "diff",
            "--no-color",
            "--no-ext-diff",
            "HEAD",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();

    let output = match output {
        Ok(out) => out,
        Err(err) => {
            return FetchOutcome::Failed {
                title,
                message: format!("failed to run git: {err}"),
            };
        }
    };
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let err = err.trim();
        let message = if err.is_empty() {
            "not a git repository".into()
        } else {
            err.to_string()
        };
        return FetchOutcome::Failed { title, message };
    }
    if output.stdout.len() > MAX_PATCH_BYTES {
        return FetchOutcome::Failed {
            title,
            message: format!(
                "diff is {:.1} MB; too large to open (limit {} MB)",
                output.stdout.len() as f64 / (1024.0 * 1024.0),
                MAX_PATCH_BYTES / (1024 * 1024)
            ),
        };
    }
    let patch = String::from_utf8_lossy(&output.stdout).into_owned();
    if patch.trim().is_empty() {
        return FetchOutcome::Clean { title };
    }
    let parsed = diff_core::parse_patch(&patch);
    let (additions, deletions) = parsed
        .files
        .iter()
        .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    FetchOutcome::Ready(ReadyDiff {
        root,
        title,
        additions,
        deletions,
        parsed,
        patch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::time::{Duration, Instant};

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
    fn fetch_counts_a_one_line_edit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        if !git(root, &["init", "-q", "-b", "main"]) {
            return;
        }
        fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        assert!(git(root, &["add", "a.txt"]));
        assert!(git(root, &["commit", "-qm", "init"]));
        fs::write(root.join("a.txt"), "one\ntwo changed\nthree\n").unwrap();

        match fetch_worktree_diff(root) {
            FetchOutcome::Ready(ready) => {
                assert_eq!(ready.additions, 1, "{ready:?}");
                assert_eq!(ready.deletions, 1, "{ready:?}");
                assert_eq!(ready.parsed.files.len(), 1);
            }
            other => panic!("expected ready diff, got {other:?}"),
        }
    }

    #[test]
    fn fetch_clean_tree_is_clean() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        if !git(root, &["init", "-q", "-b", "main"]) {
            return;
        }
        fs::write(root.join("a.txt"), "one\n").unwrap();
        assert!(git(root, &["add", "a.txt"]));
        assert!(git(root, &["commit", "-qm", "init"]));
        match fetch_worktree_diff(root) {
            FetchOutcome::Clean { .. } => {}
            other => panic!("expected clean, got {other:?}"),
        }
    }

    #[test]
    fn fetch_is_blocking_but_callable() {
        // Sanity: this helper is the one that runs git. The UI path must
        // call it from background_spawn, not render. Timing the helper
        // itself is not the invariant — see git_status for the UI-thread test.
        let start = Instant::now();
        let _ = fetch_worktree_diff(Path::new("/"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
