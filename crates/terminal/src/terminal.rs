mod mappings;
mod row_map;

mod alacritty;
mod osc133;
mod osc_notify;
mod pty_info;
mod run_tracker;
mod shell_semantics;
pub mod terminal_settings;

pub use osc_notify::{OscNotify, OscNotifyScanner, scan_osc_notify};
pub use osc133::{
    GutterKind, GutterMark, Osc133Kind, Osc133Marker, Osc133Scanner, absolute_to_display_line,
    gutter_marks_from_markers, rebase_markers_after_history_shrink,
};
pub(crate) use row_map::PointerMap;
pub use row_map::{hit_display, viewport_top_abs, y_for_display};
pub use run_tracker::{RunTracker, TrackerOut, UNRECOGNIZED_COMMAND, normalize_command};
pub use shell_semantics::{
    ClickToMove, InjectShell, TripleClickKind, absolute_to_grid_line, apply_inject_to_shell,
    clear_input_line_sequence, click_to_move_sequence, command_input_selection_range,
    command_output_range, inject_script, triple_click_kind, wrap_shell_for_inject,
    wrap_shell_for_inject_in,
};

#[cfg(not(windows))]
use anyhow::Context as _;
use anyhow::{Result, bail};
use futures_lite::future::yield_now;
use log::trace;

use futures::{
    FutureExt,
    channel::mpsc::{UnboundedReceiver, unbounded},
};

use alacritty_terminal::grid::Dimensions as _;
use itertools::Itertools as _;
use mappings::mouse::{alt_scroll, mouse_button_report, mouse_moved_report, scroll_report};
use row_geometry::{RowGeometry, ViewportPosition};

use collections::{HashMap, VecDeque};
use futures::StreamExt;
use pty_info::{ProcessIdGetter, PtyProcessInfo};
use serde::{Deserialize, Serialize};
use sleipnir_settings::{TerminalPalette, get_color_at_index as palette_get_color};
use terminal_settings::{AlternateScroll, CursorShape as SettingsCursorShape, TerminalSettings};
use urlencoding;
use util::shell::Shell;
use util::{paths::PathStyle, truncate_and_trailoff};

use std::{
    borrow::Cow,
    cmp,
    fmt::{self, Display, Formatter},
    future::Future,
    ops::{BitOr, BitOrAssign, Deref},
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
pub use vte::ansi::{Color, NamedColor, Rgb};

use gpui::{
    App, AppContext as _, BackgroundExecutor, Bounds, ClipboardItem, Context, EventEmitter,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point as GpuiPoint, ScrollWheelEvent, Size, Task, TouchPhase, Window, actions, px,
};

#[cfg(not(windows))]
use crate::alacritty::current_child_signal_mask;
use crate::alacritty::{
    AlacrittyCell, AlacrittyGridIterator, AlacrittyHyperlink, AlacrittySearch, AlacrittyTerm,
    AlacrittyTermConfig, AlacrittyTermLock, HyperlinkMatch, PtySender, RegexSearches,
    clear_saved_screen, content_text, display_offset, find_from_terminal_point, grid_text_range,
    make_content, new_term, open_pty, pty_options, pty_term_config, resize, screen_lines,
    scroll_display, scroll_to_point, search_matches, selection_text,
    set_selection as set_term_selection, spawn_event_loop, toggle_vi_mode as toggle_term_vi_mode,
    total_lines, update_selection as update_term_selection, update_selection_to_vi_cursor,
    update_vi_cursor_for_scroll, vi_goto_point, vi_motion,
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
        line: Option<i32>,
        column: Option<usize>,
    },
    /// The current command finished. `exit_code` is `None` when unknown.
    RunFinished {
        exit_code: Option<i32>,
    },
    /// Overlay triangle on a command start/end line was clicked.
    GutterClicked {
        line: i32,
    },
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
#[allow(dead_code)]
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

pub struct TerminalBuilder {
    terminal: Terminal,
    events_rx: UnboundedReceiver<PtyEvent>,
}

impl TerminalBuilder {
    pub fn new(
        working_directory: Option<PathBuf>,
        shell: Shell,
        mut env: HashMap<String, String>,
        cursor_shape: SettingsCursorShape,
        alternate_scroll: AlternateScroll,
        max_scroll_history_lines: Option<usize>,
        path_hyperlink_regexes: Vec<String>,
        path_hyperlink_timeout_ms: u64,
        window_id: u64,
        cx: &App,
        path_style: PathStyle,
    ) -> Task<Result<TerminalBuilder>> {
        let version = release_channel::AppVersion::global(cx);
        let background_executor = cx.background_executor().clone();
        #[cfg(not(windows))]
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

            let shell_params = match shell {
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

            let scrolling_history = max_scroll_history_lines
                .unwrap_or(DEFAULT_SCROLL_HISTORY_LINES)
                .min(MAX_SCROLL_HISTORY_LINES);
            let config = pty_term_config(scrolling_history, cursor_shape);

            let (events_tx, events_rx) = unbounded();
            let term = new_term(
                &config,
                TerminalBounds::default(),
                events_tx.clone(),
                alternate_scroll,
            );

            let alacritty_shell = shell_params.as_ref().map(|params| {
                (
                    params.program.clone(),
                    params.args.clone().unwrap_or_default(),
                )
            });
            let pty_options = pty_options(
                alacritty_shell,
                working_directory.clone(),
                env,
                // We pass in the foreground thread's signal mask to the child process via pty_options,
                // so terminal construction can run on a background thread without breaking Ctrl-C and other signals
                // otherwise the terminal would inherit the background executor's signal mask which blocks
                // some terminal signals
                #[cfg(not(windows))]
                child_signal_mask,
                #[cfg(windows)]
                false,
            );

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
            let pty_tx = spawn_event_loop(term.clone(), events_tx, pty, pty_options.drain_on_exit)?;

            let terminal = Terminal {
                terminal_type: TerminalType::Pty {
                    pty_tx,
                    info: Arc::new(pty_info),
                },
                term,
                term_config: config,
                title_override: terminal_title_override,
                events: VecDeque::with_capacity(10),
                last_content: Default::default(),
                last_mouse: None,
                mouse_down_position: None,
                matches: Vec::new(),
                selection_head: None,
                frozen_selection: None,
                scroll_px: px(0.),
                viewport: ViewportPosition::new(0),
                row_geometry: RowGeometry::new(16.0),
                next_link_id: 0,
                selection_phase: SelectionPhase::Ended,
                hyperlink_regex_searches: RegexSearches::new(
                    &path_hyperlink_regexes,
                    path_hyperlink_timeout_ms,
                ),
                vi_mode_enabled: false,
                last_mouse_move_time: Instant::now(),
                last_hyperlink_search_position: None,
                mouse_down_hyperlink: None,
                child_exited: None,
                keyboard_input_sent: false,
                osc133: Osc133Scanner::new(),
                last_history_size: 0,
                pending_history_shrink: 0,
                osc_notify: OscNotifyScanner::new(),
                prompt_markers: Vec::new(),
                last_busy: false,
                busy_since: None,
                run_tracker: RunTracker::default(),
                started_at: Instant::now(),
                event_loop_task: Task::ready(Ok(())),
                background_executor,
                path_style,
                cwd_history: working_directory
                    .as_ref()
                    .map(|working_directory| {
                        vec![CwdHistoryEntry {
                            scrollback_position: i32::MIN,
                            working_directory: working_directory.clone(),
                        }]
                    })
                    .unwrap_or_default(),
                pending_cwd_boundary: None,
                #[cfg(any(test, feature = "test-support"))]
                input_log: Vec::new(),
                #[cfg(any(test, feature = "test-support"))]
                pty_write_log: Default::default(),
            };

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
                TerminalType::Closed => None,
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
    Closed,
}

pub struct Terminal {
    terminal_type: TerminalType,
    term: Arc<AlacrittyTermLock>,
    term_config: AlacrittyTermConfig,
    events: VecDeque<InternalEvent>,
    /// This is only used for mouse mode cell change detection
    last_mouse: Option<(Point, SelectionSide)>,
    /// Window-relative position of the most recent left mouse-down. Used to
    /// apply a drag threshold before starting a selection (see #58970).
    mouse_down_position: Option<GpuiPoint<Pixels>>,
    pub matches: Vec<Range>,
    pub last_content: Content,
    pub selection_head: Option<Point>,
    /// Frozen "select all" range that survives alacritty's internal selection
    /// clearing (shells frequently erase lines which drops the selection).
    /// Cleared on next user input or explicit click.
    frozen_selection: Option<SelectionRange>,
    title_override: Option<String>,
    scroll_px: Pixels,
    /// Host-side sub-row remainder (ADR-0018 decision 2). Never sent to the grid.
    viewport: ViewportPosition,
    /// Block heights and the mapping both paint and hit-testing use.
    row_geometry: RowGeometry,
    next_link_id: usize,
    selection_phase: SelectionPhase,
    hyperlink_regex_searches: RegexSearches,
    vi_mode_enabled: bool,
    last_mouse_move_time: Instant,
    last_hyperlink_search_position: Option<GpuiPoint<Pixels>>,
    mouse_down_hyperlink: Option<HyperlinkMatch>,
    child_exited: Option<ExitStatus>,
    keyboard_input_sent: bool,
    /// OSC 133 scanner (M14 shell integration detect).
    osc133: Osc133Scanner,
    /// Last observed scrollback size; a shrink (e.g. `clear`'s `ED 3`) means
    /// gutter marker lines must be rebased.
    last_history_size: usize,
    /// Rows dropped from scrollback since the Block mount last looked.
    /// Accumulates across syncs so a shrink cannot be missed between paints.
    pending_history_shrink: i32,
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
            TerminalBackendEvent::Title(_) | TerminalBackendEvent::ResetTitle => {
                cx.emit(Event::TitleChanged);
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
                let (point, side) = self.pointer_map_locked(term).grid_point_and_side(*position);

                if update_term_selection(term, point, side) {
                    self.selection_head = Some(point);
                    cx.emit(Event::SelectionsChanged)
                }
            }

            InternalEvent::Copy(keep_selection) => {
                trace!("Copying selection: keep_selection={keep_selection:?}");
                let txt = selection_text(term).or_else(|| {
                    // Alacritty's selection was cleared; use the frozen range.
                    let frozen = self.frozen_selection?;
                    Some(grid_text_range(
                        term,
                        frozen.start.line,
                        frozen.start.column,
                        frozen.end.line,
                        frozen.end.column,
                    ))
                });
                if let Some(txt) = txt {
                    if !txt.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(txt));
                        cx.emit(Event::CopiedToClipboard);
                    }
                    if !keep_selection.unwrap_or_else(|| {
                        let settings = TerminalSettings::get_global(cx);
                        settings.keep_selection_on_copy
                    }) {
                        self.frozen_selection = None;
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

                let point = self.pointer_map_locked(term).grid_point(*position);

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

    /// Mapping paint and hit-testing share. The grid's `display_offset` is
    /// the integer part; [`Self::viewport_sub`] is the host remainder.
    pub(crate) fn pointer_map(&self) -> PointerMap<'_> {
        let (display_offset, history_size) = {
            let term = self.term.lock_unfair();
            (display_offset(&term), term.history_size() as i32)
        };
        self.pointer_map_parts(display_offset, history_size)
    }

    /// [`Self::pointer_map`] for callers that **already hold** the terminal lock.
    ///
    /// `process_terminal_event` is handed `term: &mut AlacrittyTerm`, i.e. the
    /// lock is held for the whole call. Going through `pointer_map` there
    /// re-enters `FairMutex::lock_unfair` on the same thread and parks forever
    /// (observed as a 100% CPU hang). Any path under a held lock uses this.
    pub(crate) fn pointer_map_locked(&self, term: &AlacrittyTerm) -> PointerMap<'_> {
        self.pointer_map_parts(display_offset(term), term.history_size() as i32)
    }

    fn pointer_map_parts(&self, display_offset: usize, history_size: i32) -> PointerMap<'_> {
        PointerMap {
            size: self.last_content.terminal_bounds,
            display_offset,
            geometry: &self.row_geometry,
            history_size,
            sub: self.viewport.sub,
        }
    }

    /// Pixel y relative to the terminal origin → cell or Block (ADR-0018).
    pub fn hit_local(&self, pos: GpuiPoint<Pixels>) -> row_geometry::HitTarget {
        self.pointer_map().hit(pos)
    }

    pub fn row_geometry(&self) -> &RowGeometry {
        &self.row_geometry
    }

    pub fn row_geometry_mut(&mut self) -> &mut RowGeometry {
        &mut self.row_geometry
    }

    pub fn viewport_sub(&self) -> f32 {
        self.viewport.sub
    }

    /// Flush the remainder so a jump lands a Block against the viewport edge.
    pub fn set_viewport_sub(&mut self, sub: f32) {
        self.viewport.sub = if sub.is_finite() && sub >= 0.0 {
            sub
        } else {
            0.0
        };
    }

    pub fn set_blocks_frozen(&mut self, frozen: bool) {
        self.row_geometry.set_frozen(frozen);
    }

    /// Rows dropped from scrollback since the last call, then reset to zero.
    ///
    /// History belongs to the terminal, so the shrink is detected here once
    /// (`sync`) rather than re-derived by every mount that stores absolute
    /// lines. The Block mount consumes this to rebase its own surfaces, which
    /// are then pushed back over `row_geometry` — so both sides shift by the
    /// same amount and geometry never keeps a stale absolute line.
    pub fn take_history_shrink(&mut self) -> i32 {
        std::mem::take(&mut self.pending_history_shrink)
    }

    /// Replace the Block set from the mount point. Heights are integer rows
    /// from `sleipnir_widget::layout`. While frozen, upsert keeps pinned heights.
    pub fn upsert_block(&mut self, block: row_geometry::Block) {
        self.row_geometry.upsert(block);
    }

    pub fn remove_block(&mut self, id: row_geometry::BlockId) {
        self.row_geometry.remove(id);
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
                let (line, column) = if inferred {
                    (None, None)
                } else {
                    self.prompt_markers
                        .iter()
                        .rev()
                        .find(|m| matches!(m.kind, Osc133Kind::CommandExecuted))
                        .map(|m| (m.line, m.column))
                        .unwrap_or((None, None))
                };
                cx.emit(Event::RunStarted {
                    command,
                    cwd: self.working_directory(),
                    inferred,
                    line,
                    column,
                });
            }
            TrackerOut::Finished { exit_code, .. } => {
                cx.emit(Event::RunFinished { exit_code });
            }
        }
    }

    /// Scroll so `absolute` line (OSC 133 marker coords) is near the top.
    /// Sets `sub` to 0 so a Block at that anchor lands flush (ADR-0018).
    pub fn scroll_to_absolute(&mut self, absolute: i32, column: usize) {
        let history = self.term.lock_unfair().history_size() as i32;
        let grid_line = absolute_to_grid_line(absolute, history);
        self.viewport.jump_to_anchor(absolute);
        self.events
            .push_back(InternalEvent::ScrollToPoint(Point::new(grid_line, column)));
    }

    pub fn history_size(&self) -> usize {
        self.term.lock_unfair().history_size()
    }

    pub fn is_alt_screen(&self) -> bool {
        self.last_content.mode.contains(Modes::ALT_SCREEN)
    }

    /// Command start/end triangles are disabled: they only mark command
    /// boundaries and are not needed by hunk jumps or the Run Ledger, so
    /// returning nothing keeps the gutter clear. The marker log is still kept
    /// up to date for prompt jump / click-to-move / command extraction.
    pub fn gutter_overlay(&self) -> Vec<GutterMark> {
        Vec::new()
    }

    pub fn emit_gutter_click(&mut self, line: i32, cx: &mut Context<Self>) {
        cx.emit(Event::GutterClicked { line });
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
        let text = grid_text_range(&term, start_line, start_col, cursor.line.0, cursor.column.0);
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
        // Flush so a Block at the prompt lands against the viewport edge.
        self.viewport.jump_to_anchor(target_line);
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

    pub fn select_all(&mut self) {
        // This action is editor-style select-all for the active command input.
        // It must never delegate to alacritty's full-buffer select-all fallback.
        let Some(range) = self.command_input_range() else {
            self.frozen_selection = None;
            self.set_selection(None);
            return;
        };
        // Freeze so it survives alacritty's internal line-erase/scroll clearing.
        let sel_range = SelectionRange {
            start: range.start(),
            end: range.end(),
            is_block: false,
        };
        self.frozen_selection = Some(sel_range);
        self.set_selection(Some(Selection::simple_range(range)));
    }

    /// Grid range of the active command input. OSC 133 supplies the precise
    /// prompt boundary; without it, selection stays on the cursor's occupied row.
    fn command_input_range(&self) -> Option<Range> {
        let term = self.term.lock_unfair();
        let history = term.history_size() as i32;
        let cursor = Point::new(
            term.grid().cursor.point.line.0,
            term.grid().cursor.point.column.0,
        );
        let occupied_columns = self
            .last_content
            .cells
            .iter()
            .filter(|cell| cell.point.line == cursor.line && cell.character() != ' ')
            .map(|cell| cell.point.column + 1)
            .max()
            .unwrap_or(0);
        let command_start = self
            .prompt_markers
            .iter()
            .rev()
            .find(|marker| matches!(marker.kind, Osc133Kind::CommandStart))
            .and_then(|marker| {
                Some(Point::new(
                    absolute_to_grid_line(marker.line?, history),
                    marker.column.unwrap_or(0),
                ))
            });
        drop(term);

        command_input_selection_range(command_start, cursor, occupied_columns)
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
        self.write_input(input);
    }

    fn write_input(&mut self, input: impl Into<Cow<'static, [u8]>>) {
        let input = input.into();
        // Any user input clears the frozen select-all range.
        self.frozen_selection = None;
        if input.contains(&b'\r') {
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

        // When the command input is fully selected (select-all) and the user
        // presses Backspace/Delete, clear the whole input line at once
        // (readline ctrl-u), matching editor-style "delete selection".
        if self.frozen_selection.is_some() {
            let key = keystroke.key.as_str();
            let plain = !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform
                && !keystroke.modifiers.function;
            if plain && matches!(key, "backspace" | "delete") {
                self.frozen_selection = None;
                self.input(clear_input_line_sequence());
                return true;
            }
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

        // A shrinking scrollback (e.g. `clear` sends `ED 3`, or an app clear
        // dropped saved lines) invalidates the absolute lines stored in the
        // gutter markers; rebase them so triangles don't strand on wrong rows.
        let history_size = terminal.history_size();
        let removed = crate::row_map::history_shrink(self.last_history_size, history_size);
        if removed > 0 {
            rebase_markers_after_history_shrink(&mut self.prompt_markers, removed);
            self.row_geometry.rebase_after_history_shrink(removed);
            // Published for the Block mount, which owns the surfaces this
            // geometry is rebuilt from and must rebase them by the same
            // amount. Consumed (and cleared) by `take_history_shrink`.
            self.pending_history_shrink = self.pending_history_shrink.saturating_add(removed);
        }
        self.last_history_size = history_size;
        let screen_lines = terminal.screen_lines();
        self.last_content = make_content(&terminal, &self.last_content);
        self.row_geometry
            .set_line_height(f32::from(self.last_content.terminal_bounds.line_height));
        self.row_geometry.set_line_count(
            i32::try_from(history_size.saturating_add(screen_lines)).unwrap_or(i32::MAX),
        );
        let alt = self.last_content.mode.contains(Modes::ALT_SCREEN);
        self.row_geometry.set_alt_screen(alt);
        if alt {
            self.viewport.sub = 0.0;
        }
        drop(terminal);

        // A frozen "select all" must survive alacritty's internal selection
        // mutation (shells constantly erase/scroll the prompt line, which drops
        // or rotates the native selection). While frozen, own the selection
        // outright regardless of alacritty's current state.
        if let Some(frozen) = self.frozen_selection {
            self.last_content.selection = Some(frozen);
            let term = self.term.lock_unfair();
            self.last_content.selection_text = Some(grid_text_range(
                &term,
                frozen.start.line,
                frozen.start.column,
                frozen.end.line,
                frozen.end.column,
            ));
            drop(term);
        }
    }

    pub fn with_renderable_cells<R>(&self, f: impl for<'a> FnOnce(RenderableCells<'a>) -> R) -> R {
        let term = self.term.lock_unfair();
        let content = term.renderable_content();
        f(RenderableCells::new(content.display_iter))
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
                let (point, side) = self.pointer_map().grid_point_and_side(position);

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

    pub fn mouse_drag(
        &mut self,
        e: &MouseMoveEvent,
        region: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        if !self.mouse_mode(e.modifiers.shift) {
            if let Some(hyperlink) = &self.mouse_down_hyperlink {
                let point = self.pointer_map().grid_point(position);

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
        // Any mouse interaction clears the frozen select-all.
        self.frozen_selection = None;
        let position = e.position - self.last_content.terminal_bounds.bounds.origin;
        let point = self.pointer_map().grid_point(position);

        if e.button == MouseButton::Left && !e.modifiers.secondary() {
            // Alt+Click always attempts cursor move (original behavior).
            // Plain single click on the prompt line also moves the cursor
            // instead of starting a selection, giving a text-editor-like feel.
            let try_move = e.modifiers.alt || (!e.modifiers.shift && e.click_count == 1);
            if try_move {
                if let Some(bytes) = self.click_to_move_bytes(point) {
                    if !bytes.is_empty() {
                        self.write_to_pty(bytes);
                    }
                    return;
                }
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
                    let (point, side) = self.pointer_map().grid_point_and_side(position);

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
            let point = self.pointer_map().grid_point(position);

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
            let point = self.pointer_map().grid_point(position);

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
                let mouse_cell_index = self.pointer_map().content_index(position);
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
                let point = self
                    .pointer_map()
                    .grid_point(e.position - self.last_content.terminal_bounds.bounds.origin);

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
            TouchPhase::Started => {
                self.scroll_px = px(0.);
                self.viewport.sub = 0.0;
                None
            }
            TouchPhase::Moved => {
                let delta = e.delta.pixel_delta(line_height).y * scroll_multiplier;

                // Preserve the v0.4.1 path for an ordinary terminal. Applying
                // the variable-height viewport to a uniform grid needlessly
                // translates every glyph by `viewport.sub`; at the bottom the
                // grid clamps Scroll::Delta while that remainder survives, so
                // trackpad momentum repeatedly paints the screen at different
                // sub-pixel positions and the text flickers.
                if self.row_geometry.blocks().next().is_none() {
                    self.viewport.sub = 0.0;
                    return Some(accumulate_uniform_wheel(
                        &mut self.scroll_px,
                        delta,
                        line_height,
                        self.last_content.terminal_bounds.height(),
                    ));
                }

                // Blocks have variable pixel heights, so retain their sub-row
                // remainder (ADR-0018 decision 2).
                self.viewport.row = usize::try_from(viewport_top_abs(
                    self.last_history_size as i32,
                    self.last_content.display_offset,
                ))
                .unwrap_or(0);
                // RowGeometry's line axis points down-document (later lines =
                // larger y), while a positive wheel delta means "scroll up"
                // (toward history). Negate on the way in and back out.
                let absolute_line_delta = self
                    .viewport
                    .apply_pixel_delta(-f32::from(delta), &self.row_geometry);
                Some(-absolute_line_delta)
            }
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
        self.client_side_working_directory()
    }

    /// Normalizes the command name of the foreground process, if one is known.
    pub fn foreground_process_command_name(&self) -> Option<String> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => info
                .current
                .read()
                .as_ref()
                .and_then(|process| foreground_process_command_from_argv(&process.argv)),
            TerminalType::Closed => None,
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
            TerminalType::Closed => None,
        }
    }

    pub(crate) fn record_cwd_change(&mut self, new_working_directory: PathBuf) {
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
        if self.cwd_history.is_empty() || history_size >= self.term_config.scrolling_history {
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
        self.title_override
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
                TerminalType::Closed => "Terminal".to_string(),
            })
    }

    /// The spawned shell child (tree root), not the foreground job group.
    pub fn shell_pid(&self) -> Option<u32> {
        match &self.terminal_type {
            TerminalType::Pty { info, .. } => {
                let pid = info.pid_getter().fallback_pid().as_u32();
                (pid > 0).then_some(pid)
            }
            TerminalType::Closed => None,
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
            TerminalType::Closed => false,
        }
    }

    fn register_task_finished(
        &mut self,
        exit_status: Option<ExitStatus>,
        cx: &mut Context<Terminal>,
    ) {
        if let Some(e) = exit_status {
            self.child_exited = Some(e);
        }
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
    }

    pub fn vi_mode_enabled(&self) -> bool {
        self.vi_mode_enabled
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let TerminalType::Pty { pty_tx, info } =
            std::mem::replace(&mut self.terminal_type, TerminalType::Closed)
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
/// Accumulate wheel pixels using the stable v0.4.1 uniform-grid behavior.
///
/// This intentionally keeps the remainder out of `ViewportPosition::sub`:
/// ordinary terminal rows are integer grid rows and must always paint flush.
fn accumulate_uniform_wheel(
    scroll_px: &mut Pixels,
    delta: Pixels,
    line_height: Pixels,
    viewport_height: Pixels,
) -> i32 {
    let old_offset = (*scroll_px / line_height) as i32;
    *scroll_px += delta;
    let new_offset = (*scroll_px / line_height) as i32;

    if viewport_height > px(0.) {
        *scroll_px %= viewport_height;
    } else {
        *scroll_px = px(0.);
    }

    new_offset - old_offset
}

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

#[cfg(test)]
mod tests {
    use super::{accumulate_uniform_wheel, terminal_looks_busy};
    use gpui::px;

    /// Regression (ADR-0018 integration): `sync` holds the terminal lock across
    /// `process_terminal_event`, which is handed `term: &mut AlacrittyTerm`.
    /// Routing coordinates through `pointer_map` there re-entered
    /// `FairMutex::lock_unfair` on the same thread and parked forever — a 100%
    /// CPU hang reproducible by opening Settings. Paths under the held lock must
    /// use `pointer_map_locked`, which takes the borrow instead of re-locking.
    ///
    /// A runtime test would have to deadlock to fail, so this inspects the
    /// source: inside `process_terminal_event` there must be no `pointer_map()`
    /// call and no `self.term.lock_unfair()`.
    #[test]
    fn process_terminal_event_never_relocks_the_terminal() {
        let src = include_str!("terminal.rs");
        let start = src
            .find("fn process_terminal_event(")
            .expect("process_terminal_event exists");
        // The next `fn` at the same indentation ends the body.
        let body_start = start + "fn process_terminal_event(".len();
        let end = src[body_start..]
            .find("\n    pub(crate) fn pointer_map(")
            .map(|off| body_start + off)
            .expect("pointer_map follows process_terminal_event");
        let body = &src[start..end];
        assert!(
            !body.contains("self.pointer_map()"),
            "process_terminal_event must use pointer_map_locked; \
             pointer_map() re-locks and self-deadlocks"
        );
        assert!(
            !body.contains("self.term.lock_unfair()"),
            "the lock is already held for this call"
        );
        assert!(
            body.contains("pointer_map_locked(term)"),
            "coordinates must still route through RowGeometry"
        );
    }

    /// Regression: v0.4.1 accumulated fractional uniform-grid wheel movement
    /// without feeding it into the paint transform. Opposite fractional
    /// gestures must cancel while dispatching the same whole-row movement.
    #[test]
    fn uniform_wheel_preserves_v0_4_1_accumulation() {
        let line_height = px(16.0);
        let viewport_height = px(640.0);
        let mut scroll_px = px(0.0);

        let up = accumulate_uniform_wheel(&mut scroll_px, px(21.0), line_height, viewport_height);
        assert_eq!(up, 1);
        let down =
            accumulate_uniform_wheel(&mut scroll_px, px(-21.0), line_height, viewport_height);
        assert_eq!(down, -1);
        assert_eq!(scroll_px, px(0.0));
    }

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
