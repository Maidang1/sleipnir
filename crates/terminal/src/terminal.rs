mod mappings;

mod alacritty;
mod osc133;
mod osc_notify;
mod pty_info;
mod run_tracker;
mod shell_semantics;
pub mod terminal_settings;

pub use osc_notify::{OscNotify, OscNotifyScanner, scan_osc_notify};
pub use osc133::{Osc133Kind, Osc133Marker, Osc133Scanner, scan_osc133};
pub use run_tracker::{RunTracker, TrackerOut, UNRECOGNIZED_COMMAND, normalize_command};
pub use shell_semantics::{
    ClickToMove, InjectShell, TripleClickKind, absolute_to_grid_line, apply_inject_to_shell,
    click_to_move_sequence, command_output_range, inject_script, triple_click_kind,
    wrap_shell_for_inject, wrap_shell_for_inject_in,
};

use anyhow::{Context as _, Result, bail};
use futures_lite::future::yield_now;
use log::trace;

use futures::{
    FutureExt,
    channel::mpsc::{UnboundedReceiver, unbounded},
};

use alacritty_terminal::grid::Dimensions as _;
use itertools::Itertools as _;
use mappings::mouse::{
    alt_scroll, grid_point, grid_point_and_side, mouse_button_report, mouse_moved_report,
    scroll_report,
};

use async_channel::{Receiver, Sender};
use collections::{HashMap, VecDeque};
use futures::StreamExt;
use pty_info::{ProcessIdGetter, PtyProcessInfo};
use serde::{Deserialize, Serialize};
use sleipnir_settings::{TerminalPalette, get_color_at_index as palette_get_color};
use task_types::{HideStrategy, Shell, ShellKind, SpawnInTerminal};
use terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape, TerminalSettings};
use urlencoding;
use util::{ResultExt as _, paths::PathStyle, truncate_and_trailoff};

use std::os::unix::process::ExitStatusExt;
use std::{
    borrow::Cow,
    cmp::{self, min},
    fmt::{self, Display, Formatter},
    future::Future,
    ops::{BitOr, BitOrAssign, Deref, Range as StdRange},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use vte::ansi::{Attr, Handler, Processor, StdSyncHandler};
pub use vte::ansi::{Color, NamedColor, Rgb};

use gpui::{
    App, AppContext as _, BackgroundExecutor, Bounds, ClipboardItem, Context, EventEmitter, Hsla,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point as GpuiPoint, ScrollWheelEvent, Size, Task, TouchPhase, Window, actions, px,
};

use crate::alacritty::{
    current_child_signal_mask,
    AlacrittyCell, AlacrittyGridIterator, AlacrittyHyperlink, AlacrittySearch, AlacrittyTerm,
    AlacrittyTermConfig, AlacrittyTermLock, HyperlinkMatch, PtySender, RegexSearches,
    append_text_to_term, apply_config, clear_saved_screen, content_text, display_offset,
    display_only_term_config, find_from_terminal_point, full_content_range, grid_text_range,
    last_non_empty_lines,
    make_content, new_term, open_pty, pty_options, pty_term_config, resize, screen_lines,
    scroll_display, scroll_to_point, search_matches, selection_text, set_default_cursor_style,
    set_selection as set_term_selection, shrink_to_used, spawn_event_loop,
    toggle_vi_mode as toggle_term_vi_mode, total_lines, update_selection as update_term_selection,
    update_selection_to_vi_cursor, update_vi_cursor_for_scroll, vi_goto_point, vi_motion,
    visible_screen_text as term_visible_screen_text,
};
use crate::mappings::colors::to_vte_rgb;
use crate::mappings::keys::to_esc_str;

/// How long the shell and its foreground job get to exit gracefully after a
/// closed terminal sends SIGHUP/SIGTERM, before being SIGKILLed. Must stay
/// comfortably below [`gpui::SHUTDOWN_TIMEOUT`] so the escalation also
/// completes when the whole app is quitting.
const PROCESS_KILL_GRACE_PERIOD: Duration = Duration::from_millis(100);

/// Sends SIGTERM to the terminal's shell and foreground process groups, and
/// returns a future that SIGKILLs whatever survives [`PROCESS_KILL_GRACE_PERIOD`].
/// Closing the PTY only delivers SIGHUP, and a foreground job that ignores
/// SIGHUP/SIGTERM would otherwise be orphaned (#47412).
///
/// Must be called while the PTY master is still open (i.e. before
/// `pty_tx.shutdown()`): reading the foreground process group requires
/// `tcgetpgrp` on the PTY fd.
fn terminate_processes_with_grace_period(
    info: Arc<PtyProcessInfo>,
    executor: BackgroundExecutor,
) -> impl Future<Output = ()> {
    let process_ids = info.capture_process_ids();
    process_ids.terminate();
    async move {
        executor.timer(PROCESS_KILL_GRACE_PERIOD).await;
        process_ids.kill();
        info.kill_child_process();
    }
}

/// Process-wide flag set by headless hosts (e.g. the eval CLI) that have no
/// controlling TTY. In such sandboxes PTY allocation and acquiring a
/// controlling terminal fail with `ENOTTY`, so when this is set terminals run
/// their command as a plain subprocess with piped output instead of through a
/// PTY. The normal editor leaves it unset to preserve the interactive PTY
/// experience.
#[derive(Clone, Copy, Default)]
pub struct HeadlessTerminal(pub bool);

impl gpui::Global for HeadlessTerminal {}

impl HeadlessTerminal {
    pub fn is_enabled(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|headless| headless.0)
    }
}

#[derive(Clone, Copy, Debug)]
enum Scroll {
    Delta(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug)]
enum ViMotion {
    Up,
    Down,
    Left,
    Right,
    First,
    Last,
    FirstOccupied,
    High,
    Middle,
    Low,
    WordLeft,
    WordRight,
    WordRightEnd,
    Bracket,
    ParagraphUp,
    ParagraphDown,
}

#[derive(Clone, Debug)]
pub struct Search {
    search: AlacrittySearch,
}

#[derive(Clone, Debug)]
struct Selection {
    ty: SelectionType,
    start: SelectionAnchor,
    end: SelectionAnchor,
    head: Point,
}

#[derive(Clone, Copy, Debug)]
struct SelectionAnchor {
    point: Point,
    side: SelectionSide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectionType {
    Simple,
    Semantic,
    Lines,
}

impl Selection {
    fn new(selection_type: SelectionType, point: Point, side: SelectionSide) -> Self {
        let anchor = SelectionAnchor { point, side };
        Self {
            ty: selection_type,
            start: anchor,
            end: anchor,
            head: point,
        }
    }

    fn simple_range(range: Range) -> Self {
        let mut selection = Self::new(SelectionType::Simple, range.start(), SelectionSide::Left);
        selection.update(range.end(), SelectionSide::Right);
        selection
    }

    fn update(&mut self, point: Point, side: SelectionSide) {
        self.end = SelectionAnchor { point, side };
        self.head = point;
    }
}

pub fn is_default_background_color(color: Color) -> bool {
    matches!(color, Color::Named(NamedColor::Background))
}

pub fn is_app_chosen_exact_color(color: Color) -> bool {
    matches!(color, Color::Spec(_) | Color::Indexed(16..=255))
}

pub type AnsiSpans = Vec<(StdRange<usize>, Option<Color>)>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParsedAnsiText {
    pub text: String,
    pub foreground_spans: AnsiSpans,
    pub background_spans: AnsiSpans,
}

pub fn parse_ansi_text(input: &[u8]) -> ParsedAnsiText {
    let mut handler = StyledAnsiTextHandler::default();
    let mut processor = Processor::<StdSyncHandler>::default();
    processor.advance(&mut handler, input);
    handler.finish()
}

pub fn strip_ansi_text(input: &[u8]) -> String {
    let mut handler = PlainAnsiTextHandler::default();
    let mut processor = Processor::<StdSyncHandler>::default();
    processor.advance(&mut handler, input);
    handler.text
}

#[derive(Default)]
struct StyledAnsiTextHandler {
    text: String,
    foreground_spans: AnsiSpans,
    background_spans: AnsiSpans,
    current_foreground_range_start: usize,
    current_background_range_start: usize,
    current_foreground_color: Option<Color>,
    current_background_color: Option<Color>,
}

impl StyledAnsiTextHandler {
    fn finish(mut self) -> ParsedAnsiText {
        if self.current_foreground_range_start < self.text.len() {
            self.foreground_spans.push((
                self.current_foreground_range_start..self.text.len(),
                self.current_foreground_color,
            ));
        }

        if self.current_background_range_start < self.text.len() {
            self.background_spans.push((
                self.current_background_range_start..self.text.len(),
                self.current_background_color,
            ));
        }

        ParsedAnsiText {
            text: self.text,
            foreground_spans: self.foreground_spans,
            background_spans: self.background_spans,
        }
    }

    fn break_foreground_span(&mut self, color: Option<Color>) {
        self.foreground_spans.push((
            self.current_foreground_range_start..self.text.len(),
            self.current_foreground_color,
        ));
        self.current_foreground_color = color;
        self.current_foreground_range_start = self.text.len();
    }

    fn break_background_span(&mut self, color: Option<Color>) {
        self.background_spans.push((
            self.current_background_range_start..self.text.len(),
            self.current_background_color,
        ));
        self.current_background_color = color;
        self.current_background_range_start = self.text.len();
    }
}

impl Handler for StyledAnsiTextHandler {
    fn input(&mut self, c: char) {
        self.text.push(c);
    }

    fn linefeed(&mut self) {
        self.text.push('\n');
    }

    fn put_tab(&mut self, count: u16) {
        self.text.extend(std::iter::repeat_n('\t', count as usize));
    }

    fn terminal_attribute(&mut self, attr: Attr) {
        match attr {
            Attr::Foreground(color) => {
                self.break_foreground_span(Some(color));
            }
            Attr::Background(color) => {
                self.break_background_span(Some(color));
            }
            Attr::Reset => {
                self.break_foreground_span(None);
                self.break_background_span(None);
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct PlainAnsiTextHandler {
    text: String,
    line_start: usize,
}

impl Handler for PlainAnsiTextHandler {
    fn input(&mut self, c: char) {
        self.text.push(c);
    }

    fn linefeed(&mut self) {
        self.text.push('\n');
        self.line_start = self.text.len();
    }

    fn carriage_return(&mut self) {
        self.text.truncate(self.line_start);
    }

    fn put_tab(&mut self, count: u16) {
        self.text.extend(std::iter::repeat_n('\t', count as usize));
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Hyperlink {
    data: HyperlinkData,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum HyperlinkData {
    Alacritty(AlacrittyHyperlink),
    Owned { id: Option<Arc<str>>, uri: Arc<str> },
}

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct Cell {
    cell: AlacrittyCell,
}

pub struct RenderableCells<'a> {
    cells: AlacrittyGridIterator<'a>,
}

#[derive(Debug, Clone)]
pub struct IndexedCell {
    pub point: Point,
    pub cell: Cell,
}

impl Deref for IndexedCell {
    type Target = Cell;

    #[inline]
    fn deref(&self) -> &Cell {
        &self.cell
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modes(u32);

impl Modes {
    pub const NONE: Self = Self(0);
    pub const APP_CURSOR: Self = Self(1 << 0);
    pub const APP_KEYPAD: Self = Self(1 << 1);
    pub const SHOW_CURSOR: Self = Self(1 << 2);
    pub const LINE_WRAP: Self = Self(1 << 3);
    pub const ORIGIN: Self = Self(1 << 4);
    pub const INSERT: Self = Self(1 << 5);
    pub const LINE_FEED_NEW_LINE: Self = Self(1 << 6);
    pub const FOCUS_IN_OUT: Self = Self(1 << 7);
    pub const ALTERNATE_SCROLL: Self = Self(1 << 8);
    pub const BRACKETED_PASTE: Self = Self(1 << 9);
    pub const SGR_MOUSE: Self = Self(1 << 10);
    pub const UTF8_MOUSE: Self = Self(1 << 11);
    pub const ALT_SCREEN: Self = Self(1 << 12);
    pub const MOUSE_REPORT_CLICK: Self = Self(1 << 13);
    pub const MOUSE_DRAG: Self = Self(1 << 14);
    pub const MOUSE_MOTION: Self = Self(1 << 15);
    pub const VI: Self = Self(1 << 16);
    pub const MOUSE_MODE: Self =
        Self(Self::MOUSE_REPORT_CLICK.0 | Self::MOUSE_DRAG.0 | Self::MOUSE_MOTION.0);

    pub const fn empty() -> Self {
        Self::NONE
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl BitOr for Modes {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Modes {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub shape: CursorShape,
    pub point: Point,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
    HollowBlock,
    Hidden,
}

impl From<SettingsCursorShape> for CursorShape {
    fn from(shape: SettingsCursorShape) -> Self {
        match shape {
            SettingsCursorShape::Block => Self::Block,
            SettingsCursorShape::Underline => Self::Underline,
            SettingsCursorShape::Bar => Self::Bar,
            SettingsCursorShape::Hollow => Self::HollowBlock,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Point {
    pub line: i32,
    pub column: usize,
}

impl Point {
    pub fn new(line: i32, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Range {
    start: Point,
    end: Point,
}

impl Range {
    pub fn new(start: Point, end: Point) -> Self {
        Self { start, end }
    }

    pub fn start(&self) -> Point {
        self.start
    }

    pub fn end(&self) -> Point {
        self.end
    }

    pub fn contains(&self, point: Point) -> bool {
        self.start <= point && point <= self.end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionRange {
    pub start: Point,
    pub end: Point,
    pub is_block: bool,
}

impl SelectionRange {
    pub fn point_range(self) -> Range {
        Range::new(self.start, self.end)
    }
}

// TODO: Un-pub
#[derive(Clone)]
pub struct Content {
    pub cells: Vec<IndexedCell>,
    pub mode: Modes,
    pub display_offset: usize,
    pub selection_text: Option<String>,
    pub selection: Option<SelectionRange>,
    pub cursor: Cursor,
    pub cursor_char: char,
    pub terminal_bounds: TerminalBounds,
    pub last_hovered_word: Option<HoveredWord>,
    pub scrolled_to_top: bool,
    pub scrolled_to_bottom: bool,
    pub bottom_row_occupied: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HoveredWord {
    pub word: String,
    pub word_match: Range,
    pub id: usize,
}

impl Default for Content {
    fn default() -> Self {
        Content {
            cells: Default::default(),
            mode: Default::default(),
            display_offset: Default::default(),
            selection_text: Default::default(),
            selection: Default::default(),
            cursor: Cursor {
                shape: CursorShape::Block,
                point: Point::new(0, 0),
            },
            cursor_char: Default::default(),
            terminal_bounds: Default::default(),
            last_hovered_word: None,
            scrolled_to_top: false,
            scrolled_to_bottom: false,
            bottom_row_occupied: false,
        }
    }
}

#[derive(PartialEq, Eq)]
enum SelectionPhase {
    Selecting,
    Ended,
}

#[cfg(test)]
mod domain_tests {
    use super::*;

    #[test]
    fn strip_ansi_text_removes_ansi_and_handles_carriage_returns() {
        let cases = [
            ("no escape codes here\n", "no escape codes here\n"),
            ("\x1b[31mhello\x1b[0m", "hello"),
            ("\x1b[1;32mfoo\x1b[0m bar", "foo bar"),
            ("progress 10%\rprogress 100%\n", "progress 100%\n"),
        ];

        for (input, expected) in cases {
            assert_eq!(strip_ansi_text(input.as_bytes()), expected);
        }
    }

    #[test]
    fn parse_ansi_text_records_foreground_and_background_spans() {
        let parsed = parse_ansi_text(b"\x1b[31mred\x1b[44mblue-bg\x1b[0mplain");

        assert_eq!(parsed.text, "redblue-bgplain");
        assert_eq!(
            parsed.foreground_spans,
            vec![
                (0..0, None),
                (0..10, Some(Color::Named(NamedColor::Red))),
                (10..15, None),
            ]
        );
        assert_eq!(
            parsed.background_spans,
            vec![
                (0..3, None),
                (3..10, Some(Color::Named(NamedColor::Blue))),
                (10..15, None),
            ]
        );
    }

    #[test]
    fn terminal_cell_clone_shares_extra_storage() {
        let mut cell = Cell::default();
        cell.push_zerowidth('a');

        let clone = cell.clone();

        match (&cell.cell.extra, &clone.cell.extra) {
            (Some(extra), Some(clone_extra)) => assert!(Arc::ptr_eq(extra, clone_extra)),
            _ => panic!("expected extra storage on both cells"),
        }
    }
}

actions!(
    terminal,
    [
        /// Clears the terminal screen.
        Clear,
        /// Copies selected text to the clipboard.
        Copy,
        /// Pastes from the clipboard.
        Paste,
        /// Pastes the text from the clipboard.
        PasteText,
        /// Shows the character palette for special characters.
        ShowCharacterPalette,
        /// Searches for text in the terminal.
        SearchTest,
        /// Scrolls up by one line.
        ScrollLineUp,
        /// Scrolls down by one line.
        ScrollLineDown,
        /// Scrolls up by one page.
        ScrollPageUp,
        /// Scrolls down by one page.
        ScrollPageDown,
        /// Scrolls up by half a page.
        ScrollHalfPageUp,
        /// Scrolls down by half a page.
        ScrollHalfPageDown,
        /// Scrolls to the top of the terminal buffer.
        ScrollToTop,
        /// Scrolls to the bottom of the terminal buffer.
        ScrollToBottom,
        /// Toggles vi mode in the terminal.
        ToggleViMode,
        /// Selects all text in the terminal.
        SelectAll,
    ]
);

/// Sends the specified text directly to the terminal (e.g. escape sequences).
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = terminal, no_json)]
pub struct SendText(pub String);

/// Sends a keystroke sequence to the terminal (parsed via `Keystroke::parse`).
#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = terminal, no_json)]
pub struct SendKeystroke(pub String);

const DEBUG_TERMINAL_WIDTH: Pixels = px(500.);
const DEBUG_TERMINAL_HEIGHT: Pixels = px(30.);
const DEBUG_CELL_WIDTH: Pixels = px(5.);
const DEBUG_LINE_HEIGHT: Pixels = px(5.);

/// Inserts Zed-specific environment variables for terminal sessions.
/// Used by both local terminals and remote terminals (via SSH).
pub fn insert_zed_terminal_env(
    env: &mut HashMap<String, String>,
    version: &impl std::fmt::Display,
) {
    env.insert("ZED_TERM".to_string(), "true".to_string());
    env.insert("TERM_PROGRAM".to_string(), "zed".to_string());
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env.insert("TERM_PROGRAM_VERSION".to_string(), version.to_string());
}

///Upward flowing events, for changing the title and such
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    TitleChanged,
    BreadcrumbsChanged,
    CloseTerminal,
    Bell,
    Wakeup,
    BlinkChanged(bool),
    SelectionsChanged,
    NewNavigationTarget(Option<MaybeNavigationTarget>),
    Open(MaybeNavigationTarget),
    /// Selection text was written to the system clipboard (⌘C or copy-on-select).
    CopiedToClipboard,
    /// A desktop-notification request via OSC 9 / OSC 777.
    Notify(String),
    /// A command started in this terminal (Run Ledger).
    RunStarted {
        command: String,
        cwd: Option<PathBuf>,
        inferred: bool,
    },
    /// The current command finished. `exit_code` is `None` when unknown.
    RunFinished { exit_code: Option<i32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathLikeTarget {
    /// File system path, absolute or relative, existing or not.
    /// Might have line and column number(s) attached as `file.rs:1:23`
    pub maybe_path: String,
    /// Current working directory of the terminal
    pub working_directory: Option<PathBuf>,
}

/// A string inside terminal, potentially useful as a URI that can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaybeNavigationTarget {
    /// HTTP, git, etc. string determined by the `URL_REGEX` regex.
    Url(String),
    /// File system path, absolute or relative, existing or not.
    /// Might have line and column number(s) attached as `file.rs:1:23`
    PathLike(PathLikeTarget),
}

#[derive(Clone)]
enum InternalEvent {
    Resize(TerminalBounds),
    Clear,
    // FocusNextMatch,
    Scroll(Scroll),
    ScrollToPoint(Point),
    SetSelection(Option<Selection>),
    UpdateSelection(GpuiPoint<Pixels>),
    FindHyperlink(GpuiPoint<Pixels>, bool),
    ProcessHyperlink(HyperlinkMatch, bool),
    // Whether keep selection when copy
    Copy(Option<bool>),
    // Vi mode events
    ToggleViMode,
    ViMotion(ViMotion),
    MoveViCursorToPoint(Point),
}

type ClipboardFormatter = Arc<dyn Fn(&str) -> String + Sync + Send + 'static>;
type ColorFormatter = Arc<dyn Fn(Rgb) -> String + Sync + Send + 'static>;
type TextAreaSizeFormatter = Arc<dyn Fn(TerminalBounds) -> String + Sync + Send + 'static>;

#[derive(Clone)]
pub(crate) enum TerminalBackendEvent {
    MouseCursorDirty,
    Title(String),
    ResetTitle,
    ClipboardStore(String),
    ClipboardLoad(ClipboardFormatter),
    ColorRequest(usize, ColorFormatter),
    PtyWrite(String),
    TextAreaSizeRequest(TextAreaSizeFormatter),
    CursorBlinkingChange,
    Wakeup,
    Bell,
    Exit,
    ChildExit(ExitStatus),
    /// Shell-integration marker payload from the alacritty handler
    /// (`"A"`, `"B"`, `"C"`, `"D;0"`).
    Osc133(String),
    /// Desktop notification from OSC 9 / OSC 777.
    DesktopNotification(String),
}

impl fmt::Debug for TerminalBackendEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MouseCursorDirty => f.write_str("MouseCursorDirty"),
            Self::Title(title) => write!(f, "Title({title})"),
            Self::ResetTitle => f.write_str("ResetTitle"),
            Self::ClipboardStore(data) => write!(f, "ClipboardStore({data})"),
            Self::ClipboardLoad(_) => f.write_str("ClipboardLoad"),
            Self::ColorRequest(index, _) => write!(f, "ColorRequest({index})"),
            Self::PtyWrite(output) => write!(f, "PtyWrite({output})"),
            Self::TextAreaSizeRequest(_) => f.write_str("TextAreaSizeRequest"),
            Self::CursorBlinkingChange => f.write_str("CursorBlinkingChange"),
            Self::Wakeup => f.write_str("Wakeup"),
            Self::Bell => f.write_str("Bell"),
            Self::Exit => f.write_str("Exit"),
            Self::ChildExit(status) => write!(f, "ChildExit({status})"),
            Self::Osc133(kind) => write!(f, "Osc133({kind})"),
            Self::DesktopNotification(msg) => write!(f, "DesktopNotification({msg})"),
        }
    }
}

enum PtyEvent {
    Event(TerminalBackendEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalBounds {
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub bounds: Bounds<Pixels>,
}

impl TerminalBounds {
    pub fn new(line_height: Pixels, cell_width: Pixels, bounds: Bounds<Pixels>) -> Self {
        TerminalBounds {
            cell_width,
            line_height,
            bounds,
        }
    }

    pub fn num_lines(&self) -> usize {
        // Tolerance to prevent f32 precision from losing a row:
        // `N * line_height / line_height` can be N-epsilon, which floor()
        // would round down, pushing the first line into invisible scrollback.
        let raw = self.bounds.size.height / self.line_height;
        raw.next_up().floor() as usize
    }

    pub fn num_columns(&self) -> usize {
        let raw = self.bounds.size.width / self.cell_width;
        raw.next_up().floor() as usize
    }

    pub fn height(&self) -> Pixels {
        self.bounds.size.height
    }

    pub fn width(&self) -> Pixels {
        self.bounds.size.width
    }

    pub fn cell_width(&self) -> Pixels {
        self.cell_width
    }

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }
}

impl Default for TerminalBounds {
    fn default() -> Self {
        TerminalBounds::new(
            DEBUG_LINE_HEIGHT,
            DEBUG_CELL_WIDTH,
            Bounds {
                origin: GpuiPoint::default(),
                size: Size {
                    width: DEBUG_TERMINAL_WIDTH,
                    height: DEBUG_TERMINAL_HEIGHT,
                },
            },
        )
    }
}

fn normalize_terminal_bounds(mut bounds: TerminalBounds) -> TerminalBounds {
    bounds.bounds.size.height = cmp::max(bounds.line_height, bounds.height());
    bounds.bounds.size.width = cmp::max(bounds.cell_width, bounds.width());
    bounds
}

#[derive(Error, Debug)]
pub struct TerminalError {
    pub directory: Option<PathBuf>,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub title_override: Option<String>,
    pub source: std::io::Error,
}

impl TerminalError {
    fn fmt_directory(&self) -> String {
        self.directory
            .clone()
            .map(|path| {
                match path
                    .into_os_string()
                    .into_string()
                    .map_err(|os_str| format!("<non-utf8 path> {}", os_str.to_string_lossy()))
                {
                    Ok(s) => s,
                    Err(s) => s,
                }
            })
            .unwrap_or_else(|| "<none specified>".to_string())
    }

    fn fmt_shell(&self) -> String {
        if let Some(title_override) = &self.title_override {
            format!(
                "{} {} ({})",
                self.program.as_deref().unwrap_or("<system defined shell>"),
                self.args.as_ref().into_iter().flatten().format(" "),
                title_override
            )
        } else {
            format!(
                "{} {}",
                self.program.as_deref().unwrap_or("<system defined shell>"),
                self.args.as_ref().into_iter().flatten().format(" ")
            )
        }
    }
}

impl Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let dir_string: String = self.fmt_directory();
        let shell = self.fmt_shell();

        write!(
            f,
            "Working directory: {} Shell command: `{}`, IOError: {}",
            dir_string, shell, self.source
        )
    }
}

// https://github.com/alacritty/alacritty/blob/cb3a79dbf6472740daca8440d5166c1d4af5029e/extra/man/alacritty.5.scd?plain=1#L207-L213
const DEFAULT_SCROLL_HISTORY_LINES: usize = 10_000;
pub const MAX_SCROLL_HISTORY_LINES: usize = 100_000;
static NEXT_INIT_COMMAND_STARTUP_MARKER_ID: AtomicU64 = AtomicU64::new(1);

const INIT_COMMAND_STARTUP_MARKER_PREFIX: &str = "__zed_init_command_ready_";
const INIT_COMMAND_STARTUP_MARKER_SUFFIX: &str = "__";
const INIT_COMMAND_STARTUP_MARKER_SEARCH_LINES: usize = 64;

fn init_command_startup_marker(marker_id: u64) -> String {
    format!("{INIT_COMMAND_STARTUP_MARKER_PREFIX}{marker_id}{INIT_COMMAND_STARTUP_MARKER_SUFFIX}")
}

fn init_command_startup_marker_command(shell_kind: ShellKind, marker_id: u64) -> String {
    // Split the marker across the command so its echo can't satisfy the
    // handshake; only the command's output contains the contiguous marker.
    match shell_kind {
        ShellKind::PowerShell | ShellKind::Pwsh => format!(
            "Write-Output ('{INIT_COMMAND_STARTUP_MARKER_PREFIX}' + '{marker_id}' + '{INIT_COMMAND_STARTUP_MARKER_SUFFIX}')"
        ),
        ShellKind::Cmd => {
            format!(
                "<nul set /p zed_init_ready={INIT_COMMAND_STARTUP_MARKER_PREFIX}&echo {marker_id}{INIT_COMMAND_STARTUP_MARKER_SUFFIX}"
            )
        }
        ShellKind::Nushell => {
            format!(
                "print $\"{INIT_COMMAND_STARTUP_MARKER_PREFIX}({marker_id}){INIT_COMMAND_STARTUP_MARKER_SUFFIX}\""
            )
        }
        ShellKind::Posix
        | ShellKind::Csh
        | ShellKind::Tcsh
        | ShellKind::Rc
        | ShellKind::Fish
        | ShellKind::Xonsh
        | ShellKind::Elvish => format!(
            "printf '%s%s%s\\n' {INIT_COMMAND_STARTUP_MARKER_PREFIX} {marker_id} {INIT_COMMAND_STARTUP_MARKER_SUFFIX}"
        ),
    }
}

pub struct TerminalBuilder {
    terminal: Terminal,
    events_rx: UnboundedReceiver<PtyEvent>,
}

impl TerminalBuilder {
    pub fn new_display_only(
        cursor_shape: SettingsCursorShape,
        alternate_scroll: AlternateScroll,
        max_scroll_history_lines: Option<usize>,
        window_id: u64,
        background_executor: &BackgroundExecutor,
        path_style: PathStyle,
    ) -> TerminalBuilder {
        Self::new_display_only_with_bounds(
            cursor_shape,
            alternate_scroll,
            max_scroll_history_lines,
            window_id,
            background_executor,
            path_style,
            TerminalBounds::default(),
        )
    }

    pub fn new_display_only_with_bounds(
        cursor_shape: SettingsCursorShape,
        alternate_scroll: AlternateScroll,
        max_scroll_history_lines: Option<usize>,
        window_id: u64,
        background_executor: &BackgroundExecutor,
        path_style: PathStyle,
        terminal_bounds: TerminalBounds,
    ) -> TerminalBuilder {
        let terminal_bounds = normalize_terminal_bounds(terminal_bounds);

        let scrolling_history = max_scroll_history_lines
            .unwrap_or(DEFAULT_SCROLL_HISTORY_LINES)
            .min(MAX_SCROLL_HISTORY_LINES);
        let config = display_only_term_config(scrolling_history, cursor_shape);

        let (events_tx, events_rx) = unbounded();
        let term = new_term(&config, terminal_bounds, events_tx, alternate_scroll);

        let terminal = Terminal {
            task: None,
            terminal_type: TerminalType::DisplayOnly,
            subprocess: None,
            completion_tx: None,
            term,
            term_config: config,
            output_processor: Processor::<StdSyncHandler>::new(),
            title_override: None,
            events: VecDeque::with_capacity(10),
            last_content: Content {
                terminal_bounds,
                ..Default::default()
            },
            last_mouse: None,
            mouse_down_position: None,
            matches: Vec::new(),

            selection_head: None,
            breadcrumb_text: String::new(),
            scroll_px: px(0.),
            next_link_id: 0,
            selection_phase: SelectionPhase::Ended,
            hyperlink_regex_searches: RegexSearches::default(),
            vi_mode_enabled: false,
            is_remote_terminal: false,
            last_mouse_move_time: Instant::now(),
            last_hyperlink_search_position: None,
            mouse_down_hyperlink: None,
            activation_script: Vec::new(),
            template: CopyTemplate {
                shell: Shell::System,
                env: HashMap::default(),
                cursor_shape,
                alternate_scroll,
                max_scroll_history_lines,
                path_hyperlink_regexes: Vec::default(),
                path_hyperlink_timeout_ms: 0,
                window_id,
            },
            child_exited: None,
            keyboard_input_sent: false,
            init_command_startup_marker: None,
            osc133: Osc133Scanner::new(),
            osc_notify: OscNotifyScanner::new(),
            prompt_markers: Vec::new(),
            last_busy: false,
            busy_since: None,
            run_tracker: RunTracker::default(),
            started_at: Instant::now(),
            init_command_startup_tx: None,
            event_loop_task: Task::ready(Ok(())),
            background_executor: background_executor.clone(),
            path_style,
            cwd_history: Vec::new(),
            pending_cwd_boundary: None,
            #[cfg(any(test, feature = "test-support"))]
            input_log: Vec::new(),
            #[cfg(any(test, feature = "test-support"))]
            pty_write_log: Default::default(),
        };

        TerminalBuilder {
            terminal,
            events_rx,
        }
    }

    pub fn new(
        working_directory: Option<PathBuf>,
        task: Option<TaskState>,
        shell: Shell,
        mut env: HashMap<String, String>,
        cursor_shape: SettingsCursorShape,
        alternate_scroll: AlternateScroll,
        max_scroll_history_lines: Option<usize>,
        path_hyperlink_regexes: Vec<String>,
        path_hyperlink_timeout_ms: u64,
        is_remote_terminal: bool,
        window_id: u64,
        completion_tx: Option<Sender<Option<ExitStatus>>>,
        cx: &App,
        activation_script: Vec<String>,
        path_style: PathStyle,
    ) -> Task<Result<TerminalBuilder>> {
        let version = release_channel::AppVersion::global(cx);
        let background_executor = cx.background_executor().clone();
        // Headless hosts (e.g. the eval CLI) have no controlling TTY, so PTY
        // allocation / acquiring a controlling terminal fails with `ENOTTY`.
        // When set, run the command as a plain subprocess instead.
        let no_pty = HeadlessTerminal::is_enabled(cx);
        let child_signal_mask = match current_child_signal_mask()
            .context("failed to capture terminal child signal mask")
        {
            Ok(signal_mask) => Some(signal_mask),
            Err(error) => return Task::ready(Err(error)),
        };
        let fut = async move {
            // Remove SHLVL so the spawned shell initializes it to 1, matching
            // the behavior of standalone terminal emulators like iTerm2/Kitty/Alacritty.
            env.remove("SHLVL");

            // If the parent environment doesn't have a locale set
            // (As is the case when launched from a .app on MacOS),
            // and the Project doesn't have a locale set, then
            // set a fallback for our child environment to use.
            if std::env::var("LANG").is_err() {
                env.entry("LANG".to_string())
                    .or_insert_with(|| "en_US.UTF-8".to_string());
            }

            insert_zed_terminal_env(&mut env, &version);

            #[derive(Default)]
            struct ShellParams {
                program: String,
                args: Option<Vec<String>>,
                title_override: Option<String>,
            }

            impl ShellParams {
                fn new(
                    program: String,
                    args: Option<Vec<String>>,
                    title_override: Option<String>,
                ) -> Self {
                    log::debug!("Using {program} as shell");
                    Self {
                        program,
                        args,
                        title_override,
                    }
                }
            }

            let shell_params = match shell.clone() {
                Shell::System => None,
                Shell::Program(program) => Some(ShellParams::new(program, None, None)),
                Shell::WithArguments {
                    program,
                    args,
                    title_override,
                } => Some(ShellParams::new(program, Some(args), title_override)),
            };
            let terminal_title_override =
                shell_params.as_ref().and_then(|e| e.title_override.clone());

            let shell_kind = shell.shell_kind(false);

            let scrolling_history = if task.is_some() {
                // Tasks like `cargo build --all` may produce a lot of output, ergo allow maximum scrolling.
                // After the task finishes, we do not allow appending to that terminal, so small tasks output should not
                // cause excessive memory usage over time.
                MAX_SCROLL_HISTORY_LINES
            } else {
                max_scroll_history_lines
                    .unwrap_or(DEFAULT_SCROLL_HISTORY_LINES)
                    .min(MAX_SCROLL_HISTORY_LINES)
            };
            let config = pty_term_config(scrolling_history, cursor_shape);

            //Spawn a task so the Alacritty EventLoop (or the subprocess reader) can communicate with us
            //TODO: Remove with a bounded sender which can be dispatched on &self
            let (events_tx, events_rx) = unbounded();
            //Set up the terminal...
            let term = new_term(
                &config,
                TerminalBounds::default(),
                events_tx.clone(),
                alternate_scroll,
            );

            // When `no_pty` is set (headless hosts), run the task as a plain
            // subprocess and pump its piped output into the same emulator the
            // PTY path would feed.
            let (terminal_type, subprocess) = if no_pty {
                let (program, args) = match &shell_params {
                    Some(params) => (
                        params.program.clone(),
                        params.args.clone().unwrap_or_default(),
                    ),
                    None => (util::shell::get_system_shell(), Vec::new()),
                };
                let subprocess = match spawn_task_subprocess(
                    program,
                    args,
                    env.clone(),
                    working_directory.clone(),
                    term.clone(),
                    events_tx,
                    &background_executor,
                ) {
                    Ok(subprocess) => subprocess,
                    Err(error) => {
                        bail!(TerminalError {
                            directory: working_directory,
                            program: shell_params.as_ref().map(|params| params.program.clone()),
                            args: shell_params.as_ref().and_then(|params| params.args.clone()),
                            title_override: terminal_title_override,
                            source: std::io::Error::other(format!("{error:#}")),
                        });
                    }
                };
                (TerminalType::DisplayOnly, Some(subprocess))
            } else {
                let alacritty_shell = shell_params.as_ref().map(|params| {
                    (
                        params.program.clone(),
                        params.args.clone().unwrap_or_default(),
                    )
                });
                let pty_options = pty_options(
                    alacritty_shell,
                    working_directory.clone(),
                    env.clone(),
                    // We pass in the foreground thread's signal mask to the child process via pty_options,
                    // so terminal construction can run on a background thread without breaking Ctrl-C and other signals
                    // otherwise the terminal would inherit the background executor's signal mask which blocks
                    // some terminal signals
                    child_signal_mask,
                );

                //Setup the pty...
                let pty = match open_pty(&pty_options, TerminalBounds::default(), window_id) {
                    Ok(pty) => pty,
                    Err(error) => {
                        bail!(TerminalError {
                            directory: working_directory,
                            program: shell_params.as_ref().map(|params| params.program.clone()),
                            args: shell_params.as_ref().and_then(|params| params.args.clone()),
                            title_override: terminal_title_override,
                            source: error,
                        });
                    }
                };

                let pty_info = PtyProcessInfo::new(ProcessIdGetter::from(&pty));

                //And connect them together
                let pty_tx =
                    spawn_event_loop(term.clone(), events_tx, pty, pty_options.drain_on_exit)?;

                (
                    TerminalType::Pty {
                        pty_tx,
                        info: Arc::new(pty_info),
                    },
                    None,
                )
            };

            let no_task = task.is_none();
            let terminal = Terminal {
                task,
                terminal_type,
                subprocess,
                completion_tx,
                term,
                term_config: config,
                output_processor: Processor::<StdSyncHandler>::new(),
                title_override: terminal_title_override,
                events: VecDeque::with_capacity(10), //Should never get this high.
                last_content: Default::default(),
                last_mouse: None,
                mouse_down_position: None,
                matches: Vec::new(),

                selection_head: None,
                breadcrumb_text: String::new(),
                scroll_px: px(0.),
                next_link_id: 0,
                selection_phase: SelectionPhase::Ended,
                hyperlink_regex_searches: RegexSearches::new(
                    &path_hyperlink_regexes,
                    path_hyperlink_timeout_ms,
                ),
                vi_mode_enabled: false,
                is_remote_terminal,
                last_mouse_move_time: Instant::now(),
                last_hyperlink_search_position: None,
                mouse_down_hyperlink: None,
                activation_script: activation_script.clone(),
                template: CopyTemplate {
                    shell,
                    env,
                    cursor_shape,
                    alternate_scroll,
                    max_scroll_history_lines,
                    path_hyperlink_regexes,
                    path_hyperlink_timeout_ms,
                    window_id,
                },
                child_exited: None,
                keyboard_input_sent: false,
                init_command_startup_marker: None,
                osc133: Osc133Scanner::new(),
                osc_notify: OscNotifyScanner::new(),
                prompt_markers: Vec::new(),
                last_busy: false,
                busy_since: None,
                run_tracker: RunTracker::default(),
                started_at: Instant::now(),
                init_command_startup_tx: None,
                event_loop_task: Task::ready(Ok(())),
                background_executor,
                path_style,
                cwd_history: if is_remote_terminal {
                    Vec::new()
                } else {
                    working_directory
                        .as_ref()
                        .map(|working_directory| {
                            vec![CwdHistoryEntry {
                                scrollback_position: i32::MIN,
                                working_directory: working_directory.clone(),
                            }]
                        })
                        .unwrap_or_default()
                },
                pending_cwd_boundary: None,
                #[cfg(any(test, feature = "test-support"))]
                input_log: Vec::new(),
                #[cfg(any(test, feature = "test-support"))]
                pty_write_log: Default::default(),
            };

            if !activation_script.is_empty() && no_task {
                for activation_script in activation_script {
                    terminal.write_to_pty(activation_script.into_bytes());
                    // Simulate enter key press
                    // NOTE(PowerShell): using `\r\n` will put PowerShell in a continuation mode (infamous >> character)
                    // and generally mess up the rendering.
                    terminal.write_to_pty(b"\x0d");
                }
                // In order to clear the screen at this point, we have two options:
                // 1. We can send a shell-specific command such as "clear" or "cls"
                // 2. We can "echo" a marker message that we will then catch when handling a Wakeup event
                //    and clear the screen using `terminal.clear()` method
                // We cannot issue a `terminal.clear()` command at this point as alacritty is evented
                // and while we have sent the activation script to the pty, it will be executed asynchronously.
                // Therefore, we somehow need to wait for the activation script to finish executing before we
                // can proceed with clearing the screen.
                terminal.write_to_pty(shell_kind.clear_screen_command().as_bytes());
                // Simulate enter key press
                terminal.write_to_pty(b"\x0d");
            }

            Ok(TerminalBuilder {
                terminal,
                events_rx,
            })
        };
        cx.background_spawn(fut)
    }

    pub fn subscribe(mut self, cx: &Context<Terminal>) -> Terminal {
        // `Terminal::drop` escalates to SIGKILL on a detached background task,
        // which never gets to run when the whole app quits: the process exits
        // as soon as the `on_app_quit` futures resolve. Perform the same
        // escalation in a quit observer, whose future keeps the app alive for
        // the grace period, so that processes ignoring SIGHUP/SIGTERM don't
        // outlive Zed (#47412). The subscription can't be stored on `Terminal`
        // (`Subscription` is not `Send`, and `TerminalBuilder` is built on a
        // background thread), so its lifetime is tied to the entity's release
        // instead.
        let app_quit_subscription = cx.on_app_quit(|terminal, cx| {
            let kill_processes = match &terminal.terminal_type {
                TerminalType::Pty { info, .. } => Some(terminate_processes_with_grace_period(
                    info.clone(),
                    cx.background_executor().clone(),
                )),
                TerminalType::DisplayOnly => None,
            };
            async move {
                if let Some(kill_processes) = kill_processes {
                    kill_processes.await;
                }
            }
        });
        cx.on_release(move |_, _| drop(app_quit_subscription))
            .detach();

        //Event loop
        self.terminal.event_loop_task = cx.spawn(async move |terminal, cx| {
            while let Some(event) = self.events_rx.next().await {
                terminal.update(cx, |terminal, cx| {
                    //Process the first event immediately for lowered latency
                    terminal.process_pty_event(event, cx);
                })?;

                'outer: loop {
                    let mut events = Vec::new();

                    #[cfg(any(test, feature = "test-support"))]
                    let mut timer = cx.background_executor().simulate_random_delay().fuse();
                    #[cfg(not(any(test, feature = "test-support")))]
                    let mut timer = cx
                        .background_executor()
                        .timer(std::time::Duration::from_millis(4))
                        .fuse();

                    let mut wakeup = false;
                    loop {
                        futures::select_biased! {
                            _ = timer => break,
                            event = self.events_rx.next() => {
                                if let Some(event) = event {
                                    if matches!(event, PtyEvent::Event(TerminalBackendEvent::Wakeup))
                                    {
                                        wakeup = true;
                                    } else {
                                        events.push(event);
                                    }

                                    if events.len() > 100 {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            },
                        }
                    }

                    if events.is_empty() && !wakeup {
                        yield_now().await;
                        break 'outer;
                    }

                    terminal.update(cx, |this, cx| {
                        if wakeup {
                            this.process_event(TerminalBackendEvent::Wakeup, cx);
                        }

                        for event in events {
                            this.process_pty_event(event, cx);
                        }
                    })?;
                    yield_now().await;
                }
            }
            anyhow::Ok(())
        });
        self.terminal
    }
}

enum TerminalType {
    Pty {
        pty_tx: PtySender,
        info: Arc<PtyProcessInfo>,
    },
    DisplayOnly,
}

pub struct Terminal {
    terminal_type: TerminalType,
    /// Set for non-PTY terminals (see [`HeadlessTerminal`]); owns the spawned
    /// subprocess and the task pumping its output into the grid.
    subprocess: Option<SubprocessHandle>,
    completion_tx: Option<Sender<Option<ExitStatus>>>,
    term: Arc<AlacrittyTermLock>,
    term_config: AlacrittyTermConfig,
    output_processor: Processor<StdSyncHandler>,
    events: VecDeque<InternalEvent>,
    /// This is only used for mouse mode cell change detection
    last_mouse: Option<(Point, SelectionSide)>,
    /// Window-relative position of the most recent left mouse-down. Used to
    /// apply a drag threshold before starting a selection (see #58970).
    mouse_down_position: Option<GpuiPoint<Pixels>>,
    pub matches: Vec<Range>,
    pub last_content: Content,
    pub selection_head: Option<Point>,

    pub breadcrumb_text: String,
    title_override: Option<String>,
    scroll_px: Pixels,
    next_link_id: usize,
    selection_phase: SelectionPhase,
    hyperlink_regex_searches: RegexSearches,
    task: Option<TaskState>,
    vi_mode_enabled: bool,
    is_remote_terminal: bool,
    last_mouse_move_time: Instant,
    last_hyperlink_search_position: Option<GpuiPoint<Pixels>>,
    mouse_down_hyperlink: Option<HyperlinkMatch>,
    template: CopyTemplate,
    activation_script: Vec<String>,
    child_exited: Option<ExitStatus>,
    keyboard_input_sent: bool,
    init_command_startup_marker: Option<String>,
    /// OSC 133 scanner (M14 shell integration detect).
    osc133: Osc133Scanner,
    osc_notify: OscNotifyScanner,
    /// Prompt/command markers with scrollback lines for jump navigation.
    prompt_markers: Vec<Osc133Marker>,
    /// Last known busy state for command-finish notify (M14).
    last_busy: bool,
    /// When the current foreground job became busy, if any.
    busy_since: Option<Instant>,
    /// OSC 133 / busy-probe → Run start/finish. Pure; emit happens at cx sites.
    run_tracker: RunTracker,
    /// Monotonic origin for Run duration math.
    started_at: Instant,
    init_command_startup_tx: Option<Sender<()>>,
    event_loop_task: Task<Result<(), anyhow::Error>>,
    background_executor: BackgroundExecutor,
    path_style: PathStyle,
    cwd_history: Vec<CwdHistoryEntry>,
    pending_cwd_boundary: Option<i32>,
    #[cfg(any(test, feature = "test-support"))]
    input_log: Vec<Vec<u8>>,
    #[cfg(any(test, feature = "test-support"))]
    pty_write_log: std::cell::RefCell<Vec<Vec<u8>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CwdHistoryEntry {
    /// Line offset in the retained scrollback buffer.
    scrollback_position: i32,
    working_directory: PathBuf,
}

struct CopyTemplate {
    shell: Shell,
    env: HashMap<String, String>,
    cursor_shape: SettingsCursorShape,
    alternate_scroll: AlternateScroll,
    max_scroll_history_lines: Option<usize>,
    path_hyperlink_regexes: Vec<String>,
    path_hyperlink_timeout_ms: u64,
    window_id: u64,
}

#[derive(Debug)]
pub struct TaskState {
    pub status: TaskStatus,
    pub completion_rx: Receiver<Option<ExitStatus>>,
    pub spawned_task: SpawnInTerminal,
}

/// A status of the current terminal tab's task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// The task had been started, but got cancelled or somehow otherwise it did not
    /// report its exit code before the terminal event loop was shut down.
    Unknown,
    /// The task is started and running currently.
    Running,
    /// After the start, the task stopped running and reported its error code back.
    Completed { success: bool },
}

impl TaskStatus {
    fn register_terminal_exit(&mut self) {
        if self == &Self::Running {
            *self = Self::Unknown;
        }
    }

    fn register_task_exit(&mut self, error_code: i32) {
        *self = TaskStatus::Completed {
            success: error_code == 0,
        };
    }
}

const FIND_HYPERLINK_THROTTLE_PX: Pixels = px(5.0);

/// Minimum pointer movement before a left click begins a selection. This keeps
/// a click that jitters by a pixel or two (such as the window-focusing click)
/// from starting a selection and, with `copy_on_select` enabled, clobbering the
/// clipboard. Mirrors the drag threshold used by gpui's `div` element.
const SELECTION_DRAG_THRESHOLD: f64 = 2.0;

impl Terminal {
    fn process_pty_event(&mut self, event: PtyEvent, cx: &mut Context<Self>) {
        match event {
            PtyEvent::Event(event) => self.process_event(event, cx),
        }
    }

    fn process_event(&mut self, event: TerminalBackendEvent, cx: &mut Context<Self>) {
        match event {
            TerminalBackendEvent::Title(title) => {
                self.breadcrumb_text = title;
                cx.emit(Event::BreadcrumbsChanged);
            }
            TerminalBackendEvent::ResetTitle => {
                self.breadcrumb_text = String::new();
                cx.emit(Event::BreadcrumbsChanged);
            }
            TerminalBackendEvent::ClipboardStore(data) => {
                cx.write_to_clipboard(ClipboardItem::new_string(data))
            }
            TerminalBackendEvent::ClipboardLoad(format) => {
                self.write_to_pty(
                    match &cx.read_from_clipboard().and_then(|item| item.text()) {
                        // The terminal only supports pasting strings, not images.
                        Some(text) => format(text),
                        _ => format(""),
                    }
                    .into_bytes(),
                )
            }
            TerminalBackendEvent::PtyWrite(out) => self.write_to_pty(out.into_bytes()),
            TerminalBackendEvent::TextAreaSizeRequest(format) => {
                self.write_to_pty(format(self.last_content.terminal_bounds).into_bytes())
            }
            TerminalBackendEvent::CursorBlinkingChange => {
                let terminal = self.term.lock();
                let blinking = terminal.cursor_style().blinking;
                cx.emit(Event::BlinkChanged(blinking));
            }
            TerminalBackendEvent::Bell => {
                cx.emit(Event::Bell);
            }
            TerminalBackendEvent::Exit => self.register_task_finished(None, cx),
            TerminalBackendEvent::MouseCursorDirty => {
                //NOOP, Handled in render
            }
            TerminalBackendEvent::Wakeup => {
                self.detect_init_command_startup_marker();
                cx.emit(Event::Wakeup);

                if let TerminalType::Pty { info, .. } = &self.terminal_type {
                    info.emit_title_changed_if_changed(cx);
                }
            }
            TerminalBackendEvent::ColorRequest(index, format) => {
                // It's important that the color request is processed here to retain relative order
                // with other PTY writes. Otherwise applications might witness out-of-order
                // responses to requests. For example: An application sending `OSC 11 ; ? ST`
                // (color request) followed by `CSI c` (request device attributes) would receive
                // the response to `CSI c` first.
                // Instead of locking, we could store the colors in `self.last_content`. But then
                // we might respond with out of date value if a "set color" sequence is immediately
                // followed by a color request sequence.

                let color = self.term.lock().colors()[index].unwrap_or_else(|| {
                    to_vte_rgb(palette_get_color(
                        index,
                        TerminalPalette::get_global(cx).as_ref(),
                    ))
                });
                self.write_to_pty(format(color).into_bytes());
            }
            TerminalBackendEvent::ChildExit(exit_status) => {
                if let Some(out) = self
                    .run_tracker
                    .on_marker(Osc133Kind::CommandFinished { status: None }, self.mono_ms())
                {
                    self.emit_tracker_out(out, cx);
                }
                self.register_task_finished(Some(exit_status), cx);
            }
            TerminalBackendEvent::Osc133(payload) => {
                if let Some(kind) = Osc133Kind::from_payload(&payload) {
                    self.record_osc133_marker(kind);
                    if let Some(out) = self.run_tracker.take_output() {
                        self.emit_tracker_out(out, cx);
                    }
                }
            }
            TerminalBackendEvent::DesktopNotification(msg) => {
                cx.emit(Event::Notify(msg));
            }
        }
    }

    pub fn selection_started(&self) -> bool {
        self.selection_phase == SelectionPhase::Selecting
    }

    fn process_terminal_event(
        &mut self,
        event: &InternalEvent,
        term: &mut AlacrittyTerm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            &InternalEvent::Resize(new_bounds) => {
                let new_bounds = normalize_terminal_bounds(new_bounds);
                trace!("Resizing: new_bounds={new_bounds:?}");

                let columns_changed =
                    self.last_content.terminal_bounds.num_columns() != new_bounds.num_columns();
                self.last_content.terminal_bounds = new_bounds;

                if let TerminalType::Pty { pty_tx, .. } = &self.terminal_type {
                    pty_tx.resize(new_bounds);
                }

                resize(term, new_bounds);
                if columns_changed {
                    self.reset_cwd_history();
                }
                // If there are matches we need to emit a wake up event to
                // invalidate the matches and recalculate their locations
                // in the new terminal layout
                if !self.matches.is_empty() {
                    cx.emit(Event::Wakeup);
                }
            }
            InternalEvent::Clear => {
                trace!("Clearing");
                clear_saved_screen(term);
                self.reset_cwd_history();
                cx.emit(Event::Wakeup);
            }
            InternalEvent::Scroll(scroll) => {
                trace!("Scrolling: scroll={scroll:?}");
                scroll_display(term, *scroll);
                self.refresh_hovered_word(window);

                if self.vi_mode_enabled {
                    update_vi_cursor_for_scroll(term, *scroll);
                    if let Some(selection_head) = update_selection_to_vi_cursor(term) {
                        self.selection_head = Some(selection_head);
                        cx.emit(Event::SelectionsChanged)
                    }
                }
            }
            InternalEvent::SetSelection(selection) => {
                trace!("Setting selection: selection={selection:?}");
                set_term_selection(term, selection.as_ref());

                if let Some(selection) = selection {
                    self.selection_head = Some(selection.head);
                }
                cx.emit(Event::SelectionsChanged)
            }
            InternalEvent::UpdateSelection(position) => {
                trace!("Updating selection: position={position:?}");
                let (point, side) = grid_point_and_side(
                    *position,
                    self.last_content.terminal_bounds,
                    display_offset(term),
                );

                if update_term_selection(term, point, side) {
                    self.selection_head = Some(point);
                    cx.emit(Event::SelectionsChanged)
                }
            }

            InternalEvent::Copy(keep_selection) => {
                trace!("Copying selection: keep_selection={keep_selection:?}");
                if let Some(txt) = selection_text(term) {
                    if !txt.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(txt));
                        cx.emit(Event::CopiedToClipboard);
                    }
                    if !keep_selection.unwrap_or_else(|| {
                        let settings = TerminalSettings::get_global(cx);
                        settings.keep_selection_on_copy
                    }) {
                        self.events.push_back(InternalEvent::SetSelection(None));
                    }
                }
            }
            InternalEvent::ScrollToPoint(point) => {
                trace!("Scrolling to point: point={point:?}");
                scroll_to_point(term, *point);
                self.refresh_hovered_word(window);
            }
            InternalEvent::MoveViCursorToPoint(point) => {
                trace!("Move vi cursor to point: point={point:?}");
                vi_goto_point(term, *point);
                self.refresh_hovered_word(window);
            }
            InternalEvent::ToggleViMode => {
                trace!("Toggling vi mode");
                self.vi_mode_enabled = !self.vi_mode_enabled;
                toggle_term_vi_mode(term);
            }
            InternalEvent::ViMotion(motion) => {
                trace!("Performing vi motion: motion={motion:?}");
                vi_motion(term, *motion);
            }
            InternalEvent::FindHyperlink(position, open) => {
                trace!("Finding hyperlink at position: position={position:?}, open={open:?}");

                let point = grid_point(
                    *position,
                    self.last_content.terminal_bounds,
                    display_offset(term),
                );

                match find_from_terminal_point(
                    term,
                    point,
                    &mut self.hyperlink_regex_searches,
                    self.path_style,
                ) {
                    Some(hyperlink) => {
                        let history_size = term.history_size();
                        self.process_hyperlink(hyperlink, *open, history_size, cx);
                    }
                    None => {
                        self.last_content.last_hovered_word = None;
                        cx.emit(Event::NewNavigationTarget(None));
                    }
                }
            }
            InternalEvent::ProcessHyperlink(hyperlink, open) => {
                // history_size must be read here since process_hyperlink cannot lock term
                // (sync() already holds the lock when dispatching events)
                let history_size = term.history_size();
                self.process_hyperlink(hyperlink.clone(), *open, history_size, cx);
            }
        }
    }

    fn process_hyperlink(
        &mut self,
        hyperlink: HyperlinkMatch,
        open: bool,
        history_size: usize,
        cx: &mut Context<Self>,
    ) {
        let HyperlinkMatch {
            text: maybe_url_or_path,
            is_url,
            range,
        } = hyperlink;
        let prev_hovered_word = self.last_content.last_hovered_word.take();
        let match_line = range.start().line;
        let working_directory = self.cwd_at_line(match_line, history_size);

        let target = if is_url {
            if let Some(path) = maybe_url_or_path.strip_prefix("file://") {
                let decoded_path = urlencoding::decode(path)
                    .map(|decoded| decoded.into_owned())
                    .unwrap_or(path.to_owned());

                MaybeNavigationTarget::PathLike(PathLikeTarget {
                    maybe_path: decoded_path,
                    working_directory,
                })
            } else {
                MaybeNavigationTarget::Url(maybe_url_or_path.clone())
            }
        } else {
            MaybeNavigationTarget::PathLike(PathLikeTarget {
                maybe_path: maybe_url_or_path.clone(),
                working_directory,
            })
        };

        if open {
            cx.emit(Event::Open(target));
        } else {
            self.update_selected_word(prev_hovered_word, range, maybe_url_or_path, target, cx);
        }
    }

    fn find_hyperlink_at_point(&mut self, point: Point) -> Option<HyperlinkMatch> {
        let term_lock = self.term.lock();
        find_from_terminal_point(
            &term_lock,
            point,
            &mut self.hyperlink_regex_searches,
            self.path_style,
        )
    }

    fn update_selected_word(
        &mut self,
        prev_word: Option<HoveredWord>,
        word_match: Range,
        word: String,
        navigation_target: MaybeNavigationTarget,
        cx: &mut Context<Self>,
    ) {
        if let Some(prev_word) = prev_word
            && prev_word.word == word
            && prev_word.word_match == word_match
        {
            self.last_content.last_hovered_word = Some(HoveredWord {
                word,
                word_match,
                id: prev_word.id,
            });
            return;
        }

        self.last_content.last_hovered_word = Some(HoveredWord {
            word,
            word_match,
            id: self.next_link_id(),
        });
        cx.emit(Event::NewNavigationTarget(Some(navigation_target)));
        cx.notify()
    }

    fn next_link_id(&mut self) -> usize {
        let res = self.next_link_id;
        self.next_link_id = self.next_link_id.wrapping_add(1);
        res
    }

    pub fn last_content(&self) -> &Content {
        &self.last_content
    }

    pub fn set_cursor_shape(&mut self, cursor_shape: SettingsCursorShape) {
        set_default_cursor_style(&mut self.term_config, cursor_shape);
        apply_config(&self.term, &self.term_config);
    }

    pub fn write_output(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        // Inject bytes directly into the terminal emulator and refresh the UI.
        // This bypasses the PTY/event loop for display-only terminals.
        let mut previous_byte_was_cr = false;
        let converted = convert_lf_to_crlf(bytes, &mut previous_byte_was_cr);

        self.ingest_osc133(&converted);
        self.ingest_osc_notify(&converted, cx);

        let mut term = self.term.lock();
        self.output_processor.advance(&mut *term, &converted);
        drop(term);
        self.detect_init_command_startup_marker();
        cx.emit(Event::Wakeup);
    }

    /// Feed OSC 133 scanner and record prompt markers at the current cursor line.
    pub fn ingest_osc133(&mut self, bytes: &[u8]) {
        for kind in self.osc133.push(bytes) {
            self.record_osc133_marker(kind);
        }
    }

    /// Record a parsed OSC 133 marker against the current cursor line.
    /// Shared by the display-only scanner and the real-PTY event path.
    pub fn record_osc133_marker(&mut self, kind: Osc133Kind) {
        let (line, column) = {
            let term = self.term.lock_unfair();
            let cursor = term.grid().cursor.point;
            (
                Some(cursor.line.0 + term.history_size() as i32),
                Some(cursor.column.0),
            )
        };
        // Keep A/B/C/D so jump-prompt, click-to-move, and output select share one log.
        self.prompt_markers
            .push(Osc133Marker { kind, line, column });
        if self.prompt_markers.len() > 500 {
            let drain = self.prompt_markers.len() - 500;
            self.prompt_markers.drain(0..drain);
        }
        let at_ms = self.mono_ms();
        match kind {
            Osc133Kind::CommandExecuted => {
                let command = self.read_command_between_start_and_cursor();
                self.run_tracker
                    .on_marker_with_command(kind, at_ms, command);
            }
            other => {
                self.run_tracker.on_marker(other, at_ms);
            }
        }
        if matches!(kind, Osc133Kind::CommandFinished { .. }) {
            // Command ended via shell integration; clear busy timer.
            self.last_busy = false;
            self.busy_since = None;
        }
    }

    fn mono_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    fn emit_tracker_out(&mut self, out: TrackerOut, cx: &mut Context<Self>) {
        match out {
            TrackerOut::Started {
                command, inferred, ..
            } => {
                cx.emit(Event::RunStarted {
                    command,
                    cwd: self.working_directory(),
                    inferred,
                });
            }
            TrackerOut::Finished { exit_code, .. } => {
                cx.emit(Event::RunFinished { exit_code });
            }
        }
    }

    /// Grid text from the most recent OSC 133 B (command start) to the cursor.
    fn read_command_between_start_and_cursor(&self) -> Option<String> {
        let start = self
            .prompt_markers
            .iter()
            .rev()
            .find(|m| matches!(m.kind, Osc133Kind::CommandStart))?;
        let start_abs = start.line?;
        let start_col = start.column.unwrap_or(0);
        let term = self.term.lock_unfair();
        let history = term.history_size() as i32;
        let cursor = term.grid().cursor.point;
        let start_line = absolute_to_grid_line(start_abs, history);
        let text = grid_text_range(
            &term,
            start_line,
            start_col,
            cursor.line.0,
            cursor.column.0,
        );
        drop(term);
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Feed OSC 9 / 777 desktop-notification requests from a byte stream,
    /// emitting `Event::Notify` for each.
    pub fn ingest_osc_notify(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        for n in self.osc_notify.push(bytes) {
            cx.emit(Event::Notify(n.message));
        }
    }

    /// Scrollback lines that mark prompt starts (for jump navigation).
    pub fn prompt_marker_lines(&self) -> Vec<i32> {
        self.prompt_markers
            .iter()
            .filter(|m| matches!(m.kind, Osc133Kind::PromptStart))
            .filter_map(|m| m.line)
            .collect()
    }

    /// Option/Alt-click: CSI left/right to the clicked cell when it is inside
    /// the current prompt. `None` if the click should fall through.
    fn click_to_move_bytes(&self, point: Point) -> Option<Vec<u8>> {
        let prompt_line_abs = self
            .prompt_markers
            .iter()
            .rev()
            .find(|m| matches!(m.kind, Osc133Kind::PromptStart))
            .and_then(|m| m.line);
        let prompt_prefix_cols = self
            .prompt_markers
            .iter()
            .rev()
            .find(|m| matches!(m.kind, Osc133Kind::CommandStart) && m.line == prompt_line_abs)
            .and_then(|m| m.column)
            .unwrap_or(0);
        // Markers are absolute (cursor.line + history_size); the click and cursor
        // points are alacritty grid lines. Convert the prompt line to grid so all
        // three share one coordinate space.
        let history_size = self.term.lock_unfair().history_size() as i32;
        let prompt_line = prompt_line_abs.map(|abs| absolute_to_grid_line(abs, history_size));
        let cursor = self.last_content.cursor.point;
        click_to_move_sequence(ClickToMove {
            click_line: point.line,
            click_column: point.column,
            cursor_line: cursor.line,
            cursor_column: cursor.column,
            prompt_line,
            prompt_prefix_cols,
            alt_screen: self.last_content.mode.contains(Modes::ALT_SCREEN),
        })
    }

    /// Jump display to the previous/next prompt marker relative to the viewport.
    /// `delta` is -1 (prev) or +1 (next). Returns true if scrolled.
    pub fn jump_prompt(&mut self, delta: i32) -> bool {
        let lines = self.prompt_marker_lines();
        if lines.is_empty() {
            return false;
        }
        let current = {
            let term = self.term.lock_unfair();
            // Top of viewport in absolute scrollback coords.
            let offset = term.grid().display_offset() as i32;
            let history = term.history_size() as i32;
            history - offset
        };
        let target = if delta < 0 {
            lines.iter().rev().find(|&&l| l < current).copied()
        } else {
            lines.iter().find(|&&l| l > current).copied()
        };
        let Some(target_line) = target else {
            return false;
        };
        // Scroll so target is near the top of the viewport.
        let term = self.term.lock_unfair();
        let history = term.history_size() as i32;
        drop(term);
        let target_offset = (history - target_line).max(0) as usize;
        let mut term = self.term.lock();
        let now = term.grid().display_offset() as i32;
        let delta_lines = target_offset as i32 - now;
        if delta_lines != 0 {
            scroll_display(&mut term, Scroll::Delta(delta_lines));
        }
        true
    }

    /// Poll busy transitions for command-finish notify.
    ///
    /// Returns `Some(duration)` when a previously-busy job just went idle and
    /// ran at least `min_secs` seconds — caller may show a system notification
    /// if the window is unfocused.
    pub fn poll_command_finish(
        &mut self,
        min_secs: u64,
        cx: &mut Context<Self>,
    ) -> Option<Duration> {
        let busy = self.looks_busy();
        let now = Instant::now();
        let at_ms = self.mono_ms();
        match (self.last_busy, busy) {
            (false, true) => {
                self.last_busy = true;
                self.busy_since = Some(now);
                if let Some(out) = self.run_tracker.on_busy_change(
                    true,
                    self.foreground_process_command_name(),
                    at_ms,
                ) {
                    self.emit_tracker_out(out, cx);
                }
                None
            }
            (true, false) => {
                self.last_busy = false;
                if let Some(out) = self.run_tracker.on_busy_change(false, None, at_ms) {
                    self.emit_tracker_out(out, cx);
                }
                let started = self.busy_since.take()?;
                let dur = now.saturating_duration_since(started);
                if dur.as_secs() >= min_secs {
                    Some(dur)
                } else {
                    None
                }
            }
            (true, true) => None,
            (false, false) => None,
        }
    }

    pub fn total_lines(&self) -> usize {
        total_lines(&self.term.lock_unfair())
    }

    /// Full contents — retained scrollback plus the visible screen — as plain
    /// text, for "export scrollback".
    pub fn scrollback_text(&self) -> String {
        content_text(&self.term.lock_unfair())
    }

    /// The currently visible screen (scrollback excluded), for accessibility.
    pub fn visible_screen_text(&self) -> String {
        term_visible_screen_text(&self.term.lock_unfair())
    }

    pub fn viewport_lines(&self) -> usize {
        screen_lines(&self.term.lock_unfair())
    }

    //To test:
    //- Activate match on terminal (scrolling and selection)
    //- Editor search snapping behavior

    pub fn activate_match(&mut self, index: usize) {
        if let Some(search_match) = self.matches.get(index).cloned() {
            self.set_selection(Some(Selection::simple_range(search_match)));
            if self.vi_mode_enabled {
                self.events
                    .push_back(InternalEvent::MoveViCursorToPoint(search_match.end()));
            } else {
                self.events
                    .push_back(InternalEvent::ScrollToPoint(search_match.start()));
            }
        }
    }

    pub fn select_matches(&mut self, matches: &[Range]) {
        let matches_to_select = self
            .matches
            .iter()
            .filter(|self_match| matches.contains(self_match))
            .cloned()
            .collect::<Vec<_>>();
        for match_to_select in matches_to_select {
            self.set_selection(Some(Selection::simple_range(match_to_select)));
        }
    }

    pub fn select_all(&mut self) {
        let term = self.term.lock();
        let range = full_content_range(&term);
        drop(term);
        self.set_selection(Some(Selection::simple_range(range)));
    }

    fn set_selection(&mut self, selection: Option<Selection>) {
        self.events
            .push_back(InternalEvent::SetSelection(selection));
    }

    pub fn copy(&mut self, keep_selection: Option<bool>) {
        self.events.push_back(InternalEvent::Copy(keep_selection));
    }

    pub fn clear(&mut self) {
        self.events.push_back(InternalEvent::Clear)
    }

    pub fn shrink_to_used(&mut self) {
        shrink_to_used(&mut self.term.lock());
    }

    pub fn scroll_line_up(&mut self) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::Delta(1)));
    }

    pub fn scroll_up_by(&mut self, lines: usize) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::Delta(lines as i32)));
    }

    pub fn scroll_line_down(&mut self) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::Delta(-1)));
    }

    pub fn scroll_down_by(&mut self, lines: usize) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::Delta(-(lines as i32))));
    }

    pub fn scroll_page_up(&mut self) {
        self.events.push_back(InternalEvent::Scroll(Scroll::PageUp));
    }

    pub fn scroll_page_down(&mut self) {
        self.events
            .push_back(InternalEvent::Scroll(Scroll::PageDown));
    }

    pub fn scroll_to_top(&mut self) {
        self.events.push_back(InternalEvent::Scroll(Scroll::Top));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.events.push_back(InternalEvent::Scroll(Scroll::Bottom));
    }

    pub fn scrolled_to_top(&self) -> bool {
        self.last_content.scrolled_to_top
    }

    pub fn scrolled_to_bottom(&self) -> bool {
        self.last_content.scrolled_to_bottom
    }

    ///Resize the terminal and the PTY.
    pub fn set_size(&mut self, new_bounds: TerminalBounds) {
        let new_bounds = normalize_terminal_bounds(new_bounds);

        let old_bounds = self.last_content.terminal_bounds;
        self.last_content.terminal_bounds = new_bounds;

        // Avoid spamming PTY resizes on pixel-level size changes (e.g. while dragging edges),
        // since those can generate excessive SIGWINCH/reflows and cause visible flicker.
        let requires_resize = old_bounds.num_lines() != new_bounds.num_lines()
            || old_bounds.num_columns() != new_bounds.num_columns()
            || old_bounds.cell_width != new_bounds.cell_width
            || old_bounds.line_height != new_bounds.line_height;

        if !requires_resize {
            return;
        }

        match self.events.back_mut() {
            Some(InternalEvent::Resize(pending_bounds)) => *pending_bounds = new_bounds,
            _ => self.events.push_back(InternalEvent::Resize(new_bounds)),
        }
    }

    /// Write the Input payload to the PTY, if applicable.
    /// (This is a no-op for display-only terminals.)
    fn write_to_pty(&self, input: impl Into<Cow<'static, [u8]>>) {
        let input = input.into();
        #[cfg(any(test, feature = "test-support"))]
        self.pty_write_log.borrow_mut().push(input.to_vec());
        if let TerminalType::Pty { pty_tx, .. } = &self.terminal_type {
            if log::log_enabled!(log::Level::Debug) {
                if let Ok(str) = str::from_utf8(&input) {
                    log::debug!("Writing to PTY: {:?}", str);
                } else {
                    log::debug!("Writing to PTY: {:?}", input);
                }
            }
            pty_tx.notify(input);
        }
    }

    pub fn input(&mut self, input: impl Into<Cow<'static, [u8]>>) {
        self.keyboard_input_sent = true;
        self.complete_init_command_startup_handshake();
        self.write_input(input);
    }

    /// Sends a shell-level marker command and returns a task that completes when
    /// the marker appears in terminal output. Already complete for non-PTY
    /// terminals or those whose child has exited.
    ///
    /// Call at most once per terminal: a second handshake drops the previous
    /// `Sender`, which would write the init command twice.
    pub fn start_init_command_startup_handshake(&mut self) -> Task<()> {
        if !self.is_pty() || self.child_exited.is_some() {
            return Task::ready(());
        }

        debug_assert!(
            self.init_command_startup_tx.is_none(),
            "start_init_command_startup_handshake called while a handshake is already in flight"
        );

        let (startup_tx, startup_rx) = async_channel::bounded(1);
        let startup_task = self.background_executor.spawn(async move {
            match startup_rx.recv().await {
                Ok(()) | Err(_) => {}
            }
        });

        let marker_id = NEXT_INIT_COMMAND_STARTUP_MARKER_ID.fetch_add(1, Ordering::Relaxed);
        self.init_command_startup_marker = Some(init_command_startup_marker(marker_id));
        self.init_command_startup_tx = Some(startup_tx);

        let shell_kind = self.template.shell.shell_kind(self.path_style.is_windows());
        let mut input = init_command_startup_marker_command(shell_kind, marker_id).into_bytes();
        input.push(b'\x0d');
        self.write_to_pty(input);

        startup_task
    }

    fn detect_init_command_startup_marker(&mut self) {
        let Some(marker) = self.init_command_startup_marker.as_deref() else {
            return;
        };

        let has_marker = {
            let term = self.term.lock_unfair();
            last_non_empty_lines(&term, INIT_COMMAND_STARTUP_MARKER_SEARCH_LINES)
                .iter()
                .any(|line| line.contains(marker))
        };

        if has_marker {
            self.complete_init_command_startup_handshake();
        }
    }

    fn complete_init_command_startup_handshake(&mut self) {
        self.init_command_startup_marker = None;
        if let Some(startup_tx) = self.init_command_startup_tx.take() {
            match startup_tx.try_send(()) {
                Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
                Err(async_channel::TrySendError::Closed(())) => {}
            }
        }
    }

    /// Write a programmatically-generated command to the PTY as if it had been
    /// typed, without marking the terminal as having received user keyboard
    /// input.
    pub fn write_init_command(&mut self, input: impl Into<Cow<'static, [u8]>>) {
        self.write_input(input);
    }

    pub fn is_pty(&self) -> bool {
        matches!(self.terminal_type, TerminalType::Pty { .. })
    }

    pub fn write_init_command_after_startup(
        &mut self,
        input: impl Into<Cow<'static, [u8]>>,
        cx: &mut Context<Self>,
    ) -> bool {
        // Ends the handshake even if the marker was never seen (timeout
        // fallback), so detection stops scanning on every wakeup.
        self.complete_init_command_startup_handshake();

        if self.keyboard_input_sent || self.child_exited.is_some() {
            return false;
        }

        self.clear_for_init_command(cx);
        self.write_init_command(input);
        true
    }

    fn clear_for_init_command(&mut self, cx: &mut Context<Self>) {
        let mut term = self.term.lock_unfair();
        clear_saved_screen(&mut term);
        self.last_content = make_content(&term, &self.last_content);
        drop(term);
        self.reset_cwd_history();
        cx.emit(Event::Wakeup);
    }

    fn write_input(&mut self, input: impl Into<Cow<'static, [u8]>>) {
        let input = input.into();
        if !self.is_remote_terminal && input.contains(&b'\r') {
            let term = self.term.lock_unfair();
            self.pending_cwd_boundary = Some(Self::scrollback_position(
                term.grid().cursor.point.line.0,
                term.history_size(),
            ));
        }

        self.events.push_back(InternalEvent::Scroll(Scroll::Bottom));
        self.events.push_back(InternalEvent::SetSelection(None));
        #[cfg(any(test, feature = "test-support"))]
        self.input_log.push(input.to_vec());

        self.write_to_pty(input);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn take_input_log(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.input_log)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn take_pty_write_log(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(self.pty_write_log.get_mut())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn keyboard_input_sent(&self) -> bool {
        self.keyboard_input_sent
    }

    pub fn toggle_vi_mode(&mut self) {
        self.events.push_back(InternalEvent::ToggleViMode);
    }

    pub fn vi_motion(&mut self, keystroke: &Keystroke) {
        if !self.vi_mode_enabled {
            return;
        }

        let key: Cow<'_, str> = if keystroke.modifiers.shift {
            Cow::Owned(keystroke.key.to_uppercase())
        } else {
            Cow::Borrowed(keystroke.key.as_str())
        };

        let motion: Option<ViMotion> = match key.as_ref() {
            "h" | "left" => Some(ViMotion::Left),
            "j" | "down" => Some(ViMotion::Down),
            "k" | "up" => Some(ViMotion::Up),
            "l" | "right" => Some(ViMotion::Right),
            "w" => Some(ViMotion::WordRight),
            "b" if !keystroke.modifiers.control => Some(ViMotion::WordLeft),
            "e" => Some(ViMotion::WordRightEnd),
            "%" => Some(ViMotion::Bracket),
            "$" => Some(ViMotion::Last),
            "0" => Some(ViMotion::First),
            "^" => Some(ViMotion::FirstOccupied),
            "H" => Some(ViMotion::High),
            "M" => Some(ViMotion::Middle),
            "L" => Some(ViMotion::Low),
            "{" => Some(ViMotion::ParagraphUp),
            "}" => Some(ViMotion::ParagraphDown),
            _ => None,
        };

        if let Some(motion) = motion {
            let cursor = self.last_content.cursor.point;
            let cursor_pos = GpuiPoint {
                x: cursor.column as f32 * self.last_content.terminal_bounds.cell_width,
                y: cursor.line as f32 * self.last_content.terminal_bounds.line_height,
            };
            self.events
                .push_back(InternalEvent::UpdateSelection(cursor_pos));
            self.events.push_back(InternalEvent::ViMotion(motion));
            return;
        }

        let scroll_motion = match key.as_ref() {
            "g" => Some(Scroll::Top),
            "G" => Some(Scroll::Bottom),
            "b" if keystroke.modifiers.control => Some(Scroll::PageUp),
            "f" if keystroke.modifiers.control => Some(Scroll::PageDown),
            "d" if keystroke.modifiers.control => {
                let amount = self.last_content.terminal_bounds.line_height().to_f64() as i32 / 2;
                Some(Scroll::Delta(-amount))
            }
            "u" if keystroke.modifiers.control => {
                let amount = self.last_content.terminal_bounds.line_height().to_f64() as i32 / 2;
                Some(Scroll::Delta(amount))
            }
            _ => None,
        };

        if let Some(scroll_motion) = scroll_motion {
            self.events.push_back(InternalEvent::Scroll(scroll_motion));
            return;
        }

        match key.as_ref() {
            "v" => {
                let point = self.last_content.cursor.point;
                let selection_type = SelectionType::Simple;
                let side = SelectionSide::Right;
                let selection = Selection::new(selection_type, point, side);
                self.events
                    .push_back(InternalEvent::SetSelection(Some(selection)));
            }

            "escape" => {
                self.events.push_back(InternalEvent::SetSelection(None));
            }

            "y" => {
                self.copy(Some(false));
            }

            "i" => {
                self.scroll_to_bottom();
                self.toggle_vi_mode();
            }
            _ => {}
        }
    }

    pub fn try_keystroke(&mut self, keystroke: &Keystroke, option_as_meta: bool) -> bool {
        if self.vi_mode_enabled {
            self.vi_motion(keystroke);
            return true;
        }

        // Keep default terminal behavior
        let esc = to_esc_str(keystroke, self.last_content.mode, option_as_meta);
        if let Some(esc) = esc {
            match esc {
                Cow::Borrowed(string) => self.input(string.as_bytes()),
                Cow::Owned(string) => self.input(string.into_bytes()),
            };
            true
        } else {
            false
        }
    }

    pub fn try_modifiers_change(
        &mut self,
        modifiers: &Modifiers,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .last_content
            .terminal_bounds
            .bounds
            .contains(&window.mouse_position())
            && modifiers.secondary()
        {
            self.refresh_hovered_word(window);
        }
        cx.notify();
    }

    ///Paste text into the terminal
    pub fn paste(&mut self, text: &str) {
        let paste_text = if self.last_content.mode.contains(Modes::BRACKETED_PASTE) {
            format!("{}{}{}", "\x1b[200~", text.replace('\x1b', ""), "\x1b[201~")
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };

        self.input(paste_text.into_bytes());
    }

    pub fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let term = self.term.clone();
        let mut terminal = term.lock_unfair();
        //Note that the ordering of events matters for event processing
        while let Some(e) = self.events.pop_front() {
            self.process_terminal_event(&e, &mut terminal, window, cx)
        }

        self.last_content = make_content(&terminal, &self.last_content);
    }

    pub fn with_renderable_cells<R>(&self, f: impl for<'a> FnOnce(RenderableCells<'a>) -> R) -> R {
        let term = self.term.lock_unfair();
        let content = term.renderable_content();
        f(RenderableCells::new(content.display_iter))
    }

    pub fn get_content(&self) -> String {
        let term = self.term.lock_unfair();
        content_text(&term)
    }

    pub fn last_n_non_empty_lines(&self, n: usize) -> Vec<String> {
        let terminal = self.term.lock_unfair();
        last_non_empty_lines(&terminal, n)
    }

    pub fn focus_in(&self) {
        if self.last_content.mode.contains(Modes::FOCUS_IN_OUT) {
            self.write_to_pty("\x1b[I".as_bytes());
        }
    }

    pub fn focus_out(&mut self) {
        if self.last_content.mode.contains(Modes::FOCUS_IN_OUT) {
            self.write_to_pty("\x1b[O".as_bytes());
        }
    }

    fn mouse_changed(&mut self, point: Point, side: SelectionSide) -> bool {
        match self.last_mouse {
            Some((old_point, old_side)) => {
                if old_point == point && old_side == side {
                    false
                } else {
                    self.last_mouse = Some((point, side));
                    true
                }
            }
            None => {
                self.last_mouse = Some((point, side));
                true
            }
        }
    }

    pub fn mouse_mode(&self, shift: bool) -> bool {
        self.last_content.mode.intersects(Modes::MOUSE_MODE) && !shift
    }

    pub fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if self.mouse_mode(e.modifiers.shift) {
            // A ctrl/cmd press on a link suppressed its button-press report in
            // `mouse_down`. Since the app never saw the press, we must swallow
            // the whole gesture rather than forward later motion/release
            // reports, which would be a press-less (malformed) sequence.
            // `mouse_up` resolves it: release on the same link opens it,
            // otherwise the gesture is dropped.
            if self.mouse_down_hyperlink.is_none() {
                let (point, side) = grid_point_and_side(
                    position,
                    self.last_content.terminal_bounds,
                    self.last_content.display_offset,
                );

                if self.mouse_changed(point, side) {
                    let bytes = mouse_moved_report(
                        point,
                        e.pressed_button,
                        e.modifiers,
                        self.last_content.mode,
                    );

                    if let Some(bytes) = bytes {
                        self.write_to_pty(bytes);
                    }
                }
            }
        } else {
            self.schedule_find_hyperlink(e.modifiers, e.position);
        }
        cx.notify();
    }

    fn schedule_find_hyperlink(&mut self, modifiers: Modifiers, position: GpuiPoint<Pixels>) {
        if self.selection_phase == SelectionPhase::Selecting
            || !modifiers.secondary()
            || !self.last_content.terminal_bounds.bounds.contains(&position)
        {
            self.last_content.last_hovered_word = None;
            return;
        }

        // Throttle hyperlink searches to avoid excessive processing
        let now = Instant::now();
        if self
            .last_hyperlink_search_position
            .map_or(true, |last_pos| {
                // Only search if mouse moved significantly or enough time passed
                let distance_moved = ((position.x - last_pos.x).abs()
                    + (position.y - last_pos.y).abs())
                    > FIND_HYPERLINK_THROTTLE_PX;
                let time_elapsed = now.duration_since(self.last_mouse_move_time).as_millis() > 100;
                distance_moved || time_elapsed
            })
        {
            self.last_mouse_move_time = now;
            self.last_hyperlink_search_position = Some(position);
            self.events.push_back(InternalEvent::FindHyperlink(
                position - self.last_content.terminal_bounds.bounds.origin,
                false,
            ));
        }
    }

    pub fn select_word_at_event_position(&mut self, e: &MouseDownEvent) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        let (point, side) = grid_point_and_side(
            position,
            self.last_content.terminal_bounds,
            self.last_content.display_offset,
        );
        let selection = Selection::new(SelectionType::Semantic, point, side);
        self.events
            .push_back(InternalEvent::SetSelection(Some(selection)));
    }

    pub fn mouse_drag(
        &mut self,
        e: &MouseMoveEvent,
        region: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if !self.mouse_mode(e.modifiers.shift) {
            if let Some(hyperlink) = &self.mouse_down_hyperlink {
                let point = grid_point(
                    position,
                    self.last_content.terminal_bounds,
                    self.last_content.display_offset,
                );

                if !hyperlink.range.contains(point) {
                    self.mouse_down_hyperlink = None;
                } else {
                    return;
                }
            }

            // Ignore tiny pointer movements so that a click that jitters by a
            // pixel or two (e.g. the window-focusing click) does not begin a
            // selection. Mirrors the drag threshold used by gpui's `div`.
            if self.selection_phase != SelectionPhase::Selecting
                && let Some(mouse_down_position) = self.mouse_down_position
                && (e.position - mouse_down_position).magnitude() <= SELECTION_DRAG_THRESHOLD
            {
                return;
            }

            self.selection_phase = SelectionPhase::Selecting;
            // Alacritty has the same ordering, of first updating the selection
            // then scrolling 15ms later
            self.events
                .push_back(InternalEvent::UpdateSelection(position));

            // Doesn't make sense to scroll the alt screen
            if !self.last_content.mode.contains(Modes::ALT_SCREEN) {
                let scroll_lines = match self.drag_line_delta(e, region) {
                    Some(value) => value,
                    None => return,
                };

                self.events
                    .push_back(InternalEvent::Scroll(Scroll::Delta(scroll_lines)));
            }

            cx.notify();
        }
    }

    fn drag_line_delta(&self, e: &MouseMoveEvent, region: Bounds<Pixels>) -> Option<i32> {
        let top = region.origin.y;
        let bottom = region.bottom_left().y;

        let scroll_lines = if e.position.y < top {
            let scroll_delta = (top - e.position.y).pow(1.1);
            (scroll_delta / self.last_content.terminal_bounds.line_height).ceil() as i32
        } else if e.position.y > bottom {
            let scroll_delta = -((e.position.y - bottom).pow(1.1));
            (scroll_delta / self.last_content.terminal_bounds.line_height).floor() as i32
        } else {
            return None;
        };

        Some(scroll_lines.clamp(-3, 3))
    }

    pub fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        let point = grid_point(
            position,
            self.last_content.terminal_bounds,
            self.last_content.display_offset,
        );

        if e.button == MouseButton::Left && e.modifiers.alt && !e.modifiers.secondary() {
            if let Some(bytes) = self.click_to_move_bytes(point) {
                if !bytes.is_empty() {
                    self.write_to_pty(bytes);
                }
                return;
            }
        }

        if e.button == MouseButton::Left
            && e.modifiers.secondary()
            && (TerminalSettings::get_global(cx).open_links_in_mouse_mode
                || !self.mouse_mode(e.modifiers.shift))
        {
            self.mouse_down_hyperlink = self.find_hyperlink_at_point(point);

            if self.mouse_down_hyperlink.is_some() {
                return;
            }
        }

        if self.mouse_mode(e.modifiers.shift) {
            let bytes =
                mouse_button_report(point, e.button, e.modifiers, true, self.last_content.mode);

            if let Some(bytes) = bytes {
                self.write_to_pty(bytes);
            }
        } else {
            match e.button {
                MouseButton::Left => {
                    self.mouse_down_position = Some(e.position);
                    let (point, side) = grid_point_and_side(
                        position,
                        self.last_content.terminal_bounds,
                        self.last_content.display_offset,
                    );

                    let selection_type = match e.click_count {
                        0 => return, //This is a release
                        1 => Some(SelectionType::Simple),
                        2 => Some(SelectionType::Semantic),
                        3 => {
                            if e.modifiers.secondary() {
                                let last_col = self
                                    .last_content
                                    .terminal_bounds
                                    .num_columns()
                                    .saturating_sub(1);
                                let history_size = self.term.lock_unfair().history_size() as i32;
                                if let TripleClickKind::CommandOutput(range) = triple_click_kind(
                                    true,
                                    &self.prompt_markers,
                                    point.line,
                                    last_col,
                                    history_size,
                                ) {
                                    self.events.push_back(InternalEvent::SetSelection(Some(
                                        Selection::simple_range(Range::new(range.start, range.end)),
                                    )));
                                    return;
                                }
                            }
                            Some(SelectionType::Lines)
                        }
                        _ => None,
                    };

                    if selection_type == Some(SelectionType::Simple) && e.modifiers.shift {
                        if self.last_content.selection.is_some() {
                            // Shift+click extends the existing selection to this point.
                            self.events
                                .push_back(InternalEvent::UpdateSelection(position));
                        } else {
                            // With no selection yet, Shift is the escape hatch for
                            // selecting text while an app has mouse tracking enabled,
                            // so anchor a selection here for the drag to extend.
                            self.events.push_back(InternalEvent::SetSelection(Some(
                                Selection::new(SelectionType::Simple, point, side),
                            )));
                        }
                        return;
                    }

                    let selection = selection_type
                        .map(|selection_type| Selection::new(selection_type, point, side));

                    if let Some(selection) = selection {
                        self.events
                            .push_back(InternalEvent::SetSelection(Some(selection)));
                    }
                }
                _ => {}
            }
        }
    }

    pub fn mouse_up(&mut self, e: &MouseUpEvent, cx: &Context<Self>) {
        let setting = TerminalSettings::get_global(cx);

        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if let Some(mouse_down_hyperlink) = self.mouse_down_hyperlink.take() {
            let point = grid_point(
                position,
                self.last_content.terminal_bounds,
                self.last_content.display_offset,
            );

            if self
                .find_hyperlink_at_point(point)
                .is_some_and(|mouse_up_hyperlink| mouse_up_hyperlink == mouse_down_hyperlink)
            {
                self.events
                    .push_back(InternalEvent::ProcessHyperlink(mouse_down_hyperlink, true));
                self.selection_phase = SelectionPhase::Ended;
                self.last_mouse = None;
                self.mouse_down_position = None;
                return;
            }

            if self.mouse_mode(e.modifiers.shift) {
                self.selection_phase = SelectionPhase::Ended;
                self.last_mouse = None;
                self.mouse_down_position = None;
                return;
            }
        }

        if self.mouse_mode(e.modifiers.shift) {
            let point = grid_point(
                position,
                self.last_content.terminal_bounds,
                self.last_content.display_offset,
            );

            let bytes =
                mouse_button_report(point, e.button, e.modifiers, false, self.last_content.mode);

            if let Some(bytes) = bytes {
                self.write_to_pty(bytes);
            }
        } else {
            if e.button == MouseButton::Left && setting.copy_on_select {
                self.copy(Some(true));
            }

            //Hyperlinks
            if self.selection_phase == SelectionPhase::Ended {
                let mouse_cell_index =
                    content_index_for_mouse(position, &self.last_content.terminal_bounds);
                if let Some(link) = self
                    .last_content
                    .cells
                    .get(mouse_cell_index)
                    .and_then(|cell| cell.hyperlink())
                {
                    cx.open_url(link.uri());
                } else if e.modifiers.secondary() {
                    self.events
                        .push_back(InternalEvent::FindHyperlink(position, true));
                }
            }
        }

        self.selection_phase = SelectionPhase::Ended;
        self.last_mouse = None;
        self.mouse_down_position = None;
    }

    ///Scroll the terminal
    pub fn scroll_wheel(&mut self, e: &ScrollWheelEvent, scroll_multiplier: f32) {
        let mouse_mode = self.mouse_mode(e.shift);
        let scroll_multiplier = if mouse_mode { 1. } else { scroll_multiplier };

        if let Some(scroll_lines) = self.determine_scroll_lines(e, scroll_multiplier)
            && scroll_lines != 0
        {
            if mouse_mode {
                let point = grid_point(
                    e.position - self.last_content.terminal_bounds.bounds.origin,
                    self.last_content.terminal_bounds,
                    self.last_content.display_offset,
                );

                if let Some(scrolls) = scroll_report(point, scroll_lines, e, self.last_content.mode)
                {
                    for scroll in scrolls {
                        self.write_to_pty(scroll);
                    }
                };
            } else if self
                .last_content
                .mode
                .contains(Modes::ALT_SCREEN | Modes::ALTERNATE_SCROLL)
                && !e.shift
            {
                self.write_to_pty(alt_scroll(scroll_lines));
            } else {
                self.events
                    .push_back(InternalEvent::Scroll(Scroll::Delta(scroll_lines)));
            }
        }
    }

    fn refresh_hovered_word(&mut self, window: &Window) {
        self.schedule_find_hyperlink(window.modifiers(), window.mouse_position());
    }

    fn determine_scroll_lines(
        &mut self,
        e: &ScrollWheelEvent,
        scroll_multiplier: f32,
    ) -> Option<i32> {
        let line_height = self.last_content.terminal_bounds.line_height;
        match e.touch_phase {
            /* Reset scroll state on started */
            TouchPhase::Started => {
                self.scroll_px = px(0.);
                None
            }
            /* Calculate the appropriate scroll lines */
            TouchPhase::Moved => {
                let old_offset = (self.scroll_px / line_height) as i32;

                self.scroll_px += e.delta.pixel_delta(line_height).y * scroll_multiplier;

                let new_offset = (self.scroll_px / line_height) as i32;

                // Whenever we hit the edges, reset our stored scroll to 0
                // so we can respond to changes in direction quickly
                self.scroll_px %= self.last_content.terminal_bounds.height();

                Some(new_offset - old_offset)
            }
            // Cancellation does not commit a scroll, same as a plain end.
            TouchPhase::Ended | TouchPhase::Cancelled => None,
        }
    }

    pub fn find_matches(&self, searcher: Search, cx: &Context<Self>) -> Task<Vec<Range>> {
        let term = self.term.clone();
        cx.background_spawn(async move {
            let term = term.lock();
            search_matches(&term, searcher)
        })
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        if self.is_remote_terminal {
            // We can't yet reliably detect the working directory of a shell on the
            // SSH host. Until we can do that, it doesn't make sense to display
            // the working directory on the client and persist that.
            None
        } else {
            self.client_side_working_directory()
        }
    }

    /// Normalizes the command name of the foreground process, if one is known.
    pub fn foreground_process_command_name(&self) -> Option<String> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => info
                .current
                .read()
                .as_ref()
                .and_then(|process| foreground_process_command_from_argv(&process.argv)),
            TerminalType::DisplayOnly => None,
        }
    }

    /// Returns the working directory of the process that's connected to the PTY.
    /// That means it returns the working directory of the local shell or program
    /// that's running inside the terminal.
    ///
    /// This does *not* return the working directory of the shell that runs on the
    /// remote host, in case Zed is connected to a remote host.
    fn client_side_working_directory(&self) -> Option<PathBuf> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => info
                .current
                .read()
                .as_ref()
                .map(|process| process.cwd.clone()),
            TerminalType::DisplayOnly => None,
        }
    }

    pub(crate) fn record_cwd_change(&mut self, new_working_directory: PathBuf) {
        if self.is_remote_terminal {
            return;
        }

        let scrollback_position = self.pending_cwd_boundary.take().unwrap_or_else(|| {
            let term = self.term.lock_unfair();
            Self::scrollback_position(term.grid().cursor.point.line.0, term.history_size())
        });
        self.cwd_history.push(CwdHistoryEntry {
            scrollback_position,
            working_directory: new_working_directory,
        });
    }

    fn reset_cwd_history(&mut self) {
        self.pending_cwd_boundary = None;
        self.cwd_history = self
            .working_directory()
            .map(|working_directory| {
                vec![CwdHistoryEntry {
                    scrollback_position: i32::MIN,
                    working_directory,
                }]
            })
            .unwrap_or_default();
    }

    fn cwd_at_line(&self, line: i32, history_size: usize) -> Option<PathBuf> {
        // Once the scrollback cap is reached, evictions move retained lines without changing
        // `history_size`, so stored row offsets no longer identify their original lines.
        if self.is_remote_terminal
            || self.cwd_history.is_empty()
            || history_size >= self.term_config.scrolling_history
        {
            return self.working_directory();
        }
        let scrollback_position = Self::scrollback_position(line, history_size);
        self.cwd_history
            .iter()
            .rev()
            .find(|entry| entry.scrollback_position <= scrollback_position)
            .map(|entry| entry.working_directory.clone())
            .or_else(|| self.working_directory())
    }

    fn scrollback_position(line: i32, history_size: usize) -> i32 {
        let history_size = i32::try_from(history_size).unwrap_or(i32::MAX);
        history_size.saturating_add(line)
    }

    pub fn title(&self, truncate: bool) -> String {
        const MAX_CHARS: usize = 25;
        match &self.task {
            Some(task_state) => {
                if truncate {
                    truncate_and_trailoff(&task_state.spawned_task.label, MAX_CHARS)
                } else {
                    task_state.spawned_task.full_label.clone()
                }
            }
            None => self
                .title_override
                .as_ref()
                .map(|title_override| title_override.to_string())
                .unwrap_or_else(|| match &self.terminal_type {
                    TerminalType::Pty { info, .. } => info
                        .current
                        .read()
                        .as_ref()
                        .map(|fpi| {
                            let process_file = fpi
                                .cwd
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default();

                            let argv = fpi.argv.as_slice();
                            let process_name = format!(
                                "{}{}",
                                fpi.name,
                                if !argv.is_empty() {
                                    format!(" {}", (argv[1..]).join(" "))
                                } else {
                                    "".to_string()
                                }
                            );
                            let (process_file, process_name) = if truncate {
                                (
                                    truncate_and_trailoff(&process_file, MAX_CHARS),
                                    truncate_and_trailoff(&process_name, MAX_CHARS),
                                )
                            } else {
                                (process_file, process_name)
                            };
                            format!("{process_file} — {process_name}")
                        })
                        .unwrap_or_else(|| "Terminal".to_string()),
                    TerminalType::DisplayOnly => "Terminal".to_string(),
                }),
        }
    }

    pub fn kill_active_task(&mut self) {
        if let Some(task) = self.task()
            && task.status == TaskStatus::Running
        {
            match &self.terminal_type {
                TerminalType::Pty { info, .. } => {
                    // First kill the foreground process group (the command running in the shell)
                    info.kill_current_process();
                    // Then kill the shell itself so that the terminal exits properly
                    // and wait_for_completed_task can complete
                    info.kill_child_process();
                }
                TerminalType::DisplayOnly => {
                    // Non-PTY task terminals own their subprocess directly.
                    if let Some(subprocess) = &self.subprocess {
                        subprocess.kill();
                    }
                }
            }
        }
    }

    pub fn pid(&self) -> Option<sysinfo::Pid> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => info.pid(),
            TerminalType::DisplayOnly => None,
        }
    }

    pub fn pid_getter(&self) -> Option<&ProcessIdGetter> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => Some(info.pid_getter()),
            TerminalType::DisplayOnly => None,
        }
    }

    /// Whether the PTY has a non-shell foreground job (close-confirm "dirty").
    ///
    /// Idle interactive shells alone are **not** dirty; a foreground process
    /// group that differs from the shell child is dirty (`sleep`, `vim`, …).
    pub fn looks_busy(&self) -> bool {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => {
                let shell = info.pid_getter().fallback_pid().as_u32();
                let fg = info.pid().map(|p| p.as_u32());
                terminal_looks_busy(fg, shell)
            }
            TerminalType::DisplayOnly => false,
        }
    }

    pub fn task(&self) -> Option<&TaskState> {
        self.task.as_ref()
    }

    pub fn wait_for_completed_task(&self, cx: &App) -> Task<Option<ExitStatus>> {
        if let Some(task) = self.task() {
            if task.status == TaskStatus::Running {
                let completion_receiver = task.completion_rx.clone();
                return cx.spawn(async move |_| completion_receiver.recv().await.ok().flatten());
            } else if let Ok(status) = task.completion_rx.try_recv() {
                return Task::ready(status);
            }
        }
        Task::ready(None)
    }

    fn register_task_finished(
        &mut self,
        exit_status: Option<ExitStatus>,
        cx: &mut Context<Terminal>,
    ) {
        if let Some(tx) = &self.completion_tx {
            tx.try_send(exit_status).ok();
        }
        if let Some(e) = exit_status {
            self.child_exited = Some(e);
        }
        self.complete_init_command_startup_handshake();
        let task = match &mut self.task {
            Some(task) => task,
            None => {
                // For interactive shells (no task), we need to differentiate:
                // 1. User-initiated exits (typed "exit", Ctrl+D, etc.) - always close,
                //    even if the shell exits with a non-zero code (e.g. after `false`).
                // 2. Shell spawn failures (bad $SHELL) - don't close, so the user sees
                //    the error. Spawn failures never receive keyboard input.
                let should_close = if self.keyboard_input_sent {
                    true
                } else {
                    self.child_exited.is_none_or(|e| e.code() == Some(0))
                };
                if should_close {
                    cx.emit(Event::CloseTerminal);
                }
                return;
            }
        };
        if task.status != TaskStatus::Running {
            return;
        }
        match exit_status.and_then(|e| e.code()) {
            Some(error_code) => {
                task.status.register_task_exit(error_code);
            }
            None => {
                task.status.register_terminal_exit();
            }
        };

        let (finished_successfully, task_line, command_line) = task_summary(task, exit_status);
        let mut lines_to_show = Vec::new();
        if task.spawned_task.show_summary {
            lines_to_show.push(task_line.as_str());
        }
        if task.spawned_task.show_command {
            lines_to_show.push(command_line.as_str());
        }
        let hide = task.spawned_task.hide;

        if !lines_to_show.is_empty() {
            // SAFETY: the invocation happens on non `TaskStatus::Running` tasks, once,
            // after either `AlacTermEvent::Exit` or `AlacTermEvent::ChildExit` events that are spawned
            // when Zed task finishes and no more output is made.
            // After the task summary is output once, no more text is appended to the terminal.
            unsafe { append_text_to_term(&mut self.term.lock(), &lines_to_show) };
        }

        match hide {
            HideStrategy::Never => {}
            HideStrategy::Always => {
                cx.emit(Event::CloseTerminal);
            }
            HideStrategy::OnSuccess => {
                if finished_successfully {
                    cx.emit(Event::CloseTerminal);
                }
            }
        }
    }

    pub fn vi_mode_enabled(&self) -> bool {
        self.vi_mode_enabled
    }

    pub fn clone_builder(&self, cx: &App, cwd: Option<PathBuf>) -> Task<Result<TerminalBuilder>> {
        let working_directory = self.working_directory().or_else(|| cwd);
        TerminalBuilder::new(
            working_directory,
            None,
            self.template.shell.clone(),
            self.template.env.clone(),
            self.template.cursor_shape,
            self.template.alternate_scroll,
            self.template.max_scroll_history_lines,
            self.template.path_hyperlink_regexes.clone(),
            self.template.path_hyperlink_timeout_ms,
            self.is_remote_terminal,
            self.template.window_id,
            None,
            cx,
            self.activation_script.clone(),
            self.path_style,
        )
    }
}

const TASK_DELIMITER: &str = "⏵ ";
fn task_summary(task: &TaskState, exit_status: Option<ExitStatus>) -> (bool, String, String) {
    let escaped_full_label = task
        .spawned_task
        .full_label
        .replace("\r\n", "\r")
        .replace('\n', "\r");
    let task_label = |suffix: &str| format!("{TASK_DELIMITER}Task `{escaped_full_label}` {suffix}");
    let (success, task_line) = match exit_status {
        Some(status) => {
            let code = status.code();
            let signal = status.signal();

            match (code, signal) {
                (Some(0), _) => (true, task_label("finished successfully")),
                (Some(code), _) => (
                    false,
                    task_label(&format!("finished with exit code: {code}")),
                ),
                (None, Some(signal)) => (
                    false,
                    task_label(&format!("terminated by signal: {signal}")),
                ),
                (None, None) => (false, task_label("finished")),
            }
        }
        None => (false, task_label("finished")),
    };
    let escaped_command_label = task
        .spawned_task
        .command_label
        .replace("\r\n", "\r")
        .replace('\n', "\r");
    let command_line = format!("{TASK_DELIMITER}Command: {escaped_command_label}");
    (success, task_line, command_line)
}

/// Converts bare LFs into CRLFs so output captured from a pipe (rather than a
/// PTY) wraps correctly in Alacritty. A PTY's line discipline performs this
/// `ONLCR` translation for us; piped output (e.g. `ls` run outside a PTY) only
/// emits `\n`, which moves Alacritty's cursor down without returning it to
/// column zero and makes the rendered output look misaligned. Alacritty has no
/// setting for this, so we insert a `\r` before each `\n` that lacks one.
fn convert_lf_to_crlf(bytes: &[u8], previous_byte_was_cr: &mut bool) -> Vec<u8> {
    let mut converted = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte == b'\n' && !*previous_byte_was_cr {
            converted.push(b'\r');
        }
        converted.push(byte);
        *previous_byte_was_cr = byte == b'\r';
    }
    converted
}

/// Owns a non-PTY task subprocess and the background task pumping its output
/// into the terminal emulator. Used by headless hosts (e.g. the eval CLI) where
/// PTY allocation fails with `ENOTTY`. Dropping this kills the child.
struct SubprocessHandle {
    child: Arc<parking_lot::Mutex<Option<util::process::Child>>>,
    _reader: Task<()>,
}

impl SubprocessHandle {
    fn kill(&self) {
        if let Some(child) = self.child.lock().as_mut() {
            child.kill().log_err();
        }
    }
}

/// Spawns `program`/`args` as a plain subprocess with piped stdout/stderr and
/// drives its output into `term`, mirroring what the Alacritty event loop does
/// for a PTY but without one. Used when [`HeadlessTerminal`] is enabled.
fn spawn_task_subprocess(
    program: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    working_directory: Option<PathBuf>,
    term: Arc<AlacrittyTermLock>,
    events_tx: futures::channel::mpsc::UnboundedSender<PtyEvent>,
    executor: &BackgroundExecutor,
) -> Result<SubprocessHandle> {
    use futures::io::AsyncReadExt as _;
    use std::process::Stdio;

    let mut command = util::command::new_std_command(&program);
    command.args(&args);
    command.envs(&env);
    if let Some(directory) = &working_directory {
        command.current_dir(directory);
    }

    let mut child =
        util::process::Child::spawn(command, Stdio::null(), Stdio::piped(), Stdio::piped())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(parking_lot::Mutex::new(Some(child)));

    let reader = executor.spawn({
        let child = child.clone();
        let executor = executor.clone();
        async move {
            // stdout and stderr are pumped concurrently, each through its own
            // parser; the shared term mutex serializes grid mutation.
            type BoxedReader = Box<dyn futures::io::AsyncRead + Unpin + Send>;
            let pump = |reader: Option<BoxedReader>| {
                let term = term.clone();
                let events_tx = events_tx.clone();
                async move {
                    let Some(mut reader) = reader else { return };
                    let mut processor = Processor::<StdSyncHandler>::new();
                    let mut buffer = [0u8; 8192];
                    let mut previous_byte_was_cr = false;
                    loop {
                        match reader.read(&mut buffer).await {
                            Ok(0) => return,
                            Err(error) => {
                                log::warn!("failed to read subprocess output: {error}");
                                return;
                            }
                            Ok(count) => {
                                let converted =
                                    convert_lf_to_crlf(&buffer[..count], &mut previous_byte_was_cr);
                                {
                                    let mut term = term.lock();
                                    processor.advance(&mut *term, &converted);
                                }
                                events_tx
                                    .unbounded_send(PtyEvent::Event(TerminalBackendEvent::Wakeup))
                                    .ok();
                            }
                        }
                    }
                }
            };
            let stdout = stdout.map(|reader| Box::new(reader) as BoxedReader);
            let stderr = stderr.map(|reader| Box::new(reader) as BoxedReader);
            futures::future::join(pump(stdout), pump(stderr)).await;

            // Both pipes are closed, so the child has exited or is about to.
            // Poll for its status without holding the lock across an await.
            let status = loop {
                let status = match child.lock().as_mut() {
                    Some(child) => match child.try_status() {
                        Ok(status) => status,
                        Err(error) => {
                            log::warn!("failed to get subprocess exit status: {error}");
                            break None;
                        }
                    },
                    None => Some(ExitStatus::default()),
                };
                match status {
                    Some(status) => break Some(status),
                    None => executor.timer(Duration::from_millis(20)).await,
                }
            };
            child.lock().take();
            let event = match status {
                Some(status) => TerminalBackendEvent::ChildExit(status),
                None => TerminalBackendEvent::Exit,
            };
            events_tx.unbounded_send(PtyEvent::Event(event)).ok();
        }
    });

    Ok(SubprocessHandle {
        child,
        _reader: reader,
    })
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(subprocess) = self.subprocess.take() {
            subprocess.kill();
        }
        if let TerminalType::Pty { pty_tx, info } =
            std::mem::replace(&mut self.terminal_type, TerminalType::DisplayOnly)
        {
            let kill_processes =
                terminate_processes_with_grace_period(info, self.background_executor.clone());
            pty_tx.shutdown();
            self.background_executor.spawn(kill_processes).detach();
        }
    }
}

impl EventEmitter<Event> for Terminal {}

fn normalize_path_command_name(command: &str) -> Option<String> {
    const MAX_COMMAND_NAME_LENGTH: usize = 64;

    let command = command.trim();
    if command.is_empty()
        || command.len() > MAX_COMMAND_NAME_LENGTH
        || command.starts_with('.')
        || command.starts_with('-')
        || command.contains('/')
        || command.contains('\\')
    {
        return None;
    }

    let mut command = command.to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1"] {
        if command.ends_with(suffix) {
            command.truncate(command.len() - suffix.len());
            break;
        }
    }

    if command.is_empty()
        || !command.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return None;
    }

    Some(command)
}

fn foreground_process_command_from_argv(argv: &[String]) -> Option<String> {
    let command = argv
        .first()
        .and_then(|command| normalize_path_command_name(command));

    if !matches!(
        command.as_deref(),
        Some("node" | "python" | "python3" | "bun" | "deno")
    ) {
        return command;
    }

    argv.iter()
        .skip(1)
        .filter_map(|argument| normalize_script_command_name(argument))
        .next()
        .or(command)
}

/// Pure busy predicate for close-confirm (M12).
///
/// Returns true when the foreground process group id differs from the shell
/// child pid (a non-shell job is running). Idle shell → foreground equals
/// shell → not busy.
pub fn terminal_looks_busy(foreground_pid: Option<u32>, shell_pid: u32) -> bool {
    match foreground_pid {
        Some(fg) if fg > 0 && shell_pid > 0 && fg != shell_pid => true,
        _ => false,
    }
}

fn normalize_script_command_name(argument: &str) -> Option<String> {
    let path = Path::new(argument);
    let file_stem = path
        .file_stem()
        .and_then(|file_stem| file_stem.to_str())
        .and_then(normalize_path_command_name)?;

    if file_stem != "index" {
        return Some(file_stem);
    }

    path.parent()
        .and_then(|parent| parent.parent())
        .and_then(|package_path| package_path.file_name())
        .and_then(|package_name| package_name.to_str())
        .and_then(|package_name| package_name.strip_suffix("-cli").or(Some(package_name)))
        .and_then(normalize_path_command_name)
}

fn content_index_for_mouse(pos: GpuiPoint<Pixels>, terminal_bounds: &TerminalBounds) -> usize {
    let col = (pos.x / terminal_bounds.cell_width()).round() as usize;
    let clamped_col = min(col, terminal_bounds.num_columns().saturating_sub(1));
    let row = (pos.y / terminal_bounds.line_height()).round() as usize;
    let clamped_row = min(row, terminal_bounds.num_lines().saturating_sub(1));
    clamped_row * terminal_bounds.num_columns() + clamped_col
}

/// Converts an 8 bit ANSI color to its GPUI equivalent.
pub fn get_color_at_index(index: usize, palette: &TerminalPalette) -> Hsla {
    palette_get_color(index, palette)
}

pub fn rgba_color(r: u8, g: u8, b: u8) -> Hsla {
    use gpui::Rgba;
    Rgba {
        r: (r as f32 / 255.),
        g: (g as f32 / 255.),
        b: (b as f32 / 255.),
        a: 1.,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::terminal_looks_busy;

    // M1: full integration tests disabled (Zed settings stack removed)

    #[test]
    fn idle_shell_is_not_busy() {
        // Foreground process group equals the shell child.
        assert!(!terminal_looks_busy(Some(42), 42));
        assert!(!terminal_looks_busy(None, 42));
        assert!(!terminal_looks_busy(Some(0), 42));
        assert!(!terminal_looks_busy(Some(42), 0));
    }

    #[test]
    fn foreground_job_is_busy() {
        // e.g. shell pid 100, `sleep` in pgid 200.
        assert!(terminal_looks_busy(Some(200), 100));
        assert!(terminal_looks_busy(Some(1), 2));
    }
}
