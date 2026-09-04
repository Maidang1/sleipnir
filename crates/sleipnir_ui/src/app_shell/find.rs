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

impl AppShell {
    pub(super) fn on_find(&mut self, _: &Find, _window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(cx);
    }

    pub(crate) fn open_find(&mut self, cx: &mut Context<Self>) {
        self.mode.open_find();
        cx.notify();
        // Re-run search if query already present.
        if !self.find_query.is_empty() {
            self.run_find(cx);
        }
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode.find_open {
            self.mode.close_find();
            self.find_debounce_gen = self.find_debounce_gen.wrapping_add(1);
            self.clear_find_matches(cx);
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    fn clear_find_matches(&mut self, cx: &mut Context<Self>) {
        // Invalidate searches that are still running so they cannot repopulate
        // matches after the query is cleared or the find bar is closed.
        self.find_gen = self.find_gen.wrapping_add(1);
        if let Some(view) = self.active_view(cx) {
            if let Some(term) = view.read(cx).terminal_entity().cloned() {
                term.update(cx, |t, _| t.matches.clear());
            }
        }
        self.find_match_count = 0;
        self.find_active_index = 0;
    }

    /// Resolve the raw query into the regex handed to alacritty's search.
    ///
    /// Literal mode escapes regex metacharacters; regex mode passes the query
    /// through unchanged. Case is controlled with the `(?i)` / `(?-i)` inline
    /// flags, which alacritty's regex engine honours regardless of its
    /// smart-case default.
    fn find_pattern(&self, query: &str) -> String {
        let pattern = if self.find_regex {
            query.to_owned()
        } else {
            regex_escape_literal(query)
        };
        if self.find_match_case {
            format!("(?-i){pattern}")
        } else {
            format!("(?i){pattern}")
        }
    }

    fn debounce_find(&mut self, cx: &mut Context<Self>) {
        self.find_debounce_gen = self.find_debounce_gen.wrapping_add(1);
        let generation = self.find_debounce_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            this.update(cx, |this, cx| {
                if this.find_debounce_gen == generation && this.mode.find_open {
                    this.run_find(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn run_find(&mut self, cx: &mut Context<Self>) {
        // An immediate search (for example Enter) supersedes pending debounce timers.
        self.find_debounce_gen = self.find_debounce_gen.wrapping_add(1);
        let query = self.find_query.clone();
        if query.is_empty() {
            self.clear_find_matches(cx);
            cx.notify();
            return;
        }
        let Some(search) = terminal::Search::new(&self.find_pattern(&query)) else {
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
        self.find_gen = self.find_gen.wrapping_add(1);
        let generation = self.find_gen;
        let task = term.update(cx, |t, cx| t.find_matches(search, cx));
        cx.spawn(async move |this, cx| {
            let matches = task.await;
            this.update(cx, |this, cx| {
                // Search completion is asynchronous. Only the newest request
                // for the pane that is still active may update UI state.
                if this.find_gen != generation
                    || !this.mode.find_open
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
                this.find_match_count = count;
                this.find_active_index = 0;
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
        if self.find_match_count == 0 {
            if self.mode.find_open && !self.find_query.is_empty() {
                self.run_find(cx);
            }
            return;
        }
        let n = self.find_match_count as i32;
        let next = (self.find_active_index as i32 + delta).rem_euclid(n) as usize;
        self.find_active_index = next;
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
        if !self.mode.find_open {
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
                    if self.find_match_count == 0 || event.keystroke.modifiers.platform {
                        self.run_find(cx);
                    } else {
                        self.step_find(1, cx);
                    }
                }
                true
            }
            "backspace" => {
                self.find_query.pop();
                self.debounce_find(cx);
                true
            }
            // ⌥⌘C toggles match-case; ⌥⌘R toggles regex (macOS find-bar convention).
            "c" if event.keystroke.modifiers.alt && event.keystroke.modifiers.platform => {
                self.find_match_case = !self.find_match_case;
                self.debounce_find(cx);
                true
            }
            "r" if event.keystroke.modifiers.alt && event.keystroke.modifiers.platform => {
                self.find_regex = !self.find_regex;
                self.debounce_find(cx);
                true
            }
            "v" if event.keystroke.modifiers.platform && !event.keystroke.modifiers.alt => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.find_query.push_str(&text.replace(['\n', '\r'], ""));
                    self.debounce_find(cx);
                }
                true
            }
            _ if event.keystroke.modifiers.platform => true,
            _ => {
                if let Some(ch) = event.keystroke.key_char.as_ref() {
                    if !ch.is_empty() && !ch.chars().any(|c| c.is_control()) {
                        self.find_query.push_str(ch);
                        self.debounce_find(cx);
                    }
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
        let query_display: SharedString = if self.find_query.is_empty() {
            "Find in scrollback…".into()
        } else {
            format!("{}|", self.find_query).into()
        };
        let query_color = if self.find_query.is_empty() {
            tokens.fg_muted
        } else {
            tokens.fg
        };
        let count: SharedString = if self.find_query.is_empty() {
            "".into()
        } else if self.find_match_count == 0 {
            "0 matches".into()
        } else {
            format!("{}/{}", self.find_active_index + 1, self.find_match_count).into()
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
            .border_b_1()
            .border_color(tokens.border)
            .child(
                div()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from("Find")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(tokens.hover)
                    .text_sm()
                    .text_color(query_color)
                    .child(query_display),
            )
            .child(
                div()
                    .id("find-match-case")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .when(self.find_match_case, |el| el.bg(tokens.accent))
                    .text_xs()
                    .text_color(if self.find_match_case {
                        on_accent
                    } else {
                        tokens.fg_muted
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find_match_case = !this.find_match_case;
                        this.debounce_find(cx);
                    }))
                    .child(SharedString::from("Aa")),
            )
            .child(
                div()
                    .id("find-regex")
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .cursor_pointer()
                    .hover(|el| el.bg(tokens.hover))
                    .when(self.find_regex, |el| el.bg(tokens.accent))
                    .text_xs()
                    .text_color(if self.find_regex {
                        on_accent
                    } else {
                        tokens.fg_muted
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.find_regex = !this.find_regex;
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
                    .rounded(px(4.0))
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
                    .rounded(px(4.0))
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
                    .rounded(px(4.0))
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

/// Escape a literal string for use inside a regex (alacritty search is regex-based).
fn regex_escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
