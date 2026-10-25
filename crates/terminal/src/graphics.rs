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

const MAX_RELATIVE_DEPTH: usize = 8;

/// Where a placement holds its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    /// Anchored to a grid cell carrying its mark, and numbered so the mark can
    /// be walked back to it.
    Grid(u32),
    /// Held only where a [`PLACEHOLDER`] cell names it: no address of its own.
    Virtual,
    /// Drawn in cells from another placement's top-left corner.
    Relative {
        parent: (u32, u32),
        offset: (i32, i32),
    },
}

/// One placement, and the keys that say how it is drawn.
///
/// `(image, placement)` is one concept — a display of it is one placement
/// whichever kind it is — so a single map holds them all and a display of one
/// kind replaces the other kind under the same key.
#[derive(Debug, Clone, Copy)]
struct Held {
    key: (u32, u32),
    anchor: Anchor,
    cols: u16,
    rows: u16,
    frame: Frame,
    source: Source,
    z: i32,
}

pub use crate::placeholder::PLACEHOLDER;

/// Images the terminal is holding, and the placements anchored in its grid.
#[derive(Default)]
pub struct Graphics {
    store: kitty::Store,
    /// Every placement, by `(image, placement id)`.
    held: HashMap<(u32, u32), Held>,
    /// Grid anchor id to the placement key it names, for the reverse walk
    /// from a cell mark.
    anchors: HashMap<u32, (u32, u32)>,
    next_anchor: u32,
    cell: Option<(f32, f32)>,
    /// One decoded frame, by `(image id, frame)`.
    ///
    /// Keyed by frame rather than by image: an animation that alternates
    /// frames otherwise misses on every other step, and each miss re-decodes
    /// the whole frame plus a per-pixel byte swap — 1080p is 8 MB each way.
    /// What validates an entry is the frame's own revision from
    /// [`kitty::Image::frame_revisions`], not the first frame's byte length,
    /// which says nothing about any other frame.
    decoded: HashMap<(u32, usize), (u64, u64, Arc<gpui::RenderImage>)>,
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
                // A placement failure is the emulator's to report, and whether
                // the client hears it is the store's policy, asked once.
                Err(error) => {
                    let failure = kitty::Reply {
                        id: display.image,
                        number: reply.as_ref().map_or(0, |reply| reply.number),
                        placement: display.placement.id,
                        error: Some(error),
                    };
                    failure
                        .answers(display.placement.quiet, display.image)
                        .then_some(failure)
                }
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
        self.locate(term, term.grid().display_offset() as i32)
            .into_iter()
            .map(|located| located.placement)
            .collect()
    }

    /// Placements on screen, decoded into the BGRA buffer gpui paints.
    /// A payload that does not decode is left out.
    pub fn painted<L: EventListener>(&mut self, term: &Term<L>) -> Vec<PaintedImage> {
        let placements = self.placements(term);
        let mut out = Vec::with_capacity(placements.len());
        // One clock read for the whole frame: the placements on screen are
        // painted against the same instant, whether they animate or not.
        let now = Instant::now();
        for placement in placements {
            let Some((width, height)) =
                self.store.get(placement.image).and_then(kitty::Image::size)
            else {
                continue;
            };
            let frame = self.frame_index(placement.image, now).unwrap_or(0);
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

    /// The frame `id` shows at `now`.
    ///
    /// Nothing but the paint path reads this, and it reads only the frame:
    /// whether another one is scheduled was asked and thrown away at the one
    /// production call site, so it is not returned any more. The clock still
    /// tracks the answer, because a frame that repeats is what the gaps say.
    pub fn frame_index(&mut self, id: u32, now: Instant) -> Option<usize> {
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
            return Some(clock.frame);
        }
        // Every advance consumes one frame's gap, so the elapsed time divided
        // by the shortest gap bounds how many there can be — the arithmetic
        // standing in for the round number this loop used to carry.
        let shortest = gaps
            .iter()
            .copied()
            .filter(|gap| *gap != 0)
            .min()
            .unwrap_or(1)
            .max(1);
        let steps = (now.saturating_duration_since(clock.shown_at).as_millis()
            / u128::from(shortest)) as usize;
        for _ in 0..=steps {
            let gap = gap_of(clock.frame);
            let due = clock.shown_at + gap;
            if !gap.is_zero() && now < due {
                return Some(clock.frame);
            }
            let mut next = clock.frame + 1;
            if next == count {
                if animation.state == AnimationState::Loading {
                    clock.waiting = true;
                    return Some(clock.frame);
                }
                if animation.loops != 0 && clock.loops + 1 >= animation.loops {
                    return Some(clock.frame);
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
        Some(clock.frame)
    }

    fn decoded(&mut self, id: u32, frame: usize) -> Option<Arc<gpui::RenderImage>> {
        let image = self.store.get(id)?;
        // The frame's own revision, zero where it has never changed: a frame
        // redrawn by `a=f` bumps it, and a new image under an existing id
        // bumps the image's.
        let frame_revision = image.frame_revisions.get(frame).copied().unwrap_or(0);
        if let Some((revision, cached_frame, cached)) = self.decoded.get(&(id, frame))
            && *revision == image.revision
            && *cached_frame == frame_revision
        {
            return Some(cached.clone());
        }
        let decoded = decode_frame(image, frame)?;
        self.decoded.insert(
            (id, frame),
            (image.revision, frame_revision, decoded.clone()),
        );
        Some(decoded)
    }

    /// Put `held` where it belongs, replacing whatever the key held before.
    ///
    /// The grid anchor a replaced placement was carrying goes with it, so a
    /// mark in the grid can never name a placement that no longer sits there.
    fn insert(&mut self, held: Held) {
        if let Some(was) = self.held.insert(held.key, held)
            && let Anchor::Grid(anchor) = was.anchor
        {
            self.anchors.remove(&anchor);
        }
        if let Anchor::Grid(anchor) = held.anchor {
            self.anchors.insert(anchor, held.key);
        }
    }

    fn place<L: EventListener>(
        &mut self,
        parser: &mut Processor<NoSync>,
        term: &mut Term<L>,
        display: kitty::Display,
    ) -> Result<(), &'static str> {
        if display.placement.parent_image != 0 {
            return self.relate(display);
        }
        let Some((cols, rows, frame, source)) = self.extent(&display) else {
            return Ok(());
        };
        if display.placement.unicode {
            self.insert(Held {
                key: (display.image, display.placement.id),
                anchor: Anchor::Virtual,
                cols: cols.max(1.0).min(u16::MAX as f32) as u16,
                rows: rows.max(1.0).min(u16::MAX as f32) as u16,
                frame,
                source,
                z: display.placement.z,
            });
            return Ok(());
        }
        let cols = (cols as usize).clamp(1, term.columns()) as u16;
        let rows = (rows as usize).clamp(1, term.screen_lines()) as u16;
        self.anchor(parser, term, display, cols, rows, frame, source);
        Ok(())
    }

    fn relate(&mut self, display: kitty::Display) -> Result<(), &'static str> {
        let key = (display.image, display.placement.id);
        let parent = (
            display.placement.parent_image,
            display.placement.parent_placement,
        );
        if display.placement.unicode {
            return Err("EINVAL:a virtual placement cannot be relative");
        }
        if !self.exists(parent) {
            return Err("ENOPARENT");
        }
        let mut depth = 1;
        let mut at = parent;
        while let Some(held) = self.held.get(&at) {
            if at == key {
                return Err("ECYCLE");
            }
            let Anchor::Relative { parent, .. } = held.anchor else {
                break;
            };
            depth += 1;
            if depth > MAX_RELATIVE_DEPTH {
                return Err("ETOODEEP");
            }
            at = parent;
        }
        let Some((cols, rows, frame, source)) = self.extent(&display) else {
            return Ok(());
        };
        self.insert(Held {
            key,
            anchor: Anchor::Relative {
                parent,
                offset: (
                    display.placement.parent_offset_x,
                    display.placement.parent_offset_y,
                ),
            },
            cols: cols.clamp(1.0, u16::MAX as f32) as u16,
            rows: rows.clamp(1.0, u16::MAX as f32) as u16,
            frame,
            source,
            z: display.placement.z,
        });
        Ok(())
    }

    /// Whether anything is held under `key`, whatever kind it is.
    fn exists(&self, key: (u32, u32)) -> bool {
        self.held.contains_key(&key)
    }

    /// Whether `image` is held by any placement, whatever kind it is.
    fn shown(&self, image: u32) -> bool {
        self.held.values().any(|held| held.key.0 == image)
    }

    fn extent(&self, display: &kitty::Display) -> Option<(f32, f32, Frame, Source)> {
        let (cell_w, cell_h) = self.cell?;
        let (width, height) = self.store.get(display.image).and_then(kitty::Image::size)?;
        let x = display.placement.source.x.min(width);
        let y = display.placement.source.y.min(height);
        let source = Source {
            x,
            y,
            width: match display.placement.source.width {
                0 => width - x,
                w => w.min(width - x),
            },
            height: match display.placement.source.height {
                0 => height - y,
                h => h.min(height - y),
            },
        };
        if source.width == 0 || source.height == 0 {
            return None;
        }
        let (source_w, source_h) = (source.width as f32, source.height as f32);
        let offset_x = (display.placement.offset_x as f32)
            .min(cell_w - 1.0)
            .max(0.0);
        let offset_y = (display.placement.offset_y as f32)
            .min(cell_h - 1.0)
            .max(0.0);
        let (cols, rows, frame) = match (display.placement.columns, display.placement.rows) {
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
        let key = (image, display.placement.id);
        let anchor = self.next_anchor % ANCHOR_MAX;
        self.next_anchor = anchor.wrapping_add(1);
        // An id is handed out again once the counter wraps, and whatever it
        // was anchoring is gone with it.
        if let Some(was) = self.anchors.remove(&anchor) {
            self.held.remove(&was);
        }

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
                    if !self.anchors.contains_key(&live) {
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
        self.insert(Held {
            key,
            anchor: Anchor::Grid(anchor),
            cols,
            rows,
            frame,
            source,
            z: display.placement.z,
        });
        if display.placement.cursor_movement == kitty::CursorMovement::After {
            for _ in 0..rows {
                parser.advance(term, b"\n");
            }
        }
    }

    /// Every placement on screen, in the order the paint path draws them:
    /// anchored first, then the virtual runs, then the relatives.
    ///
    /// One walk over the grid and one over the placements, answering one list.
    fn locate<L: EventListener>(&self, term: &Term<L>, offset: i32) -> Vec<Located> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let grid = term.grid();
        let rows = term.screen_lines();
        let cols = term.columns();
        for row in 0..rows {
            let line = Line(row as i32 - offset);
            let mut decoder = self
                .virtuals()
                .next()
                .map(|_| crate::placeholder::RowDecoder::default());
            let mut run: Option<((u32, u32), crate::placeholder::Slot, usize)> = None;
            for col in 0..cols {
                let cell = &grid[line][Column(col)];
                let marks = cell.zerowidth().unwrap_or(&[]);
                for (anchor, within) in anchor_pairs(marks) {
                    let Some(&key) = self.anchors.get(&anchor) else {
                        continue;
                    };
                    let Some(held) = self.held.get(&key) else {
                        continue;
                    };
                    if !seen.insert(anchor) {
                        continue;
                    }
                    let within = within.unwrap_or(0);
                    out.push(Located {
                        key,
                        anchor: held.anchor,
                        placement: Placement {
                            row,
                            col: col.saturating_sub(within),
                            cols: held.cols,
                            rows: held.rows,
                            image: held.key.0,
                            frame: held.frame,
                            source: held.source,
                            z: held.z,
                        },
                    });
                }
                let Some(decoder) = decoder.as_mut() else {
                    continue;
                };
                let shown = decoder
                    .cell(cell)
                    .and_then(|slot| self.shown_by(slot).map(|(key, held)| (slot, key, held)));
                let Some((slot, key, held)) = shown else {
                    run = None;
                    continue;
                };
                if let Some((run_key, last, at)) = &mut run
                    && *run_key == key
                    && last.row == slot.row
                    && last.col + 1 == slot.col
                {
                    out[*at].placement.cols += 1;
                    *last = slot;
                    continue;
                }
                run = Some((key, slot, out.len()));
                out.push(Located {
                    key,
                    anchor: Anchor::Virtual,
                    placement: Placement {
                        row,
                        col,
                        cols: 1,
                        rows: 1,
                        image: key.0,
                        frame: Frame {
                            x: held.frame.x - slot.col as f32,
                            y: held.frame.y - slot.row as f32,
                            ..held.frame
                        },
                        source: held.source,
                        z: held.z,
                    },
                });
            }
        }
        if self.relatives().next().is_none() {
            return out;
        }
        let mut at = HashMap::<(u32, u32), (i64, i64)>::new();
        for located in &out {
            if let Anchor::Grid(_) = located.anchor {
                at.insert(
                    located.key,
                    (located.placement.row as i64, located.placement.col as i64),
                );
            }
        }
        for located in &out {
            if let Anchor::Virtual = located.anchor {
                let spot = at
                    .entry(located.key)
                    .or_insert((located.placement.row as i64, located.placement.col as i64));
                *spot = (
                    spot.0.min(located.placement.row as i64),
                    spot.1.min(located.placement.col as i64),
                );
            }
        }
        for (&key, held) in self.relatives() {
            let Some((row, col)) = self.relative_spot(key, &at) else {
                continue;
            };
            if row < 0 || col < 0 || row >= rows as i64 || col >= cols as i64 {
                continue;
            }
            out.push(Located {
                key,
                anchor: held.anchor,
                placement: Placement {
                    row: row as usize,
                    col: col as usize,
                    cols: held.cols,
                    rows: held.rows,
                    image: held.key.0,
                    frame: held.frame,
                    source: held.source,
                    z: held.z,
                },
            });
        }
        out
    }

    /// Every virtual placement, by key.
    fn virtuals(&self) -> impl Iterator<Item = (&(u32, u32), &Held)> {
        self.held
            .iter()
            .filter(|(_, held)| held.anchor == Anchor::Virtual)
    }

    /// Every relative placement, by key.
    fn relatives(&self) -> impl Iterator<Item = (&(u32, u32), &Held)> {
        self.held
            .iter()
            .filter(|(_, held)| matches!(held.anchor, Anchor::Relative { .. }))
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
            let Some(held) = self.held.get(&key) else {
                return at.get(&key).map(|&(r, c)| (r + row, c + col));
            };
            let Anchor::Relative { parent, offset } = held.anchor else {
                return at.get(&key).map(|&(r, c)| (r + row, c + col));
            };
            row += offset.1 as i64;
            col += offset.0 as i64;
            key = parent;
        }
        None
    }

    /// The virtual placement a slot names, and the key it is held under.
    fn shown_by(&self, slot: crate::placeholder::Slot) -> Option<((u32, u32), Held)> {
        let key = match slot.placement {
            0 => self
                .virtuals()
                .map(|(key, _)| *key)
                .filter(|key| key.0 == slot.image)
                .min()?,
            placement => (slot.image, placement),
        };
        let held = *self.held.get(&key)?;
        (held.anchor == Anchor::Virtual
            && slot.row < held.rows as u32
            && slot.col < held.cols as u32)
            .then_some((key, held))
    }

    fn delete<L: EventListener>(&mut self, term: &Term<L>, delete: kitty::Delete) {
        use kitty::Target;
        if delete.target == Target::All && delete.free {
            self.held.clear();
            self.anchors.clear();
            self.decoded.clear();
            self.store.clear();
            return;
        }
        // The keys this delete names: by image and placement, or by the cells
        // they cover. A virtual placement has no position of its own, so a
        // delete by position never reaches one.
        let named: Vec<(u32, u32)> = if delete.target.names_images() {
            self.held
                .iter()
                .filter(|(key, held)| {
                    let z = match held.anchor {
                        Anchor::Virtual => None,
                        Anchor::Grid(_) | Anchor::Relative { .. } => Some(held.z),
                    };
                    delete.target.matches(key.0, key.1, z)
                })
                .map(|(key, _)| *key)
                .collect()
        } else {
            let cursor = term.grid().cursor.point;
            self.locate(term, 0)
                .into_iter()
                .filter(|located| located.anchor != Anchor::Virtual)
                .filter(|located| {
                    let p = &located.placement;
                    match delete.target {
                        Target::All => true,
                        Target::Cursor => Target::covers(
                            delete.target,
                            p,
                            cursor.column.0 as u32,
                            cursor.line.0.max(0) as u32,
                        ),
                        Target::Cell { col, row, z } => {
                            Target::covers(delete.target, p, col, row) && z.is_none_or(|z| p.z == z)
                        }
                        Target::Column(col) => {
                            (p.col..p.col + p.cols as usize).contains(&(col as usize))
                        }
                        Target::Row(row) => {
                            (p.row..p.row + p.rows as usize).contains(&(row as usize))
                        }
                        // By-name targets took the branch above; the arm
                        // exists so the partition is stated on both sides.
                        Target::Image { .. } | Target::Range(..) | Target::Z(_) => false,
                    }
                })
                .map(|located| located.key)
                .collect()
        };
        let mut touched = Vec::new();
        for key in named {
            if let Some(held) = self.held.remove(&key) {
                touched.push(held.key.0);
                if let Anchor::Grid(anchor) = held.anchor {
                    self.anchors.remove(&anchor);
                }
            }
        }
        // A relative whose parent is gone has nothing to be relative to.
        loop {
            let orphans: Vec<(u32, u32)> = self
                .relatives()
                .filter(|(_, held)| {
                    matches!(held.anchor, Anchor::Relative { parent, .. } if !self.exists(parent))
                })
                .map(|(key, _)| *key)
                .collect();
            if orphans.is_empty() {
                break;
            }
            for key in orphans {
                if let Some(held) = self.held.remove(&key)
                    && !self.shown(held.key.0)
                {
                    self.decoded.retain(|(id, _), _| *id != held.key.0);
                    self.store.remove(held.key.0);
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
                self.decoded.retain(|(id, _), _| *id != image);
                self.store.remove(image);
            }
        }
    }
}

/// One placement on screen, and the key it is held under.
#[derive(Clone, Copy)]
struct Located {
    key: (u32, u32),
    anchor: Anchor,
    placement: Placement,
}

#[derive(Clone, Copy)]
struct AnimClock {
    revision: u64,
    frame: usize,
    shown_at: Instant,
    loops: u32,
    waiting: bool,
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
