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

use gpui::{
    App, Bounds, Entity, InputHandler, Keystroke, Pixels, ScrollHandle, UTF16Selection, Window,
};
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

/// Resolve a UTF-16 range to a byte range, clamped to `s` and never inverted.
pub(crate) fn clamped_byte_range(s: &str, range_utf16: Range<usize>) -> Range<usize> {
    let start = utf16_to_byte(s, range_utf16.start);
    let end = utf16_to_byte(s, range_utf16.end).max(start);
    start..end
}

/// Replace `range_utf16` (clamped) with `text`. Returns the UTF-16 offset at
/// which `text` was inserted.
pub(crate) fn splice_utf16(s: &mut String, range_utf16: Range<usize>, text: &str) -> usize {
    let start = clamped_byte_range(s, range_utf16.clone()).start;
    let start_utf16 = byte_to_utf16(s, start);
    s.replace_range(clamped_byte_range(s, range_utf16), text);
    start_utf16
}

/// One overlay query box: the editable text, its IME composition range, and
/// the selection cursor over the filtered results. Every query surface
/// (palette, find, history, theme picker, inline rename) composes this instead
/// of re-implementing the same text editing and selection wrapping. A surface
/// with no result set — inline rename — simply leaves `selected` at zero.
#[derive(Clone, Default)]
pub(crate) struct QueryBox {
    pub(crate) text: String,
    pub(crate) marked: Option<Range<usize>>,
    pub(crate) selected: usize,
    pub(crate) scroll: ScrollHandle,
}

impl QueryBox {
    pub(crate) fn reset(&mut self) {
        self.text.clear();
        self.marked = None;
        self.selected = 0;
    }

    /// A box pre-filled with `text` (inline rename seeds the current label).
    pub(crate) fn seeded(text: String) -> Self {
        Self {
            text,
            ..Self::default()
        }
    }

    /// Clamp `selected` into the live result set.
    pub(crate) fn clamp_selected(&mut self, items: usize) {
        self.selected = self.selected.min(items.saturating_sub(1));
    }

    /// Move `selected` by `delta`, wrapping inside `0..items`.
    pub(crate) fn move_selected(&mut self, items: usize, delta: i32) {
        if items == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(items as i32) as usize;
        self.scroll.scroll_to_item(self.selected);
    }

    /// Handle the keystrokes every query box shares: backspace, ⌘V paste,
    /// printable typing, and up/down selection over `items` results. Returns
    /// true when the box changed. Escape, enter, and surface-specific
    /// modifiers stay with the caller.
    pub(crate) fn edit(
        &mut self,
        event: &Keystroke,
        items: usize,
        read_clipboard: &mut dyn FnMut() -> Option<String>,
    ) -> bool {
        let key = event.key.as_str();
        let modifiers = event.modifiers;
        let changed = match key {
            "backspace" => {
                self.text.pop();
                true
            }
            "v" if modifiers.platform && !modifiers.alt => {
                let Some(pasted) = read_clipboard() else {
                    return false;
                };
                self.text.push_str(&pasted.replace(['\n', '\r'], ""));
                true
            }
            "up" | "arrowup" => {
                self.move_selected(items, -1);
                true
            }
            "down" | "arrowdown" => {
                self.move_selected(items, 1);
                true
            }
            _ => match event.key_char.as_ref() {
                Some(ch) if !ch.is_empty() && !ch.chars().any(char::is_control) => {
                    self.text.push_str(ch);
                    true
                }
                _ => false,
            },
        };
        if changed && !matches!(key, "up" | "arrowup" | "down" | "arrowdown") {
            self.selected = 0;
        }
        changed
    }
}

impl AppShell {
    /// The query box backing an overlay surface.
    pub(crate) fn query_box(&self, surface: QuerySurface) -> &QueryBox {
        match surface {
            QuerySurface::Palette => &self.palette.input,
            QuerySurface::Find => &self.find.input,
            QuerySurface::History => &self.history.input,
        }
    }

    fn query_box_mut(&mut self, surface: QuerySurface) -> &mut QueryBox {
        match surface {
            QuerySurface::Palette => &mut self.palette.input,
            QuerySurface::Find => &mut self.find.input,
            QuerySurface::History => &mut self.history.input,
        }
    }

    /// Refresh the owning surface after its query changed via IME.
    fn query_changed(&mut self, surface: QuerySurface, cx: &mut gpui::Context<Self>) {
        match surface {
            QuerySurface::Palette => {
                self.palette.input.selected = 0;
                cx.notify();
            }
            QuerySurface::Find => self.debounce_find(cx),
            QuerySurface::History => {
                self.history.input.selected = 0;
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
        let fallback = self.query_box(surface).marked.clone().unwrap_or_else(|| {
            let end = utf16_len(&self.query_box(surface).text);
            end..end
        });
        let range = range_utf16.unwrap_or(fallback);
        let input = self.query_box_mut(surface);
        let start = splice_utf16(&mut input.text, range, text);
        input.marked = mark.then(|| start..start + utf16_len(text));
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
        let shell = cx.entity();
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
        let end = utf16_len(&self.shell.read(cx).query_box(self.surface).text);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.shell.read(cx).query_box(self.surface).marked.clone()
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        let shell = self.shell.read(cx);
        let query = &shell.query_box(self.surface).text;
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
            this.query_box_mut(self.surface).marked = None;
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
        Some(utf16_len(&self.shell.read(cx).query_box(self.surface).text))
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

    fn keystroke(key: &str, key_char: Option<&str>, platform: bool) -> Keystroke {
        Keystroke {
            modifiers: gpui::Modifiers {
                platform,
                ..Default::default()
            },
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    #[test]
    fn edit_backspace_pops_text_and_resets_selected() {
        let mut input = QueryBox::seeded("abc".to_string());
        input.selected = 2;
        assert!(input.edit(&keystroke("backspace", None, false), 5, &mut || None));
        assert_eq!(input.text, "ab");
        assert_eq!(input.selected, 0);
    }

    #[test]
    fn edit_paste_appends_clipboard_without_newlines() {
        let mut input = QueryBox::seeded("a".to_string());
        input.selected = 1;
        let mut clipboard = || {
            Some(
                "b
c
"
                .to_string(),
            )
        };
        assert!(input.edit(&keystroke("v", None, true), 5, &mut clipboard));
        assert_eq!(input.text, "abc");
        assert_eq!(input.selected, 0);
    }

    #[test]
    fn edit_paste_without_clipboard_changes_nothing() {
        let mut input = QueryBox::seeded("a".to_string());
        input.selected = 1;
        assert!(!input.edit(&keystroke("v", None, true), 5, &mut || None));
        assert_eq!(input.text, "a");
        assert_eq!(input.selected, 1);
    }

    #[test]
    fn edit_plain_v_types_instead_of_pasting() {
        let mut input = QueryBox::default();
        let mut clipboard = || Some("pasted".to_string());
        assert!(input.edit(&keystroke("v", Some("v"), false), 5, &mut clipboard));
        assert_eq!(input.text, "v");
    }

    #[test]
    fn edit_types_printable_but_not_control_chars() {
        let mut input = QueryBox::default();
        assert!(input.edit(&keystroke("s", Some("s"), false), 5, &mut || None));
        assert_eq!(input.text, "s");
        assert!(!input.edit(&keystroke("a", Some("\u{7}"), false), 5, &mut || None));
        assert!(!input.edit(&keystroke("x", Some(""), false), 5, &mut || None));
        assert!(!input.edit(&keystroke("c", None, true), 5, &mut || None));
        assert_eq!(input.text, "s");
    }

    #[test]
    fn edit_selection_keys_move_without_resetting_selected() {
        let mut input = QueryBox::default();
        input.selected = 1;
        assert!(input.edit(&keystroke("down", None, false), 5, &mut || None));
        assert_eq!(input.selected, 2);
        assert!(input.edit(&keystroke("up", None, false), 5, &mut || None));
        assert_eq!(input.selected, 1);
    }

    #[test]
    fn move_selected_wraps_around_both_ends() {
        let mut input = QueryBox::default();
        input.move_selected(3, -1);
        assert_eq!(input.selected, 2);
        input.move_selected(3, 1);
        assert_eq!(input.selected, 0);
    }

    #[test]
    fn move_and_clamp_with_empty_result_set() {
        let mut input = QueryBox::default();
        input.selected = 4;
        input.move_selected(0, 1);
        assert_eq!(input.selected, 0);
        input.selected = 4;
        input.clamp_selected(0);
        assert_eq!(input.selected, 0);
    }

    #[test]
    fn clamp_selected_caps_at_last_item() {
        let mut input = QueryBox::default();
        input.selected = 5;
        input.clamp_selected(3);
        assert_eq!(input.selected, 2);
        input.clamp_selected(3);
        assert_eq!(input.selected, 2);
    }

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
            let byte = s
                .char_indices()
                .nth(ch_count)
                .map(|(b, _)| b)
                .unwrap_or(s.len());
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
        // Deliberately malformed IME offsets: this is input to clamp, not a
        // range to iterate over.
        splice_utf16(&mut s, std::ops::Range { start: 9, end: 1 }, "!");
        assert_eq!(s, "hi!");
    }
}
