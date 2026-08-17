use std::os::fd::AsRawFd;
use std::{borrow::Cow, io, ops::RangeInclusive, path::PathBuf, sync::Arc};

mod hyperlinks;

use alacritty_terminal::{
    event::{Event as AlacTermEvent, EventListener, Notify, WindowSize},
    event_loop::{EventLoop, Msg, Notifier},
    grid::{Dimensions, GridIterator, Scroll as AlacScroll},
    index::{Boundary, Column, Direction as AlacDirection, Line, Point as AlacPoint},
    selection::{
        Selection as AlacSelection, SelectionRange as AlacSelectionRange,
        SelectionType as AlacSelectionType,
    },
    sync::FairMutex,
    term::{
        Config, RenderableCursor, SEMANTIC_ESCAPE_CHARS, Term, TermMode,
        cell::{Cell as AlacCell, Flags, Hyperlink as AlacHyperlink},
        search::{Match, RegexIter, RegexSearch},
    },
    tty,
    vi_mode::{ViModeCursor, ViMotion as AlacViMotion},
    vte::ansi::{
        ClearMode, CursorShape as AlacCursorShape, CursorStyle as AlacCursorStyle,
        NamedPrivateMode, PrivateMode,
    },
};
use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
use util::paths::PathStyle;
use vte::ansi::Handler;

use crate::{
    Cell, Color, Content, Cursor, CursorShape, Hyperlink, HyperlinkData, IndexedCell, Modes, Point,
    PtyEvent, Range, RenderableCells, Scroll, Search, Selection, SelectionRange, SelectionSide,
    SelectionType, TerminalBackendEvent, TerminalBounds, ViMotion,
    pty_info::ProcessIdGetter,
    terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape},
};

pub(super) use hyperlinks::{HyperlinkMatch, RegexSearches};

pub(super) type AlacrittyPty = tty::Pty;
pub(super) type AlacrittyTerm = Term<ZedListener>;
pub(super) type AlacrittyTermConfig = Config;
pub(super) type AlacrittyTermLock = FairMutex<AlacrittyTerm>;
pub(super) type AlacrittyCell = AlacCell;
pub(super) type AlacrittyGridIterator<'a> = GridIterator<'a, AlacCell>;
pub(super) type AlacrittyHyperlink = AlacHyperlink;

#[derive(Clone)]
pub(super) struct ZedListener(UnboundedSender<PtyEvent>);

#[derive(Clone, Debug)]
pub(super) struct AlacrittySearch {
    search: RegexSearch,
}

impl From<&AlacrittyPty> for ProcessIdGetter {
    fn from(pty: &AlacrittyPty) -> Self {
        Self::new(pty.file().as_raw_fd(), pty.child().id())
    }
}

pub(super) struct PtySender {
    notifier: Notifier,
}

impl PtySender {
    pub(super) fn notify(&self, input: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(input);
    }

    pub(super) fn resize(&self, bounds: TerminalBounds) {
        if let Err(error) = self
            .notifier
            .0
            .send(Msg::Resize(window_size_from_terminal_bounds(bounds)))
        {
            log::error!("failed to resize alacritty pty: {error}");
        }
    }

    pub(super) fn shutdown(&self) {
        if let Err(error) = self.notifier.0.send(Msg::Shutdown) {
            log::debug!("failed to shut down alacritty pty loop: {error}");
        }
    }
}

fn window_size_from_terminal_bounds(bounds: TerminalBounds) -> WindowSize {
    WindowSize {
        num_lines: bounds.num_lines() as u16,
        num_cols: bounds.num_columns() as u16,
        cell_width: f32::from(bounds.cell_width()) as u16,
        cell_height: f32::from(bounds.line_height()) as u16,
    }
}

pub(super) fn pty_term_config(
    scrolling_history: usize,
    cursor_shape: SettingsCursorShape,
) -> AlacrittyTermConfig {
    Config {
        scrolling_history,
        default_cursor_style: alacritty_cursor_style(cursor_shape),
        semantic_escape_chars: format!("{SEMANTIC_ESCAPE_CHARS}─"),
        ..Config::default()
    }
}

pub(super) fn current_child_signal_mask() -> io::Result<tty::SignalMask> {
    tty::SignalMask::current()
}

pub(super) fn pty_options(
    shell: Option<(String, Vec<String>)>,
    working_directory: Option<PathBuf>,
    env: impl IntoIterator<Item = (String, String)>,
    child_signal_mask: Option<tty::SignalMask>,
) -> tty::Options {
    tty::Options {
        shell: shell.map(|(program, args)| tty::Shell::new(program, args)),
        working_directory,
        drain_on_exit: true,
        env: env.into_iter().collect(),
        child_signal_mask,
    }
}

pub(super) fn open_pty(
    options: &tty::Options,
    bounds: TerminalBounds,
    window_id: u64,
) -> io::Result<AlacrittyPty> {
    tty::new(options, window_size_from_terminal_bounds(bounds), window_id)
}

pub(super) fn new_term(
    config: &AlacrittyTermConfig,
    bounds: TerminalBounds,
    events_tx: UnboundedSender<PtyEvent>,
    alternate_scroll: AlternateScroll,
) -> Arc<AlacrittyTermLock> {
    let mut term = Term::new(config.clone(), &bounds, ZedListener(events_tx));

    if let AlternateScroll::Off = alternate_scroll {
        term.unset_private_mode(PrivateMode::Named(NamedPrivateMode::AlternateScroll));
    }

    Arc::new(FairMutex::new(term))
}

pub(super) fn spawn_event_loop(
    term: Arc<AlacrittyTermLock>,
    events_tx: UnboundedSender<PtyEvent>,
    pty: AlacrittyPty,
    drain_on_exit: bool,
) -> Result<PtySender> {
    let event_loop = EventLoop::new(term, ZedListener(events_tx), pty, drain_on_exit, false)
        .context("failed to create event loop")?;
    let pty_tx = event_loop.channel();
    let _io_thread = event_loop.spawn();

    Ok(PtySender {
        notifier: Notifier(pty_tx),
    })
}

pub(super) fn resize(term: &mut AlacrittyTerm, bounds: TerminalBounds) {
    term.resize(bounds);
}

pub(super) fn display_offset(term: &AlacrittyTerm) -> usize {
    term.grid().display_offset()
}

pub(super) fn scroll_display(term: &mut AlacrittyTerm, scroll: Scroll) {
    term.scroll_display(scroll.to_alacritty());
}

pub(super) fn set_selection(term: &mut AlacrittyTerm, selection: Option<&Selection>) {
    term.selection = selection.map(Selection::to_alacritty);
}

pub(super) fn update_selection(
    term: &mut AlacrittyTerm,
    point: Point,
    side: SelectionSide,
) -> bool {
    let Some(mut selection) = term.selection.take() else {
        return false;
    };
    selection.update(point.to_alacritty(), side.to_alacritty());
    term.selection = Some(selection);
    true
}

pub(super) fn selection_text(term: &AlacrittyTerm) -> Option<String> {
    term.selection_to_string()
}

pub(super) fn scroll_to_point(term: &mut AlacrittyTerm, point: Point) {
    term.scroll_to_point(point.to_alacritty());
}

pub(super) fn vi_goto_point(term: &mut AlacrittyTerm, point: Point) {
    term.vi_goto_point(point.to_alacritty());
}

pub(super) fn toggle_vi_mode(term: &mut AlacrittyTerm) {
    term.toggle_vi_mode();
}

pub(super) fn vi_motion(term: &mut AlacrittyTerm, motion: ViMotion) {
    term.vi_motion(motion.to_alacritty());
}

fn alacritty_cursor_style(cursor_shape: SettingsCursorShape) -> AlacCursorStyle {
    AlacCursorStyle {
        shape: alacritty_cursor_shape(cursor_shape),
        blinking: false,
    }
}

fn alacritty_cursor_shape(cursor_shape: SettingsCursorShape) -> AlacCursorShape {
    match cursor_shape {
        SettingsCursorShape::Block => AlacCursorShape::Block,
        SettingsCursorShape::Underline => AlacCursorShape::Underline,
        SettingsCursorShape::Bar => AlacCursorShape::Beam,
        SettingsCursorShape::Hollow => AlacCursorShape::HollowBlock,
    }
}

impl Dimensions for TerminalBounds {
    /// Note: this is supposed to be for the back buffer's length,
    /// but we exclusively use it to resize the terminal, which does not
    /// use this method. We still have to implement it for the trait though,
    /// hence, this comment.
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.num_lines()
    }

    fn columns(&self) -> usize {
        self.num_columns()
    }
}

impl From<AlacTermEvent> for TerminalBackendEvent {
    fn from(event: AlacTermEvent) -> Self {
        match event {
            AlacTermEvent::MouseCursorDirty => Self::MouseCursorDirty,
            AlacTermEvent::Title(title) => Self::Title(title),
            AlacTermEvent::ResetTitle => Self::ResetTitle,
            AlacTermEvent::ClipboardStore(_, data) => Self::ClipboardStore(data),
            AlacTermEvent::ClipboardLoad(_, format) => Self::ClipboardLoad(format),
            AlacTermEvent::ColorRequest(index, format) => Self::ColorRequest(index, format),
            AlacTermEvent::PtyWrite(output) => Self::PtyWrite(output),
            AlacTermEvent::TextAreaSizeRequest(format) => {
                Self::TextAreaSizeRequest(Arc::new(move |bounds| {
                    format(window_size_from_terminal_bounds(bounds))
                }))
            }
            AlacTermEvent::CursorBlinkingChange => Self::CursorBlinkingChange,
            AlacTermEvent::Wakeup => Self::Wakeup,
            AlacTermEvent::Bell => Self::Bell,
            AlacTermEvent::Exit => Self::Exit,
            AlacTermEvent::ChildExit(status) => Self::ChildExit(status),
            AlacTermEvent::Osc133(kind) => Self::Osc133(kind),
            AlacTermEvent::DesktopNotification(msg) => Self::DesktopNotification(msg),
        }
    }
}

impl EventListener for ZedListener {
    fn send_event(&self, event: AlacTermEvent) {
        self.0.unbounded_send(PtyEvent::Event(event.into())).ok();
    }
}

impl Scroll {
    fn to_alacritty(self) -> AlacScroll {
        match self {
            Self::Delta(delta) => AlacScroll::Delta(delta),
            Self::PageUp => AlacScroll::PageUp,
            Self::PageDown => AlacScroll::PageDown,
            Self::Top => AlacScroll::Top,
            Self::Bottom => AlacScroll::Bottom,
        }
    }
}

impl ViMotion {
    fn to_alacritty(self) -> AlacViMotion {
        match self {
            Self::Up => AlacViMotion::Up,
            Self::Down => AlacViMotion::Down,
            Self::Left => AlacViMotion::Left,
            Self::Right => AlacViMotion::Right,
            Self::First => AlacViMotion::First,
            Self::Last => AlacViMotion::Last,
            Self::FirstOccupied => AlacViMotion::FirstOccupied,
            Self::High => AlacViMotion::High,
            Self::Middle => AlacViMotion::Middle,
            Self::Low => AlacViMotion::Low,
            Self::WordLeft => AlacViMotion::WordLeft,
            Self::WordRight => AlacViMotion::WordRight,
            Self::WordRightEnd => AlacViMotion::WordRightEnd,
            Self::Bracket => AlacViMotion::Bracket,
            Self::ParagraphUp => AlacViMotion::ParagraphUp,
            Self::ParagraphDown => AlacViMotion::ParagraphDown,
        }
    }
}

impl Search {
    pub fn new(search: &str) -> Option<Self> {
        Some(Self {
            search: AlacrittySearch {
                search: RegexSearch::new(search).ok()?,
            },
        })
    }

    fn into_alacritty(self) -> RegexSearch {
        self.search.search
    }
}

impl SelectionSide {
    fn to_alacritty(self) -> AlacDirection {
        match self {
            Self::Left => AlacDirection::Left,
            Self::Right => AlacDirection::Right,
        }
    }
}

impl SelectionType {
    fn to_alacritty(self) -> AlacSelectionType {
        match self {
            Self::Simple => AlacSelectionType::Simple,
            Self::Semantic => AlacSelectionType::Semantic,
            Self::Lines => AlacSelectionType::Lines,
        }
    }
}

impl Selection {
    fn to_alacritty(&self) -> AlacSelection {
        let mut selection = AlacSelection::new(
            self.ty.to_alacritty(),
            self.start.point.to_alacritty(),
            self.start.side.to_alacritty(),
        );
        if self.start.point != self.end.point || self.start.side != self.end.side {
            selection.update(self.end.point.to_alacritty(), self.end.side.to_alacritty());
        }
        selection
    }
}

impl Hyperlink {
    pub fn new<T: ToString>(id: Option<T>, uri: String) -> Self {
        Self {
            data: HyperlinkData::Owned {
                id: id.map(|id| Arc::from(id.to_string())),
                uri: Arc::from(uri),
            },
        }
    }

    pub fn id(&self) -> Option<&str> {
        match &self.data {
            HyperlinkData::Alacritty(hyperlink) => Some(hyperlink.id()),
            HyperlinkData::Owned { id, .. } => id.as_deref(),
        }
    }

    pub fn uri(&self) -> &str {
        match &self.data {
            HyperlinkData::Alacritty(hyperlink) => hyperlink.uri(),
            HyperlinkData::Owned { uri, .. } => uri,
        }
    }

    fn from_alacritty(hyperlink: AlacHyperlink) -> Self {
        Self {
            data: HyperlinkData::Alacritty(hyperlink),
        }
    }
}

fn terminal_hyperlink_from_alacritty(hyperlink: AlacHyperlink) -> Hyperlink {
    Hyperlink::from_alacritty(hyperlink)
}

impl From<Hyperlink> for AlacHyperlink {
    fn from(hyperlink: Hyperlink) -> Self {
        match hyperlink.data {
            HyperlinkData::Alacritty(hyperlink) => hyperlink,
            HyperlinkData::Owned { id, uri } => Self::new(id.as_deref(), uri.to_string()),
        }
    }
}

fn terminal_cell_from_alacritty(cell: &AlacCell) -> Cell {
    Cell { cell: cell.clone() }
}

impl Cell {
    #[inline]
    pub fn character(&self) -> char {
        self.cell.c
    }

    #[inline]
    pub fn foreground(&self) -> Color {
        self.cell.fg
    }

    #[inline]
    pub fn background(&self) -> Color {
        self.cell.bg
    }

    #[inline]
    pub fn zerowidth(&self) -> Option<&[char]> {
        self.cell.zerowidth()
    }

    #[cfg(test)]
    pub(crate) fn push_zerowidth(&mut self, character: char) {
        self.cell.push_zerowidth(character);
    }

    #[inline]
    pub fn hyperlink(&self) -> Option<Hyperlink> {
        self.cell.hyperlink().map(terminal_hyperlink_from_alacritty)
    }

    #[inline]
    pub fn is_inverse(&self) -> bool {
        self.cell.flags.contains(Flags::INVERSE)
    }

    #[inline]
    pub fn is_wide_char_spacer(&self) -> bool {
        self.cell.flags.contains(Flags::WIDE_CHAR_SPACER)
    }

    #[inline]
    pub fn is_dim(&self) -> bool {
        self.cell.flags.intersects(Flags::DIM)
    }

    #[inline]
    pub fn has_underline(&self) -> bool {
        self.cell.flags.intersects(Flags::ALL_UNDERLINES)
    }

    #[inline]
    pub fn has_undercurl(&self) -> bool {
        self.cell.flags.contains(Flags::UNDERCURL)
    }

    #[inline]
    pub fn has_strikeout(&self) -> bool {
        self.cell.flags.intersects(Flags::STRIKEOUT)
    }

    #[inline]
    pub fn is_bold(&self) -> bool {
        self.cell.flags.intersects(Flags::BOLD)
    }

    #[inline]
    pub fn is_italic(&self) -> bool {
        self.cell.flags.intersects(Flags::ITALIC)
    }

    #[inline]
    pub fn has_visible_style_modifier(&self) -> bool {
        self.cell
            .flags
            .intersects(Flags::ALL_UNDERLINES | Flags::INVERSE | Flags::STRIKEOUT)
    }
}

impl<'a> RenderableCells<'a> {
    pub(super) fn new(cells: GridIterator<'a, AlacCell>) -> Self {
        Self { cells }
    }
}

impl Iterator for RenderableCells<'_> {
    type Item = IndexedCell;

    fn next(&mut self) -> Option<Self::Item> {
        self.cells.next().map(|cell| IndexedCell {
            point: terminal_point_from_alacritty(cell.point),
            cell: terminal_cell_from_alacritty(cell.cell),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.cells.size_hint()
    }
}

impl Modes {
    #[cfg(test)]
    fn to_alacritty(self) -> TermMode {
        let mut mode = TermMode::empty();
        add_alacritty_mode(&mut mode, self, Self::APP_CURSOR, TermMode::APP_CURSOR);
        add_alacritty_mode(&mut mode, self, Self::APP_KEYPAD, TermMode::APP_KEYPAD);
        add_alacritty_mode(&mut mode, self, Self::SHOW_CURSOR, TermMode::SHOW_CURSOR);
        add_alacritty_mode(&mut mode, self, Self::LINE_WRAP, TermMode::LINE_WRAP);
        add_alacritty_mode(&mut mode, self, Self::ORIGIN, TermMode::ORIGIN);
        add_alacritty_mode(&mut mode, self, Self::INSERT, TermMode::INSERT);
        add_alacritty_mode(
            &mut mode,
            self,
            Self::LINE_FEED_NEW_LINE,
            TermMode::LINE_FEED_NEW_LINE,
        );
        add_alacritty_mode(&mut mode, self, Self::FOCUS_IN_OUT, TermMode::FOCUS_IN_OUT);
        add_alacritty_mode(
            &mut mode,
            self,
            Self::ALTERNATE_SCROLL,
            TermMode::ALTERNATE_SCROLL,
        );
        add_alacritty_mode(
            &mut mode,
            self,
            Self::BRACKETED_PASTE,
            TermMode::BRACKETED_PASTE,
        );
        add_alacritty_mode(&mut mode, self, Self::SGR_MOUSE, TermMode::SGR_MOUSE);
        add_alacritty_mode(&mut mode, self, Self::UTF8_MOUSE, TermMode::UTF8_MOUSE);
        add_alacritty_mode(&mut mode, self, Self::ALT_SCREEN, TermMode::ALT_SCREEN);
        add_alacritty_mode(
            &mut mode,
            self,
            Self::MOUSE_REPORT_CLICK,
            TermMode::MOUSE_REPORT_CLICK,
        );
        add_alacritty_mode(&mut mode, self, Self::MOUSE_DRAG, TermMode::MOUSE_DRAG);
        add_alacritty_mode(&mut mode, self, Self::MOUSE_MOTION, TermMode::MOUSE_MOTION);
        add_alacritty_mode(&mut mode, self, Self::VI, TermMode::VI);
        mode
    }
}

fn terminal_modes_from_alacritty(mode: TermMode) -> Modes {
    let mut terminal_modes = Modes::empty();
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::APP_CURSOR,
        Modes::APP_CURSOR,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::APP_KEYPAD,
        Modes::APP_KEYPAD,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::SHOW_CURSOR,
        Modes::SHOW_CURSOR,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::LINE_WRAP,
        Modes::LINE_WRAP,
    );
    add_terminal_mode(&mut terminal_modes, mode, TermMode::ORIGIN, Modes::ORIGIN);
    add_terminal_mode(&mut terminal_modes, mode, TermMode::INSERT, Modes::INSERT);
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::LINE_FEED_NEW_LINE,
        Modes::LINE_FEED_NEW_LINE,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::FOCUS_IN_OUT,
        Modes::FOCUS_IN_OUT,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::ALTERNATE_SCROLL,
        Modes::ALTERNATE_SCROLL,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::BRACKETED_PASTE,
        Modes::BRACKETED_PASTE,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::SGR_MOUSE,
        Modes::SGR_MOUSE,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::UTF8_MOUSE,
        Modes::UTF8_MOUSE,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::ALT_SCREEN,
        Modes::ALT_SCREEN,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::MOUSE_REPORT_CLICK,
        Modes::MOUSE_REPORT_CLICK,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::MOUSE_DRAG,
        Modes::MOUSE_DRAG,
    );
    add_terminal_mode(
        &mut terminal_modes,
        mode,
        TermMode::MOUSE_MOTION,
        Modes::MOUSE_MOTION,
    );
    add_terminal_mode(&mut terminal_modes, mode, TermMode::VI, Modes::VI);
    terminal_modes
}

fn add_terminal_mode(
    terminal_modes: &mut Modes,
    alacritty_modes: TermMode,
    alacritty_mode: TermMode,
    terminal_mode: Modes,
) {
    if alacritty_modes.contains(alacritty_mode) {
        terminal_modes.insert(terminal_mode);
    }
}

#[cfg(test)]
fn add_alacritty_mode(
    alacritty_modes: &mut TermMode,
    terminal_modes: Modes,
    terminal_mode: Modes,
    alacritty_mode: TermMode,
) {
    if terminal_modes.contains(terminal_mode) {
        alacritty_modes.insert(alacritty_mode);
    }
}

impl Cursor {
    fn from_alacritty(cursor: RenderableCursor) -> Self {
        Self {
            shape: terminal_cursor_shape_from_alacritty(cursor.shape),
            point: terminal_point_from_alacritty(cursor.point),
        }
    }
}

fn terminal_cursor_shape_from_alacritty(shape: AlacCursorShape) -> CursorShape {
    match shape {
        AlacCursorShape::Block => CursorShape::Block,
        AlacCursorShape::Underline => CursorShape::Underline,
        AlacCursorShape::Beam => CursorShape::Bar,
        AlacCursorShape::HollowBlock => CursorShape::HollowBlock,
        AlacCursorShape::Hidden => CursorShape::Hidden,
    }
}

impl Point {
    fn to_alacritty(self) -> AlacPoint {
        AlacPoint::new(Line(self.line), Column(self.column))
    }
}

fn terminal_point_from_alacritty(point: AlacPoint) -> Point {
    Point {
        line: point.line.0,
        column: point.column.0,
    }
}

impl Range {
    fn from_alacritty(range: RangeInclusive<AlacPoint>) -> Self {
        Self {
            start: terminal_point_from_alacritty(*range.start()),
            end: terminal_point_from_alacritty(*range.end()),
        }
    }
}

fn terminal_selection_range_from_alacritty(range: AlacSelectionRange) -> SelectionRange {
    SelectionRange {
        start: terminal_point_from_alacritty(range.start),
        end: terminal_point_from_alacritty(range.end),
        is_block: range.is_block,
    }
}

pub(super) fn clear_saved_screen(term: &mut Term<ZedListener>) {
    term.clear_screen(ClearMode::Saved);

    let cursor = term.grid().cursor.point;

    term.grid_mut().reset_region(..cursor.line);

    let line = term.grid()[cursor.line][..Column(term.grid().columns())]
        .iter()
        .cloned()
        .enumerate()
        .collect::<Vec<(usize, AlacCell)>>();

    for (index, cell) in line {
        term.grid_mut()[Line(0)][Column(index)] = cell;
    }

    term.grid_mut().cursor.point = AlacPoint::new(Line(0), term.grid_mut().cursor.point.column);
    let new_cursor = term.grid().cursor.point;

    if (new_cursor.line.0 as usize) < term.screen_lines() - 1 {
        term.grid_mut().reset_region((new_cursor.line + 1)..);
    }
}

pub(super) fn make_content(term: &Term<ZedListener>, last_content: &Content) -> Content {
    let content = term.renderable_content();

    let estimated_size = content.display_iter.size_hint().0;
    let mut cells = Vec::with_capacity(estimated_size);

    cells.extend(content.display_iter.map(|ic| IndexedCell {
        point: terminal_point_from_alacritty(ic.point),
        cell: terminal_cell_from_alacritty(ic.cell),
    }));

    let selection_text = if content.selection.is_some() {
        term.selection_to_string()
    } else {
        None
    };

    let bottom_line = term.screen_lines() as i32 - 1 - content.display_offset as i32;
    let bottom_row_occupied = content.cursor.point.line.0 >= bottom_line
        || cells
            .iter()
            .rev()
            .take_while(|cell| cell.point.line >= bottom_line)
            .any(|cell| cell.cell.character() != ' ');

    Content {
        cells,
        mode: terminal_modes_from_alacritty(content.mode),
        display_offset: content.display_offset,
        selection_text,
        selection: content
            .selection
            .map(terminal_selection_range_from_alacritty),
        cursor: Cursor::from_alacritty(content.cursor),
        cursor_char: term.grid()[content.cursor.point].c,
        terminal_bounds: last_content.terminal_bounds,
        last_hovered_word: last_content.last_hovered_word.clone(),
        scrolled_to_top: content.display_offset == term.history_size(),
        scrolled_to_bottom: content.display_offset == 0,
        bottom_row_occupied,
    }
}

pub(super) fn content_text(term: &Term<ZedListener>) -> String {
    let start = AlacPoint::new(term.topmost_line(), Column(0));
    let end = AlacPoint::new(term.bottommost_line(), term.last_column());
    term.bounds_to_string(start, end)
}

/// Inclusive grid range as plain text. Used to recover the command line
/// between an OSC 133 B marker and the cursor at C.
pub(super) fn grid_text_range(
    term: &Term<ZedListener>,
    start_line: i32,
    start_col: usize,
    end_line: i32,
    end_col: usize,
) -> String {
    let start = AlacPoint::new(Line(start_line), Column(start_col));
    let end = AlacPoint::new(Line(end_line), Column(end_col));
    term.bounds_to_string(start, end)
}

/// The currently visible screen (scrollback excluded), for accessibility.
pub(super) fn visible_screen_text(term: &Term<ZedListener>) -> String {
    let offset = term.grid().display_offset() as i32;
    let screen = term.screen_lines() as i32;
    // Visible rows in grid coordinates: the top row is `-offset` (0 = the
    // bottom of the scrollback / top of the viewport when scrolled to bottom).
    let top = Line(-offset);
    let bottom = Line((-offset + screen - 1).max(top.0));
    let start = AlacPoint::new(top, Column(0));
    let end = AlacPoint::new(bottom, term.last_column());
    term.bounds_to_string(start, end)
}

pub(super) fn total_lines(term: &Term<ZedListener>) -> usize {
    term.total_lines()
}

pub(super) fn screen_lines(term: &Term<ZedListener>) -> usize {
    term.screen_lines()
}

pub(super) fn full_content_range(term: &Term<ZedListener>) -> Range {
    let start = AlacPoint::new(term.topmost_line(), Column(0));
    let end = AlacPoint::new(term.bottommost_line(), term.last_column());
    Range::from_alacritty(start..=end)
}

pub(super) fn update_vi_cursor_for_scroll(term: &mut Term<ZedListener>, scroll: Scroll) {
    match scroll {
        Scroll::Delta(delta) => {
            term.vi_mode_cursor = term.vi_mode_cursor.scroll(term, delta);
        }
        Scroll::PageUp => {
            let lines = term.screen_lines() as i32;
            term.vi_mode_cursor = term.vi_mode_cursor.scroll(term, lines);
        }
        Scroll::PageDown => {
            let lines = -(term.screen_lines() as i32);
            term.vi_mode_cursor = term.vi_mode_cursor.scroll(term, lines);
        }
        Scroll::Top => {
            let point = AlacPoint::new(term.topmost_line(), Column(0));
            term.vi_mode_cursor = ViModeCursor::new(point);
        }
        Scroll::Bottom => {
            let point = AlacPoint::new(term.bottommost_line(), Column(0));
            term.vi_mode_cursor = ViModeCursor::new(point);
        }
    }
}

pub(super) fn update_selection_to_vi_cursor(term: &mut Term<ZedListener>) -> Option<Point> {
    let mut selection = term.selection.take()?;
    let point = term.vi_mode_cursor.point;
    selection.update(point, AlacDirection::Right);
    term.selection = Some(selection);
    Some(terminal_point_from_alacritty(point))
}

pub(super) fn find_from_terminal_point(
    term: &AlacrittyTerm,
    point: Point,
    regex_searches: &mut RegexSearches,
    path_style: PathStyle,
) -> Option<HyperlinkMatch> {
    let point = point.to_alacritty().grid_clamp(term, Boundary::Grid);
    hyperlinks::find_from_grid_point(term, point, regex_searches, path_style)
}

pub(super) fn search_matches(term: &AlacrittyTerm, searcher: Search) -> Vec<Range> {
    let mut searcher = searcher.into_alacritty();
    all_search_matches(term, &mut searcher)
        .map(Range::from_alacritty)
        .collect()
}

fn all_search_matches<'a, T>(
    term: &'a Term<T>,
    regex: &'a mut RegexSearch,
) -> impl Iterator<Item = Match> + 'a {
    let start = AlacPoint::new(term.grid().topmost_line(), Column(0));
    let end = AlacPoint::new(term.grid().bottommost_line(), term.grid().last_column());
    RegexIter::new(start, end, AlacDirection::Right, term, regex)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn terminal_hyperlink_from_alacritty_keeps_alacritty_storage() {
        let hyperlink = AlacHyperlink::new(Some("id"), "https://example.com".to_string());
        let hyperlink = terminal_hyperlink_from_alacritty(hyperlink);

        assert!(matches!(&hyperlink.data, HyperlinkData::Alacritty(_)));
        assert_eq!(hyperlink.id(), Some("id"));
        assert_eq!(hyperlink.uri(), "https://example.com");
    }

    #[test]
    fn terminal_cell_from_alacritty_shares_extra_storage() {
        let mut cell = AlacCell::default();
        cell.push_zerowidth('a');

        let converted = terminal_cell_from_alacritty(&cell);

        match (&cell.extra, &converted.cell.extra) {
            (Some(extra), Some(converted_extra)) => assert!(Arc::ptr_eq(extra, converted_extra)),
            _ => panic!("expected extra storage on both cells"),
        }
    }

    #[test]
    fn terminal_modes_round_trip_alacritty_flags() {
        let alacritty_modes = TermMode::APP_CURSOR
            | TermMode::BRACKETED_PASTE
            | TermMode::ALT_SCREEN
            | TermMode::MOUSE_DRAG
            | TermMode::SGR_MOUSE
            | TermMode::VI;

        let terminal_modes = terminal_modes_from_alacritty(alacritty_modes);
        assert!(terminal_modes.contains(Modes::APP_CURSOR));
        assert!(terminal_modes.contains(Modes::BRACKETED_PASTE));
        assert!(terminal_modes.contains(Modes::ALT_SCREEN));
        assert!(terminal_modes.contains(Modes::MOUSE_DRAG));
        assert!(terminal_modes.intersects(Modes::MOUSE_MODE));
        assert!(terminal_modes.contains(Modes::SGR_MOUSE));
        assert!(terminal_modes.contains(Modes::VI));
        assert!(!terminal_modes.contains(Modes::MOUSE_REPORT_CLICK));

        let alacritty_modes = terminal_modes.to_alacritty();
        assert!(alacritty_modes.contains(TermMode::APP_CURSOR));
        assert!(alacritty_modes.contains(TermMode::BRACKETED_PASTE));
        assert!(alacritty_modes.contains(TermMode::ALT_SCREEN));
        assert!(alacritty_modes.contains(TermMode::MOUSE_DRAG));
        assert!(alacritty_modes.contains(TermMode::SGR_MOUSE));
        assert!(alacritty_modes.contains(TermMode::VI));
        assert!(!alacritty_modes.contains(TermMode::MOUSE_REPORT_CLICK));
    }

    #[test]
    fn terminal_selection_range_round_trip_alacritty_range() {
        let alacritty_range = AlacSelectionRange {
            start: AlacPoint::new(Line(-2), Column(3)),
            end: AlacPoint::new(Line(4), Column(8)),
            is_block: true,
        };

        let terminal_range = terminal_selection_range_from_alacritty(alacritty_range);
        assert_eq!(
            terminal_range,
            SelectionRange {
                start: Point {
                    line: -2,
                    column: 3
                },
                end: Point { line: 4, column: 8 },
                is_block: true,
            }
        );
    }

    #[test]
    fn semantic_selection_stops_at_tree_branch() {
        let config = pty_term_config(1000, SettingsCursorShape::default());
        let (events_tx, _events_rx) = futures::channel::mpsc::unbounded();
        let mut term = Term::new(config, &TerminalBounds::default(), ZedListener(events_tx));
        for character in "└─zms-demo.target".chars() {
            term.input(character);
        }

        let selection = Selection::new(
            SelectionType::Semantic,
            Point::new(0, 2),
            SelectionSide::Left,
        );
        set_selection(&mut term, Some(&selection));

        assert_eq!(selection_text(&term).as_deref(), Some("zms-demo.target"));
    }

    #[test]
    fn visible_screen_text_excludes_scrollback() {
        let config = pty_term_config(1000, SettingsCursorShape::default());
        let (events_tx, _events_rx) = futures::channel::mpsc::unbounded();
        let mut term = Term::new(config, &TerminalBounds::default(), ZedListener(events_tx));
        // The debug screen is 100×6 cells; 130 five-char lines overflow it and
        // scroll the first rows off.
        for i in 0..130 {
            for c in format!("line{i}").chars() {
                term.input(c);
            }
        }
        let text = visible_screen_text(&term);
        assert!(text.contains("line129"), "last line visible: {text:?}");
        assert!(!text.contains("line0"), "first line scrolled off: {text:?}");
    }

    #[test]
    fn search_inline_flags_override_smart_case() {
        let config = pty_term_config(1000, SettingsCursorShape::default());
        let (events_tx, _events_rx) = futures::channel::mpsc::unbounded();
        let mut term = Term::new(config, &TerminalBounds::default(), ZedListener(events_tx));
        for character in "Hello WORLD".chars() {
            term.input(character);
        }

        // `(?i)` forces case-insensitive even though the query has uppercase,
        // defeating alacritty's smart-case (which would otherwise be sensitive).
        let insensitive = Search::new("(?i)HELLO").expect("valid regex");
        assert!(!search_matches(&term, insensitive).is_empty());

        // `(?-i)` forces case-sensitive: lowercase query misses the capital H…
        let sensitive = Search::new("(?-i)hello").expect("valid regex");
        assert!(search_matches(&term, sensitive).is_empty());

        // …and the correctly-cased query matches.
        let exact = Search::new("(?-i)Hello").expect("valid regex");
        assert!(!search_matches(&term, exact).is_empty());
    }

    #[test]
    fn maps_osc133_and_desktop_notification_events() {
        match TerminalBackendEvent::from(AlacTermEvent::Osc133("A".into())) {
            TerminalBackendEvent::Osc133(kind) => assert_eq!(kind, "A"),
            other => panic!("unexpected {other:?}"),
        }
        match TerminalBackendEvent::from(AlacTermEvent::DesktopNotification("hi".into())) {
            TerminalBackendEvent::DesktopNotification(msg) => assert_eq!(msg, "hi"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn processor_emits_osc133_and_notify_through_zed_listener() {
        use vte::ansi::{Processor, StdSyncHandler};

        let config = pty_term_config(1000, SettingsCursorShape::default());
        let (events_tx, mut events_rx) = futures::channel::mpsc::unbounded();
        let mut term = Term::new(config, &TerminalBounds::default(), ZedListener(events_tx));
        let mut processor = Processor::<StdSyncHandler>::new();
        processor.advance(&mut term, b"\x1b]133;A\x07");
        processor.advance(&mut term, b"\x1b]9;hello\x07");
        processor.advance(&mut term, b"\x1b]777;notify;done\x07");

        let mut kinds = Vec::new();
        let mut notes = Vec::new();
        while let Ok(PtyEvent::Event(ev)) = events_rx.try_recv() {
            match ev {
                TerminalBackendEvent::Osc133(k) => kinds.push(k),
                TerminalBackendEvent::DesktopNotification(m) => notes.push(m),
                _ => {}
            }
        }
        assert_eq!(kinds, ["A"]);
        assert_eq!(notes, ["hello", "done"]);
    }
}
