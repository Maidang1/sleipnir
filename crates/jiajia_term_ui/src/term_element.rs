//! Minimal terminal grid painter (M1), adapted from Zed `terminal_element` batching ideas.

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Entity, FocusHandle, GlobalElementId,
    InteractiveElement, IntoElement, LayoutId, Pixels, Point as GpuiPoint, StatefulInteractiveElement,
    Styled as _, TextRun, TextStyle, Window, fill, point, px, relative, size,
};
use itertools::Itertools;
use jiajia_settings::{TerminalPalette, TerminalSettings, get_color_at_index};
use std::time::Instant;
use terminal::{
    Cell, Color, IndexedCell, NamedColor, Terminal, TerminalBounds, is_default_background_color,
};

pub struct TermElement {
    terminal: Entity<Terminal>,
    focus: FocusHandle,
    focused: bool,
    interactivity: gpui::Interactivity,
}

impl TermElement {
    pub fn new(terminal: Entity<Terminal>, focus: FocusHandle, focused: bool) -> Self {
        Self {
            terminal,
            focus: focus.clone(),
            focused,
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
        window: &mut Window,
        cx: &mut App,
    ) {
        let pos = GpuiPoint::new(
            origin.x + self.start.column as f32 * dimensions.cell_width,
            origin.y + self.start.line as f32 * dimensions.line_height,
        );
        let _ = window
            .text_system()
            .shape_line(
                self.text.clone().into(),
                self.font_size,
                std::slice::from_ref(&self.style),
                Some(dimensions.cell_width),
            )
            .paint(
                pos,
                dimensions.line_height,
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            );
    }
}

struct BgRect {
    line: i32,
    start_col: i32,
    end_col: i32,
    color: gpui::Hsla,
}

pub struct LayoutState {
    hitbox: gpui::Hitbox,
    dimensions: TerminalBounds,
    batches: Vec<BatchedTextRun>,
    backgrounds: Vec<BgRect>,
    background_color: gpui::Hsla,
    cursor: Option<(usize, usize, char)>,
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
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, window, cx| {
                let hitbox = hitbox.unwrap();
                let settings = TerminalSettings::get_global(cx);
                let palette = TerminalPalette::get_global(cx);

                let font_family = settings
                    .font_family
                    .clone()
                    .unwrap_or_else(|| "Menlo".into());
                let font_size = settings.font_size.unwrap_or(px(14.));
                let line_height_factor = settings.line_height.value();

                let text_style = TextStyle {
                    font_family: font_family.into(),
                    font_features: settings
                        .font_features
                        .clone()
                        .unwrap_or_else(gpui::FontFeatures::disable_ligatures),
                    font_weight: settings.font_weight.unwrap_or_default(),
                    font_size: font_size.into(),
                    color: palette.foreground,
                    ..Default::default()
                };

                let font_id = cx.text_system().resolve_font(&text_style.font());
                let cell_width = cx
                    .text_system()
                    .advance(font_id, font_size, 'm')
                    .map(|a| a.width)
                    .unwrap_or(px(8.));
                let line_height = px(f32::from(font_size) * line_height_factor);

                let mut size = bounds.size;
                // Keep at least 2 columns for wide-char safety.
                if size.width < cell_width * 2.0 {
                    size.width = cell_width * 2.0;
                }

                let scale = window.scale_factor();
                let snap = |v: Pixels| Pixels::from((f32::from(v) * scale).floor() / scale);
                let origin = point(snap(bounds.origin.x), snap(bounds.origin.y));

                let dimensions = TerminalBounds::new(line_height, cell_width, Bounds { origin, size });

                let content = self.terminal.update(cx, |terminal, cx| {
                    terminal.set_size(dimensions);
                    terminal.sync(window, cx);
                    terminal.last_content().clone()
                });

                let (batches, backgrounds) =
                    layout_grid(&content.cells, &text_style, font_size, palette.as_ref(), cx);

                let cursor = content.cursor.point;
                let cursor_char = content.cursor_char;

                LayoutState {
                    hitbox,
                    dimensions,
                    batches,
                    backgrounds,
                    background_color: palette.background,
                    cursor: Some((
                        cursor.column,
                        (cursor.line + content.display_offset as i32).max(0) as usize,
                        cursor_char,
                    )),
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
        let _t0 = Instant::now();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, layout.background_color));
            let origin = layout.dimensions.bounds.origin;

            self.interactivity.paint(
                global_id,
                inspector_id,
                bounds,
                Some(&layout.hitbox),
                window,
                cx,
                |_, window, cx| {
                    for bg in &layout.backgrounds {
                        let rect = Bounds::new(
                            point(
                                origin.x + bg.start_col as f32 * layout.dimensions.cell_width,
                                origin.y + bg.line as f32 * layout.dimensions.line_height,
                            ),
                            size(
                                ((bg.end_col - bg.start_col + 1) as f32)
                                    * layout.dimensions.cell_width,
                                layout.dimensions.line_height,
                            ),
                        );
                        window.paint_quad(fill(rect, bg.color));
                    }

                    for batch in &layout.batches {
                        batch.paint(origin, &layout.dimensions, window, cx);
                    }

                    if self.focused
                        && let Some((col, line, ch)) = layout.cursor
                    {
                        let palette = TerminalPalette::get_global(cx);
                        let cursor_bounds = Bounds::new(
                            point(
                                origin.x + col as f32 * layout.dimensions.cell_width,
                                origin.y + line as f32 * layout.dimensions.line_height,
                            ),
                            size(layout.dimensions.cell_width, layout.dimensions.line_height),
                        );
                        window.paint_quad(fill(cursor_bounds, palette.cursor));
                        // invert-ish glyph
                        let style = TextRun {
                            len: ch.len_utf8(),
                            font: window.text_style().font(),
                            color: palette.background,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let _ = window
                            .text_system()
                            .shape_line(
                                ch.to_string().into(),
                                layout.dimensions.line_height * 0.75, // approx font size
                                &[style],
                                Some(layout.dimensions.cell_width),
                            )
                            .paint(
                                cursor_bounds.origin,
                                layout.dimensions.line_height,
                                gpui::TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                    }
                },
            );
        });
    }
}

impl IntoElement for TermElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

fn layout_grid(
    cells: &[IndexedCell],
    text_style: &TextStyle,
    font_size: Pixels,
    palette: &TerminalPalette,
    _cx: &App,
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

        for indexed in line {
            let cell = &indexed.cell;
            let mut fg = cell.foreground();
            let mut bg = cell.background();
            if cell.is_inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }

            if !is_default_background_color(bg) {
                let color = convert_color(&bg, palette);
                let col = indexed.point.column as i32;
                if let Some(last) = backgrounds.last_mut()
                    && last.color == color
                    && last.line == display_line
                    && last.end_col + 1 == col
                {
                    last.end_col = col;
                } else {
                    backgrounds.push(BgRect {
                        line: display_line,
                        start_col: col,
                        end_col: col,
                        color,
                    });
                }
            }

            if cell.is_wide_char_spacer() {
                continue;
            }
            if is_blank(cell) {
                continue;
            }

            let color = convert_color(&fg, palette);
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
