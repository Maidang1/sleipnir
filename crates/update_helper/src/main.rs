mod launch;
mod log;
mod process;
mod supervisor;
mod swap;

use std::fs::OpenOptions;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;
use supervisor::{AppLauncher, Clock, FileSystem, ProcessWatcher, Supervisor};
use updater::transaction::{self, HealthMarker, Phase, Transaction, TransactionError};

struct TransactionLock(std::fs::File);

impl TransactionLock {
    fn acquire(transaction_path: &Path) -> Result<Self, String> {
        let root = updater::install::updates_root()?;
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
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).mode(0o600);
        let file = options.open(root.join("lock")).map_err(|e| e.to_string())?;
        // SAFETY: flock operates on this process-owned descriptor.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("another update supervisor owns the transaction lock".into());
        }
        Ok(Self(file))
    }
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        // SAFETY: unlocks the descriptor retained by this guard.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

struct RealFileSystem {
    transaction_path: PathBuf,
    health_path: PathBuf,
}

impl FileSystem for RealFileSystem {
    fn persist(&mut self, tx: &Transaction) -> Result<(), TransactionError> {
        transaction::save_atomic(&self.transaction_path, tx)
    }

    fn transition(&mut self, tx: &mut Transaction, next: Phase) -> Result<(), TransactionError> {
        tx.transition(next)?;
        transaction::save_atomic(&self.transaction_path, tx)
    }

    fn swap(&mut self, installed: &Path, adjacent: &Path) -> Result<(), String> {
        swap::swap_paths(installed, adjacent)
    }

    fn health_marker(&mut self) -> Result<Option<HealthMarker>, String> {
        match std::fs::read(&self.health_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn cleanup_committed_backup(&mut self, adjacent: &Path) -> Result<(), String> {
        std::fs::remove_dir_all(adjacent).map_err(|e| e.to_string())?;
        if let Some(parent) = adjacent.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        Ok(())
    }
}

#[derive(Default)]
struct RealProcesses {
    old_watch: Option<process::ExitWatch>,
}
impl ProcessWatcher for RealProcesses {
    fn register_exit_watch(&mut self, pid: u32) -> Result<(), String> {
        self.old_watch = Some(process::register_exit_watch(pid)?);
        Ok(())
    }
    fn wait_for_registered_exit(&mut self, timeout_secs: u64) -> Result<bool, String> {
        self.old_watch
            .as_mut()
            .ok_or_else(|| "old process watch is not registered".to_string())?
            .wait(Duration::from_secs(timeout_secs))
    }
    fn is_alive(&mut self, pid: u32) -> Result<bool, String> {
        process::is_alive(pid)
    }
    fn terminate_and_wait(&mut self, pid: u32, timeout_secs: u64) -> Result<bool, String> {
        process::terminate_and_wait(pid, Duration::from_secs(timeout_secs))
    }
}

struct WorkspaceLauncher;
impl AppLauncher for WorkspaceLauncher {
    fn launch(&mut self, bundle: &Path) -> Result<u32, String> {
        launch::launch_application(bundle)
    }
}

struct RealClock;
impl Clock for RealClock {
    fn sleep_one_second(&mut self) {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn parse_transaction_arg() -> Result<PathBuf, String> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some("supervise".as_ref())
        || args.next().as_deref() != Some("--transaction".as_ref())
    {
        return Err("usage: sleipnir-update-helper supervise --transaction <absolute path>".into());
    }
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| "missing transaction path".to_string())?,
    );
    if args.next().is_some() || !path.is_absolute() {
        return Err("transaction path must be one absolute path".into());
    }
    Ok(path)
}

fn run() -> Result<(), String> {
    let transaction_path = parse_transaction_arg()?;
    let _lock = TransactionLock::acquire(&transaction_path)?;
    let parent = transaction_path
        .parent()
        .ok_or_else(|| "transaction path has no parent".to_string())?
        .to_path_buf();
    let mut tx = transaction::load(&transaction_path).map_err(|e| e.to_string())?;
    let log_path = parent.join("update.log");
    let _ = log::append(
        &log_path,
        &log::LogEvent {
            transaction_id: &tx.transaction_id,
            phase: "prepared",
            event: "supervisor_started",
            error_code: None,
            os_error: None,
        },
    );
    let mut supervisor = Supervisor {
        fs: RealFileSystem {
            transaction_path,
            health_path: parent.join("health-ready.json"),
        },
        processes: RealProcesses::default(),
        launcher: WorkspaceLauncher,
        clock: RealClock,
    };
    let result = supervisor.run(&mut tx);
    let event = format!("{result:?}");
    let _ = log::append(
        &log_path,
        &log::LogEvent {
            transaction_id: &tx.transaction_id,
            phase: "final",
            event: &event,
            error_code: None,
            os_error: None,
        },
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("sleipnir update helper failed: {error}");
        std::process::exit(1);
    }
}
