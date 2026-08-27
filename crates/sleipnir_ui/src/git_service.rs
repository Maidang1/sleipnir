//! Shared, bounded Git queries. All functions here are blocking and must run on
//! a background executor when called from GPUI.

use diff_core::PatchDiff;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::chrome::git_status;
use crate::chrome::workspace::git_root;

/// Hard cap on the `git diff` output we are willing to hold in memory.
pub const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;

/// A work-tree patch as raw text, with the label the UI shows for it.
#[derive(Debug)]
pub struct PatchText {
    pub root: PathBuf,
    pub title: String,
    pub patch: String,
}

/// `PatchText` plus the parsed model and line counts the diff inspector needs.
#[derive(Debug)]
pub struct ReadyPatch {
    pub root: PathBuf,
    pub title: String,
    pub additions: u32,
    pub deletions: u32,
    pub parsed: PatchDiff,
    pub patch: String,
}

#[derive(Debug)]
pub enum PatchOutcome<T> {
    Ready(T),
    Clean { title: String },
    Failed { title: String, message: String },
}

/// Fetch the work-tree patch for `cwd` as text, with non-interactive Git
/// settings and a hard output cap. Blocking: schedule it off the UI thread.
///
/// This is the cheap layer: callers that only paste or forward the patch pay
/// nothing for parsing.
pub fn fetch_patch_text(cwd: &Path) -> PatchOutcome<PatchText> {
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
            return PatchOutcome::Failed {
                title,
                message: format!("failed to run git: {err}"),
            };
        }
    };
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let message = match err.trim() {
            "" => "not a git repository".into(),
            message => message.to_string(),
        };
        return PatchOutcome::Failed { title, message };
    }
    if output.stdout.len() > MAX_PATCH_BYTES {
        return PatchOutcome::Failed {
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
        return PatchOutcome::Clean { title };
    }
    PatchOutcome::Ready(PatchText { root, title, patch })
}

/// `fetch_patch_text` plus the parse and line counts the diff inspector needs.
/// Blocking: schedule it off the UI thread.
pub fn fetch_worktree_patch(cwd: &Path) -> PatchOutcome<ReadyPatch> {
    let text = match fetch_patch_text(cwd) {
        PatchOutcome::Ready(text) => text,
        PatchOutcome::Clean { title } => return PatchOutcome::Clean { title },
        PatchOutcome::Failed { title, message } => {
            return PatchOutcome::Failed { title, message };
        }
    };
    let parsed = diff_core::parse_patch(&text.patch);
    let (additions, deletions) = parsed
        .files
        .iter()
        .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    PatchOutcome::Ready(ReadyPatch {
        root: text.root,
        title: text.title,
        additions,
        deletions,
        parsed,
        patch: text.patch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

        match fetch_worktree_patch(root) {
            PatchOutcome::Ready(ready) => {
                assert_eq!(ready.additions, 1, "{ready:?}");
                assert_eq!(ready.deletions, 1, "{ready:?}");
                assert_eq!(ready.parsed.files.len(), 1);
            }
            other => panic!("expected ready diff, got {other:?}"),
        }
    }

    /// The text layer is what the paste path uses: it must return the same raw
    /// patch without paying for a parse.
    #[test]
    fn text_layer_returns_the_raw_patch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        if !git(root, &["init", "-q", "-b", "main"]) {
            return;
        }
        fs::write(root.join("a.txt"), "one\n").unwrap();
        assert!(git(root, &["add", "a.txt"]));
        assert!(git(root, &["commit", "-qm", "init"]));
        fs::write(root.join("a.txt"), "changed\n").unwrap();

        let (text, ready) = match (fetch_patch_text(root), fetch_worktree_patch(root)) {
            (PatchOutcome::Ready(text), PatchOutcome::Ready(ready)) => (text, ready),
            other => panic!("expected both layers ready, got {other:?}"),
        };
        assert!(text.patch.contains("-one"));
        assert_eq!(text.patch, ready.patch, "layers must agree on the text");
        assert_eq!(text.title, ready.title);
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
        assert!(matches!(
            fetch_worktree_patch(root),
            PatchOutcome::Clean { .. }
        ));
        assert!(matches!(fetch_patch_text(root), PatchOutcome::Clean { .. }));
    }

    #[test]
    fn fetch_is_blocking_but_bounded() {
        let start = Instant::now();
        let _ = fetch_worktree_patch(Path::new("/"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
