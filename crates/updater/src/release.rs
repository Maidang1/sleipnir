use crate::manifest::UpdateManifest;

pub const REPO: &str = "Maidang1/sleipnir";
pub const RELEASES_PAGE: &str = "https://github.com/Maidang1/sleipnir/releases/latest";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseUrls {
    pub artifact: String,
    pub manifest: String,
    pub signature: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionPolicyError {
    InvalidCurrentVersion,
    NotNewer,
    InvalidReleaseIdentity,
}

pub fn validate_upgrade(
    current: &str,
    manifest: &UpdateManifest,
) -> Result<(), VersionPolicyError> {
    let current = semver::Version::parse(current.trim().trim_start_matches('v'))
        .map_err(|_| VersionPolicyError::InvalidCurrentVersion)?;
    if manifest.version <= current {
        return Err(VersionPolicyError::NotNewer);
    }
    release_urls(manifest)?;
    Ok(())
}

pub fn release_urls(manifest: &UpdateManifest) -> Result<ReleaseUrls, VersionPolicyError> {
    if manifest.tag != format!("v{}", manifest.version)
        || manifest.artifact != format!("Sleipnir-{}-macos.dmg", manifest.version)
    {
        return Err(VersionPolicyError::InvalidReleaseIdentity);
    }
    let base = format!(
        "https://github.com/{REPO}/releases/download/{}",
        manifest.tag
    );
    let manifest_url = format!("{base}/sleipnir-update-v1.json");
    Ok(ReleaseUrls {
        artifact: format!("{base}/{}", manifest.artifact),
        signature: format!("{manifest_url}.sig"),
        manifest: manifest_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> UpdateManifest {
        UpdateManifest {
            schema_version: 1,
            version: semver::Version::parse(version).unwrap(),
            tag: format!("v{version}"),
            artifact: format!("Sleipnir-{version}-macos.dmg"),
            size: 10,
            sha256: "a".repeat(64),
            bundle_id: "com.maidang1.sleipnir".into(),
            minimum_macos: "14.0".into(),
            minimum_updater_schema: 1,
        }
    }

    #[test]
    fn accepts_strict_upgrade() {
        assert!(validate_upgrade("0.3.1", &manifest("0.3.2")).is_ok());
    }

    #[test]
    fn rejects_same_version_and_downgrade() {
        assert_eq!(
            validate_upgrade("0.3.2", &manifest("0.3.2")).unwrap_err(),
            VersionPolicyError::NotNewer
        );
        assert_eq!(
            validate_upgrade("0.3.3", &manifest("0.3.2")).unwrap_err(),
            VersionPolicyError::NotNewer
        );
    }

    #[test]
    fn constructs_only_fixed_repository_urls() {
        let urls = release_urls(&manifest("0.3.2")).unwrap();
        assert_eq!(
            urls.artifact,
            "https://github.com/Maidang1/sleipnir/releases/download/v0.3.2/Sleipnir-0.3.2-macos.dmg"
        );
        assert_eq!(
            urls.manifest,
            "https://github.com/Maidang1/sleipnir/releases/download/v0.3.2/sleipnir-update-v1.json"
        );
        assert_eq!(urls.signature, format!("{}.sig", urls.manifest));
    }
}
