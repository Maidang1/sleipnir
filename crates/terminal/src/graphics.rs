//! Kitty placements anchored in grid cells.
//!
//! An image's position is a private-use zerowidth character written onto the
//! cells of its top row. Those characters travel with the cell through
//! scrollback and reflow, so a side table of line numbers is never rebased.
//! The table here only remembers what the grid cannot: which image, and how
//! large it is.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Processor;
use vte::ansi::Timeout;

use crate::kitty::{self, Command, Effect};
use crate::scanner::{Scanner, Segment};

/// How long a mode-2026 hold may suppress a redraw. Matches vte's own ceiling.
pub const HOLD_TIMEOUT: Duration = Duration::from_millis(150);

/// Turns off vte's synchronized-update buffer.
///
/// With the buffer on, text after `CSI ? 2026 h` is replayed at the end of
/// the hold, while a graphics command has already left the byte stream. The
/// image would land at the cursor from before that text. Reporting no pending
/// timeout keeps every byte on one path. The byte loop suppresses the redraw
/// for [`HOLD_TIMEOUT`] instead, so the grid still updates in order.
#[derive(Debug, Default)]
pub struct NoSync;

impl Timeout for NoSync {
    fn set_timeout(&mut self, _: Duration) {}
    fn clear_timeout(&mut self) {}
    fn pending_timeout(&self) -> bool {
        false
    }
}

/// Where an image sits on the visible grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub row: usize,
    pub col: usize,
    pub cols: u16,
    pub rows: u16,
    pub image: u32,
    pub frame: Frame,
    pub source: Source,
    pub z: i32,
}

/// A rectangle in cells. Edges fall inside a cell when an offset or a
/// letterbox puts them there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A rectangle in an image's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One image ready to paint. `z` below zero sits under the text.
#[derive(Clone)]
pub struct PaintedImage {
    pub row: usize,
    pub col: usize,
    pub frame: Frame,
    pub source: Source,
    pub z: i32,
    pub image: Arc<gpui::RenderImage>,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

const ANCHOR: u32 = 0xF_0000;
const ANCHOR_MAX: u32 = 0xF_FFFD - ANCHOR;
const ANCHOR_COLUMN: u32 = 0x10_0000;
const ANCHOR_COLUMN_MAX: u32 = 256;

#[derive(Debug, Clone, Copy)]
struct Placed {
    image: u32,
    placement: u32,
    cols: u16,
    rows: u16,
    frame: Frame,
    source: Source,
    z: i32,
}

struct Virtual {
    cols: u16,
    rows: u16,
    frame: Frame,
    source: Source,
    z: i32,
}

struct Relative {
    parent: (u32, u32),
    offset: (i32, i32),
    cols: u16,
    rows: u16,
    frame: Frame,
    source: Source,
    z: i32,
}

const MAX_RELATIVE_DEPTH: usize = 8;

pub use crate::placeholder::PLACEHOLDER;

/// Images the terminal is holding, and the placements anchored in its grid.
#[derive(Default)]
pub struct Graphics {
    store: kitty::Store,
    placed: HashMap<u32, Placed>,
    virtuals: HashMap<(u32, u32), Virtual>,
    relatives: HashMap<(u32, u32), Relative>,
    next_anchor: u32,
    cell: Option<(f32, f32)>,
    decoded: HashMap<u32, (u64, usize, usize, Arc<gpui::RenderImage>)>,
    clocks: HashMap<u32, AnimClock>,
}

impl Graphics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&self) -> &kitty::Store {
        &self.store
    }

    pub fn set_cell_size(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.cell = Some((width, height));
        }
    }

    pub fn set_local_media(&mut self, allow: bool) {
        self.store.set_local_media(allow);
    }

    /// Carry out one graphics command. The reply is what the client asked to
    /// hear back on the pty.
    pub fn handle<L: EventListener>(
        &mut self,
        parser: &mut Processor<NoSync>,
        term: &mut Term<L>,
        command: Command,
    ) -> Option<Vec<u8>> {
        let (effect, reply) = self.store.apply(command);
        let reply = match effect {
            Some(Effect::Display(display)) => match self.place(parser, term, display) {
                Ok(()) => reply,
                Err(error) => (display.quiet < 2).then(|| kitty::Reply {
                    id: display.image,
                    number: reply.as_ref().map_or(0, |reply| reply.number),
                    placement: display.placement,
                    error: Some(error),
                }),
            },
            Some(Effect::Delete(delete)) => {
                self.delete(term, delete);
                reply
            }
            None => reply,
        };
        reply.map(|reply| reply.bytes())
    }

    pub fn placements<L: EventListener>(&self, term: &Term<L>) -> Vec<Placement> {
        let located = self.locate(term, term.grid().display_offset() as i32);
        let mut out = Vec::new();
        out.extend(located.anchored.into_iter().map(|(_, placement)| placement));
        out.extend(located.pieces.into_iter().map(|(_, placement)| placement));
        out.extend(
            located
                .relatives
                .into_iter()
                .map(|(_, placement)| placement),
        );
        out
    }

    /// Placements on screen, decoded into the BGRA buffer gpui paints.
    /// A payload that does not decode is left out.
    pub fn painted<L: EventListener>(&mut self, term: &Term<L>) -> Vec<PaintedImage> {
        let placements = self.placements(term);
        let mut out = Vec::with_capacity(placements.len());
        for placement in placements {
            let Some((width, height)) =
                self.store.get(placement.image).and_then(kitty::Image::size)
            else {
                continue;
            };
            let frame = self
                .frame_index(placement.image, Instant::now())
                .map(|(index, _)| index)
                .unwrap_or(0);
            let Some(decoded) = self.decoded(placement.image, frame) else {
                continue;
            };
            out.push(PaintedImage {
                row: placement.row,
                col: placement.col,
                frame: placement.frame,
                source: placement.source,
                z: placement.z,
                image: decoded,
                pixel_width: width,
                pixel_height: height,
            });
        }
        out
    }

    /// The frame `id` shows at `now`, and whether another frame is scheduled.
    pub fn frame_index(&mut self, id: u32, now: Instant) -> Option<(usize, bool)> {
        use kitty::AnimationState;
        let (animation, count, gaps) = {
            let image = self.store.get(id)?;
            (image.animation, image.frame_count(), image.gaps.clone())
        };
        if count == 0 {
            return None;
        }
        let start = AnimClock {
            revision: animation.revision,
            frame: animation.current.min(count - 1),
            shown_at: now,
            loops: 0,
            waiting: false,
        };
        let clock = self.clocks.entry(id).or_insert(start);
        if clock.revision != animation.revision {
            *clock = start;
        }
        clock.frame = clock.frame.min(count - 1);
        let gap_of =
            |frame: usize| Duration::from_millis(gaps.get(frame).copied().unwrap_or(0) as u64);
        if animation.state == AnimationState::Stopped
            || count == 1
            || (0..count).all(|frame| gap_of(frame).is_zero())
        {
            return Some((clock.frame, false));
        }
        for _ in 0..10_000 {
            let gap = gap_of(clock.frame);
            let due = clock.shown_at + gap;
            if !gap.is_zero() && now < due {
                return Some((clock.frame, true));
            }
            let mut next = clock.frame + 1;
            if next == count {
                if animation.state == AnimationState::Loading {
                    clock.waiting = true;
                    return Some((clock.frame, false));
                }
                if animation.loops != 0 && clock.loops + 1 >= animation.loops {
                    return Some((clock.frame, false));
                }
                clock.loops += 1;
                next = 0;
            }
            clock.frame = next;
            clock.shown_at = if std::mem::take(&mut clock.waiting) {
                now
            } else {
                due
            };
        }
        clock.shown_at = now;
        Some((clock.frame, true))
    }

    fn decoded(&mut self, id: u32, frame: usize) -> Option<Arc<gpui::RenderImage>> {
        let image = self.store.get(id)?;
        let key = (image.revision, image.bytes.len(), frame);
        if let Some((revision, len, cached_frame, cached)) = self.decoded.get(&id)
            && *revision == key.0
            && *len == key.1
            && *cached_frame == key.2
        {
            return Some(cached.clone());
        }
        let decoded = decode_frame(image, frame)?;
        self.decoded
            .insert(id, (key.0, key.1, key.2, decoded.clone()));
        Some(decoded)
    }

    fn place<L: EventListener>(
        &mut self,
        parser: &mut Processor<NoSync>,
        term: &mut Term<L>,
        display: kitty::Display,
    ) -> Result<(), &'static str> {
        if display.parent_image != 0 {
            return self.relate(display);
        }
        let Some((cols, rows, frame, source)) = self.extent(&display) else {
            return Ok(());
        };
        let key = (display.image, display.placement);
        if display.unicode {
            self.placed
                .retain(|_, placed| (placed.image, placed.placement) != key);
            self.relatives.remove(&key);
            self.virtuals.insert(
                key,
                Virtual {
                    cols: cols.max(1.0).min(u16::MAX as f32) as u16,
                    rows: rows.max(1.0).min(u16::MAX as f32) as u16,
                    frame,
                    source,
                    z: display.z,
                },
            );
            return Ok(());
        }
        let cols = (cols as usize).clamp(1, term.columns()) as u16;
        let rows = (rows as usize).clamp(1, term.screen_lines()) as u16;
        self.anchor(parser, term, display, cols, rows, frame, source);
        Ok(())
    }

    fn relate(&mut self, display: kitty::Display) -> Result<(), &'static str> {
        let key = (display.image, display.placement);
        let parent = (display.parent_image, display.parent_placement);
        if display.unicode {
            return Err("EINVAL:a virtual placement cannot be relative");
        }
        if !self.exists(parent) {
            return Err("ENOPARENT");
        }
        let mut depth = 1;
        let mut at = parent;
        loop {
            if at == key {
                return Err("ECYCLE");
            }
            match self.relatives.get(&at) {
                Some(relative) => {
                    depth += 1;
                    at = relative.parent;
                }
                None => break,
            }
        }
        if depth > MAX_RELATIVE_DEPTH {
            return Err("ETOODEEP");
        }
        let Some((cols, rows, frame, source)) = self.extent(&display) else {
            return Ok(());
        };
        self.placed
            .retain(|_, placed| (placed.image, placed.placement) != key);
        self.virtuals.remove(&key);
        self.relatives.insert(
            key,
            Relative {
                parent,
                offset: (display.parent_offset_x, display.parent_offset_y),
                cols: cols.clamp(1.0, u16::MAX as f32) as u16,
                rows: rows.clamp(1.0, u16::MAX as f32) as u16,
                frame,
                source,
                z: display.z,
            },
        );
        Ok(())
    }

    fn exists(&self, key: (u32, u32)) -> bool {
        self.placed
            .values()
            .any(|placed| (placed.image, placed.placement) == key)
            || self.virtuals.contains_key(&key)
            || self.relatives.contains_key(&key)
    }

    fn shown(&self, image: u32) -> bool {
        self.placed.values().any(|placed| placed.image == image)
            || self.virtuals.keys().any(|held| held.0 == image)
            || self.relatives.keys().any(|held| held.0 == image)
    }

    fn extent(&self, display: &kitty::Display) -> Option<(f32, f32, Frame, Source)> {
        let (cell_w, cell_h) = self.cell?;
        let (width, height) = self.store.get(display.image).and_then(kitty::Image::size)?;
        let x = display.source_x.min(width);
        let y = display.source_y.min(height);
        let source = Source {
            x,
            y,
            width: match display.source_width {
                0 => width - x,
                w => w.min(width - x),
            },
            height: match display.source_height {
                0 => height - y,
                h => h.min(height - y),
            },
        };
        if source.width == 0 || source.height == 0 {
            return None;
        }
        let (source_w, source_h) = (source.width as f32, source.height as f32);
        let offset_x = (display.offset_x as f32).min(cell_w - 1.0).max(0.0);
        let offset_y = (display.offset_y as f32).min(cell_h - 1.0).max(0.0);
        let (cols, rows, frame) = match (display.columns, display.rows) {
            (0, 0) => (
                ((offset_x + source_w) / cell_w).ceil(),
                ((offset_y + source_h) / cell_h).ceil(),
                (offset_x, offset_y, source_w, source_h),
            ),
            (c, 0) => {
                let w = c as f32 * cell_w;
                let h = w * source_h / source_w;
                (c as f32, (h / cell_h).ceil(), (offset_x, offset_y, w, h))
            }
            (0, r) => {
                let h = r as f32 * cell_h;
                let w = h * source_w / source_h;
                ((w / cell_w).ceil(), r as f32, (offset_x, offset_y, w, h))
            }
            (c, r) if display.stretch => {
                let (w, h) = (c as f32 * cell_w, r as f32 * cell_h);
                (c as f32, r as f32, (offset_x, offset_y, w, h))
            }
            (c, r) => {
                let (box_w, box_h) = (c as f32 * cell_w, r as f32 * cell_h);
                let scale = (box_w / source_w).min(box_h / source_h);
                let (w, h) = (source_w * scale, source_h * scale);
                let x = offset_x + (box_w - w) / 2.0;
                let y = offset_y + (box_h - h) / 2.0;
                (c as f32, r as f32, (x, y, w, h))
            }
        };
        let frame = Frame {
            x: frame.0 / cell_w,
            y: frame.1 / cell_h,
            width: frame.2 / cell_w,
            height: frame.3 / cell_h,
        };
        Some((cols, rows, frame, source))
    }

    fn anchor<L: EventListener>(
        &mut self,
        parser: &mut Processor<NoSync>,
        term: &mut Term<L>,
        display: kitty::Display,
        cols: u16,
        rows: u16,
        frame: Frame,
        source: Source,
    ) {
        let image = display.image;
        self.placed
            .retain(|_, placed| placed.image != image || placed.placement != display.placement);
        let anchor = self.next_anchor % ANCHOR_MAX;
        self.next_anchor = anchor.wrapping_add(1);
        self.placed.remove(&anchor);

        let cursor = term.grid().cursor.point;
        let width = (cols as usize)
            .min(ANCHOR_COLUMN_MAX as usize)
            .min(term.columns() - cursor.column.0);
        for offset in 0..width {
            let cell = &mut term.grid_mut()[cursor.line][Column(cursor.column.0 + offset)];
            if cell
                .zerowidth()
                .is_some_and(|marks| marks.iter().any(|&mark| is_anchor(mark)))
            {
                let marks = cell.zerowidth().unwrap_or(&[]);
                let mut kept: Vec<char> = marks
                    .iter()
                    .copied()
                    .filter(|&mark| !is_anchor(mark))
                    .collect();
                for (live, column) in anchor_pairs(marks) {
                    if !self.placed.contains_key(&live) {
                        continue;
                    }
                    kept.extend(char::from_u32(ANCHOR + live));
                    kept.extend(
                        column.and_then(|column| char::from_u32(ANCHOR_COLUMN + column as u32)),
                    );
                }
                cell.extra = None;
                for mark in kept {
                    cell.push_zerowidth(mark);
                }
            }
            for mark in [ANCHOR + anchor, ANCHOR_COLUMN + offset as u32] {
                if let Some(mark) = char::from_u32(mark) {
                    cell.push_zerowidth(mark);
                }
            }
        }
        self.placed.insert(
            anchor,
            Placed {
                image,
                placement: display.placement,
                cols,
                rows,
                frame,
                source,
                z: display.z,
            },
        );
        if display.cursor_movement == kitty::CursorMovement::After {
            for _ in 0..rows {
                parser.advance(term, b"\n");
            }
        }
    }

    fn locate<L: EventListener>(&self, term: &Term<L>, offset: i32) -> Located {
        let mut anchored: Vec<(u32, Placement)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut pieces: Vec<((u32, u32), Placement)> = Vec::new();
        let grid = term.grid();
        let rows = term.screen_lines();
        let cols = term.columns();
        for row in 0..rows {
            let line = Line(row as i32 - offset);
            let mut decoder =
                (!self.virtuals.is_empty()).then(crate::placeholder::RowDecoder::default);
            let mut run: Option<((u32, u32), crate::placeholder::Slot, usize)> = None;
            for col in 0..cols {
                let cell = &grid[line][Column(col)];
                let marks = cell.zerowidth().unwrap_or(&[]);
                for (anchor, within) in anchor_pairs(marks) {
                    let Some(placed) = self.placed.get(&anchor) else {
                        continue;
                    };
                    if !seen.insert(anchor) {
                        continue;
                    }
                    let within = within.unwrap_or(0);
                    anchored.push((
                        anchor,
                        Placement {
                            row,
                            col: col.saturating_sub(within),
                            cols: placed.cols,
                            rows: placed.rows,
                            image: placed.image,
                            frame: placed.frame,
                            source: placed.source,
                            z: placed.z,
                        },
                    ));
                }
                let Some(decoder) = decoder.as_mut() else {
                    continue;
                };
                let shown = decoder.cell(cell).and_then(|slot| {
                    self.shown_by(slot)
                        .map(|(key, virtual_)| (slot, key, virtual_))
                });
                let Some((slot, key, virtual_)) = shown else {
                    run = None;
                    continue;
                };
                if let Some((run_key, last, at)) = &mut run
                    && *run_key == key
                    && last.row == slot.row
                    && last.col + 1 == slot.col
                {
                    pieces[*at].1.cols += 1;
                    *last = slot;
                    continue;
                }
                run = Some((key, slot, pieces.len()));
                pieces.push((
                    key,
                    Placement {
                        row,
                        col,
                        cols: 1,
                        rows: 1,
                        image: key.0,
                        frame: Frame {
                            x: virtual_.frame.x - slot.col as f32,
                            y: virtual_.frame.y - slot.row as f32,
                            ..virtual_.frame
                        },
                        source: virtual_.source,
                        z: virtual_.z,
                    },
                ));
            }
        }
        let mut relatives = Vec::new();
        if !self.relatives.is_empty() {
            let mut at = HashMap::<(u32, u32), (i64, i64)>::new();
            for (anchor, placement) in &anchored {
                if let Some(placed) = self.placed.get(anchor) {
                    at.insert(
                        (placed.image, placed.placement),
                        (placement.row as i64, placement.col as i64),
                    );
                }
            }
            for (key, placement) in &pieces {
                let spot = at
                    .entry(*key)
                    .or_insert((placement.row as i64, placement.col as i64));
                *spot = (
                    spot.0.min(placement.row as i64),
                    spot.1.min(placement.col as i64),
                );
            }
            for (&key, _) in &self.relatives {
                let Some((row, col)) = self.relative_spot(key, &at) else {
                    continue;
                };
                if row < 0 || col < 0 || row >= rows as i64 || col >= cols as i64 {
                    continue;
                }
                let relative = &self.relatives[&key];
                relatives.push((
                    key,
                    Placement {
                        row: row as usize,
                        col: col as usize,
                        cols: relative.cols,
                        rows: relative.rows,
                        image: key.0,
                        frame: relative.frame,
                        source: relative.source,
                        z: relative.z,
                    },
                ));
            }
        }
        Located {
            anchored,
            pieces,
            relatives,
        }
    }

    fn relative_spot(
        &self,
        key: (u32, u32),
        at: &HashMap<(u32, u32), (i64, i64)>,
    ) -> Option<(i64, i64)> {
        let mut row = 0i64;
        let mut col = 0i64;
        let mut key = key;
        for _ in 0..=MAX_RELATIVE_DEPTH {
            let Some(relative) = self.relatives.get(&key) else {
                return at.get(&key).map(|&(r, c)| (r + row, c + col));
            };
            row += relative.offset.1 as i64;
            col += relative.offset.0 as i64;
            key = relative.parent;
        }
        None
    }

    fn shown_by(&self, slot: crate::placeholder::Slot) -> Option<((u32, u32), &Virtual)> {
        let key = match slot.placement {
            0 => self
                .virtuals
                .keys()
                .filter(|image| image.0 == slot.image)
                .min()
                .copied()?,
            placement => (slot.image, placement),
        };
        let virtual_ = self.virtuals.get(&key)?;
        (slot.row < virtual_.rows as u32 && slot.col < virtual_.cols as u32)
            .then_some((key, virtual_))
    }

    fn delete<L: EventListener>(&mut self, term: &Term<L>, delete: kitty::Delete) {
        use kitty::Target;
        if delete.target == Target::All && delete.free {
            self.placed.clear();
            self.virtuals.clear();
            self.relatives.clear();
            self.decoded.clear();
            self.store.clear();
            return;
        }
        let covers = |p: &Placement, col: u32, row: u32| {
            let (col, row) = (col as usize, row as usize);
            (p.col..p.col + p.cols as usize).contains(&col)
                && (p.row..p.row + p.rows as usize).contains(&row)
        };
        let names = |image: u32, placement: u32, z: i32| match delete.target {
            Target::Image { id, placement: p } => Some(image == id && (p == 0 || placement == p)),
            Target::Range(low, high) => Some((low..=high).contains(&image)),
            Target::Z(want) => Some(z == want),
            _ => None,
        };
        let by_name = matches!(
            delete.target,
            Target::Image { .. } | Target::Range(..) | Target::Z(_)
        );
        let mut anchors = Vec::new();
        let mut keys = Vec::new();
        if by_name {
            anchors.extend(
                self.placed
                    .iter()
                    .filter(|(_, p)| names(p.image, p.placement, p.z) == Some(true))
                    .map(|(&anchor, _)| anchor),
            );
            keys.extend(
                self.relatives
                    .iter()
                    .filter(|(key, relative)| names(key.0, key.1, relative.z) == Some(true))
                    .map(|(&key, _)| key),
            );
        } else {
            let cursor = term.grid().cursor.point;
            let hit = |p: &Placement| match delete.target {
                Target::Cursor => covers(p, cursor.column.0 as u32, cursor.line.0.max(0) as u32),
                Target::Cell { col, row, z } => covers(p, col, row) && z.is_none_or(|z| p.z == z),
                Target::Column(col) => (p.col..p.col + p.cols as usize).contains(&(col as usize)),
                Target::Row(row) => (p.row..p.row + p.rows as usize).contains(&(row as usize)),
                _ => true,
            };
            let located = self.locate(term, 0);
            anchors.extend(
                located
                    .anchored
                    .into_iter()
                    .filter(|(_, placement)| hit(placement))
                    .map(|(anchor, _)| anchor),
            );
            keys.extend(
                located
                    .relatives
                    .into_iter()
                    .filter(|(_, placement)| hit(placement))
                    .map(|(key, _)| key),
            );
        }
        let mut touched = Vec::new();
        if !matches!(delete.target, Target::Z(_)) && by_name {
            self.virtuals.retain(|&(image, placement), _| {
                let hit = names(image, placement, 0) == Some(true);
                if hit {
                    touched.push(image);
                }
                !hit
            });
        }
        touched.extend(
            anchors
                .iter()
                .filter_map(|anchor| self.placed.remove(anchor))
                .map(|placed| placed.image),
        );
        for key in keys {
            if self.relatives.remove(&key).is_some() {
                touched.push(key.0);
            }
        }
        loop {
            let orphans: Vec<(u32, u32)> = self
                .relatives
                .iter()
                .filter(|(_, relative)| !self.exists(relative.parent))
                .map(|(&key, _)| key)
                .collect();
            if orphans.is_empty() {
                break;
            }
            for key in orphans {
                self.relatives.remove(&key);
                if !self.shown(key.0) {
                    self.decoded.remove(&key.0);
                    self.store.remove(key.0);
                }
            }
        }
        if !delete.free {
            return;
        }
        if let Target::Image { id, .. } = delete.target {
            touched.push(id);
        }
        for image in touched {
            if !self.shown(image) {
                self.decoded.remove(&image);
                self.store.remove(image);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AnimClock {
    revision: u64,
    frame: usize,
    shown_at: Instant,
    loops: u32,
    waiting: bool,
}

struct Located {
    anchored: Vec<(u32, Placement)>,
    pieces: Vec<((u32, u32), Placement)>,
    relatives: Vec<((u32, u32), Placement)>,
}

/// What one chunk of pty output asked the host to do.
pub struct Fed {
    pub replies: Vec<u8>,
    /// Mode 2026 edges in the order they arrived. `true` begins a hold.
    pub syncs: Vec<bool>,
}

/// Split `bytes` and advance `term` over the text, carrying graphics out on
/// `graphics`. Replies are the bytes to write back to the pty.
pub fn feed<L: EventListener>(
    parser: &mut Processor<NoSync>,
    scanner: &mut Scanner,
    term: &mut Term<L>,
    graphics: &mut Graphics,
    bytes: &[u8],
) -> Fed {
    let mut replies = Vec::new();
    let mut syncs = Vec::new();
    for segment in scanner.feed(bytes) {
        match segment {
            Segment::Text(text) => parser.advance(term, text),
            Segment::Graphics(command) => {
                if let Some(reply) = graphics.handle(parser, term, command) {
                    replies.extend(reply);
                }
            }
            Segment::Sync(hold) => syncs.push(hold),
            Segment::Sixel(_)
            | Segment::Iterm(_)
            | Segment::Directory(_)
            | Segment::CellSizeQuery => {}
        }
    }
    Fed { replies, syncs }
}

/// Whether `ch` is one of the private-use codepoints a placement anchors with.
pub fn is_anchor(ch: char) -> bool {
    let ch = ch as u32;
    (ANCHOR..ANCHOR + ANCHOR_MAX).contains(&ch)
        || (ANCHOR_COLUMN..ANCHOR_COLUMN + ANCHOR_COLUMN_MAX).contains(&ch)
}

fn anchor_pairs(marks: &[char]) -> impl Iterator<Item = (u32, Option<usize>)> + '_ {
    marks.iter().enumerate().filter_map(|(at, &mark)| {
        let anchor = (mark as u32).wrapping_sub(ANCHOR);
        if anchor >= ANCHOR_MAX {
            return None;
        }
        let column = marks.get(at + 1).and_then(|&next| {
            let index = (next as u32).wrapping_sub(ANCHOR_COLUMN);
            (index < ANCHOR_COLUMN_MAX).then_some(index as usize)
        });
        Some((anchor, column))
    })
}

/// One still frame as the BGRA picture [`gpui::RenderImage`] stores.
pub fn decode_image(image: &kitty::Image) -> Option<Arc<gpui::RenderImage>> {
    decode_frame(image, 0)
}

fn decode_frame(image: &kitty::Image, index: usize) -> Option<Arc<gpui::RenderImage>> {
    let rgba = if index == 0 {
        crate::pixels::Rgba::decode(image.format, image.width, image.height, &image.bytes)?
    } else {
        let bytes = image.frame(index)?;
        crate::pixels::Rgba::decode(kitty::Format::Rgba, image.width, image.height, bytes)?
    };
    let mut buffer = image::RgbaImage::from_raw(rgba.width, rgba.height, rgba.bytes)?;
    {
        let pixels: &mut [u8] = &mut buffer;
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Some(Arc::new(gpui::RenderImage::new([image::Frame::new(
        buffer,
    )])))
}
