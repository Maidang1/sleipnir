use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TRANSACTION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Downloaded,
    Prepared,
    WaitingForOldExit,
    Swapping,
    LaunchingCandidate,
    AwaitingHealth,
    Committed,
    RollingBack,
    RolledBack,
    Cancelled,
    ManualInstallRequired,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateErrorCode {
    InvalidStateTransition,
    InvalidTransaction,
    UnsupportedTransactionSchema,
    ManifestSignatureInvalid,
    ManifestSchemaUnsupported,
    ArtifactSizeMismatch,
    ArtifactHashMismatch,
    BundleIdentifierMismatch,
    BundleVersionMismatch,
    BundleSignatureInvalid,
    BundleLayoutInvalid,
    InstallDirectoryNotWritable,
    SwapRenameUnsupported,
    CrossVolumeStagingUnsupported,
    HelperStartFailed,
    OldProcessWatchFailed,
    OldProcessExitTimeout,
    AtomicSwapFailed,
    CandidateLaunchFailed,
    CandidateExitedEarly,
    HealthConfirmationTimeout,
    HealthConfirmationInvalid,
    CandidateTerminationFailed,
    RollbackFailed,
    RecoveryStateInconsistent,
}

#[derive(Debug)]
pub struct TransactionError {
    pub code: UpdateErrorCode,
    pub message: String,
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TransactionError {}

fn error(code: UpdateErrorCode, message: impl Into<String>) -> TransactionError {
    TransactionError {
        code,
        message: message.into(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Transaction {
    pub schema_version: u32,
    pub transaction_id: String,
    pub nonce: String,
    pub phase: Phase,
    pub old_version: String,
    pub new_version: String,
    pub old_pid: u32,
    pub helper_pid: Option<u32>,
    pub candidate_pid: Option<u32>,
    pub installed_bundle_path: PathBuf,
    pub adjacent_candidate_path: PathBuf,
    pub artifact_path: PathBuf,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub error_code: Option<UpdateErrorCode>,
    pub os_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthMarker {
    pub schema_version: u32,
    pub transaction_id: String,
    pub nonce: String,
    pub version: String,
    pub pid: u32,
    pub executable: PathBuf,
}

impl HealthMarker {
    pub fn matches(&self, transaction: &Transaction, candidate_pid: u32) -> bool {
        self.schema_version == TRANSACTION_SCHEMA_VERSION
            && self.transaction_id == transaction.transaction_id
            && self.nonce == transaction.nonce
            && self.version == transaction.new_version
            && self.pid == candidate_pid
            && transaction.candidate_pid == Some(candidate_pid)
            && self.executable
                == transaction
                    .installed_bundle_path
                    .join("Contents/MacOS/sleipnir")
    }
}

impl Transaction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_id: String,
        nonce: String,
        old_version: String,
        new_version: String,
        old_pid: u32,
        installed_bundle_path: PathBuf,
        adjacent_candidate_path: PathBuf,
        artifact_path: PathBuf,
    ) -> Result<Self, TransactionError> {
        let now = unix_time_ms();
        let value = Self {
            schema_version: TRANSACTION_SCHEMA_VERSION,
            transaction_id,
            nonce,
            phase: Phase::Downloaded,
            old_version,
            new_version,
            old_pid,
            helper_pid: None,
            candidate_pid: None,
            installed_bundle_path,
            adjacent_candidate_path,
            artifact_path,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            error_code: None,
            os_error: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.schema_version != TRANSACTION_SCHEMA_VERSION {
            return Err(error(
                UpdateErrorCode::UnsupportedTransactionSchema,
                "unsupported transaction schema",
            ));
        }
        if uuid::Uuid::parse_str(&self.transaction_id).is_err()
            || self.nonce.len() != 64
            || !self.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !self.installed_bundle_path.is_absolute()
            || !self.adjacent_candidate_path.is_absolute()
            || !self.artifact_path.is_absolute()
        {
            return Err(error(
                UpdateErrorCode::InvalidTransaction,
                "invalid update transaction",
            ));
        }
        Ok(())
    }

    pub fn transition(&mut self, next: Phase) -> Result<(), TransactionError> {
        use Phase::*;
        let legal = matches!(
            (self.phase, next),
            (Downloaded, Prepared)
                | (
                    Prepared,
                    WaitingForOldExit | Cancelled | ManualInstallRequired | RecoveryRequired
                )
                | (
                    WaitingForOldExit,
                    Prepared | Swapping | Cancelled | ManualInstallRequired | RecoveryRequired
                )
                | (
                    Swapping,
                    LaunchingCandidate | RollingBack | RecoveryRequired
                )
                | (
                    LaunchingCandidate,
                    AwaitingHealth | RollingBack | RecoveryRequired
                )
                | (AwaitingHealth, Committed | RollingBack | RecoveryRequired)
                | (RollingBack, RolledBack | RecoveryRequired)
        );
        if !legal {
            return Err(error(
                UpdateErrorCode::InvalidStateTransition,
                format!("illegal update transition {:?} -> {:?}", self.phase, next),
            ));
        }
        self.phase = next;
        self.updated_at_unix_ms = unix_time_ms();
        Ok(())
    }

    pub fn fail(&mut self, code: UpdateErrorCode, message: impl Into<String>) {
        self.error_code = Some(code);
        self.os_error = Some(message.into());
        self.updated_at_unix_ms = unix_time_ms();
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn save_atomic(path: &Path, transaction: &Transaction) -> Result<(), TransactionError> {
    transaction.validate()?;
    let parent = path.parent().ok_or_else(|| {
        error(
            UpdateErrorCode::InvalidTransaction,
            "transaction path has no parent",
        )
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    #[cfg(unix)]
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(transaction)
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&tmp)
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    std::fs::rename(&tmp, path)
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    if let Ok(directory) = std::fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Transaction, TransactionError> {
    let bytes = std::fs::read(path)
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    let transaction: Transaction = serde_json::from_slice(&bytes)
        .map_err(|err| error(UpdateErrorCode::InvalidTransaction, err.to_string()))?;
    transaction.validate()?;
    Ok(transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn transaction() -> Transaction {
        Transaction::new(
            "11111111-1111-4111-8111-111111111111".into(),
            "ab".repeat(32),
            "0.3.1".into(),
            "0.3.2".into(),
            42,
            "/Applications/Sleipnir.app".into(),
            "/Applications/.sleipnir-update-11111111-1111-4111-8111-111111111111/candidate.app"
                .into(),
            "/tmp/Sleipnir-0.3.2.dmg".into(),
        )
        .unwrap()
    }

    #[test]
    fn legal_install_path_reaches_committed() {
        let mut tx = transaction();
        for next in [
            Phase::Prepared,
            Phase::WaitingForOldExit,
            Phase::Swapping,
            Phase::LaunchingCandidate,
            Phase::AwaitingHealth,
            Phase::Committed,
        ] {
            tx.transition(next).unwrap();
        }
        assert_eq!(tx.phase, Phase::Committed);
    }

    #[test]
    fn illegal_transition_is_rejected_without_mutation() {
        let mut tx = transaction();
        let error = tx.transition(Phase::Committed).unwrap_err();
        assert_eq!(error.code, UpdateErrorCode::InvalidStateTransition);
        assert_eq!(tx.phase, Phase::Downloaded);
    }

    #[test]
    fn nonce_must_be_256_bit_hex() {
        let mut tx = transaction();
        tx.nonce = "short".into();
        assert_eq!(
            tx.validate().unwrap_err().code,
            UpdateErrorCode::InvalidTransaction
        );
        tx.nonce = "z".repeat(64);
        assert_eq!(
            tx.validate().unwrap_err().code,
            UpdateErrorCode::InvalidTransaction
        );
    }

    #[test]
    fn error_code_has_stable_snake_case_json() {
        assert_eq!(
            serde_json::to_string(&UpdateErrorCode::HealthConfirmationTimeout).unwrap(),
            "\"health_confirmation_timeout\""
        );
    }

    #[test]
    fn atomic_round_trip_preserves_transaction() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("transaction.json");
        let tx = transaction();
        save_atomic(&path, &tx).unwrap();
        assert_eq!(load(&path).unwrap(), tx);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn unknown_schema_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("transaction.json");
        let mut value = serde_json::to_value(transaction()).unwrap();
        value["schema_version"] = 99.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            load(&path).unwrap_err().code,
            UpdateErrorCode::UnsupportedTransactionSchema
        );
    }

    #[test]
    fn corrupt_json_is_structured_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("transaction.json");
        std::fs::write(&path, b"{").unwrap();
        assert_eq!(
            load(&path).unwrap_err().code,
            UpdateErrorCode::InvalidTransaction
        );
    }

    #[test]
    fn health_marker_matches_transaction_pid_version_and_path() {
        let mut tx = transaction();
        tx.candidate_pid = Some(99);
        let marker = HealthMarker {
            schema_version: 1,
            transaction_id: tx.transaction_id.clone(),
            nonce: tx.nonce.clone(),
            version: tx.new_version.clone(),
            pid: 99,
            executable: tx.installed_bundle_path.join("Contents/MacOS/sleipnir"),
        };
        assert!(marker.matches(&tx, 99));
        assert!(!marker.matches(&tx, 100));
    }
}
