//! Single-line overlay query input with IME support (palette, find, history).
//!
//! The overlay query boxes are painted divs, not real text fields, so the OS
//! input method has nothing to talk to: with a CJK input source active, typed
//! keys were dispatched as raw keystrokes and composition never started. This
//! module registers a GPUI [`InputHandler`] for the shell focus handle while a
//! query surface is open (via a zero-cost `canvas` in the query box's paint),
//! so IME composition and commits land in the query string.
//!
//! Plain (non-IME) typing keeps flowing through the existing `key_char` paths
//! in `palette_key_down` / `find_key_down` — GPUI routes a printable key to
//! exactly one of the two, so there is no double input.
//!
//! Ranges on the [`InputHandler`] trait are UTF-16 offsets into the query
//! string; the helpers below do the mapping and are unit-tested.

use gpui::{App, Bounds, Entity, InputHandler, Pixels, UTF16Selection, Window};
use std::ops::Range;

use super::AppShell;

/// Which overlay query box an input event belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuerySurface {
    Palette,
    Find,
    History,
}

/// Length of `s` in UTF-16 code units (what the platform IME counts in).
pub(crate) fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// Map a UTF-16 offset to the byte offset of the char containing it (clamped).
pub(crate) fn utf16_to_byte(s: &str, utf16_offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in s.char_indices() {
        if units >= utf16_offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    s.len()
}

/// Map a byte offset to a UTF-16 offset, clamping back to a char boundary.
pub(crate) fn byte_to_utf16(s: &str, byte_offset: usize) -> usize {
    let mut end = byte_offset.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    utf16_len(&s[..end])
}

/// Replace `range_utf16` (clamped) with `text`. Returns the UTF-16 offset at
/// which `text` was inserted.
pub(crate) fn splice_utf16(s: &mut String, range_utf16: Range<usize>, text: &str) -> usize {
    let start = utf16_to_byte(s, range_utf16.start);
    let end = utf16_to_byte(s, range_utf16.end).max(start);
    let start_utf16 = byte_to_utf16(s, start);
    s.replace_range(start..end, text);
    start_utf16
}

impl AppShell {
    pub(crate) fn query_text(&self, surface: QuerySurface) -> &str {
        match surface {
            QuerySurface::Palette => &self.palette_query,
            QuerySurface::Find => &self.find_query,
            QuerySurface::History => &self.history_query,
        }
    }

    fn query_text_mut(&mut self, surface: QuerySurface) -> &mut String {
        match surface {
            QuerySurface::Palette => &mut self.palette_query,
            QuerySurface::Find => &mut self.find_query,
            QuerySurface::History => &mut self.history_query,
        }
    }

    pub(crate) fn query_marked(&self, surface: QuerySurface) -> Option<Range<usize>> {
        match surface {
            QuerySurface::Palette => self.palette_marked.clone(),
            QuerySurface::Find => self.find_marked.clone(),
            QuerySurface::History => self.history_marked.clone(),
        }
    }

    fn set_query_marked(&mut self, surface: QuerySurface, marked: Option<Range<usize>>) {
        match surface {
            QuerySurface::Palette => self.palette_marked = marked,
            QuerySurface::Find => self.find_marked = marked,
            QuerySurface::History => self.history_marked = marked,
        }
    }

    /// Refresh the owning surface after its query changed via IME.
    fn query_changed(&mut self, surface: QuerySurface, cx: &mut gpui::Context<Self>) {
        match surface {
            QuerySurface::Palette => {
                self.palette_selected = 0;
                cx.notify();
            }
            QuerySurface::Find => self.debounce_find(cx),
            QuerySurface::History => {
                self.history_selected = 0;
                cx.notify();
            }
        }
    }

    /// IME entry point: replace `range_utf16` (or the marked range, or the tail
    /// when both are absent) with `text`. `mark` records the inserted text as
    /// the in-progress composition range.
    pub(crate) fn query_replace(
        &mut self,
        surface: QuerySurface,
        range_utf16: Option<Range<usize>>,
        text: &str,
        mark: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let fallback = self.query_marked(surface).unwrap_or_else(|| {
            let end = utf16_len(self.query_text(surface));
            end..end
        });
        let range = range_utf16.unwrap_or(fallback);
        let start = splice_utf16(self.query_text_mut(surface), range, text);
        self.set_query_marked(surface, mark.then(|| start..start + utf16_len(text)));
        self.query_changed(surface, cx);
    }

    /// Zero-sized canvas that lives inside a query box and registers the IME
    /// input handler every paint, carrying the box bounds for the IME
    /// candidate window. No-op unless the shell focus handle is focused.
    pub(crate) fn query_input_canvas(
        &self,
        surface: QuerySurface,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::Styled as _;
        let shell = cx.entity().clone();
        let focus = self.focus_handle.clone();
        gpui::canvas(
            move |bounds, _window, _cx| bounds,
            move |bounds, _prepaint, window, cx| {
                window.handle_input(
                    &focus,
                    QueryInputHandler {
                        shell: shell.clone(),
                        surface,
                        bounds,
                    },
                    cx,
                );
            },
        )
        .absolute()
        .inset_0()
    }
}

/// [`InputHandler`] over one overlay query string. Cursor is always at the
/// end; composition state is the marked range stored on `AppShell`.
pub(crate) struct QueryInputHandler {
    shell: Entity<AppShell>,
    surface: QuerySurface,
    bounds: Bounds<Pixels>,
}

impl InputHandler for QueryInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        let end = utf16_len(self.shell.read(cx).query_text(self.surface));
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.shell.read(cx).query_marked(self.surface)
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        let shell = self.shell.read(cx);
        let query = shell.query_text(self.surface);
        let start = utf16_to_byte(query, range_utf16.start);
        let end = utf16_to_byte(query, range_utf16.end).max(start);
        *adjusted_range = Some(byte_to_utf16(query, start)..byte_to_utf16(query, end));
        Some(query[start..end].to_string())
    }

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.shell.update(cx, |this, cx| {
            this.query_replace(self.surface, replacement_range, text, false, cx)
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.shell.update(cx, |this, cx| {
            this.query_replace(self.surface, range_utf16, new_text, true, cx)
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut App) {
        self.shell.update(cx, |this, cx| {
            this.set_query_marked(self.surface, None);
            cx.notify();
        });
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        Some(self.bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        Some(utf16_len(self.shell.read(cx).query_text(self.surface)))
    }

    /// Overlay query boxes exist for typing, so when a CJK input source is
    /// active the IME gets printable keys first (the terminal itself keeps the
    /// default `false` so raw keys reach the PTY).
    fn prefers_ime_for_printable_keys(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_len_counts_surrogate_pairs() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("你好"), 2);
        assert_eq!(utf16_len("a😀b"), 4);
    }

    #[test]
    fn utf16_byte_roundtrip_with_cjk_and_emoji() {
        let s = "ab你😀c";
        for ch_count in 0..=s.chars().count() {
            let byte = s.char_indices().nth(ch_count).map(|(b, _)| b).unwrap_or(s.len());
            let u16 = byte_to_utf16(s, byte);
            assert_eq!(utf16_to_byte(s, u16), byte, "roundtrip at char {ch_count}");
        }
        // Out-of-range clamps to the end.
        assert_eq!(utf16_to_byte(s, 100), s.len());
        assert_eq!(byte_to_utf16(s, 100), utf16_len(s));
    }

    #[test]
    fn byte_to_utf16_snaps_off_boundary_backwards() {
        let s = "你好"; // 3 bytes each
        assert_eq!(byte_to_utf16(s, 4), 1); // inside 好 → snaps to its start
    }

    #[test]
    fn splice_appends_at_end() {
        let mut s = "abc".to_string();
        let start = splice_utf16(&mut s, 3..3, "de");
        assert_eq!(s, "abcde");
        assert_eq!(start, 3);
    }

    #[test]
    fn splice_replaces_marked_cjk_range() {
        let mut s = "find 你好 world".to_string();
        // "你好" is UTF-16 5..7.
        let start = splice_utf16(&mut s, 5..7, "好吗");
        assert_eq!(s, "find 好吗 world");
        assert_eq!(start, 5);
    }

    #[test]
    fn splice_replaces_emoji_range_by_utf16_offsets() {
        let mut s = "a😀b".to_string();
        // 😀 is UTF-16 1..3.
        let start = splice_utf16(&mut s, 1..3, "x");
        assert_eq!(s, "axb");
        assert_eq!(start, 1);
    }

    #[test]
    fn splice_clamps_inverted_and_out_of_range_offsets() {
        let mut s = "hi".to_string();
        splice_utf16(&mut s, 9..1, "!");
        assert_eq!(s, "hi!");
    }
}
