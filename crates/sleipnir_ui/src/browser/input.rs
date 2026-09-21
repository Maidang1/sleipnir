//! Single-line GPUI address editor, including selection, clipboard and IME.
//! Uses GPUI's ElementInputHandler contract (see its input example).
use crate::app_shell::query::{byte_to_utf16, clamped_byte_range, utf16_to_byte};
use crate::chrome::ChromeTokens;
use gpui::{prelude::*, *};
use sleipnir_settings::TerminalPalette;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub(super) enum InputEvent {
    Submit(String),
    Escape,
    FocusPage,
}
impl EventEmitter<InputEvent> for AddressInput {}

pub(super) struct AddressInput {
    pub focus: FocusHandle,
    pub text: String,
    pub dirty: bool,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    line: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    selecting: bool,
}

impl AddressInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            text: String::new(),
            dirty: false,
            anchor: 0,
            cursor: 0,
            marked: None,
            line: None,
            bounds: None,
            scroll_x: px(0.),
            selecting: false,
        }
    }
    pub fn set_url(&mut self, value: String, window: &Window, cx: &mut Context<Self>) {
        if self.dirty && self.focus.is_focused(window) {
            return;
        }
        if self.text != value {
            self.text = value;
            self.anchor = self.text.len();
            self.cursor = self.anchor;
            self.marked = None;
            self.dirty = false;
            cx.notify();
        }
    }
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.anchor = 0;
        self.cursor = self.text.len();
        cx.notify();
    }
    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    fn utf16_at(&self, byte: usize) -> usize {
        byte_to_utf16(&self.text, byte)
    }
    fn byte_range(&self, range: Range<usize>) -> Range<usize> {
        clamped_byte_range(&self.text, range)
    }
    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.text.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.marked = None;
        self.dirty = true;
        cx.notify();
    }
    fn previous(&self) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|i| *i < self.cursor)
            .last()
            .unwrap_or(0)
    }
    fn next(&self) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i > self.cursor)
            .unwrap_or(self.text.len())
    }
    fn index_at(&self, p: Point<Pixels>) -> usize {
        match (&self.line, self.bounds) {
            (Some(line), Some(bounds)) => line
                .closest_index_for_x(p.x - bounds.left() + self.scroll_x)
                .min(self.text.len()),
            _ => 0,
        }
    }
    fn key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let command = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        // While the IME is composing, Enter belongs to the IME, not navigation.
        if self.marked.is_some() && matches!(key, "enter" | "escape") {
            return;
        }
        match key {
            "enter" => cx.emit(InputEvent::Submit(self.text.clone())),
            "escape" => {
                self.dirty = false;
                cx.emit(InputEvent::Escape);
            }
            "a" if command => self.select_all(cx),
            "c" | "x" if command => {
                let range = self.selection();
                if !range.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[range.clone()].to_owned(),
                    ));
                    if key == "x" {
                        self.replace(range, "", cx);
                    }
                }
            }
            "v" if command => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(self.selection(), &text, cx);
                }
            }
            "left" | "right" | "home" | "end" => {
                self.cursor = match key {
                    "home" => 0,
                    "end" => self.text.len(),
                    "left" if command => 0,
                    "right" if command => self.text.len(),
                    "left" if !modifiers.shift && !self.selection().is_empty() => {
                        self.selection().start
                    }
                    "right" if !modifiers.shift && !self.selection().is_empty() => {
                        self.selection().end
                    }
                    "left" => self.previous(),
                    _ => self.next(),
                };
                if !modifiers.shift {
                    self.anchor = self.cursor;
                }
                cx.notify();
            }
            "backspace" | "delete" => {
                let mut range = self.selection();
                if range.is_empty() {
                    if key == "backspace" {
                        range.start = self.previous();
                    } else {
                        range.end = self.next();
                    }
                }
                self.replace(range, "", cx);
            }
            "tab" => cx.emit(InputEvent::FocusPage),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl EntityInputHandler for AddressInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.byte_range(range);
        *actual = Some(self.utf16_at(range.start)..self.utf16_at(range.end));
        Some(self.text[range].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.selection();
        Some(UTF16Selection {
            range: self.utf16_at(range.start)..self.utf16_at(range.end),
            reversed: self.cursor < self.anchor,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| self.utf16_at(r.start)..self.utf16_at(r.end))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.byte_range(r))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        self.replace(range, text, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .map(|r| self.byte_range(r))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let start = range.start;
        self.replace(range, text, cx);
        let end = self.cursor;
        self.marked = (end > start).then_some(start..end);
        if let Some(selection) = selection {
            let inserted = &self.text[start..end];
            self.anchor = start + utf16_to_byte(inserted, selection.start);
            self.cursor = start + utf16_to_byte(inserted, selection.end);
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.byte_range(range);
        let line = self.line.as_ref()?;
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start) - self.scroll_x,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(range.end) - self.scroll_x,
                bounds.bottom(),
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16_at(self.index_at(p)))
    }
}

impl Render for AddressInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens =
            ChromeTokens::from_palette(&TerminalPalette::get_global(cx), window.is_window_active());
        div()
            .id("browser-address")
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .bg(tokens.content_bg)
            .border_1()
            .border_color(if self.focus.is_focused(window) {
                tokens.accent
            } else {
                tokens.border
            })
            .text_size(px(12.))
            .line_height(px(20.))
            .text_color(tokens.fg)
            .overflow_hidden()
            .track_focus(&self.focus)
            .key_context("BrowserAddress")
            .cursor_text()
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let was_focused = this.focus.is_focused(window);
                    window.focus(&this.focus, cx);
                    if !was_focused || event.click_count >= 2 {
                        this.select_all(cx);
                        this.selecting = false;
                    } else {
                        this.cursor = this.index_at(event.position);
                        if !event.modifiers.shift {
                            this.anchor = this.cursor;
                        }
                        this.selecting = true;
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.selecting {
                    this.cursor = this.index_at(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .child(AddressText {
                input: cx.entity(),
                tokens,
            })
    }
}

struct AddressText {
    input: Entity<AddressInput>,
    tokens: ChromeTokens,
}
struct PaintState {
    line: ShapedLine,
    scroll: Pixels,
}
impl IntoElement for AddressText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for AddressText {
    type RequestLayoutState = ();
    type PrepaintState = PaintState;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(20.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PaintState {
        let input = self.input.read(cx);
        let text: SharedString = if input.text.is_empty() {
            "URL / localhost:3000".into()
        } else {
            input.text.clone().into()
        };
        let style = window.text_style();
        let line = window.text_system().shape_line(
            text.clone(),
            px(12.),
            &[TextRun {
                len: text.len(),
                font: style.font(),
                color: if input.text.is_empty() {
                    self.tokens.fg_muted
                } else {
                    self.tokens.fg
                },
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let caret = line.x_for_index(input.cursor);
        let width = (bounds.size.width - px(4.)).max(px(1.));
        let scroll = if caret < input.scroll_x {
            caret
        } else if caret > input.scroll_x + width {
            caret - width
        } else {
            input.scroll_x
        };
        PaintState {
            line,
            scroll: scroll.max(px(0.)),
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let input = self.input.read(cx);
        let focus = input.focus.clone();
        let selected = input.selection();
        let cursor = input.cursor;
        let marked = input.marked.clone();
        let origin = point(bounds.left() - state.scroll, bounds.top());
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focus.is_focused(window) && !selected.is_empty() {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(
                            origin.x + state.line.x_for_index(selected.start),
                            bounds.top(),
                        ),
                        point(
                            origin.x + state.line.x_for_index(selected.end),
                            bounds.bottom(),
                        ),
                    ),
                    self.tokens.accent.opacity(0.25),
                ));
            }
            if let Err(error) = state
                .line
                .paint(origin, px(20.), TextAlign::Left, None, window, cx)
            {
                log::warn!("browser address paint: {error}");
            }
            if focus.is_focused(window) && selected.is_empty() {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + state.line.x_for_index(cursor), bounds.top()),
                        size(px(1.), bounds.size.height),
                    ),
                    self.tokens.accent,
                ));
            }
            if let Some(marked) = marked {
                window.paint_quad(fill(
                    Bounds::new(
                        point(
                            origin.x + state.line.x_for_index(marked.start),
                            bounds.bottom() - px(1.),
                        ),
                        size(
                            state.line.x_for_index(marked.end)
                                - state.line.x_for_index(marked.start),
                            px(1.),
                        ),
                    ),
                    self.tokens.accent,
                ));
            }
        });
        self.input.update(cx, |input, _| {
            input.line = Some(state.line.clone());
            input.bounds = Some(bounds);
            input.scroll_x = state.scroll;
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::app_shell::query::{byte_to_utf16, clamped_byte_range, utf16_to_byte};

    #[test]
    fn browser_address_utf16_boundaries_do_not_split_unicode() {
        let value = "a😀中";
        assert_eq!(utf16_to_byte(value, 0), 0);
        assert_eq!(utf16_to_byte(value, 1), 1);
        assert_eq!(utf16_to_byte(value, 3), 5);
        assert_eq!(utf16_to_byte(value, 4), value.len());
        assert_eq!(utf16_to_byte(value, 999), value.len());
    }

    #[test]
    fn browser_address_utf16_roundtrip_and_clamped_ranges() {
        let value = "a😀中";
        // Every UTF-16 offset resolves to a char boundary, and mapping back
        // lands on the start of the char that contains the offset.
        for utf16 in 0..=byte_to_utf16(value, value.len()) {
            let byte = utf16_to_byte(value, utf16);
            assert!(value.is_char_boundary(byte), "utf16 {utf16} -> byte {byte}");
            assert!(byte_to_utf16(value, byte) >= utf16);
        }
        // Offsets that already sit on a char boundary round-trip exactly.
        for utf16 in [0, 1, 3] {
            assert_eq!(byte_to_utf16(value, utf16_to_byte(value, utf16)), utf16);
        }
        // Inverted and out-of-range IME ranges clamp instead of panicking.
        // The inverted range is deliberately malformed input, built with the
        // struct literal so clippy's reversed_empty_ranges lint stays quiet.
        assert_eq!(
            clamped_byte_range(value, std::ops::Range { start: 9, end: 1 }),
            8..8
        );
        assert_eq!(clamped_byte_range(value, 1..999), 1..value.len());
        // Off-boundary byte offsets snap back to the containing char's start.
        assert_eq!(byte_to_utf16(value, 4), 1);
    }
}
