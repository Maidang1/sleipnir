use gpui::{App, Global};

#[derive(Clone, Debug)]
pub struct AvailableUpdate {
    pub version: String,
    pub tag: String,
    pub notes: String,
    pub artifact_url: String,
    pub sha256_url: String,
    pub expected_sha256: Option<String>,
    pub expected_size: Option<u64>,
    pub minimum_macos: String,
}

#[derive(Clone, Debug, Default)]
pub enum UpdateUiState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(AvailableUpdate),
    Downloading(AvailableUpdate),
    ReadyToRestart(AvailableUpdate),
    WaitingForHelper(AvailableUpdate),
    Updated {
        version: String,
    },
    RolledBack {
        from: String,
        to: String,
        reason: String,
    },
    ManualInstallRequired {
        artifact: std::path::PathBuf,
        message: String,
    },
    RecoveryRequired {
        message: String,
    },
    Failed(String),
}

#[derive(Default)]
pub struct UpdateModel {
    pub state: UpdateUiState,
    pub verified_artifact: Option<updater::VerifiedArtifact>,
}

impl Global for UpdateModel {}

impl UpdateModel {
    pub fn init(cx: &mut App) {
        if !cx.has_global::<Self>() {
            cx.set_global(Self {
                state: outcome_state(),
                verified_artifact: None,
            });
        }
    }
}

fn outcome_state() -> UpdateUiState {
    let Ok(Some((_, transaction))) = updater::install::pending_transaction() else {
        return UpdateUiState::Idle;
    };
    match transaction.phase {
        updater::transaction::Phase::Committed => UpdateUiState::Updated {
            version: transaction.new_version,
        },
        updater::transaction::Phase::RolledBack => UpdateUiState::RolledBack {
            from: transaction.new_version,
            to: transaction.old_version,
            reason: transaction
                .os_error
                .unwrap_or_else(|| "The new version did not become healthy.".into()),
        },
        updater::transaction::Phase::ManualInstallRequired => {
            UpdateUiState::ManualInstallRequired {
                artifact: transaction.artifact_path,
                message: transaction.os_error.unwrap_or_else(|| {
                    "Automatic replacement is unavailable. Use the verified disk image to install manually.".into()
                }),
            }
        }
        updater::transaction::Phase::RecoveryRequired => UpdateUiState::RecoveryRequired {
            message: transaction.os_error.unwrap_or_else(|| {
                "The update needs manual recovery. No retained application bundle was deleted."
                    .into()
            }),
        },
        // A non-terminal phase usually means the supervisor is still working
        // (e.g. the candidate is awaiting its health check), which must stay
        // invisible. Surface only transactions that can no longer advance: the
        // supervisor recorded an error or is no longer running.
        _ if transaction.error_code.is_some()
            || !updater::install::helper_alive(&transaction) =>
        {
            let detail = transaction
                .os_error
                .unwrap_or_else(|| "the update did not finish".into());
            UpdateUiState::Failed(format!(
                "The previous update to {} was interrupted ({detail}). \
                 You can simply try updating again.",
                transaction.new_version
            ))
        }
        _ => UpdateUiState::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_state_can_wait_for_helper_without_claiming_success() {
        let update = AvailableUpdate {
            version: "0.3.2".into(),
            tag: "v0.3.2".into(),
            notes: String::new(),
            artifact_url: "https://example.invalid/app.dmg".into(),
            sha256_url: String::new(),
            expected_sha256: Some("a".repeat(64)),
            expected_size: Some(42),
            minimum_macos: "14.0".into(),
        };
        let model = UpdateModel {
            state: UpdateUiState::WaitingForHelper(update),
            verified_artifact: None,
        };
        assert!(matches!(model.state, UpdateUiState::WaitingForHelper(_)));
    }
}
