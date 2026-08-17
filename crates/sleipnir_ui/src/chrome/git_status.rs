//! Git work-tree snapshot for the side rail: branch + dirty *line* counts.
//!
//! Branch is a cheap HEAD read. Line stats come from `git diff --numstat HEAD`
//! on a background thread — the render path only returns the last snapshot.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::workspace::git_root_in;

const CACHE_TTL: Duration = Duration::from_secs(3);

/// Branch name (or detached HEAD short sha) plus dirty line counts vs HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSnapshot {
    pub branch: String,
    /// Lines inserted vs `HEAD` (`git diff --numstat`).
    pub added: u32,
    /// Lines deleted vs `HEAD`.
    pub deleted: u32,
}



pub trait GitFs {
    fn exists(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
}

/// Real filesystem. Used by the rail.
pub struct RealFs;

impl GitFs for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }
}

#[derive(Clone)]
struct CacheEntry {
    at: Instant,
    snap: Option<GitSnapshot>,
    /// False = branch-only placeholder; a background numstat is still due.
    complete: bool,
}

static CACHE: Mutex<Option<HashMap<PathBuf, CacheEntry>>> = Mutex::new(None);
static IN_FLIGHT: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn lock_cache() -> std::sync::MutexGuard<'static, Option<HashMap<PathBuf, CacheEntry>>> {
    CACHE.lock().unwrap_or_else(|p| p.into_inner())
}

fn lock_inflight() -> std::sync::MutexGuard<'static, Option<HashSet<PathBuf>>> {
    IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner())
}

/// Last-known snapshot for render. Never runs `git` or stats the work tree
/// on the calling thread — that was flashing the rail on every keystroke.
pub fn cached_git_snapshot(cwd: &Path) -> Option<GitSnapshot> {
    let root = git_root_in(cwd, |path| RealFs.exists(path))?;
    let now = Instant::now();
    let cached = lock_cache()
        .as_ref()
        .and_then(|map| map.get(&root).cloned());
    let need_refresh = cached
        .as_ref()
        .is_none_or(|entry| !entry.complete || now.saturating_duration_since(entry.at) >= CACHE_TTL);
    if need_refresh {
        schedule_refresh(root.clone());
    }
    if let Some(entry) = cached {
        return entry.snap;
    }
    let snap = branch_only(&root);
    lock_cache()
        .get_or_insert_with(HashMap::new)
        .insert(
            root,
            CacheEntry {
                at: now,
                snap: snap.clone(),
                complete: false,
            },
        );
    snap
}

fn branch_only(root: &Path) -> Option<GitSnapshot> {
    git_snapshot_in(root, &RealFs)
}

fn schedule_refresh(root: PathBuf) {
    {
        let mut guard = lock_inflight();
        let set = guard.get_or_insert_with(HashSet::new);
        if !set.insert(root.clone()) {
            return;
        }
    }
    let _ = std::thread::Builder::new()
        .name("sleipnir-git-numstat".into())
        .spawn(move || {
            let snap = git_snapshot_with_numstat(&root);
            lock_cache()
                .get_or_insert_with(HashMap::new)
                .insert(
                    root.clone(),
                    CacheEntry {
                        at: Instant::now(),
                        snap,
                        complete: true,
                    },
                );
            if let Some(set) = lock_inflight().as_mut() {
                set.remove(&root);
            }
        });
}

/// Snapshot the work tree that contains `cwd`, or `None` if it is not a repo.
/// Line counts require a `git` binary; missing git yields a clean branch.
#[cfg(test)]
fn git_snapshot(cwd: &Path) -> Option<GitSnapshot> {
    let root = git_root_in(cwd, |path| RealFs.exists(path))?;
    git_snapshot_with_numstat(&root)
}

fn git_snapshot_with_numstat(root: &Path) -> Option<GitSnapshot> {
    let mut snap = git_snapshot_in(root, &RealFs)?;
    let (added, deleted) = git_numstat(root);
    snap.added = added;
    snap.deleted = deleted;
    Some(snap)
}

/// Branch only. Dirty line counts stay 0 — callers that need them use
/// [`cached_git_snapshot`].
pub fn git_snapshot_in(cwd: &Path, fs: &impl GitFs) -> Option<GitSnapshot> {
    let root = git_root_in(cwd, |path| fs.exists(path))?;
    let gitdir = resolve_gitdir(&root, fs)?;
    let branch = read_branch(&gitdir, fs)?;
    Some(GitSnapshot {
        branch,
        added: 0,
        deleted: 0,
    })
}

fn git_numstat(root: &Path) -> (u32, u32) {
    let output = std::process::Command::new("git")
        .args(["diff", "--numstat", "HEAD"])
        .current_dir(root)
        .output();
    match output {
        Ok(out) if out.status.success() => parse_numstat(&String::from_utf8_lossy(&out.stdout)),
        _ => (0, 0),
    }
}

/// Sum insertions / deletions from `git diff --numstat` text. Binary rows
/// (`-	-	file`) are skipped.
pub fn parse_numstat(text: &str) -> (u32, u32) {
    let mut added = 0u32;
    let mut deleted = 0u32;
    for line in text.lines() {
        let mut cols = line.split('\t');
        let Some(ins) = cols.next() else {
            continue;
        };
        let Some(del) = cols.next() else {
            continue;
        };
        if let Ok(n) = ins.parse::<u32>() {
            added = added.saturating_add(n);
        }
        if let Ok(n) = del.parse::<u32>() {
            deleted = deleted.saturating_add(n);
        }
    }
    (added, deleted)
}

fn resolve_gitdir(root: &Path, fs: &impl GitFs) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if fs.is_dir(&dot_git) {
        return Some(dot_git);
    }
    let raw = fs.read(&dot_git)?;
    let text = String::from_utf8_lossy(&raw);
    for line in text.lines() {
        let rest = line.trim();
        let Some(dir) = rest.strip_prefix("gitdir:") else {
            continue;
        };
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        let path = Path::new(dir);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        return Some(resolved);
    }
    None
}

fn read_branch(gitdir: &Path, fs: &impl GitFs) -> Option<String> {
    let raw = fs.read(&gitdir.join("HEAD"))?;
    let head = String::from_utf8_lossy(&raw).trim().to_string();
    if head.is_empty() {
        return None;
    }
    if let Some(rest) = head.strip_prefix("ref: ") {
        let name = rest
            .strip_prefix("refs/heads/")
            .unwrap_or(rest)
            .trim()
            .to_string();
        if name.is_empty() {
            return None;
        }
        return Some(name);
    }
    // Detached HEAD: object id. Show the short name.
    let sha = head
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .take(7)
        .collect::<String>();
    if sha.len() < 4 {
        return None;
    }
    Some(sha)
}



#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use tempfile::TempDir;

    fn write_repo(dir: &Path, head: &str) {
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), head).unwrap();
    }

    #[test]
    fn non_repo_is_none() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("notes.txt"), "hi").unwrap();
        assert_eq!(git_snapshot(tmp.path()), None);
    }

    #[test]
    fn branch_from_head_ref_on_clean_tree() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("README.md"), "hello").unwrap();
        write_repo(tmp.path(), "ref: refs/heads/main\n");
        let snap = git_snapshot(tmp.path()).expect("repo");
        assert_eq!(snap.branch, "main");
        assert_eq!((snap.added, snap.deleted), (0, 0), "{snap:?}");
    }

    #[test]
    fn detached_head_uses_short_sha() {
        let tmp = TempDir::new().unwrap();
        write_repo(
            tmp.path(),
            "abcdef1234567890abcdef1234567890abcdef12\n",
        );
        let snap = git_snapshot(tmp.path()).expect("repo");
        assert_eq!(snap.branch, "abcdef1");
        assert_eq!((snap.added, snap.deleted), (0, 0));
    }

    #[test]
    fn parse_numstat_sums_line_counts_and_skips_binary() {
        let text = "\
10\t2\tREADME.md
3\t0\tsrc/lib.rs
-\t-\tlogo.png
110\t3\tCHANGELOG.md
";
        assert_eq!(parse_numstat(text), (123, 5));
        assert_eq!(parse_numstat(""), (0, 0));
        let (added, deleted) = parse_numstat("110\t3\tCHANGELOG.md\n10\t2\tREADME.md\n");
        assert_eq!((added, deleted), (120, 5));
    }

    #[test]
    fn git_snapshot_counts_diff_lines_not_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if !git(&["init", "-q"]) {
            return;
        }
        fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        assert!(git(&["add", "a.txt"]));
        assert!(git(&["commit", "-qm", "init"]));
        fs::write(root.join("a.txt"), "one\ntwo changed\nthree\nfour\nfive\n").unwrap();
        let snap = git_snapshot(root).expect("repo");
        assert_eq!(snap.added, 3, "inserted lines: {snap:?}");
        assert_eq!(snap.deleted, 1, "deleted lines: {snap:?}");
    }

    struct MapFs {
        files: HashMap<PathBuf, Vec<u8>>,
        dirs: HashSet<PathBuf>,
    }

    impl MapFs {
        fn new() -> Self {
            Self {
                files: HashMap::new(),
                dirs: HashSet::new(),
            }
        }

        fn dir(&mut self, path: &str) {
            self.dirs.insert(PathBuf::from(path));
        }

        fn file(&mut self, path: &str, body: impl AsRef<[u8]>) {
            let path = PathBuf::from(path);
            if let Some(parent) = path.parent() {
                self.dirs.insert(parent.to_path_buf());
            }
            self.files.insert(path, body.as_ref().to_vec());
        }
    }

    impl GitFs for MapFs {
        fn exists(&self, path: &Path) -> bool {
            self.files.contains_key(path) || self.dirs.contains(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }

        fn read(&self, path: &Path) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
    }

    #[test]
    fn injected_fs_reads_branch_and_stays_clean() {
        let mut fs = MapFs::new();
        fs.dir("/repo");
        fs.dir("/repo/.git");
        fs.file("/repo/.git/HEAD", "ref: refs/heads/main\n");
        fs.file("/repo/README.md", "hello");
        let snap = git_snapshot_in(Path::new("/repo"), &fs).expect("repo");
        assert_eq!(snap.branch, "main");
        assert_eq!((snap.added, snap.deleted), (0, 0));
    }

    #[test]
    fn cached_snapshot_does_not_block_on_git() {
        // Render path must return immediately even on a real checkout.
        let start = Instant::now();
        let _ = cached_git_snapshot(Path::new(env!("CARGO_MANIFEST_DIR")));
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "cached_git_snapshot must not run git/numstat on the UI thread; took {elapsed:?}"
        );
    }
}
