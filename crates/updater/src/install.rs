use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::transaction::{HealthMarker, Phase, Transaction, TransactionError, save_atomic};
use rand::RngCore as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
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
    let tmp = root.join("active.json.tmp");
    let path = root.join("active.json");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
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
        .ok_or_else(|| "macOS application support directory is unavailable".to_string())
}

pub fn create_transaction(
    root: &Path,
    installed_app: &Path,
    artifact: &Path,
    old_version: &str,
    new_version: &str,
    old_pid: u32,
) -> Result<(PathBuf, Transaction), String> {
    let transaction_id = uuid::Uuid::new_v4().to_string();
    let mut nonce = [0_u8; 32];
    rand::rng().fill_bytes(&mut nonce);
    let nonce = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let transaction_dir = root.join(&transaction_id);
    std::fs::create_dir_all(&transaction_dir).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&transaction_dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let path = transaction_dir.join("transaction.json");
    let mut transaction = Transaction::new(
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
    transaction
        .transition(Phase::Prepared)
        .map_err(|e| e.to_string())?;
    save_atomic(&path, &transaction).map_err(|e| e.to_string())?;
    write_active_pointer(root, &path)?;
    Ok((path, transaction))
}

pub fn launch_supervisor(helper: &Path, transaction_path: &Path) -> Result<(), String> {
    let log_path = transaction_path
        .parent()
        .ok_or_else(|| "transaction path has no parent".to_string())?
        .join("update.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
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

pub fn acknowledge_active_outcome() -> Result<(), String> {
    let root = updates_root()?;
    let Some(path) = read_active_pointer(&root)? else {
        return Ok(());
    };
    let transaction = crate::transaction::load(&path).map_err(|e| e.to_string())?;
    if !matches!(
        transaction.phase,
        Phase::Committed
            | Phase::RolledBack
            | Phase::ManualInstallRequired
            | Phase::RecoveryRequired
    ) {
        return Err("active update has not reached a final outcome".into());
    }
    match std::fs::remove_file(root.join("active.json")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
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
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
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
}
