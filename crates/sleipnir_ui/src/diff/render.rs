//! Diff inspector overlay. Full-content, occludes the terminal.

use std::ops::Range;

use gpui::{
    Bounds, Context, Entity, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText,
    Window, canvas, deferred, div, fill, list, point, prelude::FluentBuilder as _, px, size,
};
use sleipnir_settings::TerminalPalette;

use super::{Cell, DiffView, DisplayRow, LineKind, TreeEntry, file_index_at, row_height};
use crate::app_shell::AppShell;
use crate::chrome::ChromeTokens;
use diff_core::FileStatus;

impl AppShell {
    pub(crate) fn render_diff_overlay(
        &self,
        tokens: &ChromeTokens,
        palette: &TerminalPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let font_size = self
            .font_size_override
            .or(sleipnir_settings::TerminalSettings::get_global(cx).font_size)
            .unwrap_or(px(14.0));
        let family: SharedString = sleipnir_settings::TerminalSettings::get_global(cx)
            .font_family
            .clone()
            .unwrap_or_else(|| sleipnir_settings::default_font_family().into())
            .into();
        let title: SharedString = match self.diff_view.as_ref() {
            Some(DiffView::Loading { title, .. }) => title.clone().into(),
            Some(DiffView::Ready(session)) => session.title.clone().into(),
            Some(DiffView::Message { title, .. }) => title.clone().into(),
            None => "Diff".into(),
        };
        let stats: SharedString = match self.diff_view.as_ref() {
            Some(DiffView::Ready(s)) => format!("+{} −{}", s.additions, s.deletions).into(),
            _ => "".into(),
        };

        let header =
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(tokens.border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.fg)
                        .child(title),
                )
                .child(div().text_xs().text_color(palette.ansi[2]).child(stats))
                .child(div().flex_1())
                .child(self.render_mode_chip(tokens, cx))
                .child(self.render_minimap_chip(tokens, cx))
                .child(header_chip("diff-refresh", "Refresh", tokens).on_click(
                    cx.listener(|this, _, window, cx| this.refresh_diff(true, window, cx)),
                ))
                .child(
                    header_chip("diff-send", "Send to pane", tokens)
                        .on_click(cx.listener(|this, _, _, cx| this.send_open_diff_to_pty(cx))),
                )
                .child(
                    header_chip("diff-close", "Close", tokens)
                        .on_click(cx.listener(|this, _, window, cx| this.close_diff(window, cx))),
                );

        let body = match self.diff_view.as_ref() {
            None | Some(DiffView::Loading { .. }) => centered(tokens, "Loading diff…"),
            Some(DiffView::Message { body, .. }) => centered(tokens, body.clone()),
            Some(DiffView::Ready(session)) if session.rows.is_empty() => {
                centered(tokens, "Working tree clean")
            }
            Some(DiffView::Ready(session)) => {
                let scroll = session.scroll.clone();
                let row_h = row_height(font_size);
                let list_palette = palette.clone();
                let list_tokens = tokens.clone();
                let list_entity = entity;
                let rows = list(scroll, move |ix, _window, cx| {
                    let this = list_entity.read(cx);
                    let Some(DiffView::Ready(session)) = this.diff_view.as_ref() else {
                        return div().into_any_element();
                    };
                    session
                        .rows
                        .get(ix)
                        .map(|row| {
                            render_row(
                                ix,
                                row,
                                ix == session.cursor,
                                row_h,
                                &list_tokens,
                                &list_palette,
                                &list_entity,
                            )
                        })
                        .unwrap_or_else(|| div().into_any_element())
                })
                .h_full()
                .flex_1()
                .min_w_0()
                .min_h_0();
                div()
                    .id("diff-ready-body")
                    .flex()
                    .flex_row()
                    .size_full()
                    .min_h_0()
                    .child(self.render_diff_tree(tokens, palette, cx))
                    .child(rows)
                    .when(session.minimap_visible, |el| {
                        el.child(self.render_diff_minimap(tokens, palette, cx))
                    })
                    .into_any_element()
            }
        };

        deferred(
            div()
                .id("diff-overlay")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("diff-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(gpui::Hsla::black().opacity(0.45))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(|this, _, window, cx| {
                                this.close_diff(window, cx);
                            }),
                        ),
                )
                .child(
                    div()
                        .id("diff-panel")
                        .w(gpui::relative(0.92))
                        .h(gpui::relative(0.88))
                        .flex()
                        .flex_col()
                        .bg(tokens.content_bg)
                        .border_1()
                        .border_color(tokens.border)
                        .rounded(px(10.0))
                        .overflow_hidden()
                        .font_family(family)
                        .text_size(font_size)
                        .text_color(tokens.fg)
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(header)
                        .child(
                            div()
                                .id("diff-body")
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(body),
                        ),
                ),
        )
    }

    fn render_minimap_chip(
        &self,
        tokens: &ChromeTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = matches!(
            self.diff_view.as_ref(),
            Some(DiffView::Ready(session)) if session.minimap_visible
        );
        header_chip(
            "diff-minimap",
            if on { "Minimap" } else { "Minimap off" },
            tokens,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_diff_minimap(cx)))
    }

    fn render_diff_minimap(
        &self,
        tokens: &ChromeTokens,
        palette: &TerminalPalette,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(DiffView::Ready(session)) = self.diff_view.as_ref() else {
            return div().into_any_element();
        };
        let mini = crate::diff::minimap::minimap_rows(&session.rows);
        let tokens = tokens.clone();
        let palette = palette.clone();
        div()
            .id("diff-minimap")
            .w(px(72.0))
            .h_full()
            .flex_shrink_0()
            .bg(tokens.surface)
            .border_l_1()
            .border_color(tokens.border)
            .child(
                canvas(
                    move |bounds, _, _| {
                        crate::diff::minimap::minimap_runs(&mini, f32::from(bounds.size.height))
                    },
                    move |bounds, layout, window, _cx| {
                        let x0 = f32::from(bounds.left());
                        let y0 = f32::from(bounds.top());
                        let usable = f32::from(bounds.size.width) - 6.0;
                        let half = (usable - 2.0) / 2.0;
                        for run in &layout.runs {
                            let (x, w) = match run.lane {
                                crate::diff::minimap::MinimapLane::Full => (0.0, usable * run.frac),
                                crate::diff::minimap::MinimapLane::Left => (0.0, half * run.frac),
                                crate::diff::minimap::MinimapLane::Right => {
                                    (half + 2.0, half * run.frac)
                                }
                            };
                            let y = run.start as f32 * layout.slot_h;
                            let h = if run.tick {
                                1.0
                            } else {
                                (run.end - run.start) as f32 * layout.slot_h
                            };
                            let color = match run.color {
                                crate::diff::minimap::MinimapColor::Added => {
                                    palette.ansi[2].opacity(0.85)
                                }
                                crate::diff::minimap::MinimapColor::Removed => {
                                    palette.ansi[1].opacity(0.85)
                                }
                                crate::diff::minimap::MinimapColor::Context => {
                                    tokens.fg_muted.opacity(0.35)
                                }
                                crate::diff::minimap::MinimapColor::Header => {
                                    tokens.accent.opacity(0.55)
                                }
                                crate::diff::minimap::MinimapColor::Gap => {
                                    tokens.fg_muted.opacity(0.2)
                                }
                            };
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(px(x0 + 3.0 + x), px(y0 + y)),
                                    size(px(w.max(1.0)), px(h.max(1.0))),
                                ),
                                color,
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn render_mode_chip(&self, tokens: &ChromeTokens, cx: &mut Context<Self>) -> impl IntoElement {
        let label = match self.diff_view.as_ref() {
            Some(DiffView::Ready(session)) => session.mode.label(),
            _ => "Split",
        };
        header_chip("diff-mode", label, tokens)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_diff_mode(cx)))
    }

    fn render_diff_tree(
        &self,
        tokens: &ChromeTokens,
        palette: &TerminalPalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(DiffView::Ready(session)) = self.diff_view.as_ref() else {
            return div().into_any_element();
        };
        let active = file_index_at(&session.file_rows, session.cursor);
        let mut list = div()
            .id("diff-tree-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py_1();
        for (ix, entry) in session.tree.iter().enumerate() {
            list = list.child(render_tree_row(
                ix,
                entry,
                active == Some(ix),
                tokens,
                palette,
                cx,
            ));
        }
        div()
            .id("diff-tree")
            .w(px(220.0))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(tokens.surface)
            .border_r_1()
            .border_color(tokens.border)
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from(format!(
                        "Files · {}",
                        session.tree.len()
                    ))),
            )
            .child(list)
            .into_any_element()
    }
}

fn header_chip(
    id: &'static str,
    label: &'static str,
    tokens: &ChromeTokens,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded(px(4.0))
        .text_xs()
        .text_color(tokens.fg_muted)
        .cursor_pointer()
        .hover(|el| el.bg(tokens.hover).text_color(tokens.fg))
        .child(SharedString::from(label))
}

fn centered(tokens: &ChromeTokens, text: impl Into<SharedString>) -> gpui::AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(tokens.fg_muted)
        .child(text.into())
        .into_any_element()
}

fn render_row(
    ix: usize,
    row: &DisplayRow,
    selected: bool,
    row_h: gpui::Pixels,
    tokens: &ChromeTokens,
    palette: &TerminalPalette,
    entity: &Entity<AppShell>,
) -> gpui::AnyElement {
    let base = div()
        .id(("diff-row", ix))
        .w_full()
        .min_h(row_h)
        .flex()
        .items_start()
        .when(selected, |el| el.bg(tokens.hover));
    match row {
        DisplayRow::FileHeader {
            path,
            status,
            additions,
            deletions,
        } => {
            let (label, color) = status_style(*status, palette);
            base.px_3()
                .bg(tokens.surface)
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(SharedString::from(label)),
                )
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(tokens.fg)
                        .child(SharedString::from(path.clone())),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.ansi[2])
                        .child(SharedString::from(format!("+{additions}"))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.ansi[1])
                        .child(SharedString::from(format!("−{deletions}"))),
                )
                .into_any_element()
        }
        DisplayRow::HunkHeader { label } => base
            .px_3()
            .bg(tokens.surface)
            .text_color(tokens.fg_muted)
            .child(SharedString::from(label.clone()))
            .into_any_element(),
        DisplayRow::Binary => base
            .px_3()
            .text_color(tokens.fg_muted)
            .child(SharedString::from("binary file changed"))
            .into_any_element(),
        DisplayRow::Gap {
            file_ix,
            gap_ix,
            hidden,
        } => {
            let (file_ix, gap_ix) = (*file_ix, *gap_ix);
            let entity = entity.clone();
            let noun = if *hidden == 1 { "line" } else { "lines" };
            div()
                .id(("diff-gap", (file_ix << 16) | gap_ix))
                .w_full()
                .min_h(row_h)
                .flex()
                .items_center()
                .justify_center()
                .bg(tokens.surface)
                .hover(|style| style.bg(tokens.hover))
                .cursor_pointer()
                .text_color(tokens.fg_muted)
                .child(SharedString::from(format!("⋯ {hidden} hidden {noun}")))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.expand_diff_gap(file_ix, gap_ix, cx));
                })
                .into_any_element()
        }
        DisplayRow::SplitLine { left, right } => div()
            .id(("diff-row", ix))
            .w_full()
            .min_h(row_h)
            .flex()
            .when(selected, |el| el.bg(tokens.hover))
            .child(render_split_cell(left.as_ref(), tokens, palette))
            .child(
                div()
                    .w(px(6.0))
                    .flex_shrink_0()
                    .bg(tokens.surface)
                    .border_l_1()
                    .border_r_1()
                    .border_color(tokens.border),
            )
            .child(render_split_cell(right.as_ref(), tokens, palette))
            .into_any_element(),
        DisplayRow::Line {
            old_no,
            new_no,
            kind,
            text,
            intra,
            syntax,
        } => {
            let (row_bg, word_bg, marker, marker_color) = kind_style(*kind, palette);
            let mut line = base;
            if let Some(bg) = row_bg {
                line = line.bg(bg);
            }
            line.child(gutter_no(*old_no, tokens))
                .child(gutter_no(*new_no, tokens))
                .child(
                    div()
                        .w(px(28.0))
                        .flex_shrink_0()
                        .flex()
                        .justify_center()
                        .text_color(marker_color)
                        .child(SharedString::from(marker)),
                )
                .child(wrapped_line_text(
                    text, syntax, intra, word_bg, palette, tokens,
                ))
                .into_any_element()
        }
    }
}

fn render_split_cell(
    cell: Option<&Cell>,
    tokens: &ChromeTokens,
    palette: &TerminalPalette,
) -> impl IntoElement {
    let base = div().flex_1().min_w_0().flex().items_start();
    let Some(cell) = cell else {
        return base.bg(void_cell_bg(tokens));
    };
    let (row_bg, word_bg, marker, marker_color) = kind_style(cell.kind, palette);
    let mut side = base;
    if let Some(bg) = row_bg {
        side = side.bg(bg);
    }
    side.child(gutter_no(Some(cell.no), tokens))
        .child(
            div()
                .w(px(28.0))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .text_color(marker_color)
                .child(SharedString::from(marker)),
        )
        .child(wrapped_line_text(
            &cell.text,
            &cell.syntax,
            &cell.intra,
            word_bg,
            palette,
            tokens,
        ))
}

fn wrapped_line_text(
    text: &str,
    syntax: &[(Range<usize>, syntax::Token)],
    intra: &[Range<usize>],
    word_bg: Option<gpui::Hsla>,
    palette: &TerminalPalette,
    tokens: &ChromeTokens,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .pr_2()
        .whitespace_normal()
        .child(line_content(text, syntax, intra, word_bg, palette, tokens))
}

fn void_cell_bg(tokens: &ChromeTokens) -> gpui::Hsla {
    tokens.content_bg.blend(gpui::Hsla::black().opacity(0.28))
}

fn render_tree_row(
    ix: usize,
    entry: &TreeEntry,
    active: bool,
    tokens: &ChromeTokens,
    palette: &TerminalPalette,
    cx: &mut gpui::Context<AppShell>,
) -> impl IntoElement {
    let (status_label, status_color) = status_style(entry.status, palette);
    let row = entry.row;
    let name = entry
        .path
        .rsplit('/')
        .next()
        .unwrap_or(entry.path.as_str())
        .to_string();
    let dir = entry
        .path
        .rsplit_once('/')
        .map(|(prefix, _)| prefix.to_string());
    div()
        .id(("diff-tree-row", ix))
        .px_2()
        .py_1()
        .cursor_pointer()
        .when(active, |el| el.bg(tokens.hover))
        .hover(|el| el.bg(tokens.hover))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.jump_diff_file(row, cx);
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .w(px(12.0))
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(status_color)
                        .child(SharedString::from(status_mark(status_label))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(tokens.fg)
                        .child(SharedString::from(name)),
                )
                .when(entry.additions > 0, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(palette.ansi[2])
                            .child(SharedString::from(format!("+{}", entry.additions))),
                    )
                })
                .when(entry.deletions > 0, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(palette.ansi[1])
                            .child(SharedString::from(format!("−{}", entry.deletions))),
                    )
                }),
        )
        .when_some(dir, |el, prefix| {
            el.child(
                div()
                    .pl(px(16.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_xs()
                    .text_color(tokens.fg_muted)
                    .child(SharedString::from(prefix)),
            )
        })
}

fn status_mark(label: &str) -> &'static str {
    match label {
        "added" => "A",
        "deleted" => "D",
        "modified" => "M",
        "renamed" => "R",
        "binary" => "B",
        _ => "?",
    }
}

fn gutter_no(no: Option<u32>, tokens: &ChromeTokens) -> impl IntoElement {
    div()
        .w(px(44.0))
        .flex_shrink_0()
        .text_color(tokens.fg_muted)
        .flex()
        .justify_end()
        .pr_1()
        .child(SharedString::from(
            no.map(|n| n.to_string()).unwrap_or_default(),
        ))
}

fn kind_style(
    kind: LineKind,
    palette: &TerminalPalette,
) -> (
    Option<gpui::Hsla>,
    Option<gpui::Hsla>,
    &'static str,
    gpui::Hsla,
) {
    match kind {
        LineKind::Context => (None, None, "", palette.ansi[8]),
        LineKind::Added => (
            Some(palette.ansi[2].opacity(0.12)),
            Some(palette.ansi[2].opacity(0.28)),
            "+",
            palette.ansi[2],
        ),
        LineKind::Removed => (
            Some(palette.ansi[1].opacity(0.12)),
            Some(palette.ansi[1].opacity(0.28)),
            "−",
            palette.ansi[1],
        ),
    }
}

fn status_style(status: FileStatus, palette: &TerminalPalette) -> (&'static str, gpui::Hsla) {
    match status {
        FileStatus::Added => ("added", palette.ansi[2]),
        FileStatus::Deleted => ("deleted", palette.ansi[1]),
        FileStatus::Modified => ("modified", palette.ansi[3]),
        FileStatus::Renamed => ("renamed", palette.ansi[4]),
        FileStatus::Binary => ("binary", palette.ansi[5]),
    }
}

fn line_content(
    text: &str,
    syntax: &[(Range<usize>, syntax::Token)],
    intra: &[Range<usize>],
    word_bg: Option<gpui::Hsla>,
    palette: &TerminalPalette,
    tokens: &ChromeTokens,
) -> gpui::AnyElement {
    let highlights = merge_highlights(syntax, intra, word_bg, palette, tokens);
    if highlights.is_empty() {
        return div()
            .child(SharedString::from(text.to_string()))
            .into_any_element();
    }
    StyledText::new(SharedString::from(text.to_string()))
        .with_highlights(highlights)
        .into_any_element()
}

fn merge_highlights(
    syntax: &[(Range<usize>, syntax::Token)],
    intra: &[Range<usize>],
    word_bg: Option<gpui::Hsla>,
    palette: &TerminalPalette,
    tokens: &ChromeTokens,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut bounds = Vec::with_capacity(2 * (syntax.len() + intra.len() + 1));
    for (range, _) in syntax {
        bounds.push(range.start);
        bounds.push(range.end);
    }
    for range in intra {
        bounds.push(range.start);
        bounds.push(range.end);
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let (mut si, mut ii) = (0, 0);
    for seg in bounds.windows(2) {
        let (start, end) = (seg[0], seg[1]);
        while si < syntax.len() && syntax[si].0.end <= start {
            si += 1;
        }
        while ii < intra.len() && intra[ii].end <= start {
            ii += 1;
        }
        let token = (si < syntax.len() && syntax[si].0.start <= start).then(|| syntax[si].1);
        let in_intra = ii < intra.len() && intra[ii].start <= start;
        if token.is_none() && !in_intra {
            continue;
        }
        let mut style = HighlightStyle {
            color: token.map(|t| token_color(t, palette, tokens)),
            ..Default::default()
        };
        if in_intra {
            style.background_color = word_bg;
        }
        match out.last_mut() {
            Some((prev, prev_style)) if prev.end == start && *prev_style == style => prev.end = end,
            _ => out.push((start..end, style)),
        }
    }
    out
}

fn token_color(
    token: syntax::Token,
    palette: &TerminalPalette,
    tokens: &ChromeTokens,
) -> gpui::Hsla {
    match token {
        syntax::Token::Keyword => palette.ansi[5],
        syntax::Token::Function => palette.ansi[4],
        syntax::Token::Type => palette.ansi[3],
        syntax::Token::String => palette.ansi[2],
        syntax::Token::Number | syntax::Token::Constant => palette.ansi[3],
        syntax::Token::Comment => tokens.fg_muted,
        syntax::Token::Property => palette.ansi[6],
        syntax::Token::Operator => palette.ansi[6],
        syntax::Token::Punctuation => tokens.fg_muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sleipnir_settings::{Appearance, ThemeName, palette_for_theme};

    fn palette_and_tokens() -> (TerminalPalette, ChromeTokens) {
        let palette = palette_for_theme(ThemeName::Mocha, Appearance::Dark);
        let tokens = ChromeTokens::from_palette(&palette, true);
        (palette, tokens)
    }

    #[test]
    fn merge_highlights_empty_syntax_does_not_panic() {
        let (palette, tokens) = palette_and_tokens();
        let out = merge_highlights(&[], &[4..7], Some(palette.ansi[2]), &palette, &tokens);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 4..7);
    }

    #[test]
    fn merge_highlights_empty_inputs_are_empty() {
        let (palette, tokens) = palette_and_tokens();
        assert!(merge_highlights(&[], &[], None, &palette, &tokens).is_empty());
    }
}
