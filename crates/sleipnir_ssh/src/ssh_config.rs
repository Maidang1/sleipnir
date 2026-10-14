//! Parser for OpenSSH client configuration files (`~/.ssh/config`).
//!
//! The parser is deliberately conservative: it only collects top-level `Host`
//! blocks, skips `Match` blocks entirely (their criteria are environment
//! dependent and guessing them would surface hosts that do not apply), and
//! ignores wildcard patterns such as `Host *` so the connection list only
//! shows hosts the user named explicitly.

use std::path::{Path, PathBuf};

/// A host entry discovered in an ssh client configuration file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshConfigHost {
    /// The `Host` pattern (alias) the user types after `ssh`.
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_files: Vec<String>,
    pub proxy_jump: Option<String>,
}

impl SshConfigHost {
    /// Whether the entry carries any connection information beyond the alias.
    pub fn is_bare_alias(&self) -> bool {
        self.hostname.is_none()
            && self.user.is_none()
            && self.port.is_none()
            && self.identity_files.is_empty()
            && self.proxy_jump.is_none()
    }
}

/// Maximum `Include` recursion depth. ssh itself allows deep nesting; the cap
/// only exists to keep a pathological (or cyclic) include graph bounded.
const MAX_INCLUDE_DEPTH: usize = 8;

/// Parse the text of an ssh config file into host entries.
///
/// `config_dir` is the directory `Include` patterns resolve against when they
/// are relative (ssh resolves them against `~/.ssh`).
pub fn parse_ssh_config(text: &str) -> Vec<SshConfigHost> {
    parse_ssh_config_in(text, &default_ssh_dir(), 0, &mut Vec::new())
}

/// Same as [`parse_ssh_config`] with explicit include root and visited set.
pub fn parse_ssh_config_in(
    text: &str,
    include_root: &Path,
    depth: usize,
    visited: &mut Vec<PathBuf>,
) -> Vec<SshConfigHost> {
    let mut hosts: Vec<SshConfigHost> = Vec::new();
    // One entry per valid pattern of the current `Host` line.
    let mut current: Vec<SshConfigHost> = Vec::new();
    let mut in_match = false;

    for raw_line in text.lines() {
        let line = strip_comment(raw_line);
        let Some((keyword, value)) = split_directive(line) else {
            continue;
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                // A new block ends the previous one.
                in_match = false;
                hosts.append(&mut current);
                current = start_host_block(value);
            }
            // Match criteria are environment dependent; stop collecting until
            // the next Host block so we never surface a host that may not apply.
            "match" => {
                in_match = true;
                hosts.append(&mut current);
                current.clear();
            }
            "hostname" => apply_str(&mut current, in_match, |h| &mut h.hostname, value),
            "user" => apply_str(&mut current, in_match, |h| &mut h.user, value),
            "port" => {
                if let Ok(port) = value.parse::<u16>() {
                    apply_opt(&mut current, in_match, |h| h.port = Some(port));
                }
            }
            "identityfile" => {
                let identity = unquote(value);
                apply_opt(&mut current, in_match, |h| {
                    if !identity.is_empty() {
                        h.identity_files.push(identity.clone());
                    }
                });
            }
            "proxyjump" => {
                let jump = unquote(value);
                apply_opt(&mut current, in_match, |h| {
                    if !jump.is_empty() && jump != "none" {
                        h.proxy_jump = Some(jump.clone());
                    }
                });
            }
            "include" => {
                if depth >= MAX_INCLUDE_DEPTH {
                    continue;
                }
                let pattern = unquote(value);
                if pattern.is_empty() {
                    continue;
                }
                for path in expand_include(&pattern, include_root) {
                    let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    if visited.contains(&canonical) {
                        continue;
                    }
                    visited.push(canonical);
                    match std::fs::read_to_string(&path) {
                        Ok(text) => hosts.extend(parse_ssh_config_in(
                            &text,
                            include_root,
                            depth + 1,
                            visited,
                        )),
                        Err(err) => {
                            log::debug!("ssh config include {} unreadable: {err}", path.display());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    hosts.append(&mut current);
    dedup_by_alias(hosts)
}

/// Read and parse an ssh config file, returning an empty list when it is
/// missing or unreadable (a missing config is not an error for the UI).
pub fn load_ssh_config(path: &Path) -> Vec<SshConfigHost> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_ssh_config_in(
            &text,
            &default_ssh_dir(),
            0,
            &mut vec![std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())],
        ),
        Err(err) => {
            log::debug!("ssh config {} unreadable: {err}", path.display());
            Vec::new()
        }
    }
}

/// Default location of the user's ssh client configuration.
pub fn default_ssh_config_path() -> PathBuf {
    default_ssh_dir().join("config")
}

fn default_ssh_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ssh")
}

fn start_host_block(value: &str) -> Vec<SshConfigHost> {
    let mut out = Vec::new();
    for pattern in split_patterns(value) {
        // Wildcards and negations are not connectable aliases.
        if pattern.contains('*') || pattern.contains('?') || pattern.starts_with('!') {
            continue;
        }
        out.push(SshConfigHost {
            alias: pattern,
            ..SshConfigHost::default()
        });
    }
    out
}

fn apply_str(
    hosts: &mut [SshConfigHost],
    in_match: bool,
    field: impl Fn(&mut SshConfigHost) -> &mut Option<String>,
    value: &str,
) {
    let value = unquote(value);
    apply_opt(hosts, in_match, |host| *field(host) = Some(value.clone()));
}

fn apply_opt(hosts: &mut [SshConfigHost], in_match: bool, apply: impl Fn(&mut SshConfigHost)) {
    if in_match {
        return;
    }
    for host in hosts.iter_mut() {
        apply(host);
    }
}

fn dedup_by_alias(hosts: Vec<SshConfigHost>) -> Vec<SshConfigHost> {
    // ssh applies the first obtained value for each keyword, so the earliest
    // block wins when the same alias appears twice.
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for host in hosts {
        if seen.contains(&host.alias) {
            continue;
        }
        seen.push(host.alias.clone());
        out.push(host);
    }
    out
}

/// Drop a trailing `#` comment. A `#` inside a quoted value stays literal.
fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &line[..idx],
            _ => {}
        }
    }
    line
}

/// Split `Keyword Value` / `Keyword=Value` into its parts.
fn split_directive(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(eq) = trimmed.find('=') {
        let (keyword, value) = trimmed.split_at(eq);
        if !keyword.contains(char::is_whitespace) {
            return Some((keyword.trim(), value[1..].trim()));
        }
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let keyword = parts.next()?.trim();
    let value = parts.next().unwrap_or("").trim();
    if keyword.is_empty() {
        return None;
    }
    Some((keyword, value))
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Split a `Host` value into patterns, honouring double quotes.
fn split_patterns(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in value.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Expand `~` and a trailing glob (`*` / `?`) into concrete file paths.
fn expand_include(pattern: &str, include_root: &Path) -> Vec<PathBuf> {
    let expanded = expand_tilde(pattern);
    let path = if expanded.is_absolute() {
        expanded
    } else {
        include_root.join(expanded)
    };
    if !pattern_has_glob(&path.to_string_lossy()) {
        return vec![path];
    }
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if glob_matches(&name, &file_name) {
            out.push(entry.path());
        }
    }
    out.sort();
    out
}

fn expand_tilde(pattern: &str) -> PathBuf {
    if pattern == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(rest) = pattern.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(pattern)
}

fn pattern_has_glob(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?')
}

/// Minimal glob: `*` matches any run of characters, `?` exactly one.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (None, 0usize);
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star_p = Some(p);
                p += 1;
                star_t = t;
            }
            Some('?') => {
                p += 1;
                t += 1;
            }
            Some(&pc) if pc == text[t] => {
                p += 1;
                t += 1;
            }
            _ => {
                let Some(sp) = star_p else {
                    return false;
                };
                p = sp + 1;
                star_t += 1;
                t = star_t;
            }
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<SshConfigHost> {
        // No include root on disk: includes in these fixtures are absolute.
        parse_ssh_config_in(text, Path::new("/nonexistent-ssh-dir"), 0, &mut Vec::new())
    }

    #[test]
    fn parses_basic_host_block() {
        let hosts = parse(
            "# comment\n\
             Host prod-01\n\
             \tHostName 10.0.0.5\n\
             \tUser deploy\n\
             \tPort 2222\n\
             \tIdentityFile ~/.ssh/id_ed25519\n\
             \tProxyJump bastion\n",
        );
        assert_eq!(hosts.len(), 1);
        let host = &hosts[0];
        assert_eq!(host.alias, "prod-01");
        assert_eq!(host.hostname.as_deref(), Some("10.0.0.5"));
        assert_eq!(host.user.as_deref(), Some("deploy"));
        assert_eq!(host.port, Some(2222));
        assert_eq!(host.identity_files, vec!["~/.ssh/id_ed25519".to_string()]);
        assert_eq!(host.proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn accepts_equals_style_and_is_case_insensitive() {
        let hosts = parse("HOST=web\nhostname=web.example.com\nUSER=ops\nport=22\n");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].alias, "web");
        assert_eq!(hosts[0].hostname.as_deref(), Some("web.example.com"));
        assert_eq!(hosts[0].user.as_deref(), Some("ops"));
        assert_eq!(hosts[0].port, Some(22));
    }

    #[test]
    fn skips_wildcards_negations_and_match_blocks() {
        let hosts = parse(
            "Host *\n  User root\n\
             Host !secret\n  User nobody\n\
             Host real\n  User me\n\
             Match host real\n  User shadow\n\
             Host after\n  User later\n",
        );
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, vec!["real", "after"]);
        assert_eq!(hosts[0].user.as_deref(), Some("me"));
        assert_eq!(hosts[1].user.as_deref(), Some("later"));
    }

    #[test]
    fn multiple_patterns_on_one_host_line_each_become_entries() {
        let hosts = parse("Host alpha beta\n  HostName shared.example.com\n");
        assert_eq!(hosts.len(), 2);
        assert!(
            hosts
                .iter()
                .all(|h| h.hostname.as_deref() == Some("shared.example.com"))
        );
    }

    #[test]
    fn bare_alias_entries_are_kept() {
        let hosts = parse("Host localbox\n");
        assert_eq!(hosts.len(), 1);
        assert!(hosts[0].is_bare_alias());
    }

    #[test]
    fn duplicate_alias_keeps_first_block() {
        let hosts = parse("Host dup\n  User first\nHost dup\n  User second\n");
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user.as_deref(), Some("first"));
    }

    #[test]
    fn bad_port_is_ignored_and_quoted_values_unquoted() {
        let hosts = parse("Host q\n  Port notanumber\n  User \"quoted user\"\n");
        assert_eq!(hosts[0].port, None);
        assert_eq!(hosts[0].user.as_deref(), Some("quoted user"));
    }

    #[test]
    fn include_is_loaded_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested.conf");
        std::fs::write(&nested, "Host deep\n  HostName deep.example.com\n").unwrap();
        let main = dir.path().join("config");
        std::fs::write(
            &main,
            format!(
                "Include {}\nHost top\n  HostName top.example.com\n",
                nested.display()
            ),
        )
        .unwrap();
        let hosts = load_ssh_config(&main);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert!(aliases.contains(&"top"), "{aliases:?}");
        assert!(aliases.contains(&"deep"), "{aliases:?}");
    }

    #[test]
    fn include_glob_and_cycle_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.conf"),
            "Host a\n  HostName a.example.com\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.conf"),
            "Host b\n  HostName b.example.com\n",
        )
        .unwrap();
        let main = dir.path().join("config");
        // Self-include must terminate.
        std::fs::write(
            &main,
            format!(
                "Include {}/*.conf\nInclude {}\n",
                dir.path().display(),
                main.display()
            ),
        )
        .unwrap();
        let hosts = load_ssh_config(&main);
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert!(aliases.contains(&"a"), "{aliases:?}");
        assert!(aliases.contains(&"b"), "{aliases:?}");
    }

    #[test]
    fn missing_file_yields_no_hosts() {
        assert!(load_ssh_config(Path::new("/nonexistent/ssh/config")).is_empty());
    }

    #[test]
    fn glob_matching_behaves_like_shell() {
        assert!(glob_matches("*.conf", "a.conf"));
        assert!(!glob_matches("*.conf", "a.confx"));
        assert!(glob_matches("a?c", "abc"));
        assert!(!glob_matches("a?c", "ac"));
        assert!(glob_matches("*", "anything"));
    }
}
