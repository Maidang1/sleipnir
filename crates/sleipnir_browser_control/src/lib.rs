//! Browser-only local control protocol. No terminal operations or arbitrary JS.
pub mod cli;
pub mod mcp;
pub mod transport;

use serde::{Deserialize, Serialize};

pub const ENDPOINT_ENV: &str = "SLEIPNIR_BROWSER_ENDPOINT";
pub const TOKEN_ENV: &str = "SLEIPNIR_BROWSER_TOKEN";
pub const WINDOW_ENV: &str = "SLEIPNIR_WINDOW_ID";
pub const MAX_TEXT_CHARS: usize = 20_000;
pub const MAX_URL_BYTES: usize = 8192;
pub const MAX_TITLE_CHARS: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    List,
    Status { window: u64 },
    Navigate { window: u64, url: String },
    ReadText { window: u64 },
}

impl Request {
    pub fn window(&self) -> Option<u64> {
        match self {
            Self::List => None,
            Self::Status { window } | Self::Navigate { window, .. } | Self::ReadText { window } => {
                Some(*window)
            }
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Navigate { url, .. } = self {
            validate_url(url)?;
        }
        Ok(())
    }
}

pub fn validate_url(value: &str) -> Result<(), String> {
    if value.len() > MAX_URL_BYTES || value.chars().any(char::is_control) {
        return Err("Invalid or oversized URL".into());
    }
    let url = url::Url::parse(value).map_err(|_| "An absolute HTTP/HTTPS URL is required")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only HTTP/HTTPS URLs without embedded credentials are allowed".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserStatus {
    pub window: u64,
    pub open: bool,
    pub authorized: bool,
    pub blocked: bool,
    pub loading: bool,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Windows {
        windows: Vec<BrowserStatus>,
    },
    Status {
        browser: BrowserStatus,
    },
    NavigationAccepted {
        window: u64,
        requested_url: String,
    },
    PageText {
        window: u64,
        url: String,
        title: String,
        text: String,
        truncated: bool,
        untrusted: bool,
    },
    Error {
        code: String,
        message: String,
    },
}
impl Response {
    pub fn error(code: &str, message: impl Into<String>) -> Self {
        Self::Error {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

/// Used by the UI before reading or mutating a page and again when async reads finish.
pub fn check_access(open: bool, authorized: bool, blocked: bool) -> Result<(), Response> {
    if !authorized {
        return Err(Response::error(
            "permission_denied",
            "Enable Agent access in the target browser panel first",
        ));
    }
    if !open {
        return Err(Response::error(
            "panel_closed",
            "Open the target browser panel first",
        ));
    }
    if blocked {
        return Err(Response::error(
            "overlay_open",
            "Dismiss the application overlay before browser automation",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_rejects_local_files_scripts_credentials_and_invalid_urls() {
        for value in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,x",
            "https://u:p@a.com",
            "localhost:3000",
            "https://",
        ] {
            assert!(validate_url(value).is_err(), "{value}");
        }
        assert!(validate_url("http://localhost:3000").is_ok());
        assert!(validate_url("https://example.com/").is_ok());
    }
    #[test]
    fn access_is_default_denied_and_overlays_block_automation() {
        assert!(check_access(true, false, false).is_err());
        assert!(check_access(false, true, false).is_err());
        assert!(check_access(true, true, true).is_err());
        assert!(check_access(true, true, false).is_ok());
    }
    #[test]
    fn protocol_rejects_unknown_fields_and_privileged_operations() {
        assert!(
            serde_json::from_str::<Request>(r#"{"op":"status","window":1,"script":"x"}"#).is_err()
        );
        assert!(serde_json::from_str::<Request>(r#"{"op":"evaluate","window":1}"#).is_err());
        assert!(serde_json::from_str::<Request>(r#"{"op":"send","text":"rm"}"#).is_err());
    }
}
