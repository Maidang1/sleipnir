//! The diff view's syntax paint, over [`sleipnir_syntax`].
//!
//! The path names the language — through the registry, so a fence tag and a
//! path agree — and the spans come back per line and relative to that line,
//! which is the shape the row painter wants. Grammars arrive from
//! [`sleipnir_syntax::standard`], registered once for the process: the rows it
//! links are the ones this build can paint, and a language outside them paints
//! nothing rather than something wrong.

use sleipnir_syntax::HighlightKind;
use std::ops::Range;
use std::path::Path;

/// Register the grammars this build carries, once.
fn installed() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(sleipnir_syntax::standard::install);
}

/// The language `path` is written in, if this build can paint it.
pub fn language(path: &str) -> Option<&'static sleipnir_syntax::lang::Lang> {
    installed();
    sleipnir_syntax::registry::of_path(Path::new(path))?.lang()
}

/// Per-line spans of `source` for the language `lang`.
///
/// A span that straddles a line break is split at it, and the newline itself
/// is never painted: the row painter owns the line's own whitespace.
pub fn highlight_lines(
    lang: &'static sleipnir_syntax::lang::Lang,
    source: &str,
) -> Vec<Vec<(Range<usize>, HighlightKind)>> {
    let mut starts = vec![0usize];
    starts.extend(source.match_indices('\n').map(|(at, _)| at + 1));
    let mut out: Vec<Vec<(Range<usize>, HighlightKind)>> = vec![Vec::new(); starts.len()];
    for (span, kind) in lang.highlight(source).unwrap_or_default() {
        if span.end <= span.start {
            continue;
        }
        // The first line the span reaches, and every one after it until it
        // ends: a span covers fewer lines than the file in the common case.
        let first = starts.partition_point(|start| *start <= span.start) - 1;
        for at in first..starts.len() {
            let begin = starts[at];
            if begin >= span.end {
                break;
            }
            let end = starts
                .get(at + 1)
                .map_or(source.len(), |next| next.saturating_sub(1))
                .min(span.end);
            if end > begin {
                out[at].push((span.start.max(begin) - begin..end - begin, kind));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rust_file_gets_per_line_spans() {
        let Some(lang) = language("main.rs") else {
            panic!("this build paints rust");
        };
        let spans = highlight_lines(lang, "fn main() {}\nlet x = 1;\n");
        assert_eq!(spans.len(), 3);
        assert!(
            spans[0]
                .iter()
                .any(|(range, kind)| kind == &HighlightKind::Keyword
                    && &"fn main() {}"[range.clone()] == "fn"),
            "the keyword is painted on the line it sits on: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .flatten()
                .all(|(range, _)| range.start <= range.end),
            "every span is a range: {spans:?}"
        );
    }

    #[test]
    fn an_unknown_extension_names_no_language() {
        assert!(language("notes.zz").is_none(), "nothing paints notes.zz");
        assert!(language("notes.txt").is_none());
    }

    /// The path rule A1 fixed, run from the caller that now relies on it: a
    /// non-ASCII file name resolves without panicking, and resolves to the
    /// language its extension names.
    #[test]
    fn a_non_ascii_file_name_resolves_without_panicking() {
        for (path, expected) in [
            ("資料/資料.rs", true),
            ("src/日本語.ts", true),
            // `md` is a name in the registry but no grammar is linked for it.
            ("résumé.md", false),
            ("notes.gleam", false),
        ] {
            assert_eq!(language(path).is_some(), expected, "{path}");
        }
    }

    #[test]
    fn an_empty_source_is_one_line_of_nothing() {
        let Some(lang) = language("main.rs") else {
            panic!("this build paints rust");
        };
        let spans = highlight_lines(lang, "");
        assert_eq!(spans.len(), 1);
        assert!(spans[0].is_empty());
    }
}
