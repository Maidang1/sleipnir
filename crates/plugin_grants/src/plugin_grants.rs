//! Per-plugin consent, bound to binary identity (ADR-0016 §5–§6).
//!
//! A grant has to name one plugin *and* the bytes that were approved. Grants
//! therefore live in their own file (`plugin-grants.json`), not in
//! `settings.json` (which is hand-edited).
//!
//! **`binary_hash` is the load-bearing field.** If the binary changes, the grant
//! is void and consent is asked again. Without that, a benign plugin can
//! self-update into a malicious one while keeping permissions the user approved
//! for different code. That is the single most important behaviour in this
//! crate.
//!
//! Load is corruption-tolerant and fail-closed: an unreadable, corrupt, or
//! unknown-version file is quarantined to `.bak` and treated as empty — empty
//! grants mean everything needs consent. A broken file must never fail open.
//!
//! Pure decision logic. No gpui, no dialogs, no async. The UI calls [`check`],
//! then writes a [`GrantRecord`] if the user consents.

use plugin_protocol::v2::Capability;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// On-disk schema version. Unknown versions are treated as corrupt.
pub const GRANTS_VERSION: u32 = 1;

const GRANTS_FILE: &str = "plugin-grants.json";

/// Trust tier (ADR-0016 §6). Staged rather than blocking the whole programme
/// on a cross-platform sandbox.
///
/// External installation does not ship before [`Tier::Sandboxed`] is real.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Built-in / first-party. Full capability; sandbox is n/a.
    BuiltIn,
    /// Locally authored by the user. Full, labelled "unsandboxed, local" in the UI.
    Local,
    /// Externally installed. Grants only; sandbox required. No `exec`; no
    /// network unless granted; filesystem limited to the plugin's own directory.
    Sandboxed,
}

/// One plugin's stored consent, bound to the bytes that were approved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantRecord {
    pub granted: BTreeSet<Capability>,
    /// `sha256:<hex>` of the binary at grant time. A mismatch voids the record.
    pub binary_hash: String,
    /// RFC3339 timestamp of when consent was last given.
    pub granted_at: String,
    pub tier: Tier,
}

/// On-disk envelope. Keyed by plugin id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantsFile {
    pub version: u32,
    #[serde(default)]
    pub grants: BTreeMap<String, GrantRecord>,
}

impl Default for GrantsFile {
    fn default() -> Self {
        Self {
            version: GRANTS_VERSION,
            grants: BTreeMap::new(),
        }
    }
}

/// Outcome of comparing a request against a stored grant. [`check`] never
/// denies: every gap is a consent question, not a silent block.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum Decision {
    Allowed,
    NeedsConsent {
        reason: ConsentReason,
        missing: Vec<Capability>,
    },
}

/// Why fresh consent is required. Distinct so the UI can explain the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentReason {
    FirstRun,
    /// The bytes on disk are not the bytes that were approved. Previous
    /// permissions do not carry over.
    BinaryChanged,
    NewCapabilities,
}

/// `sha256:<hex>` of a plugin binary, produced only by [`hash_binary`]. The
/// consent contract's identity type: an unhashable binary has no `BinaryHash`,
/// so a caller cannot smuggle an empty-string sentinel into [`check`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryHash(String);

impl BinaryHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Wrap a precomputed hash string. For tests and fixtures in other crates;
    /// production code obtains hashes from [`hash_binary`] only.
    pub fn from_raw(hash: String) -> Self {
        Self(hash)
    }
}

impl std::fmt::Display for BinaryHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Compare a capability request to a stored grant and the hash of the binary
/// now on disk.
///
/// Hash is checked first: a changed binary voids every previously approved
/// permission, even when the requested set is a subset of the old grant.
/// Observation / render / host-call capabilities (`Resident`, `SubscribeEvents`,
/// `Render*`, `HostCall*`) are never implied by the snapshot reads — membership
/// is exact (ADR-0016 §4).
pub fn check(
    request: &[Capability],
    record: Option<&GrantRecord>,
    actual_hash: &BinaryHash,
) -> Decision {
    let Some(record) = record else {
        return Decision::NeedsConsent {
            reason: ConsentReason::FirstRun,
            missing: request.to_vec(),
        };
    };

    if record.binary_hash != actual_hash.as_str() {
        return Decision::NeedsConsent {
            reason: ConsentReason::BinaryChanged,
            missing: request.to_vec(),
        };
    }

    let missing: Vec<Capability> = request
        .iter()
        .copied()
        .filter(|cap| !record.granted.contains(cap))
        .collect();
    if missing.is_empty() {
        Decision::Allowed
    } else {
        Decision::NeedsConsent {
            reason: ConsentReason::NewCapabilities,
            missing,
        }
    }
}

/// SHA-256 of `path`.
///
/// Streams the file; a plugin binary must not be pulled fully into memory just
/// to decide whether it is still the one that was approved.
pub fn hash_binary(path: &Path) -> io::Result<BinaryHash> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(BinaryHash(format!(
        "sha256:{}",
        hex_encode(&hasher.finalize())
    )))
}

/// RFC3339 UTC timestamp for [`GrantRecord::granted_at`]. This crate owns the
/// format so writers cannot drift from what readers persist.
pub fn now_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

fn rfc3339_from_unix(secs: u64) -> String {
    let mut days = secs / 86_400;
    let rem = secs % 86_400;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let mut year = 1970i32;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let feb = if is_leap(year) { 29 } else { 28 };
    let months = [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for dim in months {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    let day = days as u32 + 1;
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Config directory convention matching `plugin_host::default_plugin_dir_for`
/// and `sleipnir_settings::config_dir_for`: on macOS/Unix this is
/// `~/.config/sleipnir`, **not** `dirs::config_dir()` (`~/Library/Application
/// Support` on macOS). Grants must sit next to `settings.json`.
pub fn default_grants_path() -> PathBuf {
    default_grants_path_for(cfg!(windows))
}

/// Grants path for a given OS family. See [`default_grants_path`].
pub fn default_grants_path_for(windows: bool) -> PathBuf {
    let base = if windows {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir")
    };
    base.join(GRANTS_FILE)
}

/// Load grants. Missing file → empty; unreadable, corrupt, or unrecognized
/// version → quarantined as `.bak`, then empty.
///
/// **Never panics and never returns `Err`.** A broken grants file must fail
/// closed (empty = everything needs consent), never fail open.
pub fn load(path: &Path) -> GrantsFile {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return GrantsFile::default(),
        Err(_) => {
            atomic_write::quarantine(path);
            return GrantsFile::default();
        }
    };

    match parse_grants_file(&bytes) {
        Some(file) => file,
        None => {
            atomic_write::quarantine(path);
            GrantsFile::default()
        }
    }
}

/// Write `file` atomically (see `atomic_write`): stage to a sibling `.tmp`,
/// then rename over `path`.
pub fn save(path: &Path, file: &GrantsFile) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    atomic_write::save_atomic(path, &json)
}

fn parse_grants_file(bytes: &[u8]) -> Option<GrantsFile> {
    let file: GrantsFile = serde_json::from_slice(bytes).ok()?;
    (file.version == GRANTS_VERSION).then_some(file)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const GRANTED_AT: &str = "2026-01-01T00:00:00Z";

    fn record(hash: &str, caps: &[Capability]) -> GrantRecord {
        GrantRecord {
            granted: caps.iter().copied().collect(),
            binary_hash: hash.to_string(),
            granted_at: GRANTED_AT.into(),
            tier: Tier::Local,
        }
    }

    fn bh(hash: &str) -> BinaryHash {
        BinaryHash(hash.into())
    }

    fn grants_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("plugin-grants.json");
        (dir, path)
    }

    #[test]
    fn first_run_needs_consent_for_the_whole_request() {
        let request = [Capability::ReadCwd, Capability::SubscribeEvents];
        let Decision::NeedsConsent { reason, missing } = check(&request, None, &bh(HASH_A)) else {
            panic!("expected NeedsConsent");
        };
        assert_eq!(reason, ConsentReason::FirstRun);
        assert_eq!(missing, request);
    }

    #[test]
    fn exact_match_is_allowed() {
        let request = [Capability::ReadCwd, Capability::Network];
        let stored = record(HASH_A, &request);
        assert_eq!(
            check(&request, Some(&stored), &bh(HASH_A)),
            Decision::Allowed
        );
    }

    #[test]
    fn hash_mismatch_voids_the_grant() {
        // ADR-0016 §5: a benign plugin must never self-update into a malicious
        // one while keeping previously approved permissions. Capability overlap
        // is irrelevant once the bytes changed.
        let request = [Capability::ReadCwd, Capability::Network];
        let stored = record(HASH_A, &request);
        let Decision::NeedsConsent { reason, missing } =
            check(&request, Some(&stored), &bh(HASH_B))
        else {
            panic!("expected NeedsConsent");
        };
        assert_eq!(reason, ConsentReason::BinaryChanged);
        assert_eq!(missing, request);
    }

    #[test]
    fn hash_mismatch_takes_priority_over_new_capabilities() {
        let stored = record(HASH_A, &[Capability::ReadCwd]);
        let request = [Capability::ReadCwd, Capability::SubscribeEvents];
        let Decision::NeedsConsent { reason, missing } =
            check(&request, Some(&stored), &bh(HASH_B))
        else {
            panic!("expected NeedsConsent");
        };
        assert_eq!(reason, ConsentReason::BinaryChanged);
        assert_eq!(missing, request);
    }

    #[test]
    fn added_capability_needs_consent() {
        let stored = record(HASH_A, &[Capability::ReadCwd]);
        let request = [
            Capability::ReadCwd,
            Capability::Network,
            Capability::Clipboard,
        ];
        let Decision::NeedsConsent { reason, missing } =
            check(&request, Some(&stored), &bh(HASH_A))
        else {
            panic!("expected NeedsConsent");
        };
        assert_eq!(reason, ConsentReason::NewCapabilities);
        assert_eq!(missing, [Capability::Network, Capability::Clipboard]);
    }

    #[test]
    fn subset_of_a_wider_grant_is_allowed() {
        let stored = record(
            HASH_A,
            &[
                Capability::ReadCwd,
                Capability::Network,
                Capability::Clipboard,
            ],
        );
        let request = [Capability::ReadCwd, Capability::Clipboard];
        assert_eq!(
            check(&request, Some(&stored), &bh(HASH_A)),
            Decision::Allowed
        );
    }

    #[test]
    fn observation_capabilities_are_never_implied_by_snapshot_reads() {
        // ADR-0016 §4: SubscribeEvents is continuous observation, not "more
        // ReadCwd". Grant membership is exact; nothing promotes.
        let snapshots = [
            Capability::ReadSelection,
            Capability::ReadVisibleScreen,
            Capability::ReadCwd,
            Capability::ReadTitle,
            Capability::WriteTerminal,
            Capability::Clipboard,
            Capability::Network,
        ];
        let stored = record(HASH_A, &snapshots);
        for cap in [
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderBlock,
            Capability::RenderPanel,
            Capability::RenderStatus,
            Capability::HostCallNotify,
            Capability::HostCallReadScreen,
            Capability::HostCallListPanes,
            Capability::HostCallOpenPane,
            Capability::HostCallDrawScene,
        ] {
            let Decision::NeedsConsent { reason, missing } =
                check(&[cap], Some(&stored), &bh(HASH_A))
            else {
                panic!("{cap:?} must not be implied by the snapshot-read set");
            };
            assert_eq!(reason, ConsentReason::NewCapabilities);
            assert_eq!(missing, [cap]);
        }
    }

    #[test]
    fn default_grants_path_matches_settings_config_dir() {
        // Same regression as plugin_host: on macOS/Unix the file lives under
        // ~/.config/sleipnir, NOT dirs::config_dir().
        let unix = default_grants_path_for(false);
        assert!(
            unix.ends_with(".config/sleipnir/plugin-grants.json"),
            "unix grants path should be under ~/.config/sleipnir: {unix:?}"
        );
        let win = default_grants_path_for(true);
        assert!(win.ends_with("sleipnir/plugin-grants.json"), "{win:?}");
    }

    #[test]
    fn missing_file_returns_empty_without_quarantine() {
        let (_dir, path) = grants_path();
        let loaded = load(&path);
        assert_eq!(loaded, GrantsFile::default());
        assert!(!atomic_write::bak_path(&path).exists());
    }

    #[test]
    fn corrupt_file_fails_closed() {
        let (_dir, path) = grants_path();
        fs::write(&path, "NOT JSON {{{").unwrap();
        let loaded = load(&path);
        assert_eq!(
            loaded,
            GrantsFile::default(),
            "broken file must fail closed"
        );
        assert!(
            atomic_write::bak_path(&path).exists(),
            "corrupt file must be renamed to .bak"
        );
        assert!(
            !path.exists(),
            "corrupt original should be gone after quarantine"
        );
    }

    #[test]
    fn unknown_version_fails_closed() {
        let (_dir, path) = grants_path();
        fs::write(&path, r#"{"version":99,"grants":{}}"#).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded, GrantsFile::default());
        assert!(atomic_write::bak_path(&path).exists());
        assert!(!path.exists());
    }

    #[test]
    fn save_then_load_round_trips() {
        let (_dir, path) = grants_path();
        let mut file = GrantsFile::default();
        file.grants.insert(
            "port-watcher".into(),
            record(
                HASH_A,
                &[
                    Capability::ReadCwd,
                    Capability::SubscribeEvents,
                    Capability::RenderBlock,
                ],
            ),
        );
        file.grants.get_mut("port-watcher").unwrap().tier = Tier::Sandboxed;
        save(&path, &file).unwrap();
        assert!(
            !atomic_write::sibling_path(&path, ".tmp").exists(),
            "atomic save must not leave a tmp sibling"
        );
        assert_eq!(load(&path), file);
    }

    #[test]
    fn save_creates_parent_directories() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nested").join("plugin-grants.json");
        save(&path, &GrantsFile::default()).unwrap();
        assert_eq!(load(&path), GrantsFile::default());
    }

    #[test]
    fn adr_example_json_deserializes() {
        let raw = r#"{
            "version": 1,
            "grants": {
                "port-watcher": {
                    "granted": ["read_cwd", "subscribe_events", "render_block"],
                    "binary_hash": "sha256:ab12",
                    "granted_at": "2026-01-01T00:00:00Z",
                    "tier": "sandboxed"
                }
            }
        }"#;
        let file: GrantsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(file.version, 1);
        let rec = &file.grants["port-watcher"];
        assert_eq!(rec.tier, Tier::Sandboxed);
        assert_eq!(rec.binary_hash, "sha256:ab12");
        assert!(rec.granted.contains(&Capability::ReadCwd));
        assert!(rec.granted.contains(&Capability::SubscribeEvents));
        assert!(rec.granted.contains(&Capability::RenderBlock));
    }

    #[test]
    fn hash_binary_streams_and_prefixes_sha256() {
        let dir = tempfile::TempDir::new().unwrap();
        let empty = dir.path().join("empty.bin");
        fs::write(&empty, []).unwrap();
        assert_eq!(
            hash_binary(&empty).unwrap().as_str(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let hello = dir.path().join("hello.bin");
        fs::write(&hello, b"hello").unwrap();
        assert_eq!(
            hash_binary(&hello).unwrap().as_str(),
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        // Larger than the 8KiB read buffer, so a non-streaming implementation
        // is not the only path exercised.
        let big = dir.path().join("big.bin");
        let payload = vec![0x5a; 20 * 1024];
        fs::write(&big, &payload).unwrap();
        let streamed = hash_binary(&big).unwrap();
        let streamed = streamed.as_str();
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        assert_eq!(
            streamed,
            format!("sha256:{}", hex_encode(&hasher.finalize()))
        );
        assert!(streamed.starts_with("sha256:"));
        assert_eq!(streamed.len(), "sha256:".len() + 64);
    }

    #[test]
    fn rfc3339_unix_epoch() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1), "1970-01-01T00:00:01Z");
        assert_eq!(rfc3339_from_unix(1_704_067_200), "2024-01-01T00:00:00Z");
    }

    #[test]
    #[cfg(unix)]
    fn unix_permissions_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = grants_path();
        save(&path, &GrantsFile::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
