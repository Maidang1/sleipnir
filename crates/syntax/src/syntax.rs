//! UI-free tree-sitter highlighting. Language from file extension; spans are
//! per-line and relative to that line (newline excluded).

use std::ops::Range;
use std::sync::OnceLock;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Constant,
    Property,
    Operator,
    Punctuation,
}

const CAPTURES: &[(&str, Token)] = &[
    ("boolean", Token::Constant),
    ("comment", Token::Comment),
    ("constant", Token::Constant),
    ("constructor", Token::Type),
    ("function", Token::Function),
    ("keyword", Token::Keyword),
    ("number", Token::Number),
    ("operator", Token::Operator),
    ("property", Token::Property),
    ("punctuation", Token::Punctuation),
    ("string", Token::String),
    ("type", Token::Type),
    ("variable.builtin", Token::Constant),
];

pub struct Language {
    config: OnceLock<Option<HighlightConfiguration>>,
    build: fn() -> Option<HighlightConfiguration>,
}

impl Language {
    const fn new(build: fn() -> Option<HighlightConfiguration>) -> Self {
        Self {
            config: OnceLock::new(),
            build,
        }
    }

    fn config(&self) -> Option<&HighlightConfiguration> {
        self.config
            .get_or_init(|| {
                let mut config = (self.build)()?;
                let names: Vec<&str> = CAPTURES.iter().map(|&(name, _)| name).collect();
                config.configure(&names);
                Some(config)
            })
            .as_ref()
    }
}

static RUST: Language = Language::new(|| {
    HighlightConfiguration::new(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        "",
        "",
    )
    .ok()
});

static PYTHON: Language = Language::new(|| {
    HighlightConfiguration::new(
        tree_sitter_python::LANGUAGE.into(),
        "python",
        tree_sitter_python::HIGHLIGHTS_QUERY,
        "",
        "",
    )
    .ok()
});

static JAVASCRIPT: Language = Language::new(|| {
    HighlightConfiguration::new(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        "",
        tree_sitter_javascript::LOCALS_QUERY,
    )
    .ok()
});

static JSON: Language = Language::new(|| {
    HighlightConfiguration::new(
        tree_sitter_json::LANGUAGE.into(),
        "json",
        tree_sitter_json::HIGHLIGHTS_QUERY,
        "",
        "",
    )
    .ok()
});

/// `None` means render as plain text.
pub fn language_for_path(path: &str) -> Option<&'static Language> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    let lang = match ext.to_ascii_lowercase().as_str() {
        "rs" => &RUST,
        "py" | "pyi" => &PYTHON,
        "js" | "mjs" | "cjs" | "jsx" => &JAVASCRIPT,
        "json" => &JSON,
        _ => return None,
    };
    Some(lang)
}

/// Per-line, sorted, non-overlapping, line-relative byte ranges.
/// Errors degrade to empty spans.
pub fn highlight_lines(lang: &Language, source: &str) -> Vec<Vec<(Range<usize>, Token)>> {
    let mut line_starts = vec![0usize];
    for (ix, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(ix + 1);
        }
    }
    let mut out: Vec<Vec<(Range<usize>, Token)>> = vec![Vec::new(); line_starts.len()];
    let Some(config) = lang.config() else {
        return out;
    };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, source.as_bytes(), None, |_| None) else {
        return out;
    };
    let line_end = |l: usize| {
        line_starts
            .get(l + 1)
            .map_or(source.len(), |&next| next - 1)
    };

    let mut stack: Vec<Token> = Vec::new();
    let mut line = 0usize;
    for event in events {
        let Ok(event) = event else { break };
        match event {
            HighlightEvent::HighlightStart(h) => stack.push(CAPTURES[h.0].1),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(&token) = stack.last() else { continue };
                while line + 1 < line_starts.len() && line_starts[line + 1] <= start {
                    line += 1;
                }
                let mut l = line;
                loop {
                    let ls = line_starts[l];
                    let le = line_end(l);
                    let seg = start.max(ls)..end.min(le);
                    if seg.start < seg.end {
                        let rel = seg.start - ls..seg.end - ls;
                        match out[l].last_mut() {
                            Some((prev, t)) if *t == token && prev.end == rel.start => {
                                prev.end = rel.end
                            }
                            _ => out[l].push((rel, token)),
                        }
                    }
                    if end <= le + 1 || l + 1 >= line_starts.len() {
                        break;
                    }
                    l += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_extension_is_plain() {
        assert!(language_for_path("Makefile").is_none());
        assert!(language_for_path("logo.png").is_none());
        assert!(language_for_path("notes.txt").is_none());
    }

    #[test]
    fn rust_tokens_land_on_the_right_lines() {
        let lang = language_for_path("x.rs").unwrap();
        let lines = highlight_lines(lang, "fn main() {\n    let x = 1; // hi\n}");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains(&(0..2, Token::Keyword)));
        assert!(lines[0].contains(&(3..7, Token::Function)));
        assert!(lines[1].iter().any(|&(_, t)| t == Token::Keyword));
        assert!(lines[1].iter().any(|&(_, t)| t == Token::Comment));
    }

    #[test]
    fn python_highlights_and_empty_is_one_line() {
        let lang = language_for_path("x.py").unwrap();
        let lines = highlight_lines(lang, "def add(a):\n    return a\n");
        assert!(lines[0].iter().any(|&(_, t)| t == Token::Keyword));
        assert_eq!(highlight_lines(lang, ""), vec![Vec::<(Range<usize>, Token)>::new()]);
    }

    #[test]
    fn spans_are_sorted_and_nonoverlapping() {
        let lang = language_for_path("x.rs").unwrap();
        let lines = highlight_lines(lang, "/* a\nmulti\n*/ fn f() {}\n");
        for line in &lines {
            for pair in line.windows(2) {
                assert!(pair[0].0.end <= pair[1].0.start, "overlapping {line:?}");
            }
        }
    }
}
