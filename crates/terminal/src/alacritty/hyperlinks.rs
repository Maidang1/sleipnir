use alacritty_terminal::{
    Term,
    event::EventListener,
    grid::Dimensions,
    index::{Boundary, Column, Direction as AlacDirection, Point as AlacPoint},
    term::{
        cell::Flags,
        search::{Match, RegexIter, RegexSearch},
    },
};
use log::{info, warn};
use regex::Regex;
use std::{
    ops::{Index, Range as StdRange},
    time::{Duration, Instant},
};
use url::Url;
use util::paths::{PathStyle, UrlExt};

use crate::Range;

const URL_REGEX: &str = r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://|zed://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`']+"#;
const WIDE_CHAR_SPACERS: Flags =
    Flags::from_bits(Flags::LEADING_WIDE_CHAR_SPACER.bits() | Flags::WIDE_CHAR_SPACER.bits())
        .unwrap();

pub(crate) struct RegexSearches {
    url_regex: Option<RegexSearch>,
    path_hyperlink_regexes: Vec<Regex>,
    path_hyperlink_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HyperlinkMatch {
    pub(crate) text: String,
    pub(crate) is_url: bool,
    pub(crate) range: Range,
}

impl From<(String, bool, Match)> for HyperlinkMatch {
    fn from((text, is_url, range): (String, bool, Match)) -> Self {
        Self {
            text,
            is_url,
            range: Range::from_alacritty(range),
        }
    }
}

impl Default for RegexSearches {
    fn default() -> Self {
        Self::new(Vec::<String>::new(), 0)
    }
}
impl RegexSearches {
    pub(crate) fn new(
        path_hyperlink_regexes: impl IntoIterator<Item: AsRef<str>>,
        path_hyperlink_timeout_ms: u64,
    ) -> Self {
        Self {
            url_regex: RegexSearch::new(URL_REGEX).ok(),
            path_hyperlink_regexes: Self::path_hyperlink_regexes(path_hyperlink_regexes),
            path_hyperlink_timeout: Duration::from_millis(path_hyperlink_timeout_ms),
        }
    }

    fn path_hyperlink_regexes(
        path_hyperlink_regexes: impl IntoIterator<Item: AsRef<str>>,
    ) -> Vec<Regex> {
        path_hyperlink_regexes
            .into_iter()
            .filter_map(|regex| {
                Regex::new(regex.as_ref())
                    .inspect_err(|error| {
                        warn!(
                            concat!(
                                "Ignoring path hyperlink regex specified in ",
                                "`terminal.path_hyperlink_regexes`:\n\n\t{}\n\nError: {}",
                            ),
                            regex.as_ref(),
                            error
                        );
                    })
                    .ok()
            })
            .collect()
    }
}

pub(crate) fn find_from_grid_point<T: EventListener>(
    term: &Term<T>,
    point: AlacPoint,
    regex_searches: &mut RegexSearches,
    path_style: PathStyle,
) -> Option<HyperlinkMatch> {
    let grid = term.grid();
    let link = grid.index(point).hyperlink();
    let found_word = if let Some(ref url) = link {
        let mut min_index = point;
        loop {
            let new_min_index = min_index.sub(term, Boundary::Cursor, 1);
            if new_min_index == min_index || grid.index(new_min_index).hyperlink() != link {
                break;
            } else {
                min_index = new_min_index
            }
        }

        let mut max_index = point;
        loop {
            let new_max_index = max_index.add(term, Boundary::Cursor, 1);
            if new_max_index == max_index || grid.index(new_max_index).hyperlink() != link {
                break;
            } else {
                max_index = new_max_index
            }
        }

        let url = url.uri().to_owned();
        let url_match = min_index..=max_index;

        Some((url, true, url_match))
    } else {
        let (line_start, line_end) = (term.line_search_left(point), term.line_search_right(point));
        let url_match = regex_searches.url_regex.as_mut().and_then(|url_regex| {
            RegexIter::new(line_start, line_end, AlacDirection::Right, term, url_regex)
                .find(|rm| rm.contains(&point))
                .map(|url_match| {
                    let url = term.bounds_to_string(*url_match.start(), *url_match.end());
                    sanitize_url_punctuation(url, url_match, term)
                })
        });

        if let Some((url, url_match)) = url_match {
            Some((url, true, url_match))
        } else {
            path_match(
                &term,
                line_start,
                line_end,
                point,
                &mut regex_searches.path_hyperlink_regexes,
                regex_searches.path_hyperlink_timeout,
            )
            .map(|(path, path_match)| (path, false, path_match))
        }
    };

    found_word.map(|found_word| normalize_found_word(found_word, path_style))
}

fn normalize_found_word(
    found_word: (String, bool, Match),
    path_style: PathStyle,
) -> HyperlinkMatch {
    let (maybe_url_or_path, is_url, word_match) = found_word;
    normalize_hyperlink_match(
        maybe_url_or_path,
        is_url,
        Range::from_alacritty(word_match),
        path_style,
    )
}

fn normalize_hyperlink_match(
    maybe_url_or_path: String,
    is_url: bool,
    range: Range,
    path_style: PathStyle,
) -> HyperlinkMatch {
    if is_url {
        // Treat "file://" IRIs like file paths to ensure
        // that line numbers at the end of the path are
        // handled correctly.
        // Use Url::to_file_path() to properly handle Windows drive letters
        // (e.g., file:///C:/path -> C:\path)
        if maybe_url_or_path.starts_with("file://") {
            if let Ok(url) = Url::parse(&maybe_url_or_path) {
                if let Ok(path) = url.to_file_path_ext(path_style) {
                    return HyperlinkMatch {
                        text: path.to_string_lossy().into_owned(),
                        is_url: false,
                        range,
                    };
                } else if let Some(path) = try_osc8_url_to_path(url)
                    && path_style.is_posix()
                {
                    return HyperlinkMatch {
                        text: path,
                        is_url: false,
                        range,
                    };
                }
            }
            // Fallback: strip file:// prefix if URL parsing fails
            let path = maybe_url_or_path
                .strip_prefix("file://")
                .unwrap_or(&maybe_url_or_path);
            HyperlinkMatch {
                text: path.to_string(),
                is_url: false,
                range,
            }
        } else {
            HyperlinkMatch {
                text: maybe_url_or_path,
                is_url: true,
                range,
            }
        }
    } else {
        HyperlinkMatch {
            text: maybe_url_or_path,
            is_url: false,
            range,
        }
    }
}

// OSC 8 mandates that file:// URIs must be encoded as file://{host}{path}
// We need to skip the {host} part if it's set
fn try_osc8_url_to_path(url: url::Url) -> Option<String> {
    use percent_encoding::percent_decode;
    if url.scheme() != "file" {
        return None;
    }

    let bytes = url
        .path_segments()?
        .skip(1)
        .flat_map(|segment| percent_decode(segment.as_bytes()))
        .collect::<Vec<u8>>();
    bytes.try_into().ok()
}

fn sanitize_url_punctuation<T: EventListener>(
    url: String,
    url_match: Match,
    term: &Term<T>,
) -> (String, Match) {
    let mut sanitized_url = url;
    let mut chars_trimmed = 0;

    // Count parentheses in the URL
    let (open_parens, mut close_parens) =
        sanitized_url
            .chars()
            .fold((0, 0), |(opens, closes), c| match c {
                '(' => (opens + 1, closes),
                ')' => (opens, closes + 1),
                _ => (opens, closes),
            });

    // Remove trailing characters that shouldn't be at the end of URLs
    while let Some(last_char) = sanitized_url.chars().last() {
        let should_remove = match last_char {
            // These may be part of a URL but not at the end. It's not that the spec
            // doesn't allow them, but they are frequently used in plain text as delimiters
            // where they're not meant to be part of the URL.
            '.' | ',' | ':' | ';' => true,
            '(' => true,
            ')' if close_parens > open_parens => {
                close_parens -= 1;

                true
            }
            _ => false,
        };

        if should_remove {
            sanitized_url.pop();
            chars_trimmed += 1;
        } else {
            break;
        }
    }

    if chars_trimmed > 0 {
        let new_end = url_match.end().sub(term, Boundary::Grid, chars_trimmed);
        let sanitized_match = Match::new(*url_match.start(), new_end);
        (sanitized_url, sanitized_match)
    } else {
        (sanitized_url, url_match)
    }
}

/// Returns the byte offset just past the first unbalanced `(` in `s`, or `None`
/// if all parentheses are balanced. Used to strip prefixes like `Update(` from
/// path matches while preserving balanced parens in filenames like `file(copy).txt`.
fn first_unbalanced_open_paren(s: &str) -> Option<usize> {
    let mut balance: i32 = 0;
    let mut first_unmatched = None;
    for (i, c) in s.char_indices() {
        match c {
            '(' => {
                if balance == 0 {
                    first_unmatched = Some(i + c.len_utf8());
                }
                balance += 1;
            }
            ')' => {
                balance -= 1;
                if balance <= 0 {
                    balance = 0;
                    first_unmatched = None;
                }
            }
            _ => {}
        }
    }
    first_unmatched.filter(|_| balance > 0)
}

fn path_match<T>(
    term: &Term<T>,
    line_start: AlacPoint,
    line_end: AlacPoint,
    hovered: AlacPoint,
    path_hyperlink_regexes: &mut Vec<Regex>,
    path_hyperlink_timeout: Duration,
) -> Option<(String, Match)> {
    if path_hyperlink_regexes.is_empty() || path_hyperlink_timeout.as_millis() == 0 {
        return None;
    }
    debug_assert!(line_start <= hovered);
    debug_assert!(line_end >= hovered);
    let search_start_time = Instant::now();

    let timed_out = || {
        let elapsed_time = Instant::now().saturating_duration_since(search_start_time);
        (elapsed_time > path_hyperlink_timeout)
            .then_some((elapsed_time.as_millis(), path_hyperlink_timeout.as_millis()))
    };

    // This used to be: `let line = term.bounds_to_string(line_start, line_end)`, however, that
    // api compresses tab characters into a single space, whereas we require a cell accurate
    // string representation of the line. The below algorithm does this, but seems a bit odd.
    // Maybe there is a clean api for doing this, but I couldn't find it.
    let mut line = String::with_capacity(
        (line_end.line.0 - line_start.line.0 + 1) as usize * term.grid().columns(),
    );
    let first_cell = &term.grid()[line_start];
    let mut prev_len = 0;
    line.push(first_cell.c);
    let mut hovered_point_byte_offset = None;

    if line_start == hovered {
        hovered_point_byte_offset = Some(0);
    }

    for cell in term.grid().iter_from(line_start) {
        if cell.point > line_end {
            break;
        }

        if !cell.flags.intersects(WIDE_CHAR_SPACERS) {
            prev_len = line.len();
            match cell.c {
                ' ' | '\t' => line.push(' '),
                c => line.push(c),
            }
        }

        if cell.point == hovered {
            debug_assert!(hovered_point_byte_offset.is_none());
            hovered_point_byte_offset = Some(prev_len);
        }
    }
    let line = line.trim_ascii_end();
    let hovered_point_byte_offset = hovered_point_byte_offset?;
    if line.len() <= hovered_point_byte_offset {
        return None;
    }
    let found_from_range = |path_range: StdRange<usize>,
                            link_range: StdRange<usize>,
                            position: Option<(u32, Option<u32>)>| {
        let advance_point_by_str = |mut point: AlacPoint, s: &str| {
            for _ in s.chars() {
                point = term
                    .expand_wide(point, AlacDirection::Right)
                    .add(term, Boundary::Grid, 1);
            }

            // There does not appear to be an alacritty api that is
            // "move to start of current wide char", so we have to do it ourselves.
            let flags = term.grid().index(point).flags;
            if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                AlacPoint::new(point.line + 1, Column(0))
            } else if flags.contains(Flags::WIDE_CHAR_SPACER) {
                AlacPoint::new(point.line, point.column - 1)
            } else {
                point
            }
        };

        let link_start = advance_point_by_str(line_start, &line[..link_range.start]);
        let link_end = advance_point_by_str(link_start, &line[link_range]);
        let link_match = link_start
            ..=term
                .expand_wide(link_end, AlacDirection::Left)
                .sub(term, Boundary::Grid, 1);

        (
            {
                let mut path = line[path_range].to_string();
                position.inspect(|(line, column)| {
                    path += &format!(":{line}");
                    column.inspect(|column| path += &format!(":{column}"));
                });
                path
            },
            link_match,
        )
    };

    for regex in path_hyperlink_regexes {
        let mut path_found = false;

        for (line_start_offset, captures) in regex
            .captures_iter(&line)
            .map(|captures| (0usize, captures))
        {
            path_found = true;
            let match_range = captures.get(0).unwrap().range();
            let (mut path_range, line_column) = if let Some(path) = captures.name("path") {
                let parse = |name: &str| {
                    captures
                        .name(name)
                        .and_then(|capture| capture.as_str().parse().ok())
                };

                (
                    path.range(),
                    parse("line").map(|line| (line, parse("column"))),
                )
            } else {
                (match_range.clone(), None)
            };
            let mut link_range = captures
                .name("link")
                .map_or_else(|| match_range.clone(), |link| link.range());

            path_range.start += line_start_offset;
            path_range.end += line_start_offset;
            link_range.start += line_start_offset;
            link_range.end += line_start_offset;

            // Strip prefix up to the first unbalanced `(` in the matched path.
            // This handles delimiter parens like `Update(.claude/SKILL.md)` while
            // preserving balanced parens in filenames like `file(copy).txt`.
            // Analogous to `sanitize_url_punctuation` which strips unbalanced
            // trailing `)` from URLs.
            if let Some(trim) = first_unbalanced_open_paren(&line[path_range.clone()]) {
                path_range.start += trim;
                link_range.start = link_range.start.max(path_range.start);
            }

            if !link_range.contains(&hovered_point_byte_offset) {
                // No match, just skip.
                continue;
            }
            let found = found_from_range(path_range, link_range, line_column);

            if found.1.contains(&hovered) {
                return Some(found);
            }
        }

        if path_found {
            return None;
        }

        if let Some((timed_out_ms, timeout_ms)) = timed_out() {
            warn!("Timed out processing path hyperlink regexes after {timed_out_ms}ms");
            info!("{timeout_ms}ms time out specified in `terminal.path_hyperlink_timeout_ms`");
            return None;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    // M1: disabled
}
