//! Lightweight self-updater for Sleipnir.
//!
//! Queries GitHub Releases for the latest version, downloads the macOS `.zip`
//! artifact, verifies its SHA-256 against the published `.sha256` sidecar, then
//! atomically replaces the running `.app` bundle via a detached helper script
//! and relaunches.
//!
//! No Sparkle, no OpenSSL — reqwest(rustls) + sha2 + a shell helper.

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
#[cfg(unix)]
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

/// GitHub `owner/repo` slug used for the releases API and download URLs.
pub const REPO: &str = "Maidang1/sleipnir";

/// URL opened as a manual-install fallback when in-place replacement fails.
pub const RELEASES_PAGE: &str = "https://github.com/Maidang1/sleipnir/releases/latest";

const USER_AGENT: &str = "sleipnir-updater";

/// A newer release that can be installed.
#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    /// Parsed semantic version (tag with any leading `v` stripped).
    pub version: semver::Version,
    /// Original git tag (e.g. `v0.2.0`).
    pub tag: String,
    /// Release notes / body (markdown).
    pub notes: String,
    /// Direct download URL for the macOS `.zip` artifact.
    pub zip_url: String,
    /// Direct download URL for the `.zip.sha256` sidecar.
    pub sha256_url: String,
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

/// Select the macOS `.zip` and its `.sha256` sidecar from a release JSON body.
///
/// Returns `(zip_url, sha256_url)`.
pub fn pick_asset(release: &Value) -> Result<(String, String)> {
    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("release JSON missing `assets` array"))?;

    let mut zip_url = None;
    let mut sha_url = None;
    for asset in assets {
        let name = asset.get("name").and_then(Value::as_str).unwrap_or_default();
        let url = asset
            .get("browser_download_url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        if name.ends_with("-macos.zip.sha256") {
            sha_url = Some(url.to_string());
        } else if name.ends_with("-macos.zip") {
            zip_url = Some(url.to_string());
        }
    }

    let zip_url = zip_url.ok_or_else(|| anyhow!("no `*-macos.zip` asset in release"))?;
    let sha256_url =
        sha_url.ok_or_else(|| anyhow!("no `*-macos.zip.sha256` asset in release"))?;
    Ok((zip_url, sha256_url))
}

/// Parse a full release JSON document into a [`ReleaseInfo`].
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
    let (zip_url, sha256_url) = pick_asset(release)?;
    Ok(ReleaseInfo {
        version,
        tag,
        notes,
        zip_url,
        sha256_url,
    })
}

/// Extract the first hex SHA-256 token from a `.sha256` sidecar body.
///
/// Accepts both bare-digest (`<hex>`) and `shasum`-style (`<hex>  file`) forms.
fn parse_sha256_sidecar(body: &str) -> Result<String> {
    let token = body
        .split_whitespace()
        .next()
        .map(str::trim)
        .filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| anyhow!("could not parse SHA-256 from sidecar"))?;
    Ok(token.to_ascii_lowercase())
}

// ── network / IO (synchronous; run on a background thread) ──────────────────
//
// ureq is a blocking client with no async runtime dependency, so it works on
// GPUI's smol-based executor. (reqwest/hyper require a Tokio reactor and panic
// otherwise — do not reintroduce it here.)

/// Max bytes we'll read for the release `.zip` (guards against runaway reads).
const MAX_ZIP_BYTES: u64 = 512 * 1024 * 1024;

/// Query GitHub for the latest release and compare against `current_version`.
///
/// Blocking — call from `cx.background_spawn`.
pub fn fetch_latest(current_version: &str) -> Result<UpdateStatus> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("request latest release")?;
    let release: Value = resp
        .body_mut()
        .read_json()
        .context("decode release JSON")?;

    let info = parse_release(&release)?;
    if is_newer(current_version, &info.version) {
        Ok(UpdateStatus::Available(info))
    } else {
        Ok(UpdateStatus::UpToDate)
    }
}

/// Download the release `.zip`, verify its SHA-256, and return the local path.
///
/// The zip is written to `dest_dir`, which the caller owns and should clean up.
/// Blocking — call from `cx.background_spawn`.
pub fn download_and_verify(info: &ReleaseInfo, dest_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("create staging dir {}", dest_dir.display()))?;

    // Expected digest from the sidecar.
    let sidecar = ureq::get(&info.sha256_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .context("download sha256 sidecar")?
        .body_mut()
        .read_to_string()
        .context("read sha256 sidecar body")?;
    let expected = parse_sha256_sidecar(&sidecar)?;

    // Zip payload.
    let bytes = ureq::get(&info.zip_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .context("download release zip")?
        .body_mut()
        .with_config()
        .limit(MAX_ZIP_BYTES)
        .read_to_vec()
        .context("read release zip body")?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = hex_lower(&hasher.finalize());
    if actual != expected {
        bail!("SHA-256 mismatch (expected {expected}, got {actual})");
    }

    let file_name = format!("Sleipnir-{}-macos.zip", info.version);
    let zip_path = dest_dir.join(file_name);
    std::fs::write(&zip_path, &bytes)
        .with_context(|| format!("write {}", zip_path.display()))?;
    Ok(zip_path)
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
}

/// Whether this platform can swap the running install in place.
pub fn in_place_update_supported() -> bool {
    cfg!(unix)
}

/// Resolve the `.app` bundle that contains the current executable.
///
/// Returns `None` in development (running the bare `target/debug/sleipnir`)
/// and on platforms without a macOS-style bundle, which signals callers to
/// fall back to opening the releases page.
pub fn current_app_bundle_path() -> Option<PathBuf> {
    #[cfg(not(unix))]
    {
        None
    }
    #[cfg(unix)]
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

/// Extract the downloaded zip, then spawn a detached helper that waits for this
/// process to exit, atomically swaps the `.app`, and relaunches it.
///
/// On failure to install (permissions, etc.) the helper opens the releases page
/// for a manual install. Windows has no in-place helper in Phase 1.
pub fn install_and_relaunch(zip_path: &Path, app_bundle: &Path) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (zip_path, app_bundle);
        bail!("in-place update is not supported on this platform; open {RELEASES_PAGE}");
    }
    #[cfg(unix)]
    {
        install_and_relaunch_unix(zip_path, app_bundle)
    }
}

#[cfg(unix)]
fn install_and_relaunch_unix(zip_path: &Path, app_bundle: &Path) -> Result<()> {
    let stage = zip_path
        .parent()
        .ok_or_else(|| anyhow!("zip has no parent dir"))?
        .join("extract");
    // Clean any prior extraction.
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).context("create extract dir")?;

    // Use ditto to preserve resource forks / signatures.
    let status = std::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(zip_path)
        .arg(&stage)
        .status()
        .context("run ditto to extract zip")?;
    if !status.success() {
        bail!("ditto extraction failed");
    }

    // Locate the extracted .app (top-level entry ending in .app).
    let new_app = find_app_bundle(&stage)?;
    let inner_bin = new_app.join("Contents/MacOS/sleipnir");
    if !inner_bin.exists() {
        bail!(
            "extracted bundle missing executable: {}",
            inner_bin.display()
        );
    }

    let pid = std::process::id();
    let helper = write_helper_script(
        pid,
        &new_app,
        app_bundle,
        zip_path.parent().unwrap_or(Path::new("/tmp")),
    )?;

    std::process::Command::new("/bin/sh")
        .arg(&helper)
        .spawn()
        .context("spawn update helper")?;
    Ok(())
}

/// Find the single `.app` bundle directly under `dir`.
#[cfg(unix)]
fn find_app_bundle(dir: &Path) -> Result<PathBuf> {
    for entry in std::fs::read_dir(dir).context("read extract dir")? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            return Ok(path);
        }
    }
    bail!("no .app bundle found in {}", dir.display())
}

/// Write a detached shell script that performs the swap-and-relaunch.
#[cfg(unix)]
fn write_helper_script(
    pid: u32,
    new_app: &Path,
    old_app: &Path,
    tmp_dir: &Path,
) -> Result<PathBuf> {
    let script_path = tmp_dir.join("sleipnir-update-helper.sh");
    let new_app = shell_quote(new_app);
    let old_app = shell_quote(old_app);
    let releases = RELEASES_PAGE;

    let body = format!(
        r#"#!/bin/sh
# Sleipnir auto-update helper — waits for the app to quit, then swaps bundles.
set -u

# Wait (bounded) for the parent process to exit.
i=0
while kill -0 {pid} 2>/dev/null; do
  sleep 0.2
  i=$((i+1))
  if [ "$i" -gt 300 ]; then
    break
  fi
done

NEW={new_app}
OLD={old_app}
BACKUP="$OLD.bak"

# Safe swap: backup old, move new into place, remove backup on success.
if mv "$OLD" "$BACKUP" 2>/dev/null; then
  if mv "$NEW" "$OLD" 2>/dev/null; then
    # Clear quarantine so Gatekeeper does not block the freshly-moved bundle.
    xattr -cr "$OLD" 2>/dev/null || true
    rm -rf "$BACKUP" 2>/dev/null || true
    open "$OLD"
  else
    # mv failed: restore backup
    mv "$BACKUP" "$OLD" 2>/dev/null || true
    open "{releases}"
  fi
else
  # Permission or IO failure — fall back to a manual install.
  open "{releases}"
fi
"#
    );

    let mut file = std::fs::File::create(&script_path)
        .with_context(|| format!("create helper script {}", script_path.display()))?;
    file.write_all(body.as_bytes())
        .context("write helper script")?;
    let mut perms = file.metadata()?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).context("chmod helper script")?;
    Ok(script_path)
}

/// Single-quote a path for safe embedding in a POSIX shell script.
#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn pick_asset_selects_zip_and_sidecar() {
        let release = json!({
            "assets": [
                {"name": "Sleipnir-0.2.0-macos.dmg",
                 "browser_download_url": "https://x/dmg"},
                {"name": "Sleipnir-0.2.0-macos.zip",
                 "browser_download_url": "https://x/zip"},
                {"name": "Sleipnir-0.2.0-macos.zip.sha256",
                 "browser_download_url": "https://x/sha"}
            ]
        });
        let (zip, sha) = pick_asset(&release).unwrap();
        assert_eq!(zip, "https://x/zip");
        assert_eq!(sha, "https://x/sha");
    }

    #[test]
    fn pick_asset_errors_without_zip() {
        let release = json!({
            "assets": [
                {"name": "Sleipnir-0.2.0-macos.dmg",
                 "browser_download_url": "https://x/dmg"}
            ]
        });
        assert!(pick_asset(&release).is_err());
    }

    #[test]
    fn parse_release_full() {
        let release = json!({
            "tag_name": "v0.2.0",
            "body": "notes here",
            "assets": [
                {"name": "Sleipnir-0.2.0-macos.zip",
                 "browser_download_url": "https://x/zip"},
                {"name": "Sleipnir-0.2.0-macos.zip.sha256",
                 "browser_download_url": "https://x/sha"}
            ]
        });
        let info = parse_release(&release).unwrap();
        assert_eq!(info.version, semver::Version::new(0, 2, 0));
        assert_eq!(info.tag, "v0.2.0");
        assert_eq!(info.notes, "notes here");
        assert_eq!(info.zip_url, "https://x/zip");
        assert_eq!(info.sha256_url, "https://x/sha");
    }

    #[test]
    fn sha256_sidecar_parsing() {
        let hex = "a".repeat(64);
        assert_eq!(parse_sha256_sidecar(&hex).unwrap(), hex);
        // shasum-style with filename.
        let line = format!("{hex}  Sleipnir-0.2.0-macos.zip");
        assert_eq!(parse_sha256_sidecar(&line).unwrap(), hex);
        // uppercase normalized to lowercase.
        let upper = "A".repeat(64);
        assert_eq!(parse_sha256_sidecar(&upper).unwrap(), "a".repeat(64));
        assert!(parse_sha256_sidecar("short").is_err());
    }

    #[test]
    fn in_place_update_is_unix_only() {
        assert_eq!(in_place_update_supported(), cfg!(unix));
    }

    #[cfg(unix)]
    #[test]
    fn shell_quote_escapes_single_quotes() {
        let q = shell_quote(Path::new("/tmp/it's here/Sleipnir.app"));
        assert_eq!(q, "'/tmp/it'\\''s here/Sleipnir.app'");
    }
}
