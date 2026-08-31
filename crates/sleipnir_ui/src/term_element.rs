//! Terminal grid painter + input (M2).

use crate::cursor_blink_alpha;
use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, InputHandler, InteractiveElement, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point as GpuiPoint, ScrollWheelEvent,
    StatefulInteractiveElement, TextRun, TextStyle, UTF16Selection, Window, fill, point, px,
    relative, size,
};
use itertools::Itertools;
use row_geometry::{HitTarget, RowGeometry};
use sleipnir_settings::{TerminalBlink, TerminalPalette, TerminalSettings, get_color_at_index};
use std::ops::Range as StdRange;
use std::time::Instant;
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
            interactivity: Default::default(),
        }
        .track_focus(&focus)
    }
}

impl InteractiveElement for TermElement {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        &mut self.interactivity
    }
}

impl StatefulInteractiveElement for TermElement {}

struct LayoutPoint {
    line: i32,
    column: i32,
}

struct BatchedTextRun {
    start: LayoutPoint,
    text: String,
    cell_count: usize,
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
        let force_width = if dimensions.cell_width > px(0.) {
            Some(dimensions.cell_width)
        } else {
            None
        };
        if let Err(err) = window
            .text_system()
            .shape_line(
                self.text.clone().into(),
                self.font_size,
                std::slice::from_ref(&self.style),
                force_width,
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
                let (batches, backgrounds) = layout_grid(
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

                // Selection is now rendered as reverse-video inside layout_grid
                // (glyph recolored over a selection-colored cell), so there is
                // no separate selection overlay layer.

                // Search highlights (M10): paint under selection, above cell bg.
                let search_matches = terminal.read(cx).matches.clone();
                let match_color = palette.selection.opacity(0.35);
                let mut search_rects = Vec::new();
                for m in search_matches {
                    search_rects.extend(range_rects(m, content.display_offset, match_color));
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
                if e.pressed_button.is_some() && focus.is_focused(window) {
                    // bounds filled by terminal from last content during drag
                    let bounds = terminal.read(cx).last_content().terminal_bounds.bounds;
                    terminal.update(cx, |terminal, cx| {
                        terminal.mouse_drag(e, bounds, cx);
                        cx.notify();
                    });
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
    let Some(surface) = view.read(cx).blocks().get(id).cloned() else {
        return true;
    };
    if surface.stale {
        return true;
    }
    let Some(laid) = surface.laid.as_ref() else {
        return true;
    };
    if let Some(hit) = crate::plugin_block::action_at(laid, pos.col, pos.row) {
        crate::plugin_runtime::push_action(&surface.plugin_id, id, hit.action, hit.arg, cx);
    }
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
) -> (Vec<BatchedTextRun>, Vec<BgRect>) {
    let mut batches: Vec<BatchedTextRun> = Vec::new();
    let mut backgrounds: Vec<BgRect> = Vec::new();
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

        for indexed in line {
            let cell = &indexed.cell;
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
                let col = indexed.point.column as i32;
                push_bg(
                    &mut backgrounds,
                    display_line,
                    col,
                    selection_background(palette),
                );
            } else if !is_default_background_color(bg) {
                let color = convert_color(&bg, palette);
                let col = indexed.point.column as i32;
                push_bg(&mut backgrounds, display_line, col, color);
            }

            if cell.is_wide_char_spacer() || is_blank(cell) {
                continue;
            }

            let color = convert_color(
                &if selected {
                    selection_foreground(palette)
                } else {
                    fg
                },
                palette,
            );
            let run = TextRun {
                len: cell.character().len_utf8(),
                font: text_style.font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let point = LayoutPoint {
                line: display_line,
                column: indexed.point.column as i32,
            };

            if let Some(ref mut batch) = current {
                if batch.can_append(&run)
                    && batch.start.line == point.line
                    && batch.start.column + batch.cell_count as i32 == point.column
                {
                    batch.text.push(cell.character());
                    batch.cell_count += 1;
                    batch.style.len += cell.character().len_utf8();
                } else {
                    let old = current.take().unwrap();
                    batches.push(old);
                    current = Some(BatchedTextRun {
                        start: point,
                        text: cell.character().to_string(),
                        cell_count: 1,
                        style: run,
                        font_size,
                    });
                }
            } else {
                current = Some(BatchedTextRun {
                    start: point,
                    text: cell.character().to_string(),
                    cell_count: 1,
                    style: run,
                    font_size,
                });
            }
        }
    }
    if let Some(batch) = current {
        batches.push(batch);
    }
    (batches, backgrounds)
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
        LaidOutKind::Text { role, .. } | LaidOutKind::Badge { role, .. } => match role {
            sleipnir_widget::ToneRole::Foreground => palette.foreground,
            sleipnir_widget::ToneRole::Muted => palette.foreground.opacity(0.55),
            sleipnir_widget::ToneRole::Accent => palette.ansi[4],
            sleipnir_widget::ToneRole::Success => palette.ansi[2],
            sleipnir_widget::ToneRole::Warning => palette.ansi[3],
            sleipnir_widget::ToneRole::Danger => palette.ansi[1],
        },
        LaidOutKind::Attribution { .. } => palette.foreground.opacity(0.55),
        LaidOutKind::Btn { .. } => palette.ansi[4],
        LaidOutKind::Truncated => palette.ansi[3],
        _ => palette.foreground,
    };
    let text = match &node.kind {
        LaidOutKind::Text { lines, .. } => lines.join("\n"),
        LaidOutKind::Code { lines } => lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        LaidOutKind::Badge { text, .. } | LaidOutKind::Btn { text, .. } => text.clone(),
        LaidOutKind::Attribution { label, .. } => label.clone(),
        LaidOutKind::Unknown => "[?]".into(),
        LaidOutKind::Truncated => "… truncated".into(),
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
        LaidOutKind::Col | LaidOutKind::Row | LaidOutKind::Spark { .. } => return,
    };
    if text.is_empty() {
        return;
    }
    let style = TextRun {
        len: text.len(),
        font: window.text_style().font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let _ = window
        .text_system()
        .shape_line(text.into(), font_size, &[style], None)
        .paint(point(x, y), line_h, gpui::TextAlign::Left, None, window, cx);
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
