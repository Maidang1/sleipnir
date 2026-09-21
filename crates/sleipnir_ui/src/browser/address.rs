//! Address-bar input normalization. The URL policy itself is owned by
//! `sleipnir_browser_control::validate_url` — this module only turns what the
//! user typed into a candidate absolute URL, then defers to that policy.

/// Navigation policy for the native WebView: the same contract the agent
/// control protocol enforces, plus the local `about:blank` placeholder the
/// panel starts on.
pub(super) fn allowed_url(raw: &str) -> bool {
    raw == "about:blank" || sleipnir_browser_control::validate_url(raw).is_ok()
}

pub(super) fn resolve_address(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Enter a URL, for example http://localhost:3000".to_string());
    }
    if raw == "about:blank" {
        return Ok(raw.into());
    }
    if raw.chars().any(char::is_whitespace) {
        return Err("Enter a URL without spaces".to_string());
    }
    let explicit = raw.contains("://");
    let authority = raw.split(['/', '?', '#']).next().unwrap_or(raw);
    let host_port = authority.rsplit_once(':');
    let has_port = host_port.is_some_and(|(_, port)| port.parse::<u16>().is_ok());
    if !explicit && raw.contains(':') && !has_port && !raw.starts_with('[') {
        return Err("Only HTTP and HTTPS addresses are supported".to_string());
    }
    let candidate = if explicit {
        raw.to_string()
    } else {
        let host = if raw.starts_with('[') {
            authority
                .split(']')
                .next()
                .unwrap_or(authority)
                .trim_start_matches('[')
        } else if has_port {
            host_port.map(|(host, _)| host).unwrap_or(authority)
        } else {
            authority
        };
        let local = host.eq_ignore_ascii_case("localhost")
            || host.to_ascii_lowercase().ends_with(".localhost")
            || host.parse::<std::net::IpAddr>().is_ok();
        format!("{}://{raw}", if local { "http" } else { "https" })
    };
    // One policy decision, owned by the control protocol crate, applied to the
    // raw candidate so the length and control-character caps see user input.
    sleipnir_browser_control::validate_url(&candidate)?;
    // Normalize only after the policy passes, so the address bar shows the
    // canonical form (`localhost:3000` -> `http://localhost:3000/`).
    let url = url::Url::parse(&candidate).map_err(|_| "Invalid URL".to_string())?;
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_addresses_resolve_local_and_public_hosts() {
        for (input, expected) in [
            ("localhost:3000", "http://localhost:3000/"),
            ("127.0.0.1:8080/test?q=x", "http://127.0.0.1:8080/test?q=x"),
            ("[::1]:3000", "http://[::1]:3000/"),
            ("demo.localhost:5173", "http://demo.localhost:5173/"),
            ("example.com", "https://example.com/"),
            (" https://example.com/path ", "https://example.com/path"),
            ("about:blank", "about:blank"),
        ] {
            assert_eq!(resolve_address(input).as_deref(), Ok(expected), "{input}");
        }
    }

    #[test]
    fn browser_rejects_privileged_and_invalid_addresses() {
        for input in [
            "",
            "  ",
            "javascript:alert(1)",
            "data:text/html,test",
            "file:///etc/passwd",
            "mailto:x@y.com",
            "https://",
            "hello world",
            "https://user:pass@example.com",
        ] {
            assert!(resolve_address(input).is_err(), "{input}");
        }
        assert!(!allowed_url("javascript:alert(1)"));
        assert!(!allowed_url("file:///tmp/index.html"));
        assert!(allowed_url("http://localhost:3000"));
    }
}
