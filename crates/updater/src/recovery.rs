use crate::transaction::Phase;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryEvidence {
    pub phase: Phase,
    pub old_version: String,
    pub new_version: String,
    pub installed_version: Option<String>,
    pub adjacent_version: Option<String>,
    pub helper_alive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    RetainPrepared,
    WaitForSupervisor,
    RestoreOldBySwap,
    FinishCommittedCleanup,
    RecoveryRequired,
    None,
}

pub fn decide(evidence: &RecoveryEvidence) -> RecoveryAction {
    if evidence.helper_alive {
        return RecoveryAction::WaitForSupervisor;
    }
    use Phase::*;
    match evidence.phase {
        Downloaded | Prepared | WaitingForOldExit
            if evidence.installed_version.as_deref() == Some(evidence.old_version.as_str()) =>
        {
            RecoveryAction::RetainPrepared
        }
        Swapping | LaunchingCandidate | AwaitingHealth | RollingBack
            if evidence.installed_version.as_deref() == Some(evidence.new_version.as_str())
                && evidence.adjacent_version.as_deref() == Some(evidence.old_version.as_str()) =>
        {
            RecoveryAction::RestoreOldBySwap
        }
        Committed
            if evidence.installed_version.as_deref() == Some(evidence.new_version.as_str()) =>
        {
            RecoveryAction::FinishCommittedCleanup
        }
        RolledBack
            if evidence.installed_version.as_deref() == Some(evidence.old_version.as_str()) =>
        {
            RecoveryAction::None
        }
        Cancelled | ManualInstallRequired => RecoveryAction::None,
        _ => RecoveryAction::RecoveryRequired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::Phase;

    fn evidence(phase: Phase, installed: Option<&str>, adjacent: Option<&str>) -> RecoveryEvidence {
        RecoveryEvidence {
            phase,
            old_version: "0.3.1".into(),
            new_version: "0.3.2".into(),
            installed_version: installed.map(str::to_owned),
            adjacent_version: adjacent.map(str::to_owned),
            helper_alive: false,
        }
    }

    #[test]
    fn prepared_and_waiting_transactions_never_modify_old_install() {
        assert_eq!(
            decide(&evidence(Phase::Prepared, Some("0.3.1"), None)),
            RecoveryAction::RetainPrepared
        );
        assert_eq!(
            decide(&evidence(Phase::WaitingForOldExit, Some("0.3.1"), None)),
            RecoveryAction::RetainPrepared
        );
    }

    #[test]
    fn uncommitted_swapped_candidate_is_rolled_back() {
        for phase in [
            Phase::Swapping,
            Phase::LaunchingCandidate,
            Phase::AwaitingHealth,
            Phase::RollingBack,
        ] {
            assert_eq!(
                decide(&evidence(phase, Some("0.3.2"), Some("0.3.1"))),
                RecoveryAction::RestoreOldBySwap
            );
        }
    }

    #[test]
    fn committed_candidate_is_kept_and_cleaned() {
        assert_eq!(
            decide(&evidence(Phase::Committed, Some("0.3.2"), Some("0.3.1"))),
            RecoveryAction::FinishCommittedCleanup
        );
    }

    #[test]
    fn live_helper_is_never_raced() {
        let mut facts = evidence(Phase::AwaitingHealth, Some("0.3.2"), Some("0.3.1"));
        facts.helper_alive = true;
        assert_eq!(decide(&facts), RecoveryAction::WaitForSupervisor);
    }

    #[test]
    fn contradictory_or_missing_bundles_require_human_recovery() {
        assert_eq!(
            decide(&evidence(
                Phase::AwaitingHealth,
                Some("0.3.1"),
                Some("0.3.2")
            )),
            RecoveryAction::RecoveryRequired
        );
        assert_eq!(
            decide(&evidence(Phase::Committed, None, None)),
            RecoveryAction::RecoveryRequired
        );
    }
}
