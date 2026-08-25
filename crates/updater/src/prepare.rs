use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleFacts {
    pub bundle_id: String,
    pub version: String,
    pub executable_exists: bool,
    pub helper_exists: bool,
    pub signature_valid: bool,
    pub critical_paths_inside_bundle: bool,
    pub install_parent_writable: bool,
    pub same_volume: bool,
    pub swap_supported: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightError {
    BundleIdentifierMismatch,
    BundleVersionMismatch,
    BundleLayoutInvalid,
    BundleSignatureInvalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightDecision {
    Ready,
    ManualInstallRequired,
    Reject(PreflightError),
}

pub fn classify_preflight(facts: &BundleFacts, expected_version: &str) -> PreflightDecision {
    if facts.bundle_id != "com.maidang1.sleipnir" {
        return PreflightDecision::Reject(PreflightError::BundleIdentifierMismatch);
    }
    if facts.version != expected_version {
        return PreflightDecision::Reject(PreflightError::BundleVersionMismatch);
    }
    if !facts.executable_exists || !facts.helper_exists || !facts.critical_paths_inside_bundle {
        return PreflightDecision::Reject(PreflightError::BundleLayoutInvalid);
    }
    if !facts.signature_valid {
        return PreflightDecision::Reject(PreflightError::BundleSignatureInvalid);
    }
    if !facts.install_parent_writable || !facts.same_volume || !facts.swap_supported {
        return PreflightDecision::ManualInstallRequired;
    }
    PreflightDecision::Ready
}

pub fn path_is_within(bundle: &Path, candidate: &Path) -> bool {
    candidate.starts_with(bundle) && candidate != bundle
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn bundle() -> BundleFacts {
        BundleFacts {
            bundle_id: "com.maidang1.sleipnir".into(),
            version: "0.3.2".into(),
            executable_exists: true,
            helper_exists: true,
            signature_valid: true,
            critical_paths_inside_bundle: true,
            install_parent_writable: true,
            same_volume: true,
            swap_supported: true,
        }
    }

    #[test]
    fn accepts_matching_safe_bundle() {
        assert_eq!(
            classify_preflight(&bundle(), "0.3.2"),
            PreflightDecision::Ready
        );
    }

    #[test]
    fn rejects_identity_layout_version_and_signature() {
        let mutations: Vec<(fn(&mut BundleFacts), PreflightError)> = vec![
            (
                |f| f.bundle_id = "evil.app".into(),
                PreflightError::BundleIdentifierMismatch,
            ),
            (
                |f| f.version = "0.3.1".into(),
                PreflightError::BundleVersionMismatch,
            ),
            (
                |f| f.executable_exists = false,
                PreflightError::BundleLayoutInvalid,
            ),
            (
                |f| f.helper_exists = false,
                PreflightError::BundleLayoutInvalid,
            ),
            (
                |f| f.signature_valid = false,
                PreflightError::BundleSignatureInvalid,
            ),
            (
                |f| f.critical_paths_inside_bundle = false,
                PreflightError::BundleLayoutInvalid,
            ),
        ];
        for (mutate, expected) in mutations {
            let mut facts = bundle();
            mutate(&mut facts);
            assert_eq!(
                classify_preflight(&facts, "0.3.2"),
                PreflightDecision::Reject(expected)
            );
        }
    }

    #[test]
    fn unsafe_install_location_requires_manual_install() {
        for mutate in [
            (|f: &mut BundleFacts| f.install_parent_writable = false) as fn(&mut BundleFacts),
            |f: &mut BundleFacts| f.same_volume = false,
            |f: &mut BundleFacts| f.swap_supported = false,
        ] {
            let mut facts = bundle();
            mutate(&mut facts);
            assert_eq!(
                classify_preflight(&facts, "0.3.2"),
                PreflightDecision::ManualInstallRequired
            );
        }
    }

    #[test]
    fn critical_path_must_remain_inside_bundle() {
        assert!(path_is_within(
            Path::new("/tmp/Sleipnir.app"),
            Path::new("/tmp/Sleipnir.app/Contents/MacOS/sleipnir")
        ));
        assert!(!path_is_within(
            Path::new("/tmp/Sleipnir.app"),
            Path::new("/tmp/evil")
        ));
    }
}
