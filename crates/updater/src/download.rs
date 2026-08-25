use sha2::{Digest as _, Sha256};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadErrorCode {
    Io,
    SizeMismatch,
    HashMismatch,
    InvalidDestination,
}

#[derive(Debug)]
pub struct DownloadError {
    pub code: DownloadErrorCode,
    pub message: String,
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for DownloadError {}

fn download_error(code: DownloadErrorCode, message: impl Into<String>) -> DownloadError {
    DownloadError {
        code,
        message: message.into(),
    }
}

pub fn download_verified(
    mut reader: impl Read,
    expected_size: u64,
    expected_sha256: &str,
    destination_dir: &Path,
    file_name: &str,
) -> Result<PathBuf, DownloadError> {
    if file_name.is_empty() || Path::new(file_name).components().count() != 1 {
        return Err(download_error(
            DownloadErrorCode::InvalidDestination,
            "invalid artifact filename",
        ));
    }
    std::fs::create_dir_all(destination_dir)
        .map_err(|err| download_error(DownloadErrorCode::Io, err.to_string()))?;
    let final_path = destination_dir.join(file_name);
    let part_path = destination_dir.join(format!("{file_name}.part"));
    let result = (|| {
        let mut file = std::fs::File::create(&part_path)
            .map_err(|err| download_error(DownloadErrorCode::Io, err.to_string()))?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|err| download_error(DownloadErrorCode::Io, err.to_string()))?;
            if read == 0 {
                break;
            }
            total += read as u64;
            if total > expected_size {
                return Err(download_error(
                    DownloadErrorCode::SizeMismatch,
                    "artifact exceeds declared size",
                ));
            }
            hasher.update(&buffer[..read]);
            file.write_all(&buffer[..read])
                .map_err(|err| download_error(DownloadErrorCode::Io, err.to_string()))?;
        }
        if total != expected_size {
            return Err(download_error(
                DownloadErrorCode::SizeMismatch,
                "artifact size differs from manifest",
            ));
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != expected_sha256.to_ascii_lowercase() {
            return Err(download_error(
                DownloadErrorCode::HashMismatch,
                "artifact SHA-256 differs from manifest",
            ));
        }
        file.sync_all()
            .map_err(|err| download_error(DownloadErrorCode::Io, err.to_string()))?;
        std::fs::rename(&part_path, &final_path)
            .map_err(|err| download_error(DownloadErrorCode::Io, err.to_string()))?;
        Ok(final_path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Sha256;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    #[test]
    fn streams_exact_verified_artifact_into_final_path() {
        let dir = tempdir().unwrap();
        let bytes = b"verified dmg bytes";
        let path = download_verified(
            Cursor::new(bytes),
            bytes.len() as u64,
            &digest(bytes),
            dir.path(),
            "artifact.dmg",
        )
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert!(!dir.path().join("artifact.dmg.part").exists());
    }

    #[test]
    fn rejects_truncated_oversized_and_hash_mismatch_without_partial_file() {
        for (bytes, expected_size, expected_hash, code) in [
            (
                &b"short"[..],
                6,
                digest(b"short"),
                DownloadErrorCode::SizeMismatch,
            ),
            (
                &b"too-long"[..],
                3,
                digest(b"too-long"),
                DownloadErrorCode::SizeMismatch,
            ),
            (
                &b"wrong"[..],
                5,
                digest(b"right"),
                DownloadErrorCode::HashMismatch,
            ),
        ] {
            let dir = tempdir().unwrap();
            let error = download_verified(
                Cursor::new(bytes),
                expected_size,
                &expected_hash,
                dir.path(),
                "artifact.dmg",
            )
            .unwrap_err();
            assert_eq!(error.code, code);
            assert!(!dir.path().join("artifact.dmg.part").exists());
            assert!(!dir.path().join("artifact.dmg").exists());
        }
    }
}
