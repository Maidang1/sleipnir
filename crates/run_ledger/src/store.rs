//! `runs.json` persistence: versioned, atomic, corruption-tolerant.

use crate::ledger::Retention;
use crate::run::{Run, RunId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const RUNS_VERSION: u32 = 1;

const MS_PER_DAY: u64 = 24 * 60 * 60 * 1000;

#[derive(Serialize, Deserialize)]
pub struct RunsFile {
    pub version: u32,
    /// First-persist notice has been shown (Task 8 reads this).
    #[serde(default)]
    pub announced: bool,
    pub runs: Vec<Run>,
}

/// Alongside `session.json`.
pub fn default_runs_path(config_dir: &Path) -> PathBuf {
    config_dir.join("runs.json")
}

/// Load the ledger. Missing file → `(empty, announced=false)`; corrupt or
/// unrecognized version → quarantined as `.bak`, then empty.
///
/// **Never returns `Err`**: the startup path must not be blocked by the ledger
/// (spec §5). Returns `(runs, announced)`.
pub fn load_runs(path: &Path) -> (Vec<Run>, bool) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (Vec::new(), false),
        Err(_) => {
            quarantine(path);
            return (Vec::new(), false);
        }
    };

    match parse_runs_file(&bytes) {
        Some(file) => (file.runs, file.announced),
        None => {
            quarantine(path);
            (Vec::new(), false)
        }
    }
}

/// Write under a cross-process lock, merging by `RunId` while the lock is held.
///
/// The lock makes the read/modify/write cycle atomic across application
/// instances, so a concurrent save cannot drop a run. Writing `announced: true`
/// is inherent to persisting: reaching this function *is* the first persist that
/// the notice describes.
pub fn save_runs(path: &Path, runs: &[Run], retention: Retention) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(sibling_path(path, ".lock"))?;
    lock.lock()?;
    let result = save_runs_locked(path, runs, retention);
    // Report an unlock failure only when the write itself succeeded, so the
    // original error is never masked.
    match (result, lock.unlock()) {
        (Ok(()), Err(err)) => Err(err),
        (result, _) => result,
    }
}

fn save_runs_locked(path: &Path, runs: &[Run], retention: Retention) -> io::Result<()> {
    let (disk_runs, _) = load_runs(path);
    let mut merged = merge_by_id(disk_runs, runs);
    merged.sort_by_key(|run| run.started_at_unix_ms);
    apply_retention(&mut merged, retention);

    let file = RunsFile {
        version: RUNS_VERSION,
        announced: true,
        runs: merged,
    };
    let json = serde_json::to_vec_pretty(&file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    // A single staging name is safe: the lock guarantees we are the only writer,
    // so the only file that can be here is a leftover from a crashed run.
    let tmp = sibling_path(path, ".tmp");
    let staged = stage_then_publish(&tmp, path, &json);
    if staged.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    staged
}

/// Durably write `json` to `tmp`, then atomically move it onto `path`.
fn stage_then_publish(tmp: &Path, path: &Path, json: &[u8]) -> io::Result<()> {
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    output.write_all(json)?;
    output.sync_all()?;
    drop(output);
    // `rename` replaces the destination on both POSIX and Windows.
    fs::rename(tmp, path)?;
    sync_parent(path)
}

fn parse_runs_file(bytes: &[u8]) -> Option<RunsFile> {
    let file: RunsFile = serde_json::from_slice(bytes).ok()?;
    (file.version == RUNS_VERSION).then_some(file)
}

fn merge_by_id(disk: Vec<Run>, incoming: &[Run]) -> Vec<Run> {
    let mut by_id: HashMap<RunId, Run> = disk.into_iter().map(|run| (run.id, run)).collect();
    for run in incoming {
        by_id.insert(run.id, run.clone());
    }
    by_id.into_values().collect()
}

fn apply_retention(runs: &mut Vec<Run>, retention: Retention) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let cutoff = now.saturating_sub(retention.days.saturating_mul(MS_PER_DAY));
    runs.retain(|run| run.started_at_unix_ms >= cutoff);
    if runs.len() > retention.max_runs {
        let drop = runs.len() - retention.max_runs;
        runs.drain(..drop);
    }
}

fn quarantine(path: &Path) {
    let _ = fs::rename(path, bak_path(path));
}

fn bak_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_owned();
    raw.push(".bak");
    PathBuf::from(raw)
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_owned();
    raw.push(suffix);
    PathBuf::from(raw)
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::RunState;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn make_run(command: &str, started_at_unix_ms: u64) -> Run {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(),
            "launch_id": uuid::Uuid::new_v4(),
            "pane": uuid::Uuid::new_v4(),
            "command": command,
            "started_at_unix_ms": started_at_unix_ms,
            "duration": { "secs": 1, "nanos": 0 },
            "exit_code": 0,
            "state": "succeeded",
        }))
        .expect("test Run")
    }

    fn runs_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("runs.json");
        (dir, path)
    }

    #[test]
    fn save_then_load_round_trips() {
        let (_dir, path) = runs_path();
        let now = now_ms();
        let a = make_run("cargo test", now - 10);
        let b = make_run("npm test", now);
        save_runs(&path, &[a.clone(), b.clone()], Retention::default()).unwrap();
        let (loaded, announced) = load_runs(&path);
        assert!(announced);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, a.id);
        assert_eq!(loaded[0].command, "cargo test");
        assert_eq!(loaded[0].state, RunState::Succeeded);
        assert_eq!(loaded[0].started_at_unix_ms, a.started_at_unix_ms);
        assert_eq!(loaded[1].id, b.id);
        assert_eq!(loaded[1].command, "npm test");
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let (_dir, path) = runs_path();
        let (runs, announced) = load_runs(&path);
        assert!(runs.is_empty());
        assert!(!announced);
    }

    #[test]
    fn corrupt_file_is_renamed_to_bak_and_load_is_empty() {
        let (_dir, path) = runs_path();
        fs::write(&path, "NOT JSON {{{").unwrap();
        let (runs, announced) = load_runs(&path);
        assert!(runs.is_empty());
        assert!(!announced);
        assert!(
            bak_path(&path).exists(),
            "corrupt file must be renamed to .bak"
        );
        assert!(
            !path.exists(),
            "corrupt original should be gone after quarantine"
        );
    }

    #[test]
    fn unknown_version_is_treated_as_corrupt() {
        let (_dir, path) = runs_path();
        fs::write(&path, r#"{"version":99,"announced":true,"runs":[]}"#).unwrap();
        let (runs, announced) = load_runs(&path);
        assert!(runs.is_empty());
        assert!(!announced);
        assert!(bak_path(&path).exists());
    }

    #[test]
    fn save_merges_with_on_disk_runs_by_id() {
        let (_dir, path) = runs_path();
        let now = now_ms();
        let a = make_run("first", now - 50);
        let b = make_run("second", now - 10);
        save_runs(&path, std::slice::from_ref(&a), Retention::default()).unwrap();
        save_runs(&path, std::slice::from_ref(&b), Retention::default()).unwrap();
        let (loaded, _) = load_runs(&path);
        let cmds: Vec<_> = loaded.iter().map(|r| r.command.as_str()).collect();
        assert_eq!(cmds, ["first", "second"], "union by id, oldest first");
        assert_eq!(loaded[0].id, a.id);
        assert_eq!(loaded[1].id, b.id);
    }

    #[test]
    fn concurrent_saves_preserve_every_run() {
        use std::sync::{Arc, Barrier};

        let (_dir, path) = runs_path();
        let now = now_ms();
        let workers = 12;
        let barrier = Arc::new(Barrier::new(workers));
        let mut threads = Vec::new();
        for i in 0..workers {
            let path = path.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                let run = make_run(&format!("worker-{i}"), now + i as u64);
                barrier.wait();
                save_runs(&path, &[run], Retention::default()).unwrap();
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }

        let (loaded, announced) = load_runs(&path);
        assert!(
            announced,
            "any successful persist marks the notice as shown"
        );
        assert_eq!(loaded.len(), workers);
        for i in 0..workers {
            assert!(
                loaded
                    .iter()
                    .any(|run| run.command == format!("worker-{i}"))
            );
        }
    }

    #[test]
    fn save_applies_retention_before_writing() {
        let (_dir, path) = runs_path();
        let now = now_ms();
        let runs: Vec<_> = (0..600)
            .map(|i| make_run(&format!("c{i}"), now - 600 + i as u64))
            .collect();
        save_runs(
            &path,
            &runs,
            Retention {
                days: 7,
                max_runs: 500,
            },
        )
        .unwrap();
        let (loaded, _) = load_runs(&path);
        let cmds: Vec<_> = loaded.iter().map(|r| r.command.as_str()).collect();
        assert_eq!(cmds.len(), 500);
        assert_eq!(cmds.first().copied(), Some("c100"));
        assert_eq!(cmds.last().copied(), Some("c599"));
    }

    #[test]
    fn seen_and_anchor_fields_are_not_serialized() {
        let (_dir, path) = runs_path();
        let mut run = make_run("secret-ish", now_ms());
        run.seen = true;
        save_runs(&path, &[run], Retention::default()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("seen"),
            "Attention flag must stay in-memory only: {text}"
        );
        assert!(
            !text.contains("started_at_mono_ms"),
            "monotonic clock must not be persisted: {text}"
        );
        assert!(
            !text.contains("anchor"),
            "Anchor is process-local and must not appear: {text}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn unix_permissions_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = runs_path();
        save_runs(&path, &[make_run("chmod", now_ms())], Retention::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
