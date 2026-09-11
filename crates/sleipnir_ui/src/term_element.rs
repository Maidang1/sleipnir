//! Terminal grid painter + input (M2).

use crate::cursor_blink_alpha;
use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, InputHandler, InteractiveElement, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point as GpuiPoint, ScrollWheelEvent,
    StatefulInteractiveElement, StrikethroughStyle, TextRun, TextStyle, UTF16Selection,
    UnderlineStyle, Window, fill, point, px, relative, size,
};
use itertools::Itertools;
use row_geometry::{HitTarget, RowGeometry};
use sleipnir_settings::{TerminalBlink, TerminalPalette, TerminalSettings, get_color_at_index};
use std::ops::Range as StdRange;
use std::time::{Duration, Instant};
use terminal::{
    Cell, Color, CursorShape, GutterKind, IndexedCell, Modes, NamedColor, Range as TerminalRange,
    Rgb, Terminal, TerminalBounds, absolute_to_display_line, is_default_background_color,
    viewport_top_abs, y_for_display,
};

pub struct TermElement {
    terminal: Entity<Terminal>,
    view: Entity<crate::TermView>,
    focus: FocusHandle,
    focused: bool,
    /// Window-scoped zoom override; `None` uses settings font size.
    font_size_override: Option<Pixels>,
    /// Last input time for blink solid window (M11).
    last_input_at: Instant,
    /// App-reported blink preference (M11).
    terminal_wants_blink: bool,
    starfield_time: Duration,
    interactivity: gpui::Interactivity,
}

impl TermElement {
    pub fn new(
        terminal: Entity<Terminal>,
        view: Entity<crate::TermView>,
        focus: FocusHandle,
        focused: bool,
        font_size_override: Option<Pixels>,
        last_input_at: Instant,
        terminal_wants_blink: bool,
    ) -> Self {
        Self {
            terminal,
            view,
            focus: focus.clone(),
            focused,
            font_size_override,
            last_input_at,
            terminal_wants_blink,
            starfield_time: Duration::ZERO,
            interactivity: Default::default(),
        }
        .track_focus(&focus)
    }

    pub(crate) fn with_starfield_time(mut self, elapsed: Duration) -> Self {
        self.starfield_time = elapsed;
        self
    }
}

impl InteractiveElement for TermElement {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        &mut self.interactivity
    }
}

impl StatefulInteractiveElement for TermElement {}

#[derive(Clone, Copy)]
struct LayoutPoint {
    line: i32,
    column: i32,
}

struct BatchedTextRun {
    start: LayoutPoint,
    text: String,
    cell_count: usize,
    column_span: usize,
    style: TextRun,
    font_size: Pixels,
}

impl BatchedTextRun {
    fn can_append(&self, other: &TextRun) -> bool {
        self.style.font == other.font
            && self.style.color == other.color
            && self.style.background_color == other.background_color
            && self.style.underline == other.underline
            && self.style.strikethrough == other.strikethrough
    }

    fn force_width(&self, cell_width: Pixels) -> Option<Pixels> {
        if cell_width > px(0.) && self.column_span > 0 {
            Some(cell_width * self.column_span as f32)
        } else {
            None
        }
    }

    fn end_column(&self) -> i32 {
        self.start.column + (self.cell_count * self.column_span) as i32
    }

    fn can_append_run(&self, point: LayoutPoint, style: &TextRun, column_span: usize) -> bool {
        self.can_append(style)
            && self.start.line == point.line
            && self.column_span == column_span
            && self.end_column() == point.column
    }

    fn append_cell_text(&mut self, text: &str) {
        self.text.push_str(text);
        self.cell_count += 1;
        self.style.len += text.len();
    }

    fn paint(
        &self,
        origin: GpuiPoint<Pixels>,
        dimensions: &TerminalBounds,
        map: &PaintMap,
        window: &mut Window,
        cx: &mut App,
    ) {
        let pos = GpuiPoint::new(
            origin.x + self.start.column as f32 * dimensions.cell_width,
            map.y(origin, self.start.line),
        );
        if let Err(err) = window
            .text_system()
            .shape_line(
                self.text.clone().into(),
                self.font_size,
                std::slice::from_ref(&self.style),
                self.force_width(dimensions.cell_width),
            )
            .paint(
                pos,
                dimensions.line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
        {
            log::error!("terminal text paint failed: {err:?}");
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TerminalCellTextStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    undercurl: bool,
    strikeout: bool,
}

impl TerminalCellTextStyle {
    fn from_cell(cell: &Cell) -> Self {
        Self {
            bold: cell.is_bold(),
            italic: cell.is_italic(),
            underline: cell.has_underline(),
            undercurl: cell.has_undercurl(),
            strikeout: cell.has_strikeout(),
        }
    }
}

fn compose_cell_text(base: char, zerowidth: Option<&[char]>) -> String {
    let mut text = String::new();
    text.push(base);
    if let Some(zerowidth) = zerowidth {
        text.extend(zerowidth.iter().copied());
    }
    text
}

fn cell_column_span(next_is_wide_spacer: bool) -> usize {
    if next_is_wide_spacer { 2 } else { 1 }
}

fn build_cell_text_run(
    text: String,
    columns: usize,
    text_style: &TextStyle,
    color: gpui::Hsla,
    cell_style: TerminalCellTextStyle,
) -> BatchedTextRun {
    let mut font = text_style.font();
    if cell_style.bold {
        font.weight = gpui::FontWeight::BOLD;
    }
    if cell_style.italic {
        font.style = gpui::FontStyle::Italic;
    }

    BatchedTextRun {
        start: LayoutPoint { line: 0, column: 0 },
        text: text.clone(),
        cell_count: 1,
        column_span: columns,
        style: TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: cell_style.underline.then_some(UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: cell_style.undercurl,
            }),
            strikethrough: cell_style.strikeout.then_some(StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            }),
        },
        font_size: px(0.0),
    }
}

struct BgRect {
    line: i32,
    start_col: i32,
    end_col: i32,
    color: gpui::Hsla,
}

/// Display-line → pixel y via RowGeometry (ADR-0018). The only path.
#[derive(Clone)]
struct PaintMap {
    geom: RowGeometry,
    top_abs: i32,
    sub: f32,
}

impl PaintMap {
    fn y(&self, origin: GpuiPoint<Pixels>, display_line: i32) -> Pixels {
        origin.y
            + px(y_for_display(
                &self.geom,
                display_line,
                self.top_abs,
                self.sub,
            ))
    }

    fn h(&self, display_line: i32) -> Pixels {
        let h = self
            .geom
            .height_of(self.top_abs.saturating_add(display_line));
        if h.is_finite() && h > 0.0 {
            px(h)
        } else {
            px(0.0)
        }
    }
}

pub struct LayoutState {
    hitbox: gpui::Hitbox,
    dimensions: TerminalBounds,
    batches: Vec<BatchedTextRun>,
    backgrounds: Vec<BgRect>,
    selection_backgrounds: Vec<BgRect>,
    search_rects: Vec<BgRect>,
    /// Underlines for the hovered hyperlink (M11).
    hover_underlines: Vec<BgRect>,
    background_color: gpui::Hsla,
    /// Column, display line, cell char, shape — `None` when the app hid the cursor.
    cursor: Option<(usize, i32, char, CursorShape)>,
    ime_cursor_bounds: Option<Bounds<Pixels>>,
    /// True when a hyperlink is under the pointer (⌘+hover).
    hover_link: bool,
    /// Cursor blink opacity for this frame (M11).
    blink_alpha: f32,
    /// Whether to request another animation frame (M11).
    blink_animating: bool,
    gutter: Vec<GutterPaint>,
    map: PaintMap,
    block_paints: Vec<BlockPaint>,
}

struct BlockPaint {
    display_line: i32,
    layout: sleipnir_widget::Layout,
    stale: bool,
    frozen: bool,
}

struct GutterPaint {
    display_line: i32,
    kind: GutterKind,
    color: gpui::Hsla,
}

impl Element for TermElement {
    type RequestLayoutState = ();
    type PrepaintState = LayoutState;

    fn id(&self) -> Option<ElementId> {
        Some("term-element".into())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |mut style, window, cx| {
                style.size.width = relative(1.).into();
                style.size.height = relative(1.).into();
                window.request_layout(style, None, cx)
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let font_size_override = self.font_size_override;
        let last_input_at = self.last_input_at;
        let terminal_wants_blink = self.terminal_wants_blink;
        let focused = self.focused;
        let terminal = self.terminal.clone();
        let view = self.view.clone();

        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            move |_, _, hitbox, window, cx| {
                let hitbox = hitbox.unwrap();
                let settings = TerminalSettings::get_global(cx);
                let palette = TerminalPalette::get_global(cx);
                let blinking = settings.blinking;
                let font_family = settings
                    .font_family
                    .clone()
                    .unwrap_or_else(|| sleipnir_settings::default_font_family().into());
                let font_size = font_size_override
                    .or(settings.font_size)
                    .unwrap_or(px(14.))
                    .max(px(8.));
                let line_height_factor = settings.line_height.value().max(1.0);
                let font_features = settings
                    .font_features
                    .clone()
                    .unwrap_or_else(gpui::FontFeatures::disable_ligatures);
                let font_weight = settings.font_weight.unwrap_or_default();
                let font_fallbacks = settings.font_fallbacks.clone();
                let foreground = palette.foreground;

                let text_style = TextStyle {
                    font_family: font_family.into(),
                    font_features,
                    font_weight,
                    font_size: font_size.into(),
                    font_fallbacks,
                    color: foreground,
                    ..Default::default()
                };

                let font_id = cx.text_system().resolve_font(&text_style.font());
                let cell_width = cx
                    .text_system()
                    .advance(font_id, font_size, 'm')
                    .map(|a| a.width)
                    .unwrap_or(px(8.))
                    .max(px(4.));
                let line_height = px(f32::from(font_size) * line_height_factor).max(px(10.));

                let mut grid_size = bounds.size;
                if grid_size.width < cell_width * 2.0 {
                    grid_size.width = cell_width * 2.0;
                }

                let scale = window.scale_factor();
                let snap = |v: Pixels| Pixels::from((f32::from(v) * scale).floor() / scale);
                let origin = point(snap(bounds.origin.x), snap(bounds.origin.y));
                let dimensions = TerminalBounds::new(
                    line_height,
                    cell_width,
                    Bounds {
                        origin,
                        size: grid_size,
                    },
                );

                let content = terminal.update(cx, |terminal, cx| {
                    terminal.set_size(dimensions);
                    terminal.sync(window, cx);
                    terminal.last_content().clone()
                });
                view.update(cx, |v, cx| v.sync_block_lifecycle(cx));

                let history = terminal.read(cx).history_size() as i32;
                let sub = terminal.read(cx).viewport_sub();
                let geom = terminal.read(cx).row_geometry().clone();
                let top_abs = viewport_top_abs(history, content.display_offset);
                let map = PaintMap {
                    geom: geom.clone(),
                    top_abs,
                    sub,
                };
                let frozen = geom.is_frozen();
                let skip_lines: std::collections::HashSet<i32> = if content
                    .mode
                    .contains(Modes::ALT_SCREEN)
                {
                    Default::default()
                } else {
                    geom.blocks()
                        .filter(|b| b.height > 0)
                        .map(|b| {
                            absolute_to_display_line(b.anchor.line, history, content.display_offset)
                        })
                        .collect()
                };

                let selection_range = content.selection.map(|sel| sel.point_range());
                let (batches, backgrounds, selection_backgrounds) = layout_grid(
                    &content.cells,
                    &text_style,
                    font_size,
                    palette.as_ref(),
                    selection_range,
                    &skip_lines,
                );

                log::debug!(
                    "term prepaint: cells={} batches={} cell_w={:?} font={:?} cursor=({},{})",
                    content.cells.len(),
                    batches.len(),
                    cell_width,
                    text_style.font_family,
                    content.cursor.point.line,
                    content.cursor.point.column,
                );

                // Selection uses reverse-video text from layout_grid. Its
                // opaque backgrounds paint after the decorative starfield.

                // Search highlights (M10): paint under selection, above cell bg.
                // The match the find bar points at gets the cursor color so the
                // "n/m" counter stays locatable even after a click clears the
                // selection that `activate_match` also installs.
                let search_matches = terminal.read(cx).matches.clone();
                let active_match = terminal.read(cx).active_match;
                let match_color = palette.selection.opacity(0.35);
                let active_match_color = palette.cursor.opacity(0.55);
                let mut search_rects = Vec::new();
                for m in search_matches {
                    let color = if Some(m) == active_match {
                        active_match_color
                    } else {
                        match_color
                    };
                    search_rects.extend(range_rects(m, content.display_offset, color));
                }

                // URL / path hover underline (M11).
                let link_color = palette.ansi[4].opacity(0.85);
                let mut hover_underlines = Vec::new();
                let hover_link = content.last_hovered_word.is_some();
                if let Some(hovered) = content.last_hovered_word.as_ref() {
                    hover_underlines.extend(range_rects(
                        hovered.word_match,
                        content.display_offset,
                        link_color,
                    ));
                }

                let cursor_point = content.cursor.point;
                let display_line = cursor_point.line + content.display_offset as i32;
                let ime_cursor_bounds = Some(Bounds::new(
                    point(
                        origin.x + cursor_point.column as f32 * cell_width,
                        map.y(origin, display_line),
                    ),
                    size(cell_width, map.h(display_line).max(line_height)),
                ));

                // Honor DECTCEM / app cursor-hide (CSI ?25l). Full-screen TUIs
                // (e.g. Grok) leave the grid cursor on a status cell while
                // reporting Hidden — painting anyway yields a spurious blink.
                let cursor = match content.cursor.shape {
                    CursorShape::Hidden => None,
                    _ => Some((
                        cursor_point.column,
                        display_line,
                        content.cursor_char,
                        content.cursor.shape,
                    )),
                };

                let gutter = {
                    use sleipnir_settings::RunLedgerMode;
                    if content.mode.contains(Modes::ALT_SCREEN)
                        || TerminalSettings::get_global(cx).run_ledger == RunLedgerMode::Off
                    {
                        Vec::new()
                    } else {
                        let rows = dimensions.num_lines() as i32;
                        terminal
                            .read(cx)
                            .gutter_overlay()
                            .into_iter()
                            .filter_map(|mark| {
                                let display_line = absolute_to_display_line(
                                    mark.line,
                                    history,
                                    content.display_offset,
                                );
                                if display_line < 0 || display_line >= rows {
                                    return None;
                                }
                                Some(GutterPaint {
                                    display_line,
                                    kind: mark.kind,
                                    color: gutter_color(mark.status, palette.as_ref()),
                                })
                            })
                            .collect()
                    }
                };

                let blink_alpha =
                    cursor_blink_alpha(last_input_at.elapsed(), terminal_wants_blink, blinking);
                let blink_animating = focused
                    && match blinking {
                        TerminalBlink::Off => false,
                        TerminalBlink::On => true,
                        TerminalBlink::TerminalControlled => terminal_wants_blink,
                    };

                let mut block_paints = Vec::new();
                if !content.mode.contains(Modes::ALT_SCREEN) {
                    let rows = dimensions.num_lines() as i32;
                    view.read(cx).blocks().iter().for_each(|surface| {
                        let display_line = absolute_to_display_line(
                            surface.anchor.line,
                            history,
                            content.display_offset,
                        );
                        // One extra row of overscan at each edge so a sub-row
                        // remainder does not clip a partial Block.
                        if display_line < -1 || display_line > rows {
                            return;
                        }
                        let Some(laid) = surface.laid.clone() else {
                            return;
                        };
                        block_paints.push(BlockPaint {
                            display_line,
                            layout: laid,
                            stale: surface.stale,
                            frozen,
                        });
                    });
                }

                LayoutState {
                    hitbox,
                    dimensions,
                    batches,
                    backgrounds,
                    selection_backgrounds,
                    search_rects,
                    hover_underlines,
                    background_color: {
                        let op = TerminalSettings::get_global(cx)
                            .background_opacity
                            .clamp(0.15, 1.0);
                        palette.background.opacity(op)
                    },
                    cursor,
                    ime_cursor_bounds,
                    hover_link,
                    blink_alpha,
                    blink_animating,
                    gutter,
                    map,
                    block_paints,
                }
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, layout.background_color));
            let origin = layout.dimensions.bounds.origin;

            self.register_mouse_listeners(window);

            let input_handler = TerminalInputHandler {
                terminal: self.terminal.clone(),
                view: self.view.clone(),
                cursor_bounds: layout.ime_cursor_bounds,
            };

            self.interactivity.paint(
                global_id,
                inspector_id,
                bounds,
                Some(&layout.hitbox),
                window,
                cx,
                |_, window, cx| {
                    window.handle_input(&self.focus, input_handler, cx);
                    let cursor_style = if layout.hover_link {
                        gpui::CursorStyle::PointingHand
                    } else {
                        gpui::CursorStyle::IBeam
                    };
                    window.set_cursor_style(cursor_style, &layout.hitbox);

                    for bg in &layout.backgrounds {
                        paint_bg(origin, bg, &layout.dimensions, &layout.map, window);
                    }
                    if TerminalSettings::get_global(cx).starfield {
                        let palette = TerminalPalette::get_global(cx);
                        crate::starfield::paint(
                            bounds,
                            self.terminal.entity_id().as_u64(),
                            palette.foreground,
                            self.starfield_time,
                            window,
                        );
                    }
                    for bg in &layout.selection_backgrounds {
                        paint_bg(origin, bg, &layout.dimensions, &layout.map, window);
                    }
                    for bg in &layout.search_rects {
                        paint_bg(origin, bg, &layout.dimensions, &layout.map, window);
                    }
                    // Hover link underlines (M11): thin strip at bottom of each cell span.
                    for ul in &layout.hover_underlines {
                        paint_underline(origin, ul, &layout.dimensions, &layout.map, window);
                    }
                    for batch in &layout.batches {
                        batch.paint(origin, &layout.dimensions, &layout.map, window, cx);
                    }
                    for mark in &layout.gutter {
                        paint_gutter_triangle(
                            origin,
                            mark,
                            &layout.dimensions,
                            &layout.map,
                            window,
                        );
                    }
                    for block in &layout.block_paints {
                        paint_block(origin, block, &layout.dimensions, &layout.map, window, cx);
                    }

                    if self.focused
                        && let Some((col, line, ch, shape)) = layout.cursor
                    {
                        // Skip off-screen cursor (scrolled away).
                        let rows = layout.dimensions.num_lines() as i32;
                        if line >= 0 && line < rows {
                            paint_terminal_cursor(
                                shape,
                                col,
                                line,
                                ch,
                                origin,
                                &layout.dimensions,
                                &layout.map,
                                layout.blink_alpha,
                                window,
                                cx,
                            );
                        }
                    }

                    // Keep the blink animation running at ~display refresh (M11).
                    if layout.blink_animating {
                        window.request_animation_frame();
                    }
                },
            );
        });
    }
}

impl TermElement {
    fn register_mouse_listeners(&mut self, window: &mut Window) {
        let terminal = self.terminal.clone();
        let view = self.view.clone();
        let focus = self.focus.clone();

        // Forward left/right/middle so mouse-mode apps (Herdr, vim, etc.) receive
        // full click sequences. Only Left was registered before, so right-click
        // context menus inside full-screen TUIs never fired.
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            self.interactivity.on_mouse_down(button, {
                let terminal = terminal.clone();
                let view = view.clone();
                let focus = focus.clone();
                move |e: &MouseDownEvent, window, cx| {
                    window.focus(&focus, cx);
                    if button == MouseButton::Left && try_block_click(&terminal, &view, e, cx) {
                        return;
                    }
                    if button == MouseButton::Left && try_gutter_click(&terminal, e, cx) {
                        return;
                    }
                    if button == MouseButton::Right {
                        // In normal mode a right-click opens the context menu
                        // instead of reaching the terminal (mouse-reporting
                        // apps keep receiving the button).
                        let in_mouse_mode = terminal.read(cx).mouse_mode(e.modifiers.shift);
                        if !in_mouse_mode {
                            let link = terminal.update(cx, |t, _| t.link_target_at(e.position));
                            view.update(cx, |v, cx| v.open_context_menu(e.position, link, cx));
                            return;
                        }
                    }
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_down(e, cx);
                        cx.notify();
                    });
                }
            });

            self.interactivity.on_mouse_up(button, {
                let terminal = terminal.clone();
                move |e: &MouseUpEvent, _window, cx| {
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_up(e, cx);
                        cx.notify();
                    });
                }
            });
        }

        window.on_mouse_event({
            let terminal = terminal.clone();
            let hitbox = (); // hitbox checked inside via focus
            move |e: &MouseMoveEvent, phase, window, cx| {
                let _ = hitbox;
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let is_focused = focus.is_focused(window);
                if e.pressed_button.is_some() && is_focused {
                    // bounds filled by terminal from last content during drag
                    let bounds = terminal.read(cx).last_content().terminal_bounds.bounds;
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_drag(e, bounds, cx);
                        cx.notify();
                    });
                }
                if e.pressed_button.is_none() && !is_focused {
                    return;
                }
                terminal.update(cx, |terminal, cx| {
                    terminal.mouse_move(e, cx);
                });
            }
        });

        self.interactivity.on_scroll_wheel({
            move |e: &ScrollWheelEvent, _window, cx| {
                let multiplier = TerminalSettings::get_global(cx).scroll_multiplier;
                terminal.update(cx, |terminal, cx| {
                    terminal.scroll_wheel(e, multiplier);
                    // scroll events queue InternalEvent; need notify after next sync
                    cx.notify();
                });
            }
        });
    }
}

impl IntoElement for TermElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

fn paint_bg(
    origin: GpuiPoint<Pixels>,
    bg: &BgRect,
    dimensions: &TerminalBounds,
    map: &PaintMap,
    window: &mut Window,
) {
    let rect = Bounds::new(
        point(
            origin.x + bg.start_col as f32 * dimensions.cell_width,
            map.y(origin, bg.line),
        ),
        size(
            ((bg.end_col - bg.start_col + 1) as f32) * dimensions.cell_width,
            map.h(bg.line).max(px(1.)),
        ),
    );
    window.paint_quad(fill(rect, bg.color));
}

/// 1px-ish underline along the bottom of a cell span (URL hover).
fn paint_underline(
    origin: GpuiPoint<Pixels>,
    bg: &BgRect,
    dimensions: &TerminalBounds,
    map: &PaintMap,
    window: &mut Window,
) {
    let h = (map.h(bg.line) * 0.08).max(px(1.));
    let width_cells = (bg.end_col - bg.start_col + 1).max(1) as f32;
    // Cap absurd LINE_END spans to the visible width.
    let max_cols = dimensions.num_columns() as f32;
    let cols = width_cells.min(max_cols);
    let rect = Bounds::new(
        point(
            origin.x + bg.start_col as f32 * dimensions.cell_width,
            map.y(origin, bg.line) + map.h(bg.line) - h,
        ),
        size(cols * dimensions.cell_width, h),
    );
    window.paint_quad(fill(rect, bg.color));
}

fn gutter_color(status: Option<i32>, palette: &TerminalPalette) -> gpui::Hsla {
    match status {
        Some(0) => palette.ansi[2],
        Some(_) => palette.ansi[1],
        None => palette.ansi[3],
    }
}

fn paint_gutter_triangle(
    origin: GpuiPoint<Pixels>,
    mark: &GutterPaint,
    _dimensions: &TerminalBounds,
    map: &PaintMap,
    window: &mut Window,
) {
    let row_h = map.h(mark.display_line);
    let mid_y = map.y(origin, mark.display_line) + row_h * 0.5;
    let h = row_h.min(px(8.0));
    let step = px(2.0);
    let x0 = origin.x + px(1.0);
    for i in 0..3 {
        let inset = px(i as f32);
        let hh = h - inset * 2.0;
        if hh <= px(0.5) {
            break;
        }
        let x = match mark.kind {
            GutterKind::Start => x0 + step * i as f32,
            GutterKind::End => x0 + step * (2 - i) as f32,
        };
        window.paint_quad(fill(
            Bounds::new(point(x, mid_y - hh * 0.5), size(step, hh)),
            mark.color,
        ));
    }
}

fn try_block_click(
    terminal: &Entity<Terminal>,
    view: &Entity<crate::TermView>,
    e: &MouseDownEvent,
    cx: &mut App,
) -> bool {
    let content = terminal.read(cx).last_content().clone();
    if content.mode.contains(Modes::ALT_SCREEN) {
        return false;
    }
    let origin = content.terminal_bounds.bounds.origin;
    let local = gpui::point(e.position.x - origin.x, e.position.y - origin.y);
    let hit = terminal.read(cx).hit_local(local);
    let HitTarget::Block { id, local_y } = hit else {
        return false;
    };
    let cell_w = f32::from(content.terminal_bounds.cell_width);
    let line_h = f32::from(content.terminal_bounds.line_height);
    let pos = crate::plugin_panel::cell_from_pixels(f32::from(local.x), local_y, cell_w, line_h);
    // Only consume the click when it actually lands on a button. The rest of
    // a block's area must stay available for text selection and click-to-move.
    let Some(surface) = view.read(cx).blocks().get(id).cloned() else {
        return false;
    };
    let Some(laid) = surface.laid.as_ref() else {
        return false;
    };
    let Some(hit) = crate::plugin_block::action_at(laid, pos.col, pos.row) else {
        return false;
    };
    if surface.stale {
        // Dead block UI: its buttons render but must not fire.
        return true;
    }
    crate::plugin_runtime::push_action(surface.owner_instance_id, id, hit.action, hit.arg, cx);
    true
}

fn try_gutter_click(terminal: &Entity<Terminal>, e: &MouseDownEvent, cx: &mut App) -> bool {
    let content = terminal.read(cx).last_content().clone();
    if content.mode.contains(Modes::ALT_SCREEN) {
        return false;
    }
    let origin = content.terminal_bounds.bounds.origin;
    let x = e.position.x - origin.x;
    if x < px(0.) || x > px(8.) {
        return false;
    }
    let y = e.position.y - origin.y;
    if y < px(0.) {
        return false;
    }
    let history = terminal.read(cx).history_size() as i32;
    let hit = terminal.read(cx).hit_local(gpui::point(x, y));
    let abs = match hit {
        HitTarget::Cell { line } => line,
        HitTarget::Block { id, .. } => terminal
            .read(cx)
            .row_geometry()
            .get(id)
            .map(|b| b.anchor.line)
            .unwrap_or(0),
    };
    let display_line = absolute_to_display_line(abs, history, content.display_offset);
    let marks = terminal.read(cx).gutter_overlay();
    let Some(mark) = marks.into_iter().find(|m| {
        absolute_to_display_line(m.line, history, content.display_offset) == display_line
    }) else {
        return false;
    };
    terminal.update(cx, |term, cx| {
        term.emit_gutter_click(mark.line, cx);
        cx.notify();
    });
    true
}

/// Convert a terminal point range into display-space background rects.
fn range_rects(range: TerminalRange, display_offset: usize, color: gpui::Hsla) -> Vec<BgRect> {
    let mut rects = Vec::new();
    let start_line = range.start().line + display_offset as i32;
    let end_line = range.end().line + display_offset as i32;
    let start_col = range.start().column as i32;
    let end_col = range.end().column as i32;
    // Use a large sentinel for "rest of line"; will be clipped by the paint
    // bounds. i32::MAX / 2 avoids overflow when multiplied by cell_width.
    const LINE_END: i32 = i32::MAX / 2;
    // Simple single/multi-line blocks (approximate).
    if start_line == end_line {
        rects.push(BgRect {
            line: start_line,
            start_col: start_col.min(end_col),
            end_col: start_col.max(end_col),
            color,
        });
    } else {
        rects.push(BgRect {
            line: start_line,
            start_col,
            end_col: LINE_END,
            color,
        });
        for line in (start_line + 1)..end_line {
            rects.push(BgRect {
                line,
                start_col: 0,
                end_col: LINE_END,
                color,
            });
        }
        rects.push(BgRect {
            line: end_line,
            start_col: 0,
            end_col,
            color,
        });
    }
    rects
}

/// Whether a grid point falls inside an (inclusive) selection range.
fn point_in_range(point: terminal::Point, range: &TerminalRange) -> bool {
    let start = range.start();
    let end = range.end();
    // Normalize so start <= end in reading order (line, then column).
    let (start, end) = if (start.line, start.column) <= (end.line, end.column) {
        (start, end)
    } else {
        (end, start)
    };
    let after_start =
        point.line > start.line || (point.line == start.line && point.column >= start.column);
    let before_end =
        point.line < end.line || (point.line == end.line && point.column <= end.column);
    after_start && before_end
}

/// Append a single-cell background rect, coalescing with the previous rect when
/// it is the same color on the same display line and directly adjacent.
fn push_bg(backgrounds: &mut Vec<BgRect>, line: i32, col: i32, color: gpui::Hsla) {
    if let Some(last) = backgrounds.last_mut()
        && last.color == color
        && last.line == line
        && last.end_col + 1 == col
    {
        last.end_col = col;
    } else {
        backgrounds.push(BgRect {
            line,
            start_col: col,
            end_col: col,
            color,
        });
    }
}

fn layout_grid(
    cells: &[IndexedCell],
    text_style: &TextStyle,
    font_size: Pixels,
    palette: &TerminalPalette,
    selection: Option<TerminalRange>,
    skip_lines: &std::collections::HashSet<i32>,
) -> (Vec<BatchedTextRun>, Vec<BgRect>, Vec<BgRect>) {
    let mut batches: Vec<BatchedTextRun> = Vec::new();
    let mut backgrounds: Vec<BgRect> = Vec::new();
    let mut selection_backgrounds: Vec<BgRect> = Vec::new();
    let mut current: Option<BatchedTextRun> = None;

    let linegroups = cells.iter().chunk_by(|c| c.point.line);
    for (line_index, (_, line)) in linegroups.into_iter().enumerate() {
        if let Some(batch) = current.take() {
            batches.push(batch);
        }
        let display_line = line_index as i32;
        if skip_lines.contains(&display_line) {
            continue;
        }

        let mut line = line.peekable();
        while let Some(indexed) = line.next() {
            let cell = &indexed.cell;
            let columns = cell_column_span(line.peek().is_some_and(|next| {
                next.point.column == indexed.point.column + 1 && next.cell.is_wide_char_spacer()
            }));
            let mut fg = cell.foreground();
            let mut bg = cell.background();
            if cell.is_inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }

            // Selected cells get a solid highlight and a contrasting foreground,
            // matching native terminals instead of tinting the original glyph.
            let selected = selection
                .as_ref()
                .is_some_and(|sel| point_in_range(indexed.point, sel));

            if selected {
                let base_col = indexed.point.column as i32;
                let color = selection_background(palette);
                for offset in 0..columns as i32 {
                    push_bg(
                        &mut selection_backgrounds,
                        display_line,
                        base_col + offset,
                        color,
                    );
                }
            } else if !is_default_background_color(bg) {
                let color = convert_color(&bg, palette);
                let base_col = indexed.point.column as i32;
                for offset in 0..columns as i32 {
                    push_bg(&mut backgrounds, display_line, base_col + offset, color);
                }
            }

            if cell.is_wide_char_spacer() || is_blank(cell) {
                continue;
            }

            let mut color = convert_color(
                &if selected {
                    selection_foreground(palette)
                } else {
                    fg
                },
                palette,
            );
            if cell.is_dim() && !selected {
                color = color.opacity(0.55);
            }
            let text = compose_cell_text(cell.character(), cell.zerowidth());
            let mut run = build_cell_text_run(
                text.clone(),
                columns,
                text_style,
                color,
                TerminalCellTextStyle::from_cell(cell),
            );
            let point = LayoutPoint {
                line: display_line,
                column: indexed.point.column as i32,
            };
            run.start = point;
            run.font_size = font_size;

            if let Some(ref mut batch) = current {
                if batch.can_append_run(point, &run.style, run.column_span) {
                    batch.append_cell_text(&text);
                } else {
                    let old = current.take().unwrap();
                    batches.push(old);
                    current = Some(run);
                }
            } else {
                current = Some(run);
            }
        }
    }
    if let Some(batch) = current {
        batches.push(batch);
    }
    (batches, backgrounds, selection_backgrounds)
}

/// Paint the terminal cell cursor. Caller must already filter out `Hidden`.
fn paint_terminal_cursor(
    shape: CursorShape,
    col: usize,
    line: i32,
    ch: char,
    origin: GpuiPoint<Pixels>,
    dimensions: &TerminalBounds,
    map: &PaintMap,
    blink_alpha: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let palette = TerminalPalette::get_global(cx);
    let cursor_color = palette.cursor.opacity(blink_alpha.clamp(0.0, 1.0));
    let cell_origin = point(
        origin.x + col as f32 * dimensions.cell_width,
        map.y(origin, line),
    );
    let cell = size(dimensions.cell_width, map.h(line).max(px(1.)));

    match shape {
        CursorShape::Hidden => {}
        CursorShape::Block | CursorShape::HollowBlock => {
            let cursor_bounds = Bounds::new(cell_origin, cell);
            if matches!(shape, CursorShape::HollowBlock) {
                // Outline only: leave cell content visible.
                window.paint_quad(gpui::outline(
                    cursor_bounds,
                    cursor_color,
                    gpui::BorderStyle::Solid,
                ));
            } else {
                window.paint_quad(fill(cursor_bounds, cursor_color));
                // Only paint inverse glyph when cursor is mostly solid.
                if blink_alpha > 0.2 {
                    let style = TextRun {
                        len: ch.len_utf8(),
                        font: window.text_style().font(),
                        color: palette.background.opacity(blink_alpha.clamp(0.0, 1.0)),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let font_size = TerminalSettings::get_global(cx)
                        .font_size
                        .unwrap_or(px(14.));
                    let _ = window
                        .text_system()
                        .shape_line(
                            ch.to_string().into(),
                            font_size,
                            &[style],
                            Some(dimensions.cell_width),
                        )
                        .paint(
                            cursor_bounds.origin,
                            dimensions.line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                }
            }
        }
        CursorShape::Underline => {
            let row_h = map.h(line);
            let h = (row_h * 0.12).max(px(1.));
            let underline = Bounds::new(
                point(cell_origin.x, cell_origin.y + row_h - h),
                size(dimensions.cell_width, h),
            );
            window.paint_quad(fill(underline, cursor_color));
        }
        CursorShape::Bar => {
            let w = (dimensions.cell_width * 0.15).max(px(1.));
            let bar = Bounds::new(cell_origin, size(w, map.h(line)));
            window.paint_quad(fill(bar, cursor_color));
        }
    }
}

fn paint_block(
    origin: GpuiPoint<Pixels>,
    block: &BlockPaint,
    dimensions: &TerminalBounds,
    map: &PaintMap,
    window: &mut Window,
    cx: &mut App,
) {
    let palette = TerminalPalette::get_global(cx);
    let top = map.y(origin, block.display_line);
    let height = map.h(block.display_line).max(px(1.));
    let width = dimensions.bounds.size.width;
    let bounds = Bounds::new(point(origin.x, top), size(width, height));
    let bg = if block.frozen {
        palette.background.blend(gpui::Hsla::black().opacity(0.12))
    } else if block.stale {
        palette.background.blend(gpui::Hsla::black().opacity(0.2))
    } else {
        palette.background
    };
    window.paint_quad(fill(bounds, bg));
    if block.frozen {
        return;
    }
    let cell_w = dimensions.cell_width;
    let line_h = dimensions.line_height;
    let font_size = TerminalSettings::get_global(cx)
        .font_size
        .unwrap_or(px(14.));
    for node in block.layout.walk() {
        paint_laid_node(
            origin.x,
            top,
            node,
            cell_w,
            line_h,
            palette.as_ref(),
            font_size,
            window,
            cx,
        );
    }
    paint_laid_node(
        origin.x,
        top,
        &block.layout.attribution,
        cell_w,
        line_h,
        palette.as_ref(),
        font_size,
        window,
        cx,
    );
}

/// Text a Block paints for one laid-out node, or `None` when the node draws no
/// text (containers, and the two kinds painted as quads).
///
/// Pure so the Block/Panel parity this had to be fixed for is testable without
/// a window. There is deliberately **no catch-all**: a new [`LaidOutKind`] must
/// be decided here, not silently rendered as the empty string. `Spark` was
/// dropped exactly that way, and because layout still reserves its cells the
/// symptom was correctly-sized blank space with nothing logged.
fn block_text_for(kind: &sleipnir_widget::LaidOutKind) -> Option<String> {
    use sleipnir_widget::LaidOutKind;
    match kind {
        LaidOutKind::Text { lines, .. } => Some(lines.join("\n")),
        LaidOutKind::Code { lines } => Some(
            lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        LaidOutKind::Badge { text, .. } | LaidOutKind::Btn { text, .. } => Some(text.clone()),
        LaidOutKind::Attribution { label, .. } => Some(label.clone()),
        LaidOutKind::Unknown => Some("[?]".into()),
        LaidOutKind::Truncated => Some("… truncated".into()),
        // Shared ramp, so a sparkline reads the same in a Block and a Panel.
        LaidOutKind::Spark { levels } => Some(sleipnir_widget::spark_glyphs(levels)),
        // Painted as quads by the caller, which needs bounds this cannot see.
        LaidOutKind::Sep | LaidOutKind::Bar { .. } => None,
        LaidOutKind::Col | LaidOutKind::Row => None,
    }
}

/// Whether a laid-out node paints bold.
///
/// `bold` is part of the schema and the Panel painter honours it; a Block that
/// ignored it would render the same tree differently depending on where it is
/// mounted. Split out from the painter so it is testable without a `Window`.
fn block_is_bold(kind: &sleipnir_widget::LaidOutKind) -> bool {
    matches!(kind, sleipnir_widget::LaidOutKind::Text { bold: true, .. })
}

fn paint_laid_node(
    origin_x: Pixels,
    block_top: Pixels,
    node: &sleipnir_widget::LaidOut,
    cell_w: Pixels,
    line_h: Pixels,
    palette: &TerminalPalette,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    use sleipnir_widget::LaidOutKind;
    let r = node.rect;
    let x = origin_x + cell_w * r.col as f32;
    let y = block_top + line_h * r.row as f32;
    let w = cell_w * r.width as f32;
    let h = line_h * r.height.max(1) as f32;
    let color = match &node.kind {
        LaidOutKind::Text { tone, .. } | LaidOutKind::Badge { tone, .. } => match tone {
            sleipnir_widget::Tone::Fg => palette.foreground,
            sleipnir_widget::Tone::Dim => palette.foreground.opacity(0.55),
            sleipnir_widget::Tone::Accent => palette.ansi[4],
            sleipnir_widget::Tone::Ok => palette.ansi[2],
            sleipnir_widget::Tone::Warn => palette.ansi[3],
            sleipnir_widget::Tone::Err => palette.ansi[1],
        },
        LaidOutKind::Attribution { .. } => palette.foreground.opacity(0.55),
        LaidOutKind::Btn { .. } => palette.ansi[4],
        LaidOutKind::Truncated => palette.ansi[3],
        // Matches the Panel painter's accent for sparklines.
        LaidOutKind::Spark { .. } => palette.ansi[4],
        // No catch-all: a new `LaidOutKind` must be considered here, not
        // silently painted in the default foreground. The Panel painter
        // (`app_shell/plugin_paint.rs`) is exhaustive for the same reason;
        // the two must not drift.
        LaidOutKind::Col
        | LaidOutKind::Row
        | LaidOutKind::Code { .. }
        | LaidOutKind::Bar { .. }
        | LaidOutKind::Sep
        | LaidOutKind::Unknown => palette.foreground,
    };
    let mut font = window.text_style().font();
    if block_is_bold(&node.kind) {
        font.weight = gpui::FontWeight::BOLD;
    }
    let line_texts: Vec<String> = match &node.kind {
        LaidOutKind::Sep => {
            window.paint_quad(fill(Bounds::new(point(x, y), size(w, px(1.))), color));
            return;
        }
        LaidOutKind::Bar { filled, width: bw } => {
            let fill_w = w * (*filled as f32 / (*bw).max(1) as f32);
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(w, h)),
                palette.background.blend(gpui::Hsla::black().opacity(0.15)),
            ));
            window.paint_quad(fill(
                Bounds::new(point(x, y), size(fill_w, h)),
                palette.ansi[4],
            ));
            return;
        }
        LaidOutKind::Text { lines, .. } => lines.clone(),
        LaidOutKind::Code { lines } => lines.iter().map(|line| line.text.clone()).collect(),
        kind => match block_text_for(kind) {
            Some(text) => vec![text],
            None => return,
        },
    };
    if line_texts.is_empty() {
        return;
    }
    for (line_ix, text) in line_texts.into_iter().enumerate() {
        debug_assert!(
            !text.contains('\n'),
            "block line painter expects one laid-out line at a time"
        );
        if text.is_empty() {
            continue;
        }
        let style = TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let _ = window
            .text_system()
            .shape_line(text.into(), font_size, &[style], None)
            .paint(
                point(x, y + line_h * line_ix as f32),
                line_h,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
    }
}

/// Use the active theme's ANSI red for an unmistakable selected-input fill.
fn selection_background(palette: &TerminalPalette) -> gpui::Hsla {
    palette.ansi[1]
}

/// Pick a foreground that stays legible over the configured selection fill.
fn selection_foreground(palette: &TerminalPalette) -> Color {
    let background = selection_rgb(selection_background(palette));
    let foreground = selection_rgb(palette.foreground);
    let terminal_background = selection_rgb(palette.background);
    let foreground_contrast = rgb_contrast(foreground, background);
    let terminal_background_contrast = rgb_contrast(terminal_background, background);
    let selected = if terminal_background_contrast > foreground_contrast {
        terminal_background
    } else {
        foreground
    };
    Color::Spec(Rgb {
        r: selected.0,
        g: selected.1,
        b: selected.2,
    })
}

fn selection_rgb(color: gpui::Hsla) -> (u8, u8, u8) {
    let rgba: gpui::Rgba = color.into();
    (
        (rgba.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn rgb_contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f32 {
    let luminance = |rgb: (u8, u8, u8)| {
        let channel = |value: u8| {
            let value = value as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb.0) + 0.7152 * channel(rgb.1) + 0.0722 * channel(rgb.2)
    };
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn convert_color(color: &Color, palette: &TerminalPalette) -> gpui::Hsla {
    match color {
        Color::Named(named) => named_color(*named, palette),
        Color::Spec(rgb) => gpui::Rgba {
            r: rgb.r as f32 / 255.,
            g: rgb.g as f32 / 255.,
            b: rgb.b as f32 / 255.,
            a: 1.,
        }
        .into(),
        Color::Indexed(index) => get_color_at_index(*index as usize, palette),
    }
}

fn named_color(named: NamedColor, palette: &TerminalPalette) -> gpui::Hsla {
    use NamedColor::*;
    match named {
        Black => palette.ansi[0],
        Red => palette.ansi[1],
        Green => palette.ansi[2],
        Yellow => palette.ansi[3],
        Blue => palette.ansi[4],
        Magenta => palette.ansi[5],
        Cyan => palette.ansi[6],
        White => palette.ansi[7],
        BrightBlack => palette.ansi[8],
        BrightRed => palette.ansi[9],
        BrightGreen => palette.ansi[10],
        BrightYellow => palette.ansi[11],
        BrightBlue => palette.ansi[12],
        BrightMagenta => palette.ansi[13],
        BrightCyan => palette.ansi[14],
        BrightWhite => palette.ansi[15],
        Foreground => palette.foreground,
        Background => palette.background,
        Cursor => palette.cursor,
        DimBlack => palette.dim[0],
        DimRed => palette.dim[1],
        DimGreen => palette.dim[2],
        DimYellow => palette.dim[3],
        DimBlue => palette.dim[4],
        DimMagenta => palette.dim[5],
        DimCyan => palette.dim[6],
        DimWhite => palette.dim[7],
        BrightForeground => palette.bright_foreground,
        DimForeground => palette.foreground,
    }
}

fn is_blank(cell: &Cell) -> bool {
    cell.character() == ' '
        && cell.zerowidth().map(|z| z.is_empty()).unwrap_or(true)
        && !cell.has_underline()
        && !cell.has_strikeout()
}

struct TerminalInputHandler {
    terminal: Entity<Terminal>,
    view: Entity<crate::TermView>,
    cursor_bounds: Option<Bounds<Pixels>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sleipnir_settings::{Appearance, ThemeName, palette_for_theme};

    #[test]
    fn selection_backgrounds_are_separate_from_starfield_underlay() {
        let palette = palette_for_theme(ThemeName::Dracula, Appearance::Dark);
        let cells: Vec<_> = (0..3)
            .map(|column| IndexedCell {
                point: terminal::Point { line: 0, column },
                cell: Cell::default(),
            })
            .collect();
        let selection = TerminalRange::new(cells[0].point, cells[1].point);
        let (_, backgrounds, selected) = layout_grid(
            &cells,
            &TextStyle::default(),
            px(14.0),
            &palette,
            Some(selection),
            &Default::default(),
        );
        assert!(
            backgrounds.is_empty(),
            "selection must not paint below stars"
        );
        assert_eq!(selected.len(), 1, "adjacent selected cells still coalesce");
        assert_eq!(selected[0].start_col, 0);
        assert_eq!(selected[0].end_col, 1);
        assert_eq!(selected[0].color, selection_background(&palette));

        let (_, _, selected) = layout_grid(
            &cells,
            &TextStyle::default(),
            px(14.0),
            &palette,
            None,
            &Default::default(),
        );
        assert!(selected.is_empty());
    }

    /// Both mount points render one schema (ADR-0017), so a variant the Panel
    /// painter draws and the Block painter drops is an invisible failure:
    /// `sleipnir_widget::layout` still reserves the cells, so the Block shows
    /// correctly-sized blank space with nothing logged. That is exactly how
    /// `Spark` was lost.
    ///
    /// The compiler is the primary guard: neither painter has a catch-all, so
    /// a new `LaidOutKind` fails the build in both. This covers the half the
    /// compiler cannot see — a variant matched but rendered as empty text,
    /// which paints nothing at all.
    #[test]
    fn block_painter_produces_text_for_every_visible_kind() {
        use sleipnir_widget::{CodeLine, LaidOutKind};

        let visible = [
            LaidOutKind::Text {
                lines: vec!["hi".into()],
                tone: sleipnir_widget::Tone::Fg,
                bold: false,
            },
            LaidOutKind::Code {
                lines: vec![CodeLine {
                    text: "fn main(){}".into(),
                    truncated: false,
                }],
            },
            LaidOutKind::Badge {
                text: "3000".into(),
                tone: sleipnir_widget::Tone::Ok,
            },
            LaidOutKind::Spark {
                levels: vec![0, 4, 8],
            },
            LaidOutKind::Btn {
                text: "Retry".into(),
                action: "retry".into(),
                arg: None,
            },
            LaidOutKind::Unknown,
            LaidOutKind::Truncated,
            LaidOutKind::Attribution {
                plugin_id: "demo".into(),
                label: "demo".into(),
            },
        ];

        for kind in visible {
            let text = block_text_for(&kind);
            assert!(
                text.is_some_and(|t| !t.is_empty()),
                "{kind:?} renders as nothing in a Block; \
                 layout reserved its cells, so this is blank space"
            );
        }

        // Containers and the two quad-painted kinds legitimately produce no
        // text; they are drawn as geometry or not at all.
        for kind in [LaidOutKind::Col, LaidOutKind::Row] {
            assert!(block_text_for(&kind).is_none(), "{kind:?} is a container");
        }
    }

    /// A sparkline must read identically in a Block and a Panel; both go
    /// through one shared ramp.
    #[test]
    fn spark_renders_as_ramp_glyphs() {
        use sleipnir_widget::LaidOutKind;
        let text = block_text_for(&LaidOutKind::Spark {
            levels: vec![0, 4, 8],
        })
        .expect("spark renders");
        assert_eq!(text, " ▄█");
        assert_eq!(text.chars().count(), 3, "one glyph per reserved cell");
    }

    /// `bold` is in the schema and the Panel painter honours it; a Block that
    /// ignored it would render the same tree differently by mount point.
    #[test]
    fn block_painter_honours_bold_only_on_bold_text() {
        use sleipnir_widget::{LaidOutKind, Tone};

        let bold = LaidOutKind::Text {
            lines: vec!["loud".into()],
            tone: Tone::Fg,
            bold: true,
        };
        let plain = LaidOutKind::Text {
            lines: vec!["quiet".into()],
            tone: Tone::Fg,
            bold: false,
        };
        assert!(block_is_bold(&bold), "bold text must paint bold");
        assert!(!block_is_bold(&plain), "plain text must not paint bold");
        assert!(
            !block_is_bold(&LaidOutKind::Badge {
                text: "x".into(),
                tone: Tone::Ok,
            }),
            "only Text carries bold in the schema"
        );
    }

    #[test]
    fn block_text_for_joins_multiline_text_and_code_with_newlines() {
        use sleipnir_widget::{CodeLine, LaidOutKind, Tone};

        let text = block_text_for(&LaidOutKind::Text {
            lines: vec!["alpha".into(), "beta".into()],
            tone: Tone::Fg,
            bold: false,
        })
        .expect("text renders");
        assert_eq!(text, "alpha\nbeta");

        let code = block_text_for(&LaidOutKind::Code {
            lines: vec![
                CodeLine {
                    text: "let x = 1;".into(),
                    truncated: false,
                },
                CodeLine {
                    text: "x += 1;".into(),
                    truncated: false,
                },
            ],
        })
        .expect("code renders");
        assert_eq!(code, "let x = 1;\nx += 1;");
    }

    #[test]
    fn compose_cell_text_preserves_combining_graphemes() {
        let text = compose_cell_text('e', Some(&['\u{301}', '\u{20dd}']));
        assert_eq!(text, "e\u{301}\u{20dd}");
        assert_eq!(text.chars().count(), 3);
    }

    #[test]
    fn build_cell_text_run_maps_terminal_styles_and_wide_columns() {
        let palette = palette_for_theme(ThemeName::Dracula, Appearance::Dark);
        let style = TextStyle::default();
        let run = build_cell_text_run(
            "好\u{301}".into(),
            cell_column_span(true),
            &style,
            palette.foreground.opacity(0.55),
            TerminalCellTextStyle {
                bold: true,
                italic: true,
                underline: true,
                undercurl: true,
                strikeout: true,
            },
        );

        assert_eq!(run.text, "好\u{301}");
        assert_eq!(run.cell_count, 1, "one grapheme per initial run");
        assert_eq!(run.column_span, 2, "wide glyph must reserve two columns");
        assert_eq!(run.style.len, "好\u{301}".len(), "len is utf8 bytes");
        assert_eq!(run.style.font.weight, gpui::FontWeight::BOLD);
        assert_eq!(run.style.font.style, gpui::FontStyle::Italic);
        assert_eq!(
            run.style.underline,
            Some(UnderlineStyle {
                thickness: px(1.0),
                color: None,
                wavy: true,
            })
        );
        assert_eq!(
            run.style.strikethrough,
            Some(StrikethroughStyle {
                thickness: px(1.0),
                color: None,
            })
        );
        assert_eq!(run.style.color, palette.foreground.opacity(0.55));
    }

    #[test]
    fn batched_text_run_force_width_uses_per_cell_span_not_total_batch_width() {
        let mut narrow = build_cell_text_run(
            "ABC".into(),
            1,
            &TextStyle::default(),
            gpui::Hsla::white(),
            TerminalCellTextStyle::default(),
        );
        narrow.cell_count = 3;
        assert_eq!(
            narrow.force_width(px(8.0)),
            Some(px(8.0)),
            "ABC across three narrow cells must force one cell per base glyph"
        );

        let mut wide = build_cell_text_run(
            "好界".into(),
            2,
            &TextStyle::default(),
            gpui::Hsla::white(),
            TerminalCellTextStyle::default(),
        );
        wide.cell_count = 2;
        assert_eq!(
            wide.force_width(px(8.0)),
            Some(px(16.0)),
            "wide glyph batches must force two cells per base glyph"
        );
    }

    #[test]
    fn batched_text_run_append_requires_matching_span_and_column_adjacency() {
        let mut wide = build_cell_text_run(
            "好".into(),
            2,
            &TextStyle::default(),
            gpui::Hsla::white(),
            TerminalCellTextStyle::default(),
        );
        wide.start = LayoutPoint { line: 0, column: 0 };

        assert!(
            wide.can_append_run(LayoutPoint { line: 0, column: 2 }, &wide.style, 2),
            "a second two-column grapheme may follow immediately after the first"
        );
        assert!(
            !wide.can_append_run(LayoutPoint { line: 0, column: 1 }, &wide.style, 2),
            "wide batches must advance by occupied terminal columns"
        );
        assert!(
            !wide.can_append_run(LayoutPoint { line: 0, column: 2 }, &wide.style, 1),
            "CJK width 2 and ASCII width 1 must not share a batch"
        );
    }

    #[test]
    fn batched_text_run_append_preserves_combining_cluster_as_single_cell() {
        let mut run = build_cell_text_run(
            "e\u{301}".into(),
            1,
            &TextStyle::default(),
            gpui::Hsla::white(),
            TerminalCellTextStyle::default(),
        );
        run.start = LayoutPoint { line: 0, column: 4 };

        assert!(
            run.can_append_run(LayoutPoint { line: 0, column: 5 }, &run.style, 1),
            "combining grapheme still occupies exactly one terminal cell"
        );
        assert_eq!(run.force_width(px(9.0)), Some(px(9.0)));
    }

    #[test]
    fn selection_background_uses_theme_ansi_red() {
        let palette = palette_for_theme(ThemeName::Dracula, Appearance::Dark);

        assert_eq!(selection_background(&palette), palette.ansi[1]);
        assert_ne!(selection_background(&palette), palette.selection);
    }

    #[test]
    fn selection_foreground_changes_when_the_original_glyph_is_low_contrast() {
        let mut palette = palette_for_theme(ThemeName::Dracula, Appearance::Dark);
        palette.foreground = palette.selection;
        let selected = selection_foreground(&palette);

        assert_eq!(convert_color(&selected, &palette), palette.background);
        assert_ne!(convert_color(&selected, &palette), palette.selection);
    }
}

impl InputHandler for TerminalInputHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _cx: &mut App,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<StdRange<usize>> {
        None
    }

    fn text_for_range(
        &mut self,
        _: StdRange<usize>,
        _: &mut Option<StdRange<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<StdRange<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut App,
    ) {
        if text.is_empty() {
            return;
        }
        self.terminal.update(cx, |term, _| {
            term.input(text.as_bytes().to_vec());
        });
        self.view.update(cx, |_, cx| {
            cx.emit(crate::TermViewEvent::UserTyped);
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<StdRange<usize>>,
        _new_text: &str,
        _new_marked_range: Option<StdRange<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        // Marked IME text overlay can be painted later; commit still arrives via replace_text.
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut App) {}

    fn bounds_for_range(
        &mut self,
        range_utf16: StdRange<usize>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let mut bounds = self.cursor_bounds?;
        let cell_width = self
            .terminal
            .read(cx)
            .last_content()
            .terminal_bounds
            .cell_width;
        bounds.origin.x += cell_width * range_utf16.start as f32;
        Some(bounds)
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }

    fn character_index_for_point(
        &mut self,
        _point: GpuiPoint<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        Some(0)
    }
}
