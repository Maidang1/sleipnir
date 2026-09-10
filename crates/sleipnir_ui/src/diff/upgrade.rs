//! Phase-3 full-file enrichment without replacing the authoritative Git patch.

use std::io::Read;
use std::path::{Component, Path};
use std::process::Command;

use super::rows::FileUpgrade;
use diff_core::{DiffRow, FileStatus, Hunk};

/// Per-side blob cap. Larger or non-UTF-8 files stay patch-derived.
pub const MAX_UPGRADE_BLOB_BYTES: usize = 1024 * 1024;

pub struct UpgradeJob {
    pub file_ix: usize,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: FileStatus,
}

pub struct UpgradedFile {
    pub file_ix: usize,
    pub old_lines: Vec<String>,
    pub upgrade: FileUpgrade,
}

/// Accept a blob for upgrade. Size and UTF-8 are the documented gates.
pub fn accept_blob(bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_UPGRADE_BLOB_BYTES {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Contents of a regular file at HEAD, bounded before allocating its contents.
pub fn file_at_head(root: &Path, path: &str) -> Option<String> {
    if !safe_relative_path(path) {
        return None;
    }
    let entry = crate::git_service::git_output_bounded(
        Command::new("git")
            .current_dir(root)
            .args(["--literal-pathspecs", "ls-tree", "-z", "HEAD", "--", path])
            .env("GIT_TERMINAL_PROMPT", "0"),
        16 * 1024,
    )?;
    let entry = std::str::from_utf8(&entry).ok()?.strip_suffix('\0')?;
    let (metadata, found_path) = entry.split_once('\t')?;
    let mut fields = metadata.split_whitespace();
    let mode = fields.next()?;
    if !matches!(mode, "100644" | "100755") || fields.next()? != "blob" || found_path != path {
        return None;
    }
    let oid = fields.next()?;
    let bytes = crate::git_service::git_output_bounded(
        Command::new("git")
            .current_dir(root)
            .args(["cat-file", "blob", oid])
            .env("GIT_TERMINAL_PROMPT", "0"),
        MAX_UPGRADE_BLOB_BYTES,
    )?;
    accept_blob(&bytes)
}

/// Working-tree bytes, rejecting traversal, symlinks and non-regular files.
pub fn file_in_worktree(root: &Path, path: &str) -> Option<String> {
    if !safe_relative_path(path) {
        return None;
    }
    let mut full_path = root.to_path_buf();
    for component in Path::new(path).components() {
        full_path.push(component);
        let metadata = std::fs::symlink_metadata(&full_path).ok()?;
        if metadata.file_type().is_symlink() {
            return None;
        }
    }
    let metadata = std::fs::symlink_metadata(&full_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_UPGRADE_BLOB_BYTES as u64 {
        return None;
    }
    let file = std::fs::File::open(full_path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((MAX_UPGRADE_BLOB_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    accept_blob(&bytes)
}

/// Load enrichment for one file. The session validates it against its patch.
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
    // Keep the original source for highlighting. Line display uses the same
    // str::lines semantics as the patch parser, never to recompute the diff.
    let path = job.new_path.as_deref().or(job.old_path.as_deref())?;
    let lang = syntax::language_for_path(path);
    let spans = |text: &str| match lang {
        Some(lang) if !text.is_empty() => syntax::highlight_lines(lang, text),
        _ => Vec::new(),
    };
    Some(UpgradedFile {
        file_ix: job.file_ix,
        old_lines: old_text.lines().map(str::to_string).collect(),
        upgrade: FileUpgrade {
            old_spans: spans(&old_text),
            new_spans: spans(&new_text),
            new_lines: new_text.lines().map(str::to_string).collect(),
            expanded: std::collections::HashSet::new(),
        },
    })
}

impl UpgradedFile {
    /// Every patch row must match its numbered source line, and every hidden
    /// gap must still be shared context. Otherwise keep the patch-only view.
    pub fn matches_hunks(&self, hunks: &[Hunk]) -> bool {
        let old = &self.old_lines;
        let new = &self.upgrade.new_lines;
        let (mut old_cursor, mut new_cursor) = (0usize, 0usize);
        for hunk in hunks {
            let start = |no: u32, count: u32| {
                if count == 0 {
                    Some(no as usize)
                } else {
                    no.checked_sub(1).map(|n| n as usize)
                }
            };
            let (Some(old_start), Some(new_start)) = (
                start(hunk.old_start, hunk.old_count),
                start(hunk.new_start, hunk.new_count),
            ) else {
                return false;
            };
            let (Some(old_gap), Some(new_gap)) = (
                old.get(old_cursor..old_start),
                new.get(new_cursor..new_start),
            ) else {
                return false;
            };
            if old_gap != new_gap {
                return false;
            }
            old_cursor = old_start;
            new_cursor = new_start;
            for row in &hunk.rows {
                let matches = |lines: &[String], cursor: usize, no: u32, text: &str| {
                    no as usize == cursor + 1 && lines.get(cursor).map(String::as_str) == Some(text)
                };
                match row {
                    DiffRow::Context {
                        old_no,
                        new_no,
                        text,
                    } => {
                        if !matches(old, old_cursor, *old_no, text)
                            || !matches(new, new_cursor, *new_no, text)
                        {
                            return false;
                        }
                        old_cursor += 1;
                        new_cursor += 1;
                    }
                    DiffRow::Removed { old_no, text, .. } => {
                        if !matches(old, old_cursor, *old_no, text) {
                            return false;
                        }
                        old_cursor += 1;
                    }
                    DiffRow::Added { new_no, text, .. } => {
                        if !matches(new, new_cursor, *new_no, text) {
                            return false;
                        }
                        new_cursor += 1;
                    }
                }
            }
            if old_cursor - old_start != hunk.old_count as usize
                || new_cursor - new_start != hunk.new_count as usize
            {
                return false;
            }
        }
        old.get(old_cursor..) == new.get(new_cursor..)
    }
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
        // Build an isolated fixture without invoking reference-update hooks,
        // which may be tied to the developer's active workspace.
        if args.first() == Some(&"init") {
            fs::create_dir_all(dir.join(".git/objects")).unwrap();
            fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
            fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            // Keep fixture bytes stable across host Git installations. Git for
            // Windows commonly defaults core.autocrlf=true, which otherwise
            // normalizes the staged LF blob and can erase newline-only edits.
            fs::write(dir.join(".git/config"), "[core]\n\tautocrlf = false\n").unwrap();
            return true;
        }
        if args.first() == Some(&"commit") {
            let tree = Command::new("git")
                .arg("write-tree")
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(tree.status.success());
            let commit = Command::new("git")
                .args([
                    "commit-tree",
                    String::from_utf8(tree.stdout).unwrap().trim(),
                    "-m",
                    "fixture",
                ])
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(commit.status.success());
            fs::write(dir.join(".git/refs/heads/main"), commit.stdout).unwrap();
            return true;
        }
        let slot = std::env::var("GIT_CONFIG_COUNT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_COUNT", (slot + 1).to_string())
            .env(format!("GIT_CONFIG_KEY_{slot}"), "commit.gpgsign")
            .env(format!("GIT_CONFIG_VALUE_{slot}"), "false")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn session_for_edit(root: &Path, old: &str, new: &str) -> super::super::DiffSession {
        assert!(git(root, &["init", "-q", "-b", "main"]));
        fs::write(root.join("note.rs"), old).unwrap();
        assert!(git(root, &["add", "note.rs"]));
        assert!(git(root, &["commit", "-qm", "init"]));
        fs::write(root.join("note.rs"), new).unwrap();
        let crate::git_service::PatchOutcome::Ready(ready) =
            crate::git_service::fetch_worktree_patch(root)
        else {
            panic!("expected a Git patch");
        };
        super::super::DiffSession::from_ready(ready, super::super::ViewMode::Unified)
    }

    fn apply_note_upgrade(session: &mut super::super::DiffSession) {
        let job = UpgradeJob {
            file_ix: 0,
            old_path: Some("note.rs".into()),
            new_path: Some("note.rs".into()),
            status: FileStatus::Modified,
        };
        session.apply_upgrades(run_upgrade(&session.root, vec![job]));
    }

    #[test]
    fn upgrade_preserves_git_newline_only_hunks_and_counts() {
        for (old, new) in [
            ("fn main() {}\n", "fn main() {}"),
            ("fn main() {}\r\n", "fn main() {}\n"),
        ] {
            let tmp = tempfile::TempDir::new().unwrap();
            let mut session = session_for_edit(tmp.path(), old, new);
            let before = format!("{:?}", session.parsed);
            let counts = (session.additions, session.deletions);
            assert_eq!(counts, (1, 1));
            apply_note_upgrade(&mut session);
            assert_eq!(format!("{:?}", session.parsed), before);
            assert_eq!((session.additions, session.deletions), counts);
            assert!(session.upgrades.contains_key(&0));
        }
    }

    #[test]
    fn upgrade_rejects_stale_patch_lines_and_hidden_context() {
        for stale in ["line 10 changed\n", "line 1\n"] {
            let tmp = tempfile::TempDir::new().unwrap();
            let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
            let new = old.replace("line 10\n", "line 10 changed\n");
            let mut session = session_for_edit(tmp.path(), &old, &new);
            let before = format!("{:?}", session.parsed);
            fs::write(
                tmp.path().join("note.rs"),
                new.replace(stale, "stale content\n"),
            )
            .unwrap();
            apply_note_upgrade(&mut session);
            assert_eq!(format!("{:?}", session.parsed), before);
            assert!(
                session.upgrades.is_empty(),
                "stale content must not enrich patch"
            );
        }
    }

    #[test]
    fn upgrade_preserves_git_context_and_gap_expansion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 10\n", "line 10 changed\n");
        let mut session = session_for_edit(tmp.path(), &old, &new);
        session.parsed.files[0].hunks[0].section = "Git section stays authoritative".into();
        let before = format!("{:?}", session.parsed);
        apply_note_upgrade(&mut session);
        assert_eq!(format!("{:?}", session.parsed), before);
        assert!(session.expand_gap(0, 0).is_some());
        assert!(session.rows.iter().any(|row| matches!(row,
            super::super::DisplayRow::Line { old_no: Some(1), new_no: Some(1), text, .. }
                if text == "line 1")));
    }

    #[test]
    fn worktree_rejects_absolute_and_parent_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("repo");
        fs::create_dir(&root).unwrap();
        fs::write(tmp.path().join("outside"), "outside\n").unwrap();
        assert!(file_in_worktree(&root, "../outside").is_none());
        assert!(file_in_worktree(&root, tmp.path().join("outside").to_str().unwrap()).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn worktree_rejects_symlinks_and_symlinked_parents() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("dir")).unwrap();
        fs::write(root.join("dir/note"), "text\n").unwrap();
        std::os::unix::fs::symlink("dir/note", root.join("link")).unwrap();
        std::os::unix::fs::symlink("dir", root.join("linked-dir")).unwrap();
        assert!(file_in_worktree(root, "link").is_none());
        assert!(file_in_worktree(root, "linked-dir/note").is_none());
        assert!(file_in_worktree(root, "dir").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn head_rejects_symlink_blobs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        assert!(git(root, &["init", "-q", "-b", "main"]));
        std::os::unix::fs::symlink("missing", root.join("link")).unwrap();
        assert!(git(root, &["add", "link"]));
        assert!(git(root, &["commit", "-qm", "init"]));
        assert!(file_at_head(root, "link").is_none());
    }

    #[test]
    fn upgrade_blob_readers_reject_oversize_and_accept_exact_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        assert!(git(root, &["init", "-q", "-b", "main"]));
        fs::write(root.join("large"), vec![b'x'; MAX_UPGRADE_BLOB_BYTES + 1]).unwrap();
        fs::write(root.join("exact"), vec![b'x'; MAX_UPGRADE_BLOB_BYTES]).unwrap();
        assert!(git(root, &["add", "."]));
        assert!(git(root, &["commit", "-qm", "init"]));
        assert!(file_at_head(root, "large").is_none());
        assert!(file_in_worktree(root, "large").is_none());
        assert_eq!(
            file_at_head(root, "exact").unwrap().len(),
            MAX_UPGRADE_BLOB_BYTES
        );
        assert_eq!(
            file_in_worktree(root, "exact").unwrap().len(),
            MAX_UPGRADE_BLOB_BYTES
        );
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
        let crate::git_service::PatchOutcome::Ready(ready) =
            crate::git_service::fetch_worktree_patch(root)
        else {
            panic!("expected patch");
        };
        let hunks = &ready
            .parsed
            .files
            .iter()
            .find(|file| file.display_path() == "note.rs")
            .unwrap()
            .hunks;
        assert!(text.matches_hunks(hunks));
        assert!(text.upgrade.new_lines.len() >= 20);
        let (_, _, hidden) = diff_core::gap_span(hunks, 0, text.upgrade.new_lines.len() as u32);
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
