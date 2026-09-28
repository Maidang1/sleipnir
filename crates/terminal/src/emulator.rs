//! A headless terminal: pty bytes in, grid and images out.
//!
//! The live app drives the same [`crate::graphics::feed`] from its pty thread.
//! This wrapper exists so the graphics protocol can be tested without one.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::Selection;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{CursorShape, Processor};

use crate::graphics::{self, Graphics, NoSync, PaintedImage};
use crate::kitty;
use crate::scanner::Scanner;

pub use crate::graphics::{Frame, Placement, Source};

/// Kept so a placement scrolled off the screen can be scrolled back.
const SCROLLBACK_LINES: usize = 10_000;

/// Viewport size passed to `Term::new`.
#[derive(Debug, Clone, Copy)]
pub struct GridSize {
    cols: u16,
    rows: u16,
}

impl GridSize {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.max(2),
            rows: rows.max(1),
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// Cursor position in viewport coordinates. Row 0 is the top of the visible grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Default)]
struct Silent;

impl EventListener for Silent {
    fn send_event(&self, _: Event) {}
}

/// One terminal's grid, parser, and graphics store.
pub struct Emulator {
    term: Term<Silent>,
    parser: Processor<NoSync>,
    scanner: Scanner,
    graphics: Graphics,
}

impl Emulator {
    pub fn new(cols: u16, rows: u16) -> Self {
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &GridSize::new(cols, rows), Silent),
            parser: Processor::new(),
            scanner: Scanner::new(),
            graphics: Graphics::new(),
        }
    }

    pub fn set_cell_size(&mut self, width: f32, height: f32) {
        self.graphics.set_cell_size(width, height);
    }

    pub fn set_local_media(&mut self, allow: bool) {
        self.graphics.set_local_media(allow);
    }

    pub fn graphics(&self) -> &kitty::Store {
        self.graphics.store()
    }

    /// Advance over `bytes`. The returned bytes are graphics replies.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        graphics::feed(
            &mut self.parser,
            &mut self.scanner,
            &mut self.term,
            &mut self.graphics,
            bytes,
        )
        .replies
    }

    pub fn placements(&self) -> Vec<Placement> {
        self.graphics.placements(&self.term)
    }

    pub fn painted(&mut self) -> Vec<PaintedImage> {
        self.graphics.painted(&self.term)
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(GridSize::new(cols, rows));
    }

    pub fn scroll(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn grid_point(&self, viewport_row: usize, col: usize) -> Point {
        Point::new(
            Line(viewport_row as i32 - self.term.grid().display_offset() as i32),
            Column(col.min(self.term.columns().saturating_sub(1))),
        )
    }

    pub fn start_selection(&mut self, ty: SelectionType, point: Point, side: Side) {
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    pub fn update_selection(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .map(|text| text.replace(graphics::is_anchor, ""))
            .filter(|text| !text.is_empty())
    }

    pub fn cursor(&self) -> Option<CursorSnapshot> {
        let content = self.term.renderable_content();
        if content.cursor.shape == CursorShape::Hidden {
            return None;
        }
        let point = content.cursor.point;
        let row = point.line.0 + self.term.grid().display_offset() as i32;
        if row < 0 || row >= self.term.screen_lines() as i32 {
            return None;
        }
        Some(CursorSnapshot {
            row: row as usize,
            col: point.column.0,
        })
    }

    pub fn row_text(&self, viewport_row: usize) -> String {
        let grid = self.term.grid();
        let line = Line(viewport_row as i32 - grid.display_offset() as i32);
        let mut text = String::new();
        for col in 0..grid.columns() {
            let cell = &grid[line][Column(col)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            text.push(cell.c);
        }
        while text.ends_with(' ') {
            text.pop();
        }
        text
    }
}

// Re-export the selection vocabulary the graphics tests name.
pub use alacritty_terminal::index::Side;
pub use alacritty_terminal::selection::SelectionType;
