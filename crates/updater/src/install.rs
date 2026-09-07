use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::transaction::{HealthMarker, Phase, Transaction, TransactionError, save_atomic};
#[cfg(target_os = "macos")]
use rand::RngCore as _;
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::Write as _;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt as _;
#[cfg(target_os = "macos")]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Deserialize, Serialize)]
struct ActivePointer {
    schema_version: u32,
    transaction_path: PathBuf,
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
    let path = root.join("active.json");
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

pub fn read_active_pointer(root: &Path) -> Result<Option<PathBuf>, String> {
    let path = root.join("active.json");
    let bytes = match std::fs::read(path) {
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
    if read_active_pointer(root)?.is_some() {
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
}

pub fn activate_prepared_transaction(
    root: &Path,
    path: &Path,
    transaction: &mut Transaction,
) -> Result<(), String> {
    transaction
        .transition(Phase::Prepared)
        .map_err(|e| e.to_string())?;
    save_atomic(path, transaction).map_err(|e| e.to_string())?;
    write_active_pointer(root, path)
}

#[cfg(target_os = "macos")]
pub fn launch_supervisor(helper: &Path, transaction_path: &Path) -> Result<(), String> {
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
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn wait_for_supervisor_ready(
    transaction_path: &Path,
    timeout: Duration,
) -> Result<bool, String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let transaction = crate::transaction::load(transaction_path).map_err(|e| e.to_string())?;
        if transaction.phase == Phase::WaitingForOldExit && transaction.helper_pid.is_some() {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(false)
}

pub fn pending_transaction() -> Result<Option<(PathBuf, Transaction)>, String> {
    let root = updates_root()?;
    let Some(path) = read_active_pointer(&root)? else {
        return Ok(None);
    };
    let transaction = crate::transaction::load(&path).map_err(|e| e.to_string())?;
    Ok(Some((path, transaction)))
}

pub fn clear_active_pointer(root: &Path) -> Result<(), String> {
    match std::fs::remove_file(root.join("active.json")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn acknowledge_active_outcome() -> Result<(), String> {
    let root = updates_root()?;
    acknowledge_outcome_at(&root)
}

fn acknowledge_outcome_at(root: &Path) -> Result<(), String> {
    let Some(path) = read_active_pointer(root)? else {
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
    // A transaction that recorded an error belongs to a supervisor that has
    // already exited, so it will never advance on its own and is safe to clear.
    if !final_outcome && transaction.error_code.is_none() {
        return Err("active update has not reached a final outcome".into());
    }
    clear_active_pointer(root)
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 performs an existence/permission check only.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

/// Whether the supervisor that owns `transaction` is still running.
pub fn helper_alive(transaction: &Transaction) -> bool {
    transaction.helper_pid.is_some_and(process_alive)
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
fn swap_paths(first: &Path, second: &Path) -> Result<(), String> {
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

/// Resolve a leftover active transaction so a new update can start.
///
/// The supervisor can die between phases (crash, reboot, `kill -9`), leaving
/// `active.json` pointing at a transaction that will never advance; without
/// recovery `new_transaction` would refuse every later update.
///
/// Returns `Err` when a live supervisor still owns the transaction, or when
/// the on-disk state is contradictory and needs manual recovery.
#[cfg(target_os = "macos")]
pub fn recover_active_transaction(root: &Path) -> Result<(), String> {
    use crate::recovery::{RecoveryAction, RecoveryEvidence};
    use crate::transaction::UpdateErrorCode;

    let Some(path) = read_active_pointer(root)? else {
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
    };
    // Drop the staged `.app` copy and its staging directory, if any survived.
    fn discard_staging(transaction: &Transaction) {
        let _ = std::fs::remove_dir_all(&transaction.adjacent_candidate_path);
        if let Some(parent) = transaction.adjacent_candidate_path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    match crate::recovery::decide(&evidence) {
        RecoveryAction::WaitForSupervisor => {
            Err("another update transaction is already active".into())
        }
        RecoveryAction::RetainPrepared => {
            // The old install was never touched; the staged candidate is junk.
            discard_staging(&transaction);
            clear_active_pointer(root)
        }
        RecoveryAction::FinishCommittedCleanup | RecoveryAction::None => {
            discard_staging(&transaction);
            clear_active_pointer(root)
        }
        RecoveryAction::RestoreOldBySwap => {
            // The candidate was swapped in but never committed; put the
            // retained old bundle back before starting over.
            if transaction.phase != Phase::RollingBack {
                transaction
                    .transition(Phase::RollingBack)
                    .map_err(|e| e.to_string())?;
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
            clear_active_pointer(root)
        }
        RecoveryAction::RecoveryRequired => {
            crate::transaction::force_recovery_required(
                &mut transaction,
                UpdateErrorCode::RecoveryStateInconsistent,
                "the interrupted update left the installation in an inconsistent state",
            );
            let _ = save_atomic(&path, &transaction);
            Err("the previous update left the installation in an inconsistent \
                 state; reinstall manually from the releases page"
                .into())
        }
    }
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
        // The supervisor records a failure and dies: phase stays Prepared.
        transaction.fail(
            UpdateErrorCode::OldProcessExitTimeout,
            "simulated supervisor death",
        );
        save_atomic(&path, &transaction).unwrap();
        std::fs::create_dir_all(&transaction.adjacent_candidate_path).unwrap();

        recover_active_transaction(root.path()).unwrap();
        assert!(read_active_pointer(root.path()).unwrap().is_none());
        assert!(!transaction.adjacent_candidate_path.exists());

        // A later update can open a fresh transaction instead of being
        // permanently blocked by the zombie.
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
        // The swap already happened: candidate installed, old bundle adjacent.
        write_fake_bundle(&installed, "0.3.2");
        write_fake_bundle(&transaction.adjacent_candidate_path, "0.3.1");

        recover_active_transaction(root.path()).unwrap();
        assert_eq!(bundle_version(&installed).as_deref(), Some("0.3.1"));
        assert!(!transaction.adjacent_candidate_path.exists());
        assert!(read_active_pointer(root.path()).unwrap().is_none());
        let restored = crate::transaction::load(&path).unwrap();
        assert_eq!(restored.phase, Phase::RolledBack);
    }
}
