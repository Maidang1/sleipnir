//! Atomic publication for the small on-disk JSON stores.
//!
//! `plugin-grants.json` and `runs.json` share one write discipline: stage to a
//! sibling `.tmp` (`0600` on Unix), `rename` over the target, fsync the parent
//! directory, and quarantine a corrupt file to `.bak` rather than failing open.
//! This crate is that discipline, std-only.

#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Options for [`save_atomic_with`].
#[derive(Clone, Copy, Debug)]
pub struct SaveOptions {
    /// Unix file-permission mode for the staged temp file. Default `0o600`.
    #[cfg(unix)]
    pub mode: u32,
    /// Unix directory-permission mode applied to `create_dir_all`-created
    /// parents. `None` skips the explicit `set_permissions` call (the umask
    /// decides). Default `None`.
    #[cfg(unix)]
    pub parent_mode: Option<u32>,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            #[cfg(unix)]
            mode: 0o600,
            #[cfg(unix)]
            parent_mode: None,
        }
    }
}

/// Write `bytes` to `path` atomically: parent dirs are created, the payload is
/// staged to a sibling `.tmp`, then renamed over `path`. A failed publish
/// deletes the leftover tmp.
pub fn save_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    save_atomic_with(path, bytes, SaveOptions::default())
}

/// Like [`save_atomic`] but with explicit options for Unix permissions.
pub fn save_atomic_with(path: &Path, bytes: &[u8], opts: SaveOptions) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            if let Some(dir_mode) = opts.parent_mode {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(dir_mode))?;
            }
        }
    }
    let tmp = sibling_path(path, ".tmp");
    let staged = stage_then_publish(&tmp, path, bytes, &opts);
    if staged.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    staged
}

/// Acquire an exclusive advisory lock on a sibling `.lock` file of `path`, run
/// `body`, then release the lock. Parent directories are created if missing.
///
/// The lock is cross-process (advisory `File::lock`). On drop or error the
/// lock is released.
pub fn with_file_lock<T>(path: &Path, body: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let lock_path = sibling_path(path, ".lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.lock()?;

    let result = body();
    match (result, lock.unlock()) {
        (Ok(_), Err(err)) => Err(err),
        (result, _) => result,
    }
}

/// Move a corrupt file aside so the next load starts empty.
pub fn quarantine(path: &Path) {
    let _ = fs::rename(path, bak_path(path));
}

pub fn bak_path(path: &Path) -> PathBuf {
    sibling_path(path, ".bak")
}

pub fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_owned();
    raw.push(suffix);
    PathBuf::from(raw)
}

/// Durably write `bytes` to `tmp`, then atomically move it onto `path`.
fn stage_then_publish(
    tmp: &Path,
    path: &Path,
    bytes: &[u8],
    _opts: &SaveOptions,
) -> io::Result<()> {
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        output.set_permissions(fs::Permissions::from_mode(_opts.mode))?;
    }
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    // `rename` replaces the destination on both POSIX and Windows.
    fs::rename(tmp, path)?;
    sync_parent(path)
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
    use std::sync::mpsc;

    #[test]
    fn save_atomic_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        save_atomic(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(!sibling_path(&path, ".tmp").exists());
    }

    #[test]
    fn save_atomic_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("test.json");
        save_atomic(&path, b"nested").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "nested");
    }

    #[test]
    #[cfg(unix)]
    fn save_atomic_default_mode_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("perms.json");
        save_atomic(&path, b"x").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    #[cfg(unix)]
    fn save_atomic_with_parent_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("locked");
        let path = parent.join("test.json");
        save_atomic_with(
            &path,
            b"x",
            SaveOptions {
                mode: 0o600,
                parent_mode: Some(0o700),
            },
        )
        .unwrap();
        let mode = fs::metadata(&parent).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn with_file_lock_runs_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.json");
        let result = with_file_lock(&path, || Ok(42)).unwrap();
        assert_eq!(result, 42);
        assert!(sibling_path(&path, ".lock").exists());
    }

    #[test]
    fn with_file_lock_serializes_access() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        fs::write(&path, "0").unwrap();

        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let p1 = path.clone();
        let t1 = std::thread::spawn(move || {
            with_file_lock(&p1, || {
                ready_tx.send(()).unwrap();
                release_rx
                    .recv()
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                fs::write(&p1, "1")?;
                Ok(())
            })
            .unwrap();
        });

        ready_rx.recv().unwrap();

        let p2 = path.clone();
        let t2 = std::thread::spawn(move || {
            with_file_lock(&p2, || {
                let val = fs::read_to_string(&p2)?;
                assert_eq!(val, "1", "thread 2 must see thread 1's write");
                fs::write(&p2, "2")?;
                Ok(())
            })
            .unwrap();
        });

        release_tx.send(()).unwrap();
        t1.join().unwrap();
        t2.join().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "2");
    }

    #[test]
    fn quarantine_renames_to_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.json");
        fs::write(&path, "bad").unwrap();
        quarantine(&path);
        assert!(!path.exists());
        assert!(bak_path(&path).exists());
    }
}
