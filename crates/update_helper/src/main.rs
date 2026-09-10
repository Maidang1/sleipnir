#[cfg(target_os = "macos")]
mod launch;
#[cfg(target_os = "macos")]
mod log;
#[cfg(target_os = "macos")]
mod process;
#[cfg(any(target_os = "macos", test))]
mod supervisor;
#[cfg(target_os = "macos")]
mod swap;

#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::time::Duration;
#[cfg(target_os = "macos")]
use supervisor::{AppLauncher, Clock, FileSystem, ProcessWatcher, Supervisor};
#[cfg(target_os = "macos")]
use updater::transaction::{self, HealthMarker, Phase, Transaction, TransactionError};

#[cfg(target_os = "macos")]
struct RealFileSystem {
    transaction_path: PathBuf,
    health_path: PathBuf,
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
#[derive(Default)]
struct RealProcesses {
    old_watch: Option<process::ExitWatch>,
}
#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
struct WorkspaceLauncher;
#[cfg(target_os = "macos")]
impl AppLauncher for WorkspaceLauncher {
    fn launch(&mut self, bundle: &Path) -> Result<u32, String> {
        launch::launch_application(bundle)
    }
}

#[cfg(target_os = "macos")]
struct RealClock;
#[cfg(target_os = "macos")]
impl Clock for RealClock {
    fn sleep_one_second(&mut self) {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
fn run() -> Result<(), String> {
    let transaction_path = parse_transaction_arg()?;
    let _lock = updater::install::UpdateLock::acquire_for_transaction(&transaction_path)?;
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

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = run() {
        eprintln!("sleipnir update helper failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("sleipnir-update-helper is supported only on macOS");
    std::process::exit(2);
}
