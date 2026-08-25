#[path = "../src/supervisor.rs"]
mod supervisor;

use std::collections::VecDeque;
use std::path::Path;
use supervisor::*;
use updater::transaction::{HealthMarker, Phase, Transaction, TransactionError, UpdateErrorCode};

struct FakeFs {
    swaps: usize,
    marker: Option<HealthMarker>,
    swap_fails_at: Option<usize>,
}

impl FileSystem for FakeFs {
    fn transition(&mut self, tx: &mut Transaction, next: Phase) -> Result<(), TransactionError> {
        tx.transition(next)
    }
    fn swap(&mut self, _: &Path, _: &Path) -> Result<(), String> {
        self.swaps += 1;
        if self.swap_fails_at == Some(self.swaps) { Err("swap failed".into()) } else { Ok(()) }
    }
    fn health_marker(&mut self) -> Result<Option<HealthMarker>, String> { Ok(self.marker.clone()) }
}

struct FakeProcesses {
    old_exited: Result<bool, String>,
    alive: VecDeque<Result<bool, String>>,
    terminate: Result<bool, String>,
}

impl ProcessWatcher for FakeProcesses {
    fn wait_for_exit(&mut self, _: u32, _: u64) -> Result<bool, String> { self.old_exited.clone() }
    fn is_alive(&mut self, _: u32) -> Result<bool, String> { self.alive.pop_front().unwrap_or(Ok(true)) }
    fn terminate_and_wait(&mut self, _: u32, _: u64) -> Result<bool, String> { self.terminate.clone() }
}

struct FakeLauncher(Result<u32, String>);
impl AppLauncher for FakeLauncher {
    fn launch(&mut self, _: &Path) -> Result<u32, String> { self.0.clone() }
}

#[derive(Default)]
struct FakeClock(u64);
impl Clock for FakeClock { fn sleep_one_second(&mut self) { self.0 += 1; } }

fn transaction() -> Transaction {
    let mut tx = Transaction::new(
        "11111111-1111-4111-8111-111111111111".into(), "ab".repeat(32),
        "0.3.1".into(), "0.3.2".into(), 42,
        "/Applications/Sleipnir.app".into(),
        "/Applications/.sleipnir-update-11111111-1111-4111-8111-111111111111/candidate.app".into(),
        "/tmp/artifact.dmg".into(),
    ).unwrap();
    tx.transition(Phase::Prepared).unwrap();
    tx
}

fn marker(tx: &Transaction, pid: u32) -> HealthMarker {
    HealthMarker {
        schema_version: 1,
        transaction_id: tx.transaction_id.clone(),
        nonce: tx.nonce.clone(),
        version: tx.new_version.clone(),
        pid,
        executable: tx.installed_bundle_path.join("Contents/MacOS/sleipnir"),
    }
}

fn harness(tx: &Transaction) -> Supervisor<FakeFs, FakeProcesses, FakeLauncher, FakeClock> {
    Supervisor {
        fs: FakeFs { swaps: 0, marker: Some(marker(tx, 99)), swap_fails_at: None },
        processes: FakeProcesses { old_exited: Ok(true), alive: VecDeque::new(), terminate: Ok(true) },
        launcher: FakeLauncher(Ok(99)),
        clock: FakeClock::default(),
    }
}

#[test]
fn healthy_candidate_commits_after_stability_window() {
    let mut tx = transaction();
    let mut supervisor = harness(&tx);
    assert_eq!(supervisor.run(&mut tx), SupervisorResult::Committed);
    assert_eq!(tx.phase, Phase::Committed);
    assert_eq!(supervisor.fs.swaps, 1);
    assert_eq!(supervisor.clock.0, 5);
}

#[test]
fn old_process_timeout_never_swaps() {
    let mut tx = transaction();
    let mut supervisor = harness(&tx);
    supervisor.processes.old_exited = Ok(false);
    assert_eq!(supervisor.run(&mut tx), SupervisorResult::Stopped(UpdateErrorCode::OldProcessExitTimeout));
    assert_eq!(supervisor.fs.swaps, 0);
}

#[test]
fn launch_failure_rolls_back() {
    let mut tx = transaction();
    let mut supervisor = harness(&tx);
    supervisor.launcher = FakeLauncher(Err("launch failed".into()));
    assert_eq!(supervisor.run(&mut tx), SupervisorResult::RolledBack(UpdateErrorCode::CandidateLaunchFailed));
    assert_eq!(tx.phase, Phase::RolledBack);
    assert_eq!(supervisor.fs.swaps, 2);
}

#[test]
fn health_timeout_terminates_candidate_and_rolls_back() {
    let mut tx = transaction();
    let mut supervisor = harness(&tx);
    supervisor.fs.marker = None;
    assert_eq!(supervisor.run(&mut tx), SupervisorResult::RolledBack(UpdateErrorCode::HealthConfirmationTimeout));
    assert_eq!(supervisor.clock.0, 60);
    assert_eq!(supervisor.fs.swaps, 2);
}

#[test]
fn candidate_that_cannot_stop_is_left_for_recovery() {
    let mut tx = transaction();
    let mut supervisor = harness(&tx);
    supervisor.fs.marker = None;
    supervisor.processes.terminate = Ok(false);
    assert_eq!(supervisor.run(&mut tx), SupervisorResult::RecoveryRequired(UpdateErrorCode::CandidateTerminationFailed));
    assert_eq!(supervisor.fs.swaps, 1);
    assert_eq!(tx.phase, Phase::RecoveryRequired);
}

#[test]
fn rollback_swap_failure_preserves_recovery_state() {
    let mut tx = transaction();
    let mut supervisor = harness(&tx);
    supervisor.launcher = FakeLauncher(Err("launch failed".into()));
    supervisor.fs.swap_fails_at = Some(2);
    assert_eq!(supervisor.run(&mut tx), SupervisorResult::RecoveryRequired(UpdateErrorCode::RollbackFailed));
    assert_eq!(tx.phase, Phase::RecoveryRequired);
}
