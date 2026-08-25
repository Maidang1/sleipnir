use std::path::Path;
use updater::transaction::{HealthMarker, Phase, Transaction, TransactionError, UpdateErrorCode};

pub trait FileSystem {
    fn transition(&mut self, tx: &mut Transaction, next: Phase) -> Result<(), TransactionError>;
    fn swap(&mut self, installed: &Path, adjacent: &Path) -> Result<(), String>;
    fn health_marker(&mut self) -> Result<Option<HealthMarker>, String>;
}

pub trait ProcessWatcher {
    fn wait_for_exit(&mut self, pid: u32, timeout_secs: u64) -> Result<bool, String>;
    fn is_alive(&mut self, pid: u32) -> Result<bool, String>;
    fn terminate_and_wait(&mut self, pid: u32, timeout_secs: u64) -> Result<bool, String>;
}

pub trait AppLauncher {
    fn launch(&mut self, bundle: &Path) -> Result<u32, String>;
}

pub trait Clock {
    fn sleep_one_second(&mut self);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorResult {
    Committed,
    RolledBack(UpdateErrorCode),
    Stopped(UpdateErrorCode),
    RecoveryRequired(UpdateErrorCode),
}

pub struct Supervisor<F, P, L, C> {
    pub fs: F,
    pub processes: P,
    pub launcher: L,
    pub clock: C,
}

impl<F: FileSystem, P: ProcessWatcher, L: AppLauncher, C: Clock> Supervisor<F, P, L, C> {
    pub fn run(&mut self, tx: &mut Transaction) -> SupervisorResult {
        if !matches!(tx.phase, Phase::Prepared | Phase::WaitingForOldExit) {
            return SupervisorResult::RecoveryRequired(UpdateErrorCode::RecoveryStateInconsistent);
        }
        if tx.phase == Phase::Prepared
            && self.fs.transition(tx, Phase::WaitingForOldExit).is_err()
        {
            return SupervisorResult::Stopped(UpdateErrorCode::InvalidStateTransition);
        }
        match self.processes.wait_for_exit(tx.old_pid, 60) {
            Ok(true) => {}
            Ok(false) => return SupervisorResult::Stopped(UpdateErrorCode::OldProcessExitTimeout),
            Err(_) => return SupervisorResult::Stopped(UpdateErrorCode::OldProcessWatchFailed),
        }
        if self.fs.transition(tx, Phase::Swapping).is_err() {
            return SupervisorResult::Stopped(UpdateErrorCode::InvalidStateTransition);
        }
        if self.fs.swap(&tx.installed_bundle_path, &tx.adjacent_candidate_path).is_err() {
            return SupervisorResult::Stopped(UpdateErrorCode::AtomicSwapFailed);
        }
        if self.fs.transition(tx, Phase::LaunchingCandidate).is_err() {
            return self.rollback(tx, None, UpdateErrorCode::InvalidStateTransition);
        }
        let candidate_pid = match self.launcher.launch(&tx.installed_bundle_path) {
            Ok(pid) => pid,
            Err(_) => return self.rollback(tx, None, UpdateErrorCode::CandidateLaunchFailed),
        };
        tx.candidate_pid = Some(candidate_pid);
        if self.fs.transition(tx, Phase::AwaitingHealth).is_err() {
            return self.rollback(tx, Some(candidate_pid), UpdateErrorCode::InvalidStateTransition);
        }
        for _ in 0..60 {
            match self.processes.is_alive(candidate_pid) {
                Ok(true) => {}
                _ => return self.rollback(tx, None, UpdateErrorCode::CandidateExitedEarly),
            }
            match self.fs.health_marker() {
                Ok(Some(marker)) if marker.matches(tx, candidate_pid) => {
                    for _ in 0..5 {
                        self.clock.sleep_one_second();
                        if self.processes.is_alive(candidate_pid) != Ok(true) {
                            return self.rollback(tx, None, UpdateErrorCode::CandidateExitedEarly);
                        }
                    }
                    if self.fs.transition(tx, Phase::Committed).is_err() {
                        return SupervisorResult::RecoveryRequired(UpdateErrorCode::RecoveryStateInconsistent);
                    }
                    return SupervisorResult::Committed;
                }
                _ => self.clock.sleep_one_second(),
            }
        }
        self.rollback(tx, Some(candidate_pid), UpdateErrorCode::HealthConfirmationTimeout)
    }

    fn rollback(
        &mut self,
        tx: &mut Transaction,
        candidate_pid: Option<u32>,
        cause: UpdateErrorCode,
    ) -> SupervisorResult {
        if let Some(pid) = candidate_pid {
            if self.processes.terminate_and_wait(pid, 10) != Ok(true) {
                let _ = self.fs.transition(tx, Phase::RecoveryRequired);
                return SupervisorResult::RecoveryRequired(UpdateErrorCode::CandidateTerminationFailed);
            }
        }
        if self.fs.transition(tx, Phase::RollingBack).is_err()
            || self.fs.swap(&tx.installed_bundle_path, &tx.adjacent_candidate_path).is_err()
        {
            let _ = self.fs.transition(tx, Phase::RecoveryRequired);
            return SupervisorResult::RecoveryRequired(UpdateErrorCode::RollbackFailed);
        }
        if self.fs.transition(tx, Phase::RolledBack).is_err() {
            return SupervisorResult::RecoveryRequired(UpdateErrorCode::RecoveryStateInconsistent);
        }
        SupervisorResult::RolledBack(cause)
    }
}
