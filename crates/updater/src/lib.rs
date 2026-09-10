//! Lightweight self-updater for Sleipnir.
//!
//! Queries GitHub Releases for the latest version, downloads the macOS `.dmg`
//! artifact, verifies its SHA-256 against the published `.sha256` sidecar, then
//! atomically replaces the running `.app` bundle via a detached helper script
//! and relaunches through a bundled transactional Rust supervisor.
//!
//! No Sparkle or OpenSSL runtime dependency: network verification uses rustls,
//! Ed25519, and SHA-256.

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
#[cfg(target_os = "macos")]
use std::time::Duration;

/// GitHub `owner/repo` slug used for the releases API and download URLs.
pub const REPO: &str = "Maidang1/sleipnir";

/// URL opened as a manual-install fallback when in-place replacement fails.
pub const RELEASES_PAGE: &str = "https://github.com/Maidang1/sleipnir/releases/latest";

#[cfg(target_os = "macos")]
const USER_AGENT: &str = "sleipnir-updater";

/// Ed25519 release-signing public key. The matching private key is stored only
/// in the GitHub Actions `SLEIPNIR_UPDATE_SIGNING_KEY` secret.
#[cfg(target_os = "macos")]
const UPDATE_PUBLIC_KEY: [u8; 32] = [
    0x6e, 0xfa, 0xce, 0x05, 0x7e, 0x12, 0xe6, 0xfe, 0x60, 0xe1, 0x44, 0x72, 0x62, 0x8b, 0x37, 0x5b,
    0x65, 0x0d, 0x61, 0xb7, 0xb5, 0x3d, 0xf6, 0xd7, 0xb9, 0x21, 0x75, 0x8c, 0x44, 0x6b, 0x18, 0x44,
];

#[cfg(target_os = "macos")]
fn download_small(url: &str, limit: u64) -> Result<Vec<u8>> {
    ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .with_context(|| format!("download {url}"))?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .with_context(|| format!("read {url}"))
}

/// A newer release that can be installed.
#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    /// Parsed semantic version (tag with any leading `v` stripped).
    pub version: semver::Version,
    /// Original git tag (e.g. `v0.2.0`).
    pub tag: String,
    /// Release notes / body (markdown).
    pub notes: String,
    /// Direct download URL for the macOS `.dmg` artifact.
    pub artifact_url: String,
    /// Direct download URL for the `.dmg.sha256` sidecar (legacy bootstrap only).
    pub sha256_url: String,
    /// Signed manifest digest, when available.
    pub expected_sha256: Option<String>,
    /// Signed manifest byte count, when available.
    pub expected_size: Option<u64>,
    /// Minimum macOS version required by the signed manifest, if known.
    pub minimum_macos: String,
}

/// A downloaded artifact whose release identity has already been validated.
#[derive(Clone, Debug)]
pub struct VerifiedArtifact {
    pub path: PathBuf,
    pub version: semver::Version,
    pub minimum_macos: String,
}

/// Result of a version check.
#[derive(Clone, Debug)]
pub enum UpdateStatus {
    /// The running build is at or ahead of the latest release.
    UpToDate,
    /// A newer release is available.
    Available(ReleaseInfo),
}

// ── pure logic (unit-tested) ────────────────────────────────────────────────

/// Parse a release tag into a semantic version, tolerating a leading `v`.
pub fn parse_tag(tag: &str) -> Result<semver::Version> {
    let trimmed = tag.trim().trim_start_matches('v');
    semver::Version::parse(trimmed).with_context(|| format!("invalid version tag: {tag:?}"))
}

/// Whether `latest` is strictly newer than the `current` build version.
///
/// A malformed `current` version is treated as "very old" so the update is
/// still offered rather than silently skipped.
pub fn is_newer(current: &str, latest: &semver::Version) -> bool {
    match semver::Version::parse(current.trim().trim_start_matches('v')) {
        Ok(cur) => *latest > cur,
        Err(_) => true,
    }
}

/// Asset filename marker and file extension for the macOS release artifact.
/// macOS ships `*-macos.dmg`; the SHA-256 sidecar is `<artifact>.sha256`.
///
/// In-place updates are macOS-only by design, so this exists only on macOS:
/// other platforms cannot reach dmg matching at compile time.
#[cfg(target_os = "macos")]
pub fn platform_asset_markers() -> (&'static str, &'static str) {
    ("-macos", ".dmg")
}

/// Select the release artifact and its `.sha256` sidecar for the current OS.
///
/// Returns `(artifact_url, sha256_url)`.
#[cfg(target_os = "macos")]
pub fn pick_asset(release: &Value) -> Result<(String, String)> {
    let (marker, extension) = platform_asset_markers();
    pick_asset_for(release, marker, extension)
}

/// Select a release artifact matching `marker` + `extension` (and its sidecar).
/// Testable for any OS by passing explicit markers.
pub fn pick_asset_for(release: &Value, marker: &str, extension: &str) -> Result<(String, String)> {
    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("release JSON missing `assets` array"))?;

    let mut artifact_url = None;
    let mut sha_url = None;
    for asset in assets {
        let name = asset
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let url = asset
            .get("browser_download_url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if url.is_empty() || !name.contains(marker) {
            continue;
        }
        if name.ends_with(".sha256") {
            sha_url = Some(url.to_string());
        } else if name.ends_with(extension) {
            artifact_url = Some(url.to_string());
        }
    }

    let artifact_url =
        artifact_url.ok_or_else(|| anyhow!("no `*{marker}*{extension}` asset in release"))?;
    let sha256_url =
        sha_url.ok_or_else(|| anyhow!("no `*{marker}*{extension}.sha256` asset in release"))?;
    Ok((artifact_url, sha256_url))
}

/// Parse a full release JSON document into a [`ReleaseInfo`].
#[cfg(target_os = "macos")]
pub fn parse_release(release: &Value) -> Result<ReleaseInfo> {
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("release JSON missing `tag_name`"))?
        .to_string();
    let version = parse_tag(&tag)?;
    let notes = release
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let (artifact_url, sha256_url) = pick_asset(release)?;
    Ok(ReleaseInfo {
        version,
        tag,
        notes,
        artifact_url,
        sha256_url,
        expected_sha256: None,
        expected_size: None,
        minimum_macos: String::new(),
    })
}

// ── network / IO (synchronous; run on a background thread) ──────────────────
//
// ureq is a blocking client with no async runtime dependency, so it works on
// GPUI's smol-based executor. (reqwest/hyper require a Tokio reactor and panic
// otherwise — do not reintroduce it here.)

/// Max bytes we'll read for the release `.dmg` (guards against runaway reads).
#[cfg(target_os = "macos")]
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// Query GitHub for the latest release and compare against `current_version`.
///
/// Blocking — call from `cx.background_spawn`. The signed-manifest update
/// channel only ships macOS artifacts, so on other platforms this fails
/// without touching the network and points at the releases page.
pub fn fetch_latest(current_version: &str) -> Result<UpdateStatus> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = current_version;
        bail!("in-place update is not supported on this platform; open {RELEASES_PAGE}");
    }
    #[cfg(target_os = "macos")]
    {
        fetch_latest_macos(current_version)
    }
}

#[cfg(target_os = "macos")]
fn fetch_latest_macos(current_version: &str) -> Result<UpdateStatus> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .context("request latest release")?;
    if resp.status().as_u16() == 403 {
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        if header("x-ratelimit-remaining").as_deref() == Some("0") {
            // Anonymous GitHub API quota is 60 requests/hour per IP, which a
            // shared egress IP exhausts quickly.
            let retry = header("x-ratelimit-reset")
                .and_then(|value| value.parse::<u64>().ok())
                .map(|reset| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let minutes = reset.saturating_sub(now).div_ceil(60);
                    format!(" Try again in about {minutes} minute(s).")
                })
                .unwrap_or_default();
            bail!("GitHub rate-limited the update check.{retry}");
        }
        bail!("GitHub refused the update check (HTTP 403)");
    }
    if !resp.status().is_success() {
        bail!("latest release request failed with HTTP {}", resp.status());
    }
    let release: Value = resp.body_mut().read_json().context("decode release JSON")?;
    let tag = release
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("release JSON missing `tag_name`"))?;
    let latest = parse_tag(tag)?;
    if !is_newer(current_version, &latest) {
        return Ok(UpdateStatus::UpToDate);
    }

    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("release JSON missing `assets` array"))?;
    let asset_url = |name: &str| {
        assets.iter().find_map(|asset| {
            (asset.get("name").and_then(Value::as_str) == Some(name))
                .then(|| asset.get("browser_download_url").and_then(Value::as_str))
                .flatten()
        })
    };
    let manifest_url = asset_url("sleipnir-update-v1.json")
        .ok_or_else(|| anyhow!("release is missing signed update manifest"))?;
    let signature_url = asset_url("sleipnir-update-v1.json.sig")
        .ok_or_else(|| anyhow!("release is missing update manifest signature"))?;
    let manifest_bytes = download_small(manifest_url, 64 * 1024)?;
    let signature = download_small(signature_url, 1024)?;
    let manifest =
        crate::manifest::verify_and_parse(&manifest_bytes, &signature, &UPDATE_PUBLIC_KEY)
            .map_err(|err| anyhow!("verify update manifest: {err}"))?;
    if manifest.version != latest || manifest.tag != tag {
        bail!("signed manifest does not match latest release tag");
    }
    crate::release::validate_upgrade(current_version, &manifest)
        .map_err(|err| anyhow!("release version rejected: {err:?}"))?;
    let urls = crate::release::release_urls(&manifest)
        .map_err(|err| anyhow!("release identity rejected: {err:?}"))?;
    let notes = release
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(UpdateStatus::Available(ReleaseInfo {
        version: manifest.version,
        tag: manifest.tag,
        notes,
        artifact_url: urls.artifact,
        sha256_url: String::new(),
        expected_sha256: Some(manifest.sha256),
        expected_size: Some(manifest.size),
        minimum_macos: manifest.minimum_macos,
    }))
}

/// Download the release `.dmg`, verify its SHA-256, and return the local path.
///
/// The dmg is written to `dest_dir`, which the caller owns and should clean up.
/// Blocking — call from `cx.background_spawn`. Non-macOS platforms have no
/// in-place update artifact to download.
pub fn download_and_verify(info: &ReleaseInfo, dest_dir: &Path) -> Result<VerifiedArtifact> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (info, dest_dir);
        bail!("in-place update is not supported on this platform; open {RELEASES_PAGE}");
    }
    #[cfg(target_os = "macos")]
    {
        download_and_verify_macos(info, dest_dir)
    }
}

#[cfg(target_os = "macos")]
fn download_and_verify_macos(info: &ReleaseInfo, dest_dir: &Path) -> Result<VerifiedArtifact> {
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("create staging dir {}", dest_dir.display()))?;

    let expected = info
        .expected_sha256
        .clone()
        .ok_or_else(|| anyhow!("release is missing signed SHA-256 metadata"))?;
    let expected_size = info
        .expected_size
        .ok_or_else(|| anyhow!("release is missing signed size metadata"))?;
    if expected_size > MAX_ARTIFACT_BYTES {
        bail!("release artifact exceeds maximum size");
    }
    let mut response = ureq::get(&info.artifact_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .context("download release dmg")?;
    let (_, extension) = platform_asset_markers();
    let file_name = format!("Sleipnir-{}-downloaded{}", info.version, extension);
    let path = crate::download::download_verified(
        response.body_mut().as_reader(),
        expected_size,
        &expected,
        dest_dir,
        &file_name,
    )
    .map_err(|err| anyhow!("download release dmg: {err}"))?;
    Ok(VerifiedArtifact {
        path,
        version: info.version.clone(),
        minimum_macos: info.minimum_macos.clone(),
    })
}

/// Whether this platform can swap the running install in place.
pub fn in_place_update_supported() -> bool {
    in_place_update_supported_for(cfg!(target_os = "macos"))
}

fn in_place_update_supported_for(macos: bool) -> bool {
    macos
}

/// Resolve the `.app` bundle that contains the current executable.
///
/// Returns `None` in development (running the bare `target/debug/sleipnir`)
/// and on platforms without a macOS-style bundle, which signals callers to
/// fall back to opening the releases page.
pub fn current_app_bundle_path() -> Option<PathBuf> {
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().ok()?;
        // .../Sleipnir.app/Contents/MacOS/sleipnir
        let macos = exe.parent()?; // MacOS
        let contents = macos.parent()?; // Contents
        let app = contents.parent()?; // Sleipnir.app
        if app.extension().and_then(|e| e.to_str()) == Some("app") {
            Some(app.to_path_buf())
        } else {
            None
        }
    }
}

/// Mount the downloaded dmg, copy the `.app` out, detach the image, then spawn
/// a detached helper that waits for this process to exit, atomically swaps the
/// `.app`, and relaunches it.
///
/// On failure to install (permissions, etc.) the helper opens the releases page
/// for a manual install. Non-macOS platforms have no in-place helper.
pub fn install_and_relaunch(artifact: &VerifiedArtifact, app_bundle: &Path) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (artifact, app_bundle);
        bail!("in-place update is not supported on this platform; open {RELEASES_PAGE}");
    }
    #[cfg(target_os = "macos")]
    {
        install_and_relaunch_macos(artifact, app_bundle)
    }
}

#[cfg(target_os = "macos")]
fn install_and_relaunch_macos(artifact: &VerifiedArtifact, app_bundle: &Path) -> Result<()> {
    ensure_host_meets_minimum_macos(&artifact.minimum_macos)?;

    let dmg_path = &artifact.path;
    let stage = dmg_path
        .parent()
        .ok_or_else(|| anyhow!("dmg has no parent dir"))?
        .join("extract");
    // Clean any prior extraction.
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).context("create extract dir")?;

    // Mount the dmg at a private mountpoint (no Finder window, no browse).
    let mount = dmg_path
        .parent()
        .ok_or_else(|| anyhow!("dmg has no parent dir"))?
        .join("mnt");
    let _ = std::process::Command::new("/usr/bin/hdiutil")
        .arg("detach")
        .arg(&mount)
        .arg("-force")
        .status();
    let _ = std::fs::remove_dir_all(&mount);
    std::fs::create_dir_all(&mount).context("create mount dir")?;
    let status = std::process::Command::new("/usr/bin/hdiutil")
        .arg("attach")
        .arg(dmg_path)
        .arg("-nobrowse")
        .arg("-noautoopen")
        .arg("-mountpoint")
        .arg(&mount)
        .status()
        .context("run hdiutil attach")?;
    if !status.success() {
        bail!("hdiutil attach failed");
    }

    // Copy the .app out of the mounted image, then detach it (always).
    let copy_result = (|| -> Result<()> {
        let mounted_app = find_app_bundle(&mount)?;
        let out = stage.join(
            mounted_app
                .file_name()
                .unwrap_or_else(|| "Sleipnir.app".as_ref()),
        );
        // Use ditto to preserve resource forks / signatures.
        let status = std::process::Command::new("/usr/bin/ditto")
            .arg(&mounted_app)
            .arg(&out)
            .status()
            .context("run ditto to copy app out of dmg")?;
        if !status.success() {
            bail!("ditto copy failed");
        }
        Ok(())
    })();

    let _ = std::process::Command::new("/usr/bin/hdiutil")
        .arg("detach")
        .arg(&mount)
        .arg("-force")
        .status();
    copy_result?;

    // Locate the extracted .app (top-level entry ending in .app).
    let new_app = find_app_bundle(&stage)?;
    let inner_bin = new_app.join("Contents/MacOS/sleipnir");
    if !inner_bin.exists() {
        bail!(
            "extracted bundle missing executable: {}",
            inner_bin.display()
        );
    }

    // Materialize the candidate beside the installed app so RENAME_SWAP is
    // same-volume and atomic. Failure here leaves the running app untouched.
    let root = crate::install::updates_root().map_err(|error| anyhow!(error))?;
    // A previous transaction may be stuck (supervisor died mid-update); finish
    // or roll it back first instead of refusing every later update.
    crate::install::recover_active_transaction(&root).map_err(|error| anyhow!(error))?;
    let (transaction_path, mut transaction) = crate::install::new_transaction(
        &root,
        app_bundle,
        dmg_path,
        &inner_bundle_version(app_bundle)?,
        &artifact.version.to_string(),
        std::process::id(),
    )
    .map_err(|error| anyhow!(error))?;
    let result = (|| -> Result<()> {
        let candidate = &transaction.adjacent_candidate_path;
        let candidate_parent = candidate
            .parent()
            .ok_or_else(|| anyhow!("candidate path has no parent"))?;
        std::fs::create_dir_all(candidate_parent)
            .with_context(|| format!("create adjacent staging {}", candidate_parent.display()))?;
        let _ = std::fs::remove_dir_all(candidate);
        let status = std::process::Command::new("/usr/bin/ditto")
            .arg(&new_app)
            .arg(candidate)
            .status()
            .context("copy candidate beside installed app")?;
        if !status.success() {
            bail!(
                "cannot stage update beside {}; install manually",
                app_bundle.display()
            );
        }

        validate_candidate_bundle(candidate, app_bundle, &artifact.version.to_string())?;
        let packaged_helper = candidate.join("Contents/MacOS/sleipnir-update-helper");
        crate::install::activate_prepared_transaction(&root, &transaction_path, &mut transaction)
            .map_err(|error| anyhow!(error))?;
        let transaction_dir = transaction_path.parent().expect("transaction has parent");
        let helper = transaction_dir.join("update-helper");
        std::fs::copy(&packaged_helper, &helper).context("copy update supervisor")?;
        let mut permissions = std::fs::metadata(&helper)?.permissions();
        use std::os::unix::fs::PermissionsExt as _;
        permissions.set_mode(0o700);
        std::fs::set_permissions(&helper, permissions)?;
        let mut helper_child = crate::install::launch_supervisor(&helper, &transaction_path)
            .map_err(|error| anyhow!(error))?;
        crate::install::wait_for_supervisor_ready(
            &transaction_path,
            &mut helper_child,
            Duration::from_secs(5),
        )
        .map_err(anyhow::Error::new)?;
        Ok(())
    })();
    if result.is_err() {
        let safe_cleanup = result.as_ref().err().and_then(|error| {
            error
                .downcast_ref::<crate::install::WaitForSupervisorReadyError>()
                .map(|error| {
                    error.cleanup_disposition() == crate::install::CleanupDisposition::Safe
                })
        });
        if safe_cleanup != Some(false) {
            let _ = crate::install::clear_active_pointer(&root, &transaction_path);
        }
        if safe_cleanup.unwrap_or(true) {
            let _ = std::fs::remove_dir_all(&transaction.adjacent_candidate_path);
        }
    }
    result
}

/// Find the single `.app` bundle directly under `dir`.
#[cfg(target_os = "macos")]
fn find_app_bundle(dir: &Path) -> Result<PathBuf> {
    for entry in std::fs::read_dir(dir).context("read extract dir")? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            return Ok(path);
        }
    }
    bail!("no .app bundle found in {}", dir.display())
}

#[cfg(target_os = "macos")]
fn validate_candidate_bundle(
    app: &Path,
    installed_app: &Path,
    expected_version: &str,
) -> Result<()> {
    let facts = collect_bundle_facts(app, installed_app)?;
    match crate::prepare::classify_preflight(&facts, expected_version) {
        crate::prepare::PreflightDecision::Ready => Ok(()),
        crate::prepare::PreflightDecision::ManualInstallRequired => {
            bail!("automatic replacement is unavailable for the current installation location")
        }
        crate::prepare::PreflightDecision::Reject(
            crate::prepare::PreflightError::BundleIdentifierMismatch,
        ) => {
            bail!("candidate bundle identifier does not match Sleipnir")
        }
        crate::prepare::PreflightDecision::Reject(
            crate::prepare::PreflightError::BundleVersionMismatch,
        ) => {
            bail!("candidate bundle version changed during preparation")
        }
        crate::prepare::PreflightDecision::Reject(
            crate::prepare::PreflightError::BundleLayoutInvalid,
        ) => {
            bail!("candidate bundle is missing a required executable")
        }
        crate::prepare::PreflightDecision::Reject(
            crate::prepare::PreflightError::BundleSignatureInvalid,
        ) => {
            bail!("candidate bundle signature is invalid")
        }
    }
}

#[cfg(target_os = "macos")]
fn inner_bundle_version(app: &Path) -> Result<String> {
    let output = std::process::Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleShortVersionString")
        .output()
        .context("read candidate bundle version")?;
    if !output.status.success() {
        bail!("candidate bundle version is unreadable");
    }
    let version = String::from_utf8(output.stdout)
        .context("candidate bundle version is not UTF-8")?
        .trim()
        .to_string();
    parse_tag(&version)?;
    Ok(version)
}

#[cfg(target_os = "macos")]
fn inner_bundle_build_version(app: &Path) -> Result<String> {
    let output = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleVersion")
        .output()
        .context("read candidate bundle build version")?;
    if !output.status.success() {
        bail!("candidate bundle build version is unreadable");
    }
    let version = String::from_utf8(output.stdout)
        .context("candidate bundle build version is not UTF-8")?
        .trim()
        .to_string();
    parse_tag(&version)?;
    Ok(version)
}

#[cfg(target_os = "macos")]
fn read_bundle_identifier(app: &Path) -> Result<String> {
    let bundle_id = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleIdentifier")
        .output()
        .context("read candidate bundle identifier")?;
    if !bundle_id.status.success() {
        bail!("candidate bundle identifier is unreadable");
    }
    Ok(String::from_utf8(bundle_id.stdout)
        .context("candidate bundle identifier is not UTF-8")?
        .trim()
        .to_string())
}

#[cfg(target_os = "macos")]
fn canonical_path(path: &Path, context: &str) -> Result<PathBuf> {
    std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize {context}: {}", path.display()))
}

#[cfg(target_os = "macos")]
fn collect_bundle_facts(app: &Path, installed_app: &Path) -> Result<crate::prepare::BundleFacts> {
    let executable = app.join("Contents/MacOS/sleipnir");
    let helper = app.join("Contents/MacOS/sleipnir-update-helper");
    let executable_canonical = canonical_path(&executable, "candidate executable")?;
    let helper_canonical = canonical_path(&helper, "candidate helper")?;
    let app_canonical = canonical_path(app, "candidate bundle")?;
    let installed_canonical = canonical_path(installed_app, "installed bundle")?;

    let signature_valid = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .status()
        .context("verify candidate bundle signature")?
        .success();

    let candidate_parent = app
        .parent()
        .ok_or_else(|| anyhow!("candidate path has no parent"))?;
    let installed_parent = installed_app
        .parent()
        .ok_or_else(|| anyhow!("installed app has no parent"))?;

    Ok(crate::prepare::BundleFacts {
        bundle_id: read_bundle_identifier(app)?,
        version: inner_bundle_version(app)?,
        build_version: inner_bundle_build_version(app)?,
        executable_exists: executable.is_file(),
        helper_exists: helper.is_file(),
        signature_valid,
        critical_paths_inside_bundle: crate::prepare::path_is_within(
            &app_canonical,
            &executable_canonical,
        ) && crate::prepare::path_is_within(
            &app_canonical,
            &helper_canonical,
        ),
        target_install_path_valid: installed_app.is_absolute()
            && !std::fs::symlink_metadata(installed_app)
                .map(|meta| meta.file_type().is_symlink())
                .unwrap_or(false)
            && installed_canonical
                == canonical_path(installed_parent, "installed app parent")?.join(
                    installed_app
                        .file_name()
                        .ok_or_else(|| anyhow!("installed app has no file name"))?,
                ),
        install_parent_writable: install_parent_is_writable(installed_parent),
        same_volume: same_volume(candidate_parent, installed_parent)?,
        swap_supported: swap_support_probe(installed_parent)?,
    })
}

#[cfg(target_os = "macos")]
fn install_parent_is_writable(parent: &Path) -> bool {
    let probe = parent.join(format!(".sleipnir-write-probe-{}", uuid::Uuid::new_v4()));
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn same_volume(first_parent: &Path, second_parent: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt as _;
    Ok(std::fs::metadata(first_parent)?.dev() == std::fs::metadata(second_parent)?.dev())
}

#[cfg(target_os = "macos")]
fn swap_support_probe(parent: &Path) -> Result<bool> {
    let probe_dir = parent.join(format!(".sleipnir-swap-probe-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&probe_dir)
        .with_context(|| format!("create swap probe dir {}", probe_dir.display()))?;
    let first = probe_dir.join("first");
    let second = probe_dir.join("second");
    let result = (|| -> Result<bool> {
        std::fs::write(&first, b"a")?;
        std::fs::write(&second, b"b")?;
        match crate::install::swap_paths(&first, &second) {
            Ok(()) => {
                crate::install::swap_paths(&first, &second).map_err(|error| anyhow!(error))?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    })();
    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
    let _ = std::fs::remove_dir(&probe_dir);
    result
}

#[cfg(target_os = "macos")]
fn host_meets_minimum_macos_version(host_version: &str, minimum: &str) -> Result<bool> {
    fn parse(version: &str) -> Result<Vec<u64>> {
        let trimmed = version.trim();
        if trimmed.is_empty() {
            bail!("version string is empty");
        }
        trimmed
            .split('.')
            .map(|part| {
                part.parse::<u64>()
                    .with_context(|| format!("invalid macOS version component: {part}"))
            })
            .collect()
    }

    let mut host = parse(host_version)?;
    let mut required = parse(minimum)?;
    let width = host.len().max(required.len());
    host.resize(width, 0);
    required.resize(width, 0);
    Ok(host >= required)
}

#[cfg(target_os = "macos")]
fn current_macos_version() -> Result<String> {
    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .context("read host macOS version")?;
    if !output.status.success() {
        bail!("host macOS version is unreadable");
    }
    Ok(String::from_utf8(output.stdout)
        .context("host macOS version is not UTF-8")?
        .trim()
        .to_string())
}

#[cfg(target_os = "macos")]
fn ensure_host_meets_minimum_macos(minimum: &str) -> Result<()> {
    if minimum.trim().is_empty() {
        return Ok(());
    }
    let host = current_macos_version()?;
    if host_meets_minimum_macos_version(&host, minimum)? {
        Ok(())
    } else {
        bail!("update requires macOS {minimum} or newer; host is {host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn parse_tag_strips_v() {
        assert_eq!(parse_tag("v0.2.0").unwrap(), semver::Version::new(0, 2, 0));
        assert_eq!(parse_tag("0.2.0").unwrap(), semver::Version::new(0, 2, 0));
        assert_eq!(
            parse_tag("  v1.4.9 ").unwrap(),
            semver::Version::new(1, 4, 9)
        );
        assert!(parse_tag("not-a-version").is_err());
    }

    #[test]
    fn is_newer_compares_semver() {
        let latest = semver::Version::new(0, 2, 0);
        assert!(is_newer("0.1.1", &latest));
        assert!(is_newer("v0.1.9", &latest));
        assert!(!is_newer("0.2.0", &latest));
        assert!(!is_newer("0.3.0", &latest));
        // Malformed current -> treat as very old (offer update).
        assert!(is_newer("garbage", &latest));
    }

    #[test]
    fn is_newer_respects_prerelease() {
        let latest = semver::Version::parse("0.2.0").unwrap();
        // A stable release is newer than the same-number prerelease.
        assert!(is_newer("0.2.0-beta.1", &latest));
    }

    #[test]
    fn pick_asset_selects_dmg_and_sidecar() {
        let release = json!({
            "assets": [
                {"name": "Sleipnir-0.2.0-windows-x64.exe",
                 "browser_download_url": "https://x/exe"},
                {"name": "Sleipnir-0.2.0-macos.dmg",
                 "browser_download_url": "https://x/dmg"},
                {"name": "Sleipnir-0.2.0-macos.dmg.sha256",
                 "browser_download_url": "https://x/sha"}
            ]
        });
        let (dmg, sha) = pick_asset_for(&release, "-macos", ".dmg").unwrap();
        assert_eq!(dmg, "https://x/dmg");
        assert_eq!(sha, "https://x/sha");
    }

    #[test]
    fn pick_asset_errors_without_dmg() {
        let release = json!({
            "assets": [
                {"name": "Sleipnir-0.2.0-windows-x64.exe",
                 "browser_download_url": "https://x/exe"}
            ]
        });
        assert!(pick_asset_for(&release, "-macos", ".dmg").is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn parse_release_full() {
        let (marker, extension) = platform_asset_markers();
        let asset_name = format!("Sleipnir-0.2.0{marker}myarch{extension}");
        let release = json!({
            "tag_name": "v0.2.0",
            "body": "notes here",
            "assets": [
                {"name": asset_name,
                 "browser_download_url": "https://x/zip"},
                {"name": format!("{asset_name}.sha256"),
                 "browser_download_url": "https://x/sha"}
            ]
        });
        let info = parse_release(&release).unwrap();
        assert_eq!(info.version, semver::Version::new(0, 2, 0));
        assert_eq!(info.tag, "v0.2.0");
        assert_eq!(info.notes, "notes here");
        assert_eq!(info.artifact_url, "https://x/zip");
        assert_eq!(info.sha256_url, "https://x/sha");
        assert_eq!(info.expected_sha256, None);
        assert_eq!(info.expected_size, None);
        assert_eq!(info.minimum_macos, "");
    }

    #[test]
    fn verified_artifact_carries_path_version_and_minimum_macos() {
        let artifact = VerifiedArtifact {
            path: PathBuf::from("/tmp/Sleipnir.dmg"),
            version: semver::Version::new(0, 3, 2),
            minimum_macos: "14.0".into(),
        };
        assert_eq!(artifact.path, PathBuf::from("/tmp/Sleipnir.dmg"));
        assert_eq!(artifact.version, semver::Version::new(0, 3, 2));
        assert_eq!(artifact.minimum_macos, "14.0");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn minimum_macos_gate_compares_dotted_versions() {
        assert!(host_meets_minimum_macos_version("14.6.1", "14.0").unwrap());
        assert!(host_meets_minimum_macos_version("14.0", "14.0").unwrap());
        assert!(!host_meets_minimum_macos_version("13.6.9", "14.0").unwrap());
        assert!(!host_meets_minimum_macos_version("14.0", "14.0.1").unwrap());
    }

    #[test]
    fn in_place_update_is_macos_only() {
        assert!(in_place_update_supported_for(true));
        assert!(!in_place_update_supported_for(false));
        assert_eq!(in_place_update_supported(), cfg!(target_os = "macos"));

        let src = include_str!("lib.rs");
        assert!(
            src.contains("in_place_update_supported_for(cfg!(target_os = \"macos\"))"),
            "capability detection must be macOS-specific"
        );
        assert!(
            !src.contains("pub fn in_place_update_supported() -> bool {\n    cfg!(unix)"),
            "Linux is Unix but must never enter DMG replacement"
        );
    }

    #[test]
    fn production_install_path_uses_rust_supervisor_not_shell_script() {
        let src = include_str!("lib.rs");
        let forbidden_shell_helper = ["fn write_", "helper_script("].concat();
        assert!(!src.contains(&forbidden_shell_helper));
        let forbidden_shell = ["/bin/", "sh"].concat();
        assert!(!src.contains(&forbidden_shell));
        assert!(src.contains("launch_supervisor"));
        assert!(src.contains("wait_for_supervisor_ready"));
    }

    #[test]
    fn production_install_path_uses_verified_artifact_preflight_and_preserves_wait_error_type() {
        let src = include_str!("lib.rs");
        let production = src
            .split("#[cfg(test)]")
            .next()
            .expect("production section before tests");
        assert!(
            production.contains("pub fn download_and_verify(info: &ReleaseInfo, dest_dir: &Path) -> Result<VerifiedArtifact>"),
            "download contract must return a typed verified artifact"
        );
        assert!(
            production.contains("pub fn install_and_relaunch(artifact: &VerifiedArtifact, app_bundle: &Path) -> Result<()>"),
            "install contract must accept the verified artifact"
        );
        assert!(
            production.contains("classify_preflight("),
            "production install must run the real preflight classifier"
        );
        assert!(
            !production.contains("match &info.expected_sha256"),
            "production download must not fall back to unsigned sidecar metadata"
        );
        assert!(
            !production.contains(".map_err(anyhow::Error::msg)?"),
            "WaitForSupervisorReadyError must not be type-erased"
        );
    }
}

pub mod download;
pub mod install;
pub mod manifest;
pub mod prepare;
pub mod recovery;
pub mod release;
pub mod transaction;
