use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UpdateManifest {
    pub schema_version: u32,
    pub version: semver::Version,
    pub tag: String,
    pub artifact: String,
    pub size: u64,
    pub sha256: String,
    pub bundle_id: String,
    pub minimum_macos: String,
    pub minimum_updater_schema: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestErrorCode {
    SignatureInvalid,
    SchemaUnsupported,
    InvalidFields,
}

#[derive(Debug)]
pub struct ManifestError {
    pub code: ManifestErrorCode,
    pub message: String,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ManifestError {}

fn manifest_error(code: ManifestErrorCode, message: impl Into<String>) -> ManifestError {
    ManifestError {
        code,
        message: message.into(),
    }
}

pub fn verify_and_parse(
    bytes: &[u8],
    signature_bytes: &[u8],
    public_key: &[u8; 32],
) -> Result<UpdateManifest, ManifestError> {
    let signature = Signature::from_slice(signature_bytes).map_err(|_| {
        manifest_error(
            ManifestErrorCode::SignatureInvalid,
            "invalid manifest signature",
        )
    })?;
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| {
        manifest_error(
            ManifestErrorCode::SignatureInvalid,
            "invalid update public key",
        )
    })?;
    key.verify(bytes, &signature).map_err(|_| {
        manifest_error(
            ManifestErrorCode::SignatureInvalid,
            "manifest signature verification failed",
        )
    })?;
    let manifest: UpdateManifest = serde_json::from_slice(bytes)
        .map_err(|err| manifest_error(ManifestErrorCode::InvalidFields, err.to_string()))?;
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.minimum_updater_schema > MANIFEST_SCHEMA_VERSION
    {
        return Err(manifest_error(
            ManifestErrorCode::SchemaUnsupported,
            "unsupported manifest schema",
        ));
    }
    let expected_artifact = format!("Sleipnir-{}-macos.dmg", manifest.version);
    if manifest.tag != format!("v{}", manifest.version)
        || manifest.artifact != expected_artifact
        || manifest.bundle_id != "com.maidang1.sleipnir"
        || manifest.size == 0
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(manifest_error(
            ManifestErrorCode::InvalidFields,
            "invalid manifest fields",
        ));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn fixture_manifest() -> Vec<u8> {
        br#"{"schema_version":1,"version":"0.3.2","tag":"v0.3.2","artifact":"Sleipnir-0.3.2-macos.dmg","size":7012345,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","bundle_id":"com.maidang1.sleipnir","minimum_macos":"14.0","minimum_updater_schema":1}"#.to_vec()
    }

    fn signed_fixture() -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let bytes = fixture_manifest();
        let signature = signing.sign(&bytes).to_bytes().to_vec();
        (bytes, signature, signing.verifying_key().to_bytes())
    }

    #[test]
    fn verifies_exact_manifest_bytes() {
        let (bytes, signature, key) = signed_fixture();
        let manifest = verify_and_parse(&bytes, &signature, &key).unwrap();
        assert_eq!(manifest.version.to_string(), "0.3.2");
        assert_eq!(manifest.artifact, "Sleipnir-0.3.2-macos.dmg");
    }

    #[test]
    fn rejects_one_byte_manifest_mutation() {
        let (mut bytes, signature, key) = signed_fixture();
        let index = bytes.iter().position(|byte| *byte == b'2').unwrap();
        bytes[index] = b'3';
        assert_eq!(
            verify_and_parse(&bytes, &signature, &key).unwrap_err().code,
            ManifestErrorCode::SignatureInvalid
        );
    }

    #[test]
    fn rejects_malformed_signature() {
        let (bytes, _, key) = signed_fixture();
        assert_eq!(
            verify_and_parse(&bytes, b"short", &key).unwrap_err().code,
            ManifestErrorCode::SignatureInvalid
        );
    }

    #[test]
    fn rejects_unsupported_schema_after_signature_verification() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let bytes = fixture_manifest()
            .into_iter()
            .map(|byte| if byte == b'1' { b'9' } else { byte })
            .collect::<Vec<_>>();
        let signature = signing.sign(&bytes).to_bytes();
        assert_eq!(
            verify_and_parse(&bytes, &signature, &signing.verifying_key().to_bytes())
                .unwrap_err()
                .code,
            ManifestErrorCode::SchemaUnsupported
        );
    }

    #[test]
    fn rejects_wrong_artifact_identity_and_digest() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let mut value: serde_json::Value = serde_json::from_slice(&fixture_manifest()).unwrap();
        value["artifact"] = "other.dmg".into();
        value["sha256"] = "not-a-digest".into();
        let bytes = serde_json::to_vec(&value).unwrap();
        let signature = signing.sign(&bytes).to_bytes();
        assert_eq!(
            verify_and_parse(&bytes, &signature, &signing.verifying_key().to_bytes())
                .unwrap_err()
                .code,
            ManifestErrorCode::InvalidFields
        );
    }
}
