//! Command-line redaction applied at capture time (spec §4).
//!
//! This is a heuristic, not a guarantee. It drops obvious secret shapes
//! (env values, known flags, Authorization headers, URL userinfo / secret
//! query params, high-entropy tokens) and truncates long lines. It does
//! not claim to catch every secret.

/// Max command length in Unicode characters, including the trailing `…`.
const MAX_CHARS: usize = 256;

const SENSITIVE_FLAGS: &[&str] = &[
    "--token",
    "--password",
    "--api-key",
    "--secret",
    "-p",
    "--pass",
];

/// Redact a command line at capture time. Heuristic, not a guarantee.
pub fn redact_command(raw: &str) -> String {
    let tokens = tokenize(raw);
    let mut out = Vec::with_capacity(tokens.len());
    let mut prev_sensitive = false;

    for token in tokens {
        if prev_sensitive {
            out.push("…".to_string());
            prev_sensitive = is_sensitive_flag(unwrap_quotes(&token).0);
            continue;
        }

        let (inner, quote) = unwrap_quotes(&token);
        let redacted = redact_token(inner);
        out.push(rewrap(&redacted, quote));
        prev_sensitive = is_sensitive_flag(inner);
    }

    truncate_chars(&out.join(" "))
}

fn tokenize(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for c in raw.chars() {
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            current.push(c);
            quote = Some(c);
        } else if c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn unwrap_quotes(token: &str) -> (&str, Option<char>) {
    let bytes = token.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if first == last && (first == b'"' || first == b'\'') {
            return (&token[1..token.len() - 1], Some(first as char));
        }
    }
    (token, None)
}

fn rewrap(inner: &str, quote: Option<char>) -> String {
    match quote {
        Some(q) => format!("{q}{inner}{q}"),
        None => inner.to_string(),
    }
}

fn is_sensitive_flag(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    SENSITIVE_FLAGS.contains(&lower.as_str())
}

fn redact_token(token: &str) -> String {
    if let Some(redacted) = redact_env_prefix(token) {
        return redacted;
    }
    if let Some(redacted) = redact_authorization(token) {
        return redacted;
    }
    if token.contains("://") {
        return redact_url(token);
    }
    if is_high_entropy(token) {
        return "…".to_string();
    }
    token.to_string()
}

fn redact_env_prefix(token: &str) -> Option<String> {
    let eq = token.find('=')?;
    let key = &token[..eq];
    if key.is_empty() {
        return None;
    }
    let mut chars = key.chars();
    let first = chars.next()?;
    if !first.is_ascii_uppercase() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return None;
    }
    Some(format!("{key}=…"))
}

fn redact_authorization(token: &str) -> Option<String> {
    let lower = token.to_ascii_lowercase();
    let idx = lower.find("authorization:")?;
    let colon = idx + "authorization:".len();
    let mut out = token[..colon].to_string();
    if token[colon..].starts_with(' ') {
        out.push(' ');
    }
    out.push('…');
    Some(out)
}

fn redact_url(token: &str) -> String {
    let Some(scheme_end) = token.find("://") else {
        return token.to_string();
    };
    let after_scheme = scheme_end + 3;
    let rest = &token[after_scheme..];
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];

    let mut out = token[..after_scheme].to_string();
    if let Some(at) = authority.rfind('@') {
        let userinfo = &authority[..at];
        let host = &authority[at + 1..];
        if let Some(colon) = userinfo.find(':') {
            out.push_str(&userinfo[..=colon]);
            out.push('…');
            out.push('@');
            out.push_str(host);
        } else {
            out.push_str(authority);
        }
    } else {
        out.push_str(authority);
    }
    out.push_str(&redact_query(&rest[auth_end..]));
    out
}

fn redact_query(s: &str) -> String {
    let Some(qpos) = s.find('?') else {
        return s.to_string();
    };
    let mut out = s[..=qpos].to_string();
    let after_q = &s[qpos + 1..];
    let (query, frag) = match after_q.find('#') {
        Some(i) => (&after_q[..i], &after_q[i..]),
        None => (after_q, ""),
    };

    for (i, part) in query.split('&').enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&redact_query_pair(part));
    }
    out.push_str(frag);
    out
}

fn redact_query_pair(part: &str) -> String {
    let Some(eq) = part.find('=') else {
        return part.to_string();
    };
    let key = &part[..eq];
    match key.to_ascii_lowercase().as_str() {
        "token" | "key" | "sig" | "secret" => format!("{key}=…"),
        _ => part.to_string(),
    }
}

fn is_high_entropy(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() <= 20 {
        return false;
    }
    let mut has_alpha = false;
    let mut has_digit = false;
    for c in chars {
        match c {
            'A'..='Z' | 'a'..='z' => has_alpha = true,
            '0'..='9' => has_digit = true,
            '+' | '/' | '=' | '_' | '-' => {}
            _ => return false,
        }
    }
    has_alpha && has_digit
}

fn truncate_chars(s: &str) -> String {
    let count = s.chars().count();
    if count <= MAX_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_CHARS - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::redact_command;

    #[test]
    fn env_prefix_keeps_key_drops_value() {
        assert_eq!(
            redact_command("AWS_SECRET_ACCESS_KEY=abc123 aws s3 ls"),
            "AWS_SECRET_ACCESS_KEY=… aws s3 ls"
        );
    }

    #[test]
    fn sensitive_flag_values_are_dropped() {
        assert_eq!(
            redact_command("gh auth login --token ghp_averyrealtoken0000"),
            "gh auth login --token …"
        );
        assert_eq!(
            redact_command("curl -H \"Authorization: Bearer eyJhbGciOiJIUzI1NiJ9\" https://api.example.com"),
            "curl -H \"Authorization: …\" https://api.example.com"
        );
    }

    #[test]
    fn url_userinfo_and_secret_query_are_dropped() {
        assert_eq!(
            redact_command("git clone https://user:hunter2@example.com/x.git"),
            "git clone https://user:…@example.com/x.git"
        );
        assert_eq!(
            redact_command("curl 'https://example.com/a?token=abcdefghijklmnopqrstuvwxyz&x=1'"),
            "curl 'https://example.com/a?token=…&x=1'"
        );
    }

    #[test]
    fn high_entropy_bare_tokens_are_dropped() {
        assert_eq!(
            redact_command("deploy sk-live-4f9a8c2b7e1d6a3f5c8b9e0d1a2b3c4d"),
            "deploy …"
        );
    }

    #[test]
    fn ordinary_commands_are_untouched() {
        for cmd in ["cargo build --release", "npm test", "git commit -m \"fix: thing\""] {
            assert_eq!(redact_command(cmd), cmd, "must not mangle ordinary commands");
        }
    }

    #[test]
    fn long_command_is_truncated_to_256_chars() {
        let long = format!("echo {}", "a".repeat(400));
        let out = redact_command(&long);
        assert!(out.chars().count() <= 256);
        assert!(out.ends_with('…'));
    }
}
