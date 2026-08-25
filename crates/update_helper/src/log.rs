use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

#[derive(Serialize)]
pub struct LogEvent<'a> {
    pub transaction_id: &'a str,
    pub phase: &'a str,
    pub event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_error: Option<&'a str>,
}

pub fn append(path: &Path, event: &LogEvent<'_>) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, event).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn log_schema_has_no_nonce_or_environment_field() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("update.log");
        append(
            &path,
            &LogEvent {
                transaction_id: "tx",
                phase: "prepared",
                event: "ready",
                error_code: None,
                os_error: None,
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(!text.contains("nonce"));
        assert!(!text.contains("environment"));
        assert!(!text.contains("terminal"));
    }
}
