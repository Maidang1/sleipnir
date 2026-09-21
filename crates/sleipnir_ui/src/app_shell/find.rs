//! Find in scrollback: the bar, its key handling and the search state.
//!
//! A child module of `app_shell` so it can drive the shell's private find
//! state without widening it to the crate.

use gpui::{
    ClickEvent, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use super::{AppShell, Find, FindNext, FindPrev};
use crate::chrome::ChromeTokens;
use crate::chrome::pixel;

/// Find-in-scrollback state, owned as one domain instead of nine loose fields
/// on [`AppShell`]. Grouping them here means the shell composes a `FindState`
/// rather than accreting `find_*` fields, and the pure query→regex mapping
/// ([`FindState::pattern`]) is unit-testable without a live terminal.
#[derive(Default)]
pub(crate) struct FindState {
    /// The query box: editable text, IME composition range, selection.
    pub(crate) input: crate::app_shell::query::QueryBox,
    /// Monotonic generation used to discard stale debounce timers.
    pub(crate) debounce_gen: u64,
    /// Monotonic request id used to discard stale asynchronous search results.
    search_gen: u64,
    match_count: usize,
    active_index: usize,
    /// Terminal pane the running/last find targeted, so a workspace commit
    /// can re-run the search when the active pane changes.
    searched_term: Option<gpui::EntityId>,
    /// Regex mode: treat the query as a raw regex instead of a literal (⌥⌘R).
    regex: bool,
    /// Match case (⌥⌘C). Off = case-insensitive; on = case-sensitive.
    match_case: bool,
}

impl FindState {
    /// Reset the pure (non-cx) find state on teardown: clear the IME
    /// composition range and bump the debounce generation so any pending
    /// timer fires into a stale generation and is discarded.
    pub(crate) fn teardown(&mut self) {
        self.input.marked = None;
        self.debounce_gen = self.debounce_gen.wrapping_add(1);
    }

    /// Resolve the raw query into the regex handed to alacritty's search.
    ///
    /// Literal mode escapes regex metacharacters; regex mode passes the query
    /// through unchanged. Case is controlled with the `(?i)` / `(?-i)` inline
    /// flags, which alacritty's regex engine honours regardless of its
    /// smart-case default.
    fn pattern(&self, query: &str) -> String {
        let pattern = if self.regex {
            query.to_owned()
        } else {
            regex::escape(query)
        };
        if self.match_case {
            format!("(?-i){pattern}")
        } else {
            format!("(?i){pattern}")
        }
    }
}

impl AppShell {
    pub(super) fn on_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(window, cx);
    }

    pub(crate) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_input(crate::ui_mode::InputMode::Find, cx);
        window.focus(&self.focus_handle, cx);
        cx.notify();
        if !self.find.input.text.is_empty() {
            self.run_find(cx);
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.is_find() {
            self.set_input(crate::ui_mode::InputMode::Terminal, cx);
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn clear_find_matches(&mut self, cx: &mut Context<Self>) {
        // Invalidate searches that are still running so they cannot repopulate
        // matches after the query is cleared or the find bar is closed.
        self.find.search_gen = self.find.search_gen.wrapping_add(1);
        if let Some(view) = self.active_view(cx) {
            if let Some(term) = view.read(cx).terminal_entity().cloned() {
                term.update(cx, |t, _| t.clear_matches());
            }
        }
        self.find.match_count = 0;
        self.find.active_index = 0;
    }

    /// Re-run the search if the active pane changed since the last run so the
    /// count and highlights describe the pane actually on screen. Called from
    /// `commit_workspace` (tab switches and pane focus moves).
    pub(crate) fn refresh_find_for_active_pane(&mut self, cx: &mut Context<Self>) {
        if !self.input.is_find() || self.find.input.text.is_empty() {
            return;
        }
        let Some(view) = self.active_view(cx) else {
            return;
        };
        if Some(view.entity_id()) == self.find.searched_term {
            return;
        }
        self.run_find(cx);
    }

    pub(super) fn debounce_find(&mut self, cx: &mut Context<Self>) {
        self.find.debounce_gen = self.find.debounce_gen.wrapping_add(1);
        let generation = self.find.debounce_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            this.update(cx, |this, cx| {
                if this.find.debounce_gen == generation && this.input.is_find() {
                    this.run_find(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn run_find(&mut self, cx: &mut Context<Self>) {
        // An immediate search (for example Enter) supersedes pending debounce timers.
        self.find.debounce_gen = self.find.debounce_gen.wrapping_add(1);
        let query = self.find.input.text.clone();
        if query.is_empty() {
            self.clear_find_matches(cx);
            cx.notify();
            return;
        }
        let Some(search) = terminal::Search::new(&self.find.pattern(&query)) else {
            self.clear_find_matches(cx);
            cx.notify();
            return;
        };
        let Some(view) = self.active_view(cx) else {
            return;
        };
        let Some(term) = view.read(cx).terminal_entity().cloned() else {
            return;
        };
        // Remember which pane this search targeted so a workspace commit can
        // re-run it when the active pane changes (tab switch, focus move).
        self.find.searched_term = Some(term.entity_id());
        self.find.search_gen = self.find.search_gen.wrapping_add(1);
        let generation = self.find.search_gen;
        let task = term.update(cx, |t, cx| t.find_matches(search, cx));
        cx.spawn(async move |this, cx| {
            let matches = task.await;
            this.update(cx, |this, cx| {
                // Search completion is asynchronous. Only the newest request
                // for the pane that is still active may update UI state.
                if this.find.search_gen != generation
                    || !this.input.is_find()
                    || this.active_view(cx).as_ref() != Some(&view)
                {
                    return;
                }
                let count = matches.len();
                term.update(cx, |t, _| {
                    t.matches = matches;
                    if count > 0 {
                        t.activate_match(0);
                    }
                });
                this.find.match_count = count;
                this.find.active_index = 0;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn on_find_next(
        &mut self,
        _: &FindNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_find(1, cx);
    }

    pub(super) fn on_find_prev(
        &mut self,
        _: &FindPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_find(-1, cx);
    }

    fn step_find(&mut self, delta: i32, cx: &mut Context<Self>) {
        if self.find.match_count == 0 {
            if self.input.is_find() && !self.find.input.text.is_empty() {
                self.run_find(cx);
            }
            return;
        }
        let n = self.find.match_count as i32;
        let next = (self.find.active_index as i32 + delta).rem_euclid(n) as usize;
        self.find.active_index = next;
        if let Some(view) = self.active_view(cx) {
            if let Some(term) = view.read(cx).terminal_entity().cloned() {
                term.update(cx, |t, _| t.activate_match(next));
            }
        }
        cx.notify();
    }

    pub(super) fn find_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.input.is_find() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.close_find(window, cx);
                true
            }
            "enter" => {
                if event.keystroke.modifiers.shift {
                    self.step_find(-1, cx);
                } else {
                    // First Enter runs search; subsequent Enter = next match.
                    if self.find.match_count == 0 || event.keystroke.modifiers.platform {
                        self.run_find(cx);
                    } else {
                        self.step_find(1, cx);
                    }
                }
                true
            }
            // ⌥⌘C toggles match-case; ⌥⌘R toggles regex (macOS find-bar convention).
            "c" if event.keystroke.modifiers.alt && event.keystroke.modifiers.platform => {
                self.find.match_case = !self.find.match_case;
                self.debounce_find(cx);
                true
            }
            "r" if event.keystroke.modifiers.alt && event.keystroke.modifiers.platform => {
                self.find.regex = !self.find.regex;
                self.debounce_find(cx);
                true
            }
            // Swallow other ⌘-keys so they don't reach the PTY; ⌘V falls
            // through to `edit` for paste.
            _ if event.keystroke.modifiers.platform && key != "v" => true,
            _ => {
                let changed = self.find.input.edit(&event.keystroke, 0, &mut || {
                    cx.read_from_clipboard().and_then(|item| item.text())
                });
                if changed {
                    self.debounce_find(cx);
                }
                // Swallow non-platform keys so they don't go to the PTY.
                true
            }
        }
    }

    pub(super) fn render_find_bar(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query_display: SharedString = if self.find.input.text.is_empty() {
            "Find in scrollback…".into()
        } else {
            format!("{}|", self.find.input.text).into()
        };
        let query_color = if self.find.input.text.is_empty() {
            tokens.fg_muted
        } else {
            tokens.fg
        };
        let count: SharedString = if self.find.input.text.is_empty() {
            "".into()
        } else if self.find.match_count == 0 {
            "0 matches".into()
        } else {
            format!("{}/{}", self.find.active_index + 1, self.find.match_count).into()
        };
        // Legible on-accent foreground for the active toggle buttons.
        let on_accent = if tokens.accent.l < 0.5 {
            Hsla::white()
        } else {
            Hsla::black()
        };

        div()
            .id("find-bar")
            .w_full()
            .h(px(36.0))
            .px_3()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .bg(tokens.content_bg)
            .border_b(pixel::PIXEL_BORDER)
            .border_color(tokens.border)
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from("Find")),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .py_1()
                    .rounded(px(0.0))
                    .bg(tokens.hover)
                    .text_sm()
                    .text_color(query_color)
                    .child(query_display)
                    .child(
                        self.query_input_canvas(crate::app_shell::query::QuerySurface::Find, cx),
                    ),
            )
            .child(
                div()
                    .id("find-match-case")
                    .px_2()
                    .py_0p5()
                    .rounded(px(0.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .when(self.find.match_case, |el| el.bg(tokens.accent))
                    .text_xs()
                    .text_color(if self.find.match_case {
                        on_accent
                    } else {
                        tokens.fg_muted
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find.match_case = !this.find.match_case;
                        this.debounce_find(cx);
                    }))
                    .child(SharedString::from("Aa")),
            )
            .child(
                div()
                    .id("find-regex")
                    .px_2()
                    .py_0p5()
                    .rounded(px(0.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .when(self.find.regex, |el| el.bg(tokens.accent))
                    .text_xs()
                    .text_color(if self.find.regex {
                        on_accent
                    } else {
                        tokens.fg_muted
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find.regex = !this.find.regex;
                        this.debounce_find(cx);
                    }))
                    .child(SharedString::from(".*")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .min_w(px(72.0))
                    .child(count),
            )
            .child(
                div()
                    .id("find-prev")
                    .px_2()
                    .py_0p5()
                    .rounded(px(0.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_sm()
                    .text_color(tokens.fg)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.step_find(-1, cx);
                    }))
                    .child(SharedString::from("↑")),
            )
            .child(
                div()
                    .id("find-next")
                    .px_2()
                    .py_0p5()
                    .rounded(px(0.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_sm()
                    .text_color(tokens.fg)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.step_find(1, cx);
                    }))
                    .child(SharedString::from("↓")),
            )
            .child(
                div()
                    .id("find-close")
                    .px_2()
                    .py_0p5()
                    .rounded(px(0.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .text_sm()
                    .text_color(tokens.fg_muted)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.close_find(window, cx);
                    }))
                    .child(SharedString::from("✕")),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::FindState;

    fn find(regex: bool, match_case: bool) -> FindState {
        FindState {
            regex,
            match_case,
            ..FindState::default()
        }
    }

    #[test]
    fn literal_case_insensitive_escapes_and_prefixes_i_flag() {
        // Default mode: metacharacters are escaped and case is ignored.
        assert_eq!(find(false, false).pattern("a.b*"), r"(?i)a\.b\*");
    }

    #[test]
    fn literal_case_sensitive_uses_negative_i_flag() {
        assert_eq!(find(false, true).pattern("a.b"), r"(?-i)a\.b");
    }

    #[test]
    fn regex_mode_passes_pattern_through_unescaped() {
        assert_eq!(find(true, false).pattern("a.b*"), "(?i)a.b*");
        assert_eq!(find(true, true).pattern("a.b*"), "(?-i)a.b*");
    }

    #[test]
    fn teardown_clears_marked_and_bumps_debounce_gen() {
        let mut state = FindState {
            debounce_gen: 41,
            ..FindState::default()
        };
        state.input.marked = Some(0..3);
        state.teardown();
        assert!(state.input.marked.is_none(), "marked must be cleared");
        assert_eq!(state.debounce_gen, 42, "debounce_gen must be bumped by 1");
    }

    #[test]
    fn teardown_wraps_debounce_gen_on_overflow() {
        let mut state = FindState {
            debounce_gen: u64::MAX,
            ..FindState::default()
        };
        state.teardown();
        assert_eq!(state.debounce_gen, 0, "debounce_gen must wrap to 0");
    }
}
