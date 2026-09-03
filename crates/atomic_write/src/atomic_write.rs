//! Atomic publication for the small on-disk JSON stores.
//!
//! `plugin-grants.json` and `runs.json` share one write discipline: stage to a
//! sibling `.tmp` (`0600` on Unix), `rename` over the target, fsync the parent
//! directory, and quarantine a corrupt file to `.bak` rather than failing open.
//! This crate is that discipline, std-only.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` atomically: parent dirs are created, the payload is
/// staged to a sibling `.tmp`, then renamed over `path`. A failed publish
/// deletes the leftover tmp.
pub fn save_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp = sibling_path(path, ".tmp");
    let staged = stage_then_publish(&tmp, path, bytes);
    if staged.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    staged
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
fn stage_then_publish(tmp: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
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
