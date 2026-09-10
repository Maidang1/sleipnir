use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
use std::{error::Error as StdError, fmt};

use crate::transaction::{HealthMarker, Phase, Transaction, TransactionError, save_atomic};
#[cfg(target_os = "macos")]
use rand::RngCore as _;
#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::AsRawFd as _;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt as _;
#[cfg(target_os = "macos")]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
#[cfg(target_os = "macos")]
use std::process::{Child, Command, ExitStatus, Stdio};

#[derive(Deserialize, Serialize)]
struct ActivePointer {
    schema_version: u32,
    transaction_path: PathBuf,
}

#[cfg(unix)]
const UPDATE_LOCK_BUSY: &str = "update transaction is busy";

#[cfg(unix)]
pub struct UpdateLock(File);

#[cfg(unix)]
impl UpdateLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        Self::acquire_with(root, false)
    }

    fn try_acquire(root: &Path) -> Result<Self, String> {
        Self::acquire_with(root, true)
    }

    fn acquire_with(root: &Path, nonblocking: bool) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(target_os = "macos")]
        options.mode(0o600);
        let file = options.open(root.join("lock")).map_err(|e| e.to_string())?;
        let mut operation = libc::LOCK_EX;
        if nonblocking {
            operation |= libc::LOCK_NB;
        }
        // SAFETY: flock operates on this process-owned descriptor.
        if unsafe { libc::flock(file.as_raw_fd(), operation) } != 0 {
            let error = std::io::Error::last_os_error();
            if nonblocking && error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(UPDATE_LOCK_BUSY.into());
            }
            return Err(error.to_string());
        }
        Ok(Self(file))
    }

    #[cfg(target_os = "macos")]
    pub fn acquire_for_transaction(transaction_path: &Path) -> Result<Self, String> {
        let root = updates_root()?;
        let canonical_root = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
        let metadata = std::fs::symlink_metadata(transaction_path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink()
            || transaction_path.file_name().and_then(|name| name.to_str())
                != Some("transaction.json")
            || !std::fs::canonicalize(transaction_path)
                .map_err(|e| e.to_string())?
                .starts_with(&canonical_root)
        {
            return Err("transaction path is outside the update root or is a symlink".into());
        }
        Self::acquire(&root)
    }
}

#[cfg(unix)]
impl Drop for UpdateLock {
    fn drop(&mut self) {
        // SAFETY: unlocks the descriptor retained by this guard.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(unix)]
fn with_update_lock<T>(
    root: &Path,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _lock = UpdateLock::try_acquire(root)?;
    action()
}

#[cfg(not(unix))]
fn with_update_lock<T>(_: &Path, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    action()
}

fn active_pointer_path(root: &Path) -> PathBuf {
    root.join("active.json")
}

fn write_active_pointer_unlocked(root: &Path, transaction_path: &Path) -> Result<(), String> {
    if !transaction_path.is_absolute() {
        return Err("transaction path must be absolute".into());
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let value = ActivePointer {
        schema_version: 1,
        transaction_path: transaction_path.to_path_buf(),
    };
    let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    let tmp = root.join(format!("active-{}.json.tmp", std::process::id()));
    let path = active_pointer_path(root);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(target_os = "macos")]
    options.mode(0o600);
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    match std::fs::hard_link(&tmp, &path) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&tmp);
            Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
                "another update transaction is already active".into()
            } else {
                error.to_string()
            })
        }
    }
}

fn read_active_pointer_unlocked(root: &Path) -> Result<Option<PathBuf>, String> {
    let bytes = match std::fs::read(active_pointer_path(root)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let value: ActivePointer = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if value.schema_version != 1 || !value.transaction_path.is_absolute() {
        return Err("invalid active transaction pointer".into());
    }
    Ok(Some(value.transaction_path))
}

fn clear_active_pointer_unlocked(
    root: &Path,
    expected_path: Option<&Path>,
) -> Result<bool, String> {
    let path = active_pointer_path(root);
    if let Some(expected_path) = expected_path {
        match read_active_pointer_unlocked(root)? {
            Some(active) if active == expected_path => {}
            Some(_) => return Ok(false),
            None => return Ok(false),
        }
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub fn adjacent_candidate_path(
    installed_app: &Path,
    transaction_id: &str,
) -> Result<PathBuf, String> {
    let parent = installed_app
        .parent()
        .ok_or_else(|| "installed app has no parent".to_string())?;
    if transaction_id.is_empty() || transaction_id.contains('/') {
        return Err("invalid transaction id".into());
    }
    Ok(parent.join(format!(".sleipnir-update-{transaction_id}/candidate.app")))
}

pub fn write_active_pointer(root: &Path, transaction_path: &Path) -> Result<(), String> {
    with_update_lock(root, || {
        write_active_pointer_unlocked(root, transaction_path)
    })
}

pub fn read_active_pointer(root: &Path) -> Result<Option<PathBuf>, String> {
    read_active_pointer_unlocked(root)
}

pub fn persist_prepared(path: &Path, transaction: &Transaction) -> Result<(), TransactionError> {
    save_atomic(path, transaction)
}

pub fn updates_root() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|path| path.join("Sleipnir/updates"))
        .ok_or_else(|| "application data directory is unavailable".to_string())
}

#[cfg(target_os = "macos")]
pub fn new_transaction(
    root: &Path,
    installed_app: &Path,
    artifact: &Path,
    old_version: &str,
    new_version: &str,
    old_pid: u32,
) -> Result<(PathBuf, Transaction), String> {
    with_update_lock(root, || {
        if read_active_pointer_unlocked(root)?.is_some() {
            return Err("another update transaction is already active".into());
        }
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let mut nonce = [0_u8; 32];
        rand::rng().fill_bytes(&mut nonce);
        let nonce = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let transaction_dir = root.join(&transaction_id);
        std::fs::create_dir_all(&transaction_dir).map_err(|e| e.to_string())?;
        std::fs::set_permissions(&transaction_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let path = transaction_dir.join("transaction.json");
        let transaction = Transaction::new(
            transaction_id,
            nonce,
            old_version.to_string(),
            new_version.to_string(),
            old_pid,
            installed_app.to_path_buf(),
            adjacent_candidate_path(
                installed_app,
                path.parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
            )?,
            artifact.to_path_buf(),
        )
        .map_err(|e| e.to_string())?;
        Ok((path, transaction))
    })
}

pub fn activate_prepared_transaction(
    root: &Path,
    path: &Path,
    transaction: &mut Transaction,
) -> Result<(), String> {
    with_update_lock(root, || {
        transaction
            .transition(Phase::Prepared)
            .map_err(|e| e.to_string())?;
        save_atomic(path, transaction).map_err(|e| e.to_string())?;
        write_active_pointer_unlocked(root, path)
    })
}

#[cfg(target_os = "macos")]
pub fn launch_supervisor(helper: &Path, transaction_path: &Path) -> Result<Child, String> {
    let log_path = transaction_path
        .parent()
        .ok_or_else(|| "transaction path has no parent".to_string())?
        .join("update.log");
    let mut log_options = OpenOptions::new();
    log_options.create(true).append(true).mode(0o600);
    let stdout = log_options.open(&log_path).map_err(|e| e.to_string())?;
    let stderr = stdout.try_clone().map_err(|e| e.to_string())?;
    Command::new(helper)
        .arg("supervise")
        .arg("--transaction")
        .arg(transaction_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn stop_helper(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().map_err(|e| e.to_string())?;
    let kill_start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Ok(());
        }
        if kill_start.elapsed() >= timeout {
            return Err("failed to reap update supervisor".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "macos")]
fn format_exit_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("update supervisor exited before becoming ready with status {code}"),
        None => "update supervisor exited before becoming ready by signal".into(),
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupDisposition {
    Safe,
    Unsafe,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct WaitForSupervisorReadyError {
    message: String,
    cleanup: CleanupDisposition,
}

#[cfg(target_os = "macos")]
impl WaitForSupervisorReadyError {
    fn safe(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cleanup: CleanupDisposition::Safe,
        }
    }

    fn unsafe_cleanup(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            cleanup: CleanupDisposition::Unsafe,
        }
    }

    pub fn cleanup_disposition(&self) -> CleanupDisposition {
        self.cleanup
    }
}

#[cfg(target_os = "macos")]
impl fmt::Display for WaitForSupervisorReadyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

#[cfg(target_os = "macos")]
impl StdError for WaitForSupervisorReadyError {}

#[cfg(target_os = "macos")]
fn stop_helper_for_readiness_failure(
    child: &mut Child,
    message: impl Into<String>,
) -> WaitForSupervisorReadyError {
    let message = message.into();
    match stop_helper(child, Duration::from_secs(2)) {
        Ok(()) => WaitForSupervisorReadyError::safe(message),
        Err(error) => WaitForSupervisorReadyError::unsafe_cleanup(format!(
            "{message}; additionally failed to stop update supervisor: {error}"
        )),
    }
}

#[cfg(target_os = "macos")]
pub fn wait_for_supervisor_ready(
    transaction_path: &Path,
    child: &mut Child,
    timeout: Duration,
) -> Result<(), WaitForSupervisorReadyError> {
    let child_pid = child.id();
    let start = Instant::now();
    while start.elapsed() < timeout {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(WaitForSupervisorReadyError::safe(format_exit_status(
                    status,
                )));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(stop_helper_for_readiness_failure(
                    child,
                    format!("failed to observe update supervisor status: {error}"),
                ));
            }
        }
        let transaction = match crate::transaction::load(transaction_path) {
            Ok(transaction) => transaction,
            Err(error) => {
                return Err(stop_helper_for_readiness_failure(
                    child,
                    format!(
                        "failed to load transaction while waiting for update supervisor readiness: {error}"
                    ),
                ));
            }
        };
        if transaction.phase == Phase::WaitingForOldExit
            && transaction.helper_pid == Some(child_pid)
        {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(WaitForSupervisorReadyError::safe(format_exit_status(
                        status,
                    )));
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    return Err(stop_helper_for_readiness_failure(
                        child,
                        format!("failed to observe update supervisor status: {error}"),
                    ));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(stop_helper_for_readiness_failure(
        child,
        "update supervisor did not become ready",
    ))
}

pub fn pending_transaction() -> Result<Option<(PathBuf, Transaction)>, String> {
    let root = updates_root()?;
    let Some(path) = read_active_pointer(&root)? else {
        return Ok(None);
    };
    let transaction = crate::transaction::load(&path).map_err(|e| e.to_string())?;
    Ok(Some((path, transaction)))
}

pub fn clear_active_pointer(root: &Path, expected_path: &Path) -> Result<bool, String> {
    with_update_lock(root, || {
        clear_active_pointer_unlocked(root, Some(expected_path))
    })
}

pub fn acknowledge_active_outcome() -> Result<(), String> {
    let root = updates_root()?;
    acknowledge_outcome_at(&root)
}

fn acknowledge_outcome_at(root: &Path) -> Result<(), String> {
    with_update_lock(root, || {
        let Some(path) = read_active_pointer_unlocked(root)? else {
            return Ok(());
        };
        let transaction = crate::transaction::load(&path).map_err(|e| e.to_string())?;
        let final_outcome = matches!(
            transaction.phase,
            Phase::Committed
                | Phase::RolledBack
                | Phase::ManualInstallRequired
                | Phase::RecoveryRequired
        );
        if !final_outcome && transaction.error_code.is_none() {
            return Err("active update has not reached a final outcome".into());
        }
        let _ = clear_active_pointer_unlocked(root, Some(&path))?;
        Ok(())
    })
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 performs an existence/permission check only.
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

pub fn helper_alive(transaction: &Transaction) -> bool {
    transaction.helper_pid.is_some_and(process_alive)
}

pub fn candidate_alive(transaction: &Transaction) -> bool {
    transaction.candidate_pid.is_some_and(process_alive)
}

pub fn old_process_alive(transaction: &Transaction) -> bool {
    process_alive(transaction.old_pid)
}

#[cfg(target_os = "macos")]
fn bundle_version(app: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleShortVersionString")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!version.is_empty()).then_some(version)
}

#[cfg(target_os = "macos")]
const RENAME_SWAP: libc::c_uint = 0x0000_0002;
#[cfg(target_os = "macos")]
const AT_FDCWD: libc::c_int = -2;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn renameatx_np(
        from_fd: libc::c_int,
        from: *const libc::c_char,
        to_fd: libc::c_int,
        to: *const libc::c_char,
        flags: libc::c_uint,
    ) -> libc::c_int;
}

#[cfg(target_os = "macos")]
pub fn swap_paths(first: &Path, second: &Path) -> Result<(), String> {
    let first = CString::new(first.as_os_str().as_bytes())
        .map_err(|_| "first path contains NUL".to_string())?;
    let second = CString::new(second.as_os_str().as_bytes())
        .map_err(|_| "second path contains NUL".to_string())?;
    // SAFETY: both C strings are NUL terminated and remain alive for the call.
    let result = unsafe {
        renameatx_np(
            AT_FDCWD,
            first.as_ptr(),
            AT_FDCWD,
            second.as_ptr(),
            RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(target_os = "macos")]
fn discard_staging(transaction: &Transaction) {
    let _ = std::fs::remove_dir_all(&transaction.adjacent_candidate_path);
    if let Some(parent) = transaction.adjacent_candidate_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

#[cfg(target_os = "macos")]
pub fn recover_active_transaction(root: &Path) -> Result<(), String> {
    use crate::recovery::{RecoveryAction, RecoveryEvidence};
    use crate::transaction::UpdateErrorCode;

    with_update_lock(root, || {
        let Some(path) = read_active_pointer_unlocked(root)? else {
            return Ok(());
        };
        let mut transaction = crate::transaction::load(&path).map_err(|e| e.to_string())?;
        let evidence = RecoveryEvidence {
            phase: transaction.phase,
            old_version: transaction.old_version.clone(),
            new_version: transaction.new_version.clone(),
            installed_version: bundle_version(&transaction.installed_bundle_path),
            adjacent_version: bundle_version(&transaction.adjacent_candidate_path),
            helper_alive: helper_alive(&transaction),
            candidate_alive: candidate_alive(&transaction),
            old_pid_alive: old_process_alive(&transaction),
        };
        match crate::recovery::decide(&evidence) {
            RecoveryAction::WaitForSupervisor => {
                Err("another update transaction is already active".into())
            }
            RecoveryAction::WaitForCandidateExit => Err(
                "the candidate application is still running; retry recovery after it exits".into(),
            ),
            RecoveryAction::CleanPreparedState => {
                discard_staging(&transaction);
                let _ = clear_active_pointer_unlocked(root, Some(&path))?;
                Ok(())
            }
            RecoveryAction::FinishCommittedCleanup | RecoveryAction::None => {
                discard_staging(&transaction);
                let _ = clear_active_pointer_unlocked(root, Some(&path))?;
                Ok(())
            }
            RecoveryAction::FinalizeRolledBack => {
                transaction
                    .transition(Phase::RolledBack)
                    .map_err(|e| e.to_string())?;
                if transaction.os_error.is_none() {
                    transaction.os_error =
                        Some("the interrupted update was rolled back".to_string());
                }
                save_atomic(&path, &transaction).map_err(|e| e.to_string())?;
                discard_staging(&transaction);
                let _ = clear_active_pointer_unlocked(root, Some(&path))?;
                Ok(())
            }
            RecoveryAction::RestoreOldBySwap => {
                if transaction.phase != Phase::RollingBack {
                    transaction
                        .transition(Phase::RollingBack)
                        .map_err(|e| e.to_string())?;
                    save_atomic(&path, &transaction).map_err(|e| e.to_string())?;
                }
                swap_paths(
                    &transaction.installed_bundle_path,
                    &transaction.adjacent_candidate_path,
                )?;
                transaction
                    .transition(Phase::RolledBack)
                    .map_err(|e| e.to_string())?;
                if transaction.os_error.is_none() {
                    transaction.os_error =
                        Some("the interrupted update was rolled back".to_string());
                }
                save_atomic(&path, &transaction).map_err(|e| e.to_string())?;
                discard_staging(&transaction);
                let _ = clear_active_pointer_unlocked(root, Some(&path))?;
                Ok(())
            }
            RecoveryAction::RecoveryRequired => {
                crate::transaction::force_recovery_required(
                    &mut transaction,
                    UpdateErrorCode::RecoveryStateInconsistent,
                    "the interrupted update left the installation in an inconsistent state",
                );
                let _ = save_atomic(&path, &transaction);
                Err(
                    "the previous update left the installation in an inconsistent state; reinstall manually from the releases page"
                        .into(),
                )
            }
        }
    })
}

pub fn write_health_marker(
    transaction_path: &Path,
    version: &str,
    executable: &Path,
) -> Result<bool, String> {
    let transaction = crate::transaction::load(transaction_path).map_err(|e| e.to_string())?;
    if transaction.phase != Phase::AwaitingHealth
        || transaction.new_version != version
        || executable
            != transaction
                .installed_bundle_path
                .join("Contents/MacOS/sleipnir")
    {
        return Ok(false);
    }
    let pid = std::process::id();
    if transaction.candidate_pid != Some(pid) {
        return Ok(false);
    }
    let marker = HealthMarker {
        schema_version: crate::transaction::TRANSACTION_SCHEMA_VERSION,
        transaction_id: transaction.transaction_id,
        nonce: transaction.nonce,
        version: version.to_string(),
        pid,
        executable: executable.to_path_buf(),
    };
    let bytes = serde_json::to_vec_pretty(&marker).map_err(|e| e.to_string())?;
    let path = transaction_path.parent().unwrap().join("health-ready.json");
    let tmp = transaction_path
        .parent()
        .unwrap()
        .join("health-ready.json.tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(target_os = "macos")]
    options.mode(0o600);
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::Command;
    use std::process::Stdio;
    use tempfile::tempdir;

    #[test]
    fn active_pointer_round_trips_absolute_transaction_path() {
        let root = tempdir().unwrap();
        let transaction = root.path().join("tx/transaction.json");
        std::fs::create_dir_all(transaction.parent().unwrap()).unwrap();
        write_active_pointer(root.path(), &transaction).unwrap();
        assert_eq!(read_active_pointer(root.path()).unwrap(), Some(transaction));
    }

    #[test]
    fn active_pointer_rejects_relative_path() {
        let root = tempdir().unwrap();
        assert!(write_active_pointer(root.path(), Path::new("relative.json")).is_err());
    }

    #[test]
    fn candidate_staging_path_is_adjacent_to_install() {
        let path = adjacent_candidate_path(
            Path::new("/Applications/Sleipnir.app"),
            "11111111-1111-4111-8111-111111111111",
        )
        .unwrap();
        assert_eq!(
            path,
            Path::new(
                "/Applications/.sleipnir-update-11111111-1111-4111-8111-111111111111/candidate.app"
            )
        );
    }

    #[test]
    fn clear_active_pointer_respects_expected_owner() {
        let root = tempdir().unwrap();
        let tx_a = root.path().join("a/transaction.json");
        let tx_b = root.path().join("b/transaction.json");
        std::fs::create_dir_all(tx_a.parent().unwrap()).unwrap();
        std::fs::create_dir_all(tx_b.parent().unwrap()).unwrap();
        let value = ActivePointer {
            schema_version: 1,
            transaction_path: tx_b.clone(),
        };
        std::fs::write(
            active_pointer_path(root.path()),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        assert!(!clear_active_pointer(root.path(), &tx_a).unwrap());
        assert_eq!(read_active_pointer(root.path()).unwrap(), Some(tx_b));
    }

    #[test]
    fn acknowledge_refuses_a_healthy_in_flight_transaction() {
        let root = tempdir().unwrap();
        let mut transaction = Transaction::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "ab".repeat(32),
            "0.3.1".into(),
            "0.3.2".into(),
            42,
            root.path().join("Sleipnir.app"),
            root.path().join("candidate.app"),
            root.path().join("update.dmg"),
        )
        .unwrap();
        transaction.transition(Phase::Prepared).unwrap();
        let path = root.path().join("tx/transaction.json");
        save_atomic(&path, &transaction).unwrap();
        write_active_pointer(root.path(), &path).unwrap();
        assert!(acknowledge_outcome_at(root.path()).is_err());
        assert!(read_active_pointer(root.path()).unwrap().is_some());
    }

    #[test]
    fn acknowledge_clears_an_errored_non_terminal_transaction() {
        let root = tempdir().unwrap();
        let mut transaction = Transaction::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "ab".repeat(32),
            "0.3.1".into(),
            "0.3.2".into(),
            42,
            root.path().join("Sleipnir.app"),
            root.path().join("candidate.app"),
            root.path().join("update.dmg"),
        )
        .unwrap();
        transaction.transition(Phase::Prepared).unwrap();
        transaction.fail(
            crate::transaction::UpdateErrorCode::OldProcessWatchFailed,
            "simulated supervisor failure",
        );
        let path = root.path().join("tx/transaction.json");
        save_atomic(&path, &transaction).unwrap();
        write_active_pointer(root.path(), &path).unwrap();
        acknowledge_outcome_at(root.path()).unwrap();
        assert!(read_active_pointer(root.path()).unwrap().is_none());
    }

    #[cfg(target_os = "macos")]
    fn write_fake_bundle(app: &Path, version: &str) {
        let contents = app.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <plist version=\"1.0\"><dict>\n\
                 <key>CFBundleShortVersionString</key><string>{version}</string>\n\
                 </dict></plist>\n"
            ),
        )
        .unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn create_transaction_refuses_a_second_active_update() {
        let root = tempdir().unwrap();
        let installed = root.path().join("Sleipnir.app");
        let artifact = root.path().join("update.dmg");
        let (path, mut transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
        activate_prepared_transaction(root.path(), &path, &mut transaction).unwrap();
        let error =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap_err();
        assert!(error.contains("already active"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn read_active_pointer_stays_lock_free_while_mutations_return_busy() {
        let root = tempdir().unwrap();
        std::fs::create_dir_all(root.path()).unwrap();
        std::fs::write(root.path().join("lock"), b"").unwrap();
        let transaction = root.path().join("tx/transaction.json");
        std::fs::create_dir_all(transaction.parent().unwrap()).unwrap();
        write_active_pointer(root.path(), &transaction).unwrap();

        let mut holder = Command::new("python3")
            .arg("-u")
            .arg("-c")
            .arg(
                "import fcntl, sys, time\n".to_owned()
                    + "f = open(sys.argv[1], 'a+')\n"
                    + "fcntl.flock(f, fcntl.LOCK_EX)\n"
                    + "print('ready', flush=True)\n"
                    + "time.sleep(float(sys.argv[2]))\n",
            )
            .arg(root.path().join("lock"))
            .arg("0.5")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = String::new();
        let stdout = holder.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let bytes = reader.read_line(&mut ready).unwrap();
        assert!(bytes > 0, "lock holder exited before signaling readiness");
        assert_eq!(ready.trim_end(), "ready");
        drop(reader);

        let start = Instant::now();
        assert_eq!(
            read_active_pointer(root.path()).unwrap(),
            Some(transaction.clone())
        );
        assert!(start.elapsed() < Duration::from_millis(200));

        let start = Instant::now();
        let error = clear_active_pointer(root.path(), &transaction).unwrap_err();
        assert_eq!(error, UPDATE_LOCK_BUSY);
        assert!(start.elapsed() < Duration::from_millis(200));
        let status = holder.wait().unwrap();
        assert!(
            status.success(),
            "lock holder exited unsuccessfully: {status}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn wait_for_supervisor_ready_reaps_only_spawned_helper_on_timeout() {
        let root = tempdir().unwrap();
        let tx_path = root.path().join("tx/transaction.json");
        std::fs::create_dir_all(tx_path.parent().unwrap()).unwrap();
        let mut tx = Transaction::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "ab".repeat(32),
            "0.3.1".into(),
            "0.3.2".into(),
            42,
            root.path().join("Sleipnir.app"),
            root.path().join("candidate.app"),
            root.path().join("update.dmg"),
        )
        .unwrap();
        tx.transition(Phase::Prepared).unwrap();
        save_atomic(&tx_path, &tx).unwrap();

        let helper = root.path().join("fake-helper.sh");
        std::fs::write(&helper, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let mut perms = std::fs::metadata(&helper).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&helper, perms).unwrap();

        let mut child = launch_supervisor(&helper, &tx_path).unwrap();
        let err = wait_for_supervisor_ready(&tx_path, &mut child, Duration::from_millis(150))
            .unwrap_err();
        assert!(err.to_string().contains("did not become ready"));
        assert_eq!(err.cleanup_disposition(), CleanupDisposition::Safe);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn wait_for_supervisor_ready_requires_transaction_to_bind_the_owned_child() {
        let root = tempdir().unwrap();
        let tx_path = root.path().join("tx/transaction.json");
        std::fs::create_dir_all(tx_path.parent().unwrap()).unwrap();
        let mut tx = Transaction::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "ab".repeat(32),
            "0.3.1".into(),
            "0.3.2".into(),
            42,
            root.path().join("Sleipnir.app"),
            root.path().join("candidate.app"),
            root.path().join("update.dmg"),
        )
        .unwrap();
        tx.transition(Phase::Prepared).unwrap();
        tx.transition(Phase::WaitingForOldExit).unwrap();
        let helper = root.path().join("fake-helper.sh");
        std::fs::write(&helper, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let mut perms = std::fs::metadata(&helper).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&helper, perms).unwrap();

        let mut child = launch_supervisor(&helper, &tx_path).unwrap();
        tx.helper_pid = Some(if child.id() == 1 { 2 } else { 1 });
        save_atomic(&tx_path, &tx).unwrap();

        let err = wait_for_supervisor_ready(&tx_path, &mut child, Duration::from_millis(150))
            .unwrap_err();
        assert!(err.to_string().contains("did not become ready"));
        assert_eq!(err.cleanup_disposition(), CleanupDisposition::Safe);
        assert!(child.try_wait().unwrap().is_some());

        let mut child = launch_supervisor(&helper, &tx_path).unwrap();
        tx.helper_pid = Some(child.id());
        save_atomic(&tx_path, &tx).unwrap();
        wait_for_supervisor_ready(&tx_path, &mut child, Duration::from_secs(1)).unwrap();
        stop_helper(&mut child, Duration::from_secs(1)).unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn wait_for_supervisor_ready_reports_early_exit() {
        let root = tempdir().unwrap();
        let tx_path = root.path().join("tx/transaction.json");
        std::fs::create_dir_all(tx_path.parent().unwrap()).unwrap();
        let mut tx = Transaction::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "ab".repeat(32),
            "0.3.1".into(),
            "0.3.2".into(),
            42,
            root.path().join("Sleipnir.app"),
            root.path().join("candidate.app"),
            root.path().join("update.dmg"),
        )
        .unwrap();
        tx.transition(Phase::Prepared).unwrap();
        save_atomic(&tx_path, &tx).unwrap();

        let helper = root.path().join("fake-helper.sh");
        std::fs::write(&helper, "#!/bin/sh\nexit 7\n").unwrap();
        let mut perms = std::fs::metadata(&helper).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&helper, perms).unwrap();

        let mut child = launch_supervisor(&helper, &tx_path).unwrap();
        let err =
            wait_for_supervisor_ready(&tx_path, &mut child, Duration::from_secs(1)).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("exited before becoming ready")
                || message.contains("did not become ready"),
            "unexpected readiness error: {message}"
        );
        assert_eq!(err.cleanup_disposition(), CleanupDisposition::Safe);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn wait_for_supervisor_ready_reaps_helper_when_transaction_load_fails() {
        let root = tempdir().unwrap();
        let tx_path = root.path().join("tx/transaction.json");
        std::fs::create_dir_all(tx_path.parent().unwrap()).unwrap();
        std::fs::write(&tx_path, b"{not json").unwrap();

        let helper = root.path().join("fake-helper.sh");
        std::fs::write(&helper, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let mut perms = std::fs::metadata(&helper).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&helper, perms).unwrap();

        let mut child = launch_supervisor(&helper, &tx_path).unwrap();
        let err =
            wait_for_supervisor_ready(&tx_path, &mut child, Duration::from_secs(1)).unwrap_err();
        assert!(err.to_string().contains("failed to load transaction"));
        assert_eq!(err.cleanup_disposition(), CleanupDisposition::Safe);
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recovery_cleans_pre_swap_crash_without_touching_old_install() {
        use crate::transaction::UpdateErrorCode;
        let root = tempdir().unwrap();
        let install_parent = tempdir().unwrap();
        let installed = install_parent.path().join("Sleipnir.app");
        write_fake_bundle(&installed, "0.3.1");
        let artifact = root.path().join("update.dmg");
        let (path, mut transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
        activate_prepared_transaction(root.path(), &path, &mut transaction).unwrap();
        transaction.transition(Phase::WaitingForOldExit).unwrap();
        transaction.transition(Phase::Swapping).unwrap();
        transaction.fail(
            UpdateErrorCode::AtomicSwapFailed,
            "simulated crash before swap syscall completed",
        );
        save_atomic(&path, &transaction).unwrap();
        write_fake_bundle(&transaction.adjacent_candidate_path, "0.3.2");

        recover_active_transaction(root.path()).unwrap();
        assert_eq!(bundle_version(&installed).as_deref(), Some("0.3.1"));
        assert!(!transaction.adjacent_candidate_path.exists());
        assert!(read_active_pointer(root.path()).unwrap().is_none());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recovery_refuses_to_touch_running_candidate_pid() {
        let root = tempdir().unwrap();
        let install_parent = tempdir().unwrap();
        let installed = install_parent.path().join("Sleipnir.app");
        write_fake_bundle(&installed, "0.3.2");
        let artifact = root.path().join("update.dmg");
        let (path, mut transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
        activate_prepared_transaction(root.path(), &path, &mut transaction).unwrap();
        transaction.transition(Phase::WaitingForOldExit).unwrap();
        transaction.transition(Phase::Swapping).unwrap();
        transaction.transition(Phase::LaunchingCandidate).unwrap();
        transaction.transition(Phase::AwaitingHealth).unwrap();
        transaction.candidate_pid = Some(std::process::id());
        save_atomic(&path, &transaction).unwrap();
        write_fake_bundle(&transaction.adjacent_candidate_path, "0.3.1");

        let err = recover_active_transaction(root.path()).unwrap_err();
        assert!(err.contains("candidate application is still running"));
        assert!(read_active_pointer(root.path()).unwrap().is_some());
        assert!(transaction.adjacent_candidate_path.exists());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recovery_clears_prepared_zombie_and_allows_a_new_update() {
        use crate::transaction::UpdateErrorCode;
        let root = tempdir().unwrap();
        let install_parent = tempdir().unwrap();
        let installed = install_parent.path().join("Sleipnir.app");
        write_fake_bundle(&installed, "0.3.1");
        let artifact = root.path().join("update.dmg");
        let (path, mut transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
        activate_prepared_transaction(root.path(), &path, &mut transaction).unwrap();
        transaction.fail(
            UpdateErrorCode::OldProcessExitTimeout,
            "simulated supervisor death",
        );
        save_atomic(&path, &transaction).unwrap();
        std::fs::create_dir_all(&transaction.adjacent_candidate_path).unwrap();

        recover_active_transaction(root.path()).unwrap();
        assert!(read_active_pointer(root.path()).unwrap().is_none());
        assert!(!transaction.adjacent_candidate_path.exists());
        let (_path, _transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recovery_rolls_back_an_uncommitted_swap() {
        use crate::transaction::UpdateErrorCode;
        let root = tempdir().unwrap();
        let install_parent = tempdir().unwrap();
        let installed = install_parent.path().join("Sleipnir.app");
        write_fake_bundle(&installed, "0.3.1");
        let artifact = root.path().join("update.dmg");
        let (path, mut transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
        activate_prepared_transaction(root.path(), &path, &mut transaction).unwrap();
        transaction.transition(Phase::WaitingForOldExit).unwrap();
        transaction.transition(Phase::Swapping).unwrap();
        transaction.transition(Phase::LaunchingCandidate).unwrap();
        transaction.transition(Phase::AwaitingHealth).unwrap();
        transaction.fail(
            UpdateErrorCode::HealthConfirmationTimeout,
            "simulated supervisor death after swap",
        );
        save_atomic(&path, &transaction).unwrap();
        write_fake_bundle(&installed, "0.3.2");
        write_fake_bundle(&transaction.adjacent_candidate_path, "0.3.1");

        recover_active_transaction(root.path()).unwrap();
        assert_eq!(bundle_version(&installed).as_deref(), Some("0.3.1"));
        assert!(!transaction.adjacent_candidate_path.exists());
        assert!(read_active_pointer(root.path()).unwrap().is_none());
        let restored = crate::transaction::load(&path).unwrap();
        assert_eq!(restored.phase, Phase::RolledBack);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn recovery_finalizes_completed_rollback_after_crash() {
        let root = tempdir().unwrap();
        let install_parent = tempdir().unwrap();
        let installed = install_parent.path().join("Sleipnir.app");
        write_fake_bundle(&installed, "0.3.1");
        let artifact = root.path().join("update.dmg");
        let (path, mut transaction) =
            new_transaction(root.path(), &installed, &artifact, "0.3.1", "0.3.2", 42).unwrap();
        activate_prepared_transaction(root.path(), &path, &mut transaction).unwrap();
        transaction.transition(Phase::WaitingForOldExit).unwrap();
        transaction.transition(Phase::Swapping).unwrap();
        transaction.transition(Phase::LaunchingCandidate).unwrap();
        transaction.transition(Phase::AwaitingHealth).unwrap();
        transaction.transition(Phase::RollingBack).unwrap();
        save_atomic(&path, &transaction).unwrap();
        write_fake_bundle(&transaction.adjacent_candidate_path, "0.3.2");

        recover_active_transaction(root.path()).unwrap();
        let restored = crate::transaction::load(&path).unwrap();
        assert_eq!(restored.phase, Phase::RolledBack);
        assert!(!transaction.adjacent_candidate_path.exists());
        assert!(read_active_pointer(root.path()).unwrap().is_none());
    }
}
