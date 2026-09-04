//! Chrome fuzzy history over a shell history file. Does not take over the PTY line.

/// One unique command from a history file, newest occurrence first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryHit {
    pub command: String,
}

/// Parse bash/zsh-style history text (ignores `: ts:0;` prefixes).
pub fn parse_history_file(text: &str) -> Vec<HistoryHit> {    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw in text.lines().rev() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let cmd = strip_zsh_extended(line);
        if cmd.is_empty() || !seen.insert(cmd.to_string()) {
            continue;
        }
        out.push(HistoryHit {
            command: cmd.to_string(),
        });
    }
    out
}

fn strip_zsh_extended(line: &str) -> &str {
    // `: 1710000000:0;cargo test`
    if let Some(rest) = line.strip_prefix(':') {
        if let Some((_, cmd)) = rest.split_once(';') {
            return cmd.trim();
        }
    }
    line
}

/// Load the user's shell history hits: $HISTFILE, then ~/.zsh_history,
/// ~/.bash_history, then the PowerShell history on Windows. Empty when none
/// is readable.
pub fn load_history_hits() -> Vec<HistoryHit> {
    let text = std::env::var("HISTFILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| {
            dirs::home_dir().and_then(|h| {
                std::fs::read_to_string(h.join(".zsh_history"))
                    .ok()
                    .or_else(|| std::fs::read_to_string(h.join(".bash_history")).ok())
            })
        })
        .or_else(|| {
            dirs::data_dir().and_then(|d| {
                std::fs::read_to_string(
                    d.join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt"),
                )
                .ok()
            })
        })
        .unwrap_or_default();
    parse_history_file(&text)
}

/// Case-insensitive subsequence match. Empty query returns the first `limit` items.
pub fn filter_history<'a>(
    hits: &'a [HistoryHit],
    query: &str,
    limit: usize,
) -> Vec<&'a HistoryHit> {
    let q = query.trim().to_lowercase();
    hits.iter()
        .filter(|h| q.is_empty() || subsequence(&h.command.to_lowercase(), &q))
        .take(limit)
        .collect()
}

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    for ch in needle.chars() {
        if !it.any(|h| h == ch) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_zsh_extended() {
        let text = ": 1:0;cargo test\nls\n: 2:0;cargo test\ngit status\n";
        let hits = parse_history_file(text);
        assert_eq!(
            hits.iter().map(|h| h.command.as_str()).collect::<Vec<_>>(),
            vec!["git status", "cargo test", "ls"]
        );
    }

    #[test]
    fn fuzzy_subsequence_and_limit() {
        let hits = parse_history_file("cargo test\ncargo build\nls\n");
        let found = filter_history(&hits, "cgt", 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "cargo test");
        assert_eq!(filter_history(&hits, "", 2).len(), 2);
    }
}
