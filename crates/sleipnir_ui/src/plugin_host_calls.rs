//! Plugin-initiated host calls (ADR-0016 §3–§4).
//!
//! A `Call` is a plugin asking the host to do something the control surface
//! already exposes locally (ADR-0011): notify, read a pane, list panes, open a
//! pane, focus a pane, type into a pane, send an allowlisted key, request
//! close of a pane. v2 adds no new power — it changes *who* may ask, which
//! is why each verb is a separately granted capability and is **never implied**
//! by a snapshot-read or snapshot-write permission. [`Capability::WriteTerminal`]
//! covers user-invoked `Output::Insert` into the active pane; it does not grant
//! [`HostCall::FocusPane`], [`HostCall::SendText`], [`HostCall::SendKey`], or
//! [`HostCall::RequestClosePane`].
//!
//! Every `Call` id must produce exactly one `Reply`. A silent drop (denied,
//! missing pane, rate limit, dead plugin) would leave a resident plugin
//! waiting forever. Denial is [`HostCallResult::Error`], not absence.
//!
//! Pure decision logic. No gpui, no window, no process spawn. The shell
//! executes the plan and always calls `reply`.

use plugin_protocol::v2::{Capability, HostCall, HostCallResult, PaneInfo, RunId};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::pane_tree::PaneKey;

/// Notify title cap. A plugin-controlled string is interpolated into a
/// platform notification; unbounded titles are a spam vector.
pub const MAX_NOTIFY_TITLE: usize = 80;
/// Notify body cap.
pub const MAX_NOTIFY_BODY: usize = 500;
/// Visible-screen cap. Scrollback is not included (mirrors `capture`).
pub const MAX_SCREEN_CHARS: usize = 64 * 1024;
/// OpenPane command string cap, before argv split.
pub const MAX_OPEN_COMMAND_CHARS: usize = 1024;
/// OpenPaneArgv program length cap.
pub const MAX_OPEN_PROGRAM_CHARS: usize = 256;
/// OpenPaneArgv argv entry count cap (not counting `program`).
pub const MAX_OPEN_ARGS: usize = 16;
/// OpenPaneArgv per-arg length cap.
pub const MAX_OPEN_ARG_CHARS: usize = 256;
/// Sliding window for per-plugin host-call rate limiting.
pub const RATE_WINDOW_MS: u64 = 5_000;
/// Max accepted calls per plugin in [`RATE_WINDOW_MS`].
pub const RATE_MAX_CALLS: u32 = 10;
pub use plugin_protocol::v2::MAX_SEND_TEXT_CHARS;

/// Allowlisted logical keys for [`HostCall::SendKey`]. Names, not bytes;
/// sufficient for interruption and prompt navigation. Unknown names are
/// rejected rather than forwarded as encoded input.
pub const SEND_KEY_NAMES: &[&str] = &[
    "ctrl-c", "escape", "enter", "tab", "up", "down", "left", "right",
];

/// Canonical logical key after parsing a [`HostCall::SendKey`] name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalKey {
    CtrlC,
    Escape,
    Enter,
    Tab,
    Up,
    Down,
    Left,
    Right,
}

impl LogicalKey {
    /// Parse a plugin-supplied key name. Case-insensitive; `ctrl+c` / `esc`
    /// / `return` / `arrow-*` are accepted aliases. Unknown names error.
    pub fn parse(name: &str) -> Result<Self, String> {
        let normalized = name.trim().to_ascii_lowercase().replace('_', "-");
        let canonical = match normalized.as_str() {
            "ctrl-c" | "ctrl+c" => Self::CtrlC,
            "escape" | "esc" => Self::Escape,
            "enter" | "return" => Self::Enter,
            "tab" => Self::Tab,
            "up" | "arrow-up" => Self::Up,
            "down" | "arrow-down" => Self::Down,
            "left" | "arrow-left" => Self::Left,
            "right" | "arrow-right" => Self::Right,
            "" => return Err("key is empty".into()),
            other => {
                return Err(format!(
                    "unknown key {other:?}; allowed: {}",
                    SEND_KEY_NAMES.join(", ")
                ));
            }
        };
        Ok(canonical)
    }

    /// gpui `Keystroke::parse` token for the terminal mapping table.
    pub fn keystroke_str(self) -> &'static str {
        match self {
            Self::CtrlC => "ctrl-c",
            Self::Escape => "escape",
            Self::Enter => "enter",
            Self::Tab => "tab",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// Program + argv for OpenPane. Never a shell command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// The workspace side effects a planned [`CallPlan`] executes against. One
/// executor for both plugin host calls and `sleipnir-ctl`: the shell provides
/// an implementation backed by its live panes, and the control surface backs
/// the same verbs with the live window(s). Keeping this trait gpui-free means
/// `CallPlan::execute` stays unit-testable against a fake.
///
/// [`CallPlan::Notify`] is deliberately absent: notification is a host side
/// effect the caller performs next to `execute`, not a workspace verb, so it is
/// never a second copy of List/Send here.
pub trait WorkspaceIo {
    /// Terminal (PTY) panes across every window. Panel leaves are excluded, so
    /// the result never needs a second identity filter.
    fn list_terminal_panes(&mut self) -> Vec<PaneInfo>;
    /// Visible screen text of a terminal pane. Uncapped; `execute` caps the
    /// plugin `ReadScreen` reply while ctl `Capture` keeps the full text.
    fn read_screen(&mut self, pane: PaneKey) -> Result<String, String>;
    /// Open a new terminal pane. Returns the protocol reply directly because
    /// the success payload carries the new [`PaneKey`].
    fn open_pane(&mut self, cwd: Option<String>, command: Option<OpenCommand>) -> HostCallResult;
    /// Scroll a pane to (and focus) the output of a run.
    fn scroll_to_run(&mut self, run_id: RunId) -> Result<(), String>;
    /// Focus a terminal pane.
    fn focus_pane(&mut self, pane: PaneKey) -> Result<(), String>;
    /// Type text into a terminal pane (paste-aware `insert_text`, never CSI).
    fn send_text(&mut self, pane: PaneKey, text: String, enter: bool) -> Result<(), String>;
    /// Send one allowlisted logical key to a terminal pane.
    fn send_key(&mut self, pane: PaneKey, key: LogicalKey) -> Result<(), String>;
    /// Request that a terminal pane close (busy panes still confirm).
    fn request_close_pane(&mut self, pane: PaneKey) -> Result<(), String>;
}

fn ok_or_error(result: Result<(), String>) -> HostCallResult {
    match result {
        Ok(()) => HostCallResult::Ok,
        Err(message) => error_result(message),
    }
}


/// What the UI should do for one `Call`. Always ends in a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallPlan {
    /// No side effect: send this result as the Reply.
    Reply(HostCallResult),
    Notify {
        title: String,
        body: String,
    },
    ReadScreen {
        pane: PaneKey,
    },
    ListPanes,
    OpenPane {
        cwd: Option<String>,
        command: Option<OpenCommand>,
    },
    ScrollToRun {
        run_id: RunId,
    },
    FocusPane {
        pane: PaneKey,
    },
    SendText {
        pane: PaneKey,
        text: String,
        enter: bool,
    },
    SendKey {
        pane: PaneKey,
        key: LogicalKey,
    },
    RequestClosePane {
        pane: PaneKey,
    },
}

impl CallPlan {
    /// Run a planned call's workspace side effects and produce its `Reply`.
    ///
    /// [`CallPlan::Reply`] short-circuits (no side effect). [`CallPlan::Notify`]
    /// is not workspace IO: the caller handles it and never routes it here, so
    /// hitting it is a host bug rather than a silent drop — it still returns a
    /// reply so the one-call-one-reply contract holds.
    pub fn execute(self, io: &mut impl WorkspaceIo) -> HostCallResult {
        match self {
            CallPlan::Reply(result) => result,
            // Notify is a host side effect performed next to execute, not a
            // workspace verb. Planners route it separately; reaching it here
            // means the caller forgot to.
            CallPlan::Notify { .. } => error_result("notify is not a workspace call"),
            CallPlan::ListPanes => HostCallResult::Panes {
                panes: io.list_terminal_panes(),
            },
            CallPlan::ReadScreen { pane } => match io.read_screen(pane) {
                Ok(text) => HostCallResult::Screen {
                    text: cap_screen(text),
                },
                Err(message) => error_result(message),
            },
            CallPlan::OpenPane { cwd, command } => io.open_pane(cwd, command),
            CallPlan::ScrollToRun { run_id } => ok_or_error(io.scroll_to_run(run_id)),
            CallPlan::FocusPane { pane } => ok_or_error(io.focus_pane(pane)),
            CallPlan::SendText { pane, text, enter } => {
                ok_or_error(io.send_text(pane, text, enter))
            }
            CallPlan::SendKey { pane, key } => ok_or_error(io.send_key(pane, key)),
            CallPlan::RequestClosePane { pane } => ok_or_error(io.request_close_pane(pane)),
        }
    }
}


/// Per-plugin sliding-window limiter. Drops are counted so the Monitor can
/// show a resident plugin that is hammering Notify / OpenPane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostCallLimiter {
    windows: BTreeMap<String, VecDeque<u64>>,
    dropped: BTreeMap<String, u64>,
}

impl HostCallLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// True if this call is inside the budget. A rejection still requires a
    /// Reply; the UI must not drop the id.
    pub fn allow(&mut self, plugin_id: &str, now_ms: u64) -> bool {
        let q = self.windows.entry(plugin_id.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= RATE_WINDOW_MS)
        {
            q.pop_front();
        }
        if q.len() as u32 >= RATE_MAX_CALLS {
            *self.dropped.entry(plugin_id.to_string()).or_insert(0) += 1;
            return false;
        }
        q.push_back(now_ms);
        true
    }

    pub fn dropped_counts(&self) -> &BTreeMap<String, u64> {
        &self.dropped
    }
}

/// Plan one inbound `Call`. Capability is checked first so a missing grant is
/// reported as itself rather than as a rate-limit. Rate limiting then bounds
/// a granted plugin that would otherwise spam Notify or OpenPane.
pub fn plan_call(
    plugin_id: &str,
    call: &HostCall,
    granted: &[Capability],
    limiter: &mut HostCallLimiter,
    now_ms: u64,
) -> CallPlan {
    let need = call.required_capability();
    if !granted.contains(&need) {
        return CallPlan::Reply(HostCallResult::Error {
            message: format!("capability {need:?} not granted"),
        });
    }
    if !limiter.allow(plugin_id, now_ms) {
        return CallPlan::Reply(HostCallResult::Error {
            message: "rate limited".into(),
        });
    }
    match call {
        HostCall::Notify { title, body } => {
            let title = cap_chars(title, MAX_NOTIFY_TITLE);
            let body = cap_chars(body, MAX_NOTIFY_BODY);
            CallPlan::Notify { title, body }
        }
        HostCall::ReadScreen { pane } => CallPlan::ReadScreen { pane: *pane },
        HostCall::ListPanes => CallPlan::ListPanes,
        HostCall::OpenPane { cwd, command } => match parse_open_pane(cwd, command) {
            Ok((cwd, command)) => CallPlan::OpenPane { cwd, command },
            Err(message) => CallPlan::Reply(HostCallResult::Error { message }),
        },
        HostCall::OpenPaneArgv { cwd, program, args } => {
            match parse_open_pane_argv(cwd, program, args) {
                Ok((cwd, command)) => CallPlan::OpenPane {
                    cwd,
                    command: Some(command),
                },
                Err(message) => CallPlan::Reply(HostCallResult::Error { message }),
            }
        }
        HostCall::ScrollToRun { run_id } => CallPlan::ScrollToRun { run_id: *run_id },
        HostCall::FocusPane { pane } => CallPlan::FocusPane { pane: *pane },
        HostCall::SendText { pane, text, enter } => match validate_send_text(text) {
            Ok(()) => CallPlan::SendText {
                pane: *pane,
                text: text.clone(),
                enter: *enter,
            },
            Err(message) => CallPlan::Reply(HostCallResult::Error { message }),
        },
        HostCall::SendKey { pane, key } => match LogicalKey::parse(key) {
            Ok(key) => CallPlan::SendKey { pane: *pane, key },
            Err(message) => CallPlan::Reply(HostCallResult::Error { message }),
        },
        HostCall::RequestClosePane { pane } => CallPlan::RequestClosePane { pane: *pane },
    }
}

fn validate_send_text(text: &str) -> Result<(), String> {
    if text.chars().count() > MAX_SEND_TEXT_CHARS {
        return Err(format!(
            "text exceeds length cap of {MAX_SEND_TEXT_CHARS} characters"
        ));
    }
    Ok(())
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn parse_open_cwd(cwd: &Option<String>) -> Result<Option<String>, String> {
    match cwd.as_deref() {
        None => Ok(None),
        Some(raw) if raw.trim().is_empty() => Err("cwd is empty".into()),
        Some(raw) if raw.contains('\0') => Err("cwd contains NUL".into()),
        Some(raw) => Ok(Some(raw.to_string())),
    }
}

fn parse_open_pane(
    cwd: &Option<String>,
    command: &Option<String>,
) -> Result<(Option<String>, Option<OpenCommand>), String> {
    let cwd = parse_open_cwd(cwd)?;
    let command = match command.as_deref() {
        None => None,
        Some(raw) => Some(parse_open_command(raw)?),
    };
    Ok((cwd, command))
}

fn parse_open_pane_argv(
    cwd: &Option<String>,
    program: &str,
    args: &[String],
) -> Result<(Option<String>, OpenCommand), String> {
    let cwd = parse_open_cwd(cwd)?;
    if program.trim().is_empty() {
        return Err("program is empty".into());
    }
    if program.contains('\0') {
        return Err("program contains NUL".into());
    }
    if program.chars().count() > MAX_OPEN_PROGRAM_CHARS {
        return Err("program exceeds length cap".into());
    }
    if args.len() > MAX_OPEN_ARGS {
        return Err(format!("too many args (max {MAX_OPEN_ARGS})"));
    }
    for arg in args {
        if arg.contains('\0') {
            return Err("arg contains NUL".into());
        }
        if arg.chars().count() > MAX_OPEN_ARG_CHARS {
            return Err("arg exceeds length cap".into());
        }
    }
    Ok((
        cwd,
        OpenCommand {
            program: program.to_string(),
            args: args.to_vec(),
        },
    ))
}

/// argv handed to `spawn_term_view`. Never rejoins into a shell line.
pub fn spawn_argv(command: OpenCommand) -> (String, Vec<String>) {
    (command.program, command.args)
}

/// Split `command` into program + argv on whitespace.
///
/// No quote processing and no `sh -c`. Quote processing *is* a shell; piping
/// the plugin string through one would let OpenPane run arbitrary scripts
/// (ADR-0013: arguments are passed directly).
pub fn parse_open_command(command: &str) -> Result<OpenCommand, String> {
    if command.chars().count() > MAX_OPEN_COMMAND_CHARS {
        return Err("command exceeds length cap".into());
    }
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return Err("command is empty".into());
    };
    Ok(OpenCommand {
        program: program.to_string(),
        args: parts.map(str::to_string).collect(),
    })
}

/// Truncate visible screen text at a char boundary.
pub fn cap_screen(text: String) -> String {
    cap_chars(&text, MAX_SCREEN_CHARS)
}

/// Classify a terminal-pane target. Used by ReadScreen, FocusPane, SendText,
/// SendKey, and RequestClosePane. The caller already required the matching
/// host-call capability; no snapshot-read or [`Capability::WriteTerminal`]
/// grant is consulted here.
pub fn read_screen_access(
    pane: PaneKey,
    terminal_keys: &BTreeSet<PaneKey>,
    panel_keys: &BTreeSet<PaneKey>,
) -> Result<(), String> {
    if panel_keys.contains(&pane) {
        return Err("pane is a plugin panel, not a terminal".into());
    }
    if !terminal_keys.contains(&pane) {
        return Err(format!("pane {pane} not found"));
    }
    Ok(())
}

pub fn error_result(message: impl Into<String>) -> HostCallResult {
    HostCallResult::Error {
        message: message.into(),
    }
}

/// Reply for [`HostCall::SendText`] after attempting insertion. A loading or
/// failed pane has no PTY; that is an Error, not a silent Ok.
pub const SEND_TEXT_NO_TERMINAL: &str = "pane has no terminal yet";
/// Reply for [`HostCall::SendKey`] when the pane is in terminal vi mode.
/// `try_keystroke` would consume arrows/escape as scrollback motion.
pub const SEND_KEY_VI_MODE: &str =
    "terminal vi mode is active; the key would be consumed as scrollback motion";

/// Whether a plugin [`HostCall::SendKey`] may be delivered to this pane.
///
/// Loading/failed panes have no PTY. Vi mode handles keys as scrollback
/// motion (`Terminal::try_keystroke` returns true without writing to the
/// PTY), so the plugin must not be told the interrupt/navigation key was
/// sent.
pub fn send_key_ready(has_terminal: bool, vi_mode: bool) -> Result<(), String> {
    if !has_terminal {
        return Err(SEND_TEXT_NO_TERMINAL.into());
    }
    if vi_mode {
        return Err(SEND_KEY_VI_MODE.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn key(n: u128) -> PaneKey {
        Uuid::from_u128(n)
    }

    /// In-memory `WorkspaceIo` so `CallPlan::execute` can be exercised without
    /// gpui. Only the terminal panes it is seeded with exist; everything else
    /// is a missing-pane error, mirroring the shell's `read_screen_access`.
    #[derive(Default)]
    struct FakeWorkspace {
        panes: Vec<PaneInfo>,
        screens: BTreeMap<PaneKey, String>,
        sent_text: Vec<(PaneKey, String, bool)>,
        sent_keys: Vec<(PaneKey, LogicalKey)>,
        focused: Vec<PaneKey>,
        closed: Vec<PaneKey>,
        opened: Option<(Option<String>, Option<OpenCommand>)>,
        open_reply: Option<HostCallResult>,
    }

    impl FakeWorkspace {
        fn with_terminal(pane: PaneKey, screen: &str) -> Self {
            let mut me = Self::default();
            me.panes.push(PaneInfo {
                pane,
                cwd: Some("/work".into()),
                title: Some("shell".into()),
                busy: false,
            });
            me.screens.insert(pane, screen.into());
            me
        }

        fn has(&self, pane: PaneKey) -> bool {
            self.panes.iter().any(|p| p.pane == pane)
        }
    }

    impl WorkspaceIo for FakeWorkspace {
        fn list_terminal_panes(&mut self) -> Vec<PaneInfo> {
            self.panes.clone()
        }
        fn read_screen(&mut self, pane: PaneKey) -> Result<String, String> {
            self.screens
                .get(&pane)
                .cloned()
                .ok_or_else(|| format!("pane {pane} not found"))
        }
        fn open_pane(
            &mut self,
            cwd: Option<String>,
            command: Option<OpenCommand>,
        ) -> HostCallResult {
            self.opened = Some((cwd, command));
            self.open_reply
                .clone()
                .unwrap_or(HostCallResult::Pane { pane: key(999) })
        }
        fn scroll_to_run(&mut self, _run_id: RunId) -> Result<(), String> {
            Ok(())
        }
        fn focus_pane(&mut self, pane: PaneKey) -> Result<(), String> {
            if !self.has(pane) {
                return Err(format!("pane {pane} not found"));
            }
            self.focused.push(pane);
            Ok(())
        }
        fn send_text(&mut self, pane: PaneKey, text: String, enter: bool) -> Result<(), String> {
            if !self.has(pane) {
                return Err(format!("pane {pane} not found"));
            }
            self.sent_text.push((pane, text, enter));
            Ok(())
        }
        fn send_key(&mut self, pane: PaneKey, key: LogicalKey) -> Result<(), String> {
            if !self.has(pane) {
                return Err(format!("pane {pane} not found"));
            }
            self.sent_keys.push((pane, key));
            Ok(())
        }
        fn request_close_pane(&mut self, pane: PaneKey) -> Result<(), String> {
            if !self.has(pane) {
                return Err(format!("pane {pane} not found"));
            }
            self.closed.push(pane);
            Ok(())
        }
    }

    fn notify(title: &str, body: &str) -> HostCall {
        HostCall::Notify {
            title: title.into(),
            body: body.into(),
        }
    }

    #[test]
    fn each_call_denied_without_its_capability_is_an_error_reply() {
        let mut limiter = HostCallLimiter::new();
        let calls = [
            notify("t", "b"),
            HostCall::ReadScreen { pane: key(1) },
            HostCall::ListPanes,
            HostCall::OpenPane {
                cwd: None,
                command: None,
            },
            HostCall::OpenPaneArgv {
                cwd: None,
                program: "codex".into(),
                args: vec![],
            },
            HostCall::ScrollToRun { run_id: key(7) },
            HostCall::FocusPane { pane: key(1) },
            HostCall::SendText {
                pane: key(1),
                text: "hi".into(),
                enter: false,
            },
            HostCall::SendKey {
                pane: key(1),
                key: "ctrl-c".into(),
            },
            HostCall::RequestClosePane { pane: key(1) },
        ];
        for call in calls {
            let plan = plan_call("demo", &call, &[], &mut limiter, 0);
            match plan {
                CallPlan::Reply(HostCallResult::Error { message }) => {
                    assert!(
                        message.contains("not granted"),
                        "denial must be an Error reply, not a drop: {message}"
                    );
                }
                other => panic!("expected Error reply, got {other:?}"),
            }
        }
    }

    #[test]
    fn every_call_id_gets_a_plan_including_denial() {
        // The drain loop maps one inbound Call to one plan. There is no
        // branch that produces "no reply".
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call("demo", &HostCall::ListPanes, &[], &mut limiter, 0);
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
    }

    #[test]
    fn read_screen_not_implied_by_v1_read_visible_screen() {
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "demo",
            &HostCall::ReadScreen { pane: key(1) },
            &[Capability::ReadVisibleScreen],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("HostCallReadScreen") || message.contains("not granted"));
            }
            other => panic!("read_visible_screen must not imply ReadScreen: {other:?}"),
        }
    }

    #[test]
    fn read_screen_on_a_panel_pane_key_fails() {
        let terminal = key(1);
        let panel = key(2);
        let mut terminals = BTreeSet::new();
        terminals.insert(terminal);
        let mut panels = BTreeSet::new();
        panels.insert(panel);
        assert!(
            read_screen_access(panel, &terminals, &panels)
                .unwrap_err()
                .contains("plugin panel")
        );
        assert!(
            read_screen_access(key(99), &terminals, &panels)
                .unwrap_err()
                .contains("not found")
        );
        assert!(read_screen_access(terminal, &terminals, &panels).is_ok());
    }

    #[test]
    fn targeted_input_uses_the_same_terminal_access_rules() {
        // FocusPane / SendText / SendKey / RequestClosePane execute against
        // read_screen_access so a plugin panel is never a write/close target
        // and a missing pane never falls back to the focused one.
        let terminal = key(1);
        let panel = key(2);
        let mut terminals = BTreeSet::new();
        terminals.insert(terminal);
        let mut panels = BTreeSet::new();
        panels.insert(panel);
        for pane in [panel, key(99)] {
            assert!(read_screen_access(pane, &terminals, &panels).is_err());
        }
    }

    #[test]
    fn notify_length_caps_are_enforced() {
        let mut limiter = HostCallLimiter::new();
        let title: String = std::iter::repeat_n('t', MAX_NOTIFY_TITLE + 40).collect();
        let body: String = std::iter::repeat_n('b', MAX_NOTIFY_BODY + 40).collect();
        let plan = plan_call(
            "demo",
            &notify(&title, &body),
            &[Capability::HostCallNotify],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::Notify { title, body } => {
                assert_eq!(title.chars().count(), MAX_NOTIFY_TITLE);
                assert_eq!(body.chars().count(), MAX_NOTIFY_BODY);
            }
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    #[test]
    fn screen_cap_truncates_at_char_boundary() {
        let s: String = std::iter::repeat_n('字', 10).collect();
        let capped = cap_screen(s);
        // 10 CJK chars is under the cap.
        assert_eq!(capped.chars().count(), 10);
        let huge: String = std::iter::repeat_n('a', MAX_SCREEN_CHARS + 50).collect();
        assert_eq!(cap_screen(huge).chars().count(), MAX_SCREEN_CHARS);
    }

    #[test]
    fn rate_limiting_drops_with_an_accounted_counter_and_still_replies() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallNotify];
        let mut accepted = 0u32;
        let mut denied = 0u32;
        for i in 0..RATE_MAX_CALLS + 5 {
            match plan_call("demo", &notify("t", "b"), &granted, &mut limiter, 1_000) {
                CallPlan::Notify { .. } => accepted += 1,
                CallPlan::Reply(HostCallResult::Error { message }) => {
                    assert_eq!(message, "rate limited");
                    denied += 1;
                }
                other => panic!("unexpected {other:?} at i={i}"),
            }
        }
        assert_eq!(accepted, RATE_MAX_CALLS);
        assert_eq!(denied, 5);
        assert_eq!(
            limiter.dropped_counts().get("demo").copied().unwrap_or(0),
            5
        );
        // Window expiry admits another call.
        match plan_call(
            "demo",
            &notify("t", "b"),
            &granted,
            &mut limiter,
            1_000 + RATE_WINDOW_MS,
        ) {
            CallPlan::Notify { .. } => {}
            other => panic!("window should have expired: {other:?}"),
        }
    }

    #[test]
    fn open_command_is_argv_not_a_shell_line() {
        let cmd = parse_open_command("cargo test --all").unwrap();
        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, ["test", "--all"]);
        assert!(parse_open_command("").is_err());
        let long: String = std::iter::repeat_n('x', MAX_OPEN_COMMAND_CHARS + 1).collect();
        assert!(parse_open_command(&long).is_err());
        // Quotes are literal tokens, not shell syntax.
        let quoted = parse_open_command(r#"echo "hello world""#).unwrap();
        assert_eq!(quoted.program, "echo");
        assert_eq!(quoted.args, [r#""hello"#, r#"world""#]);
    }

    #[test]
    fn open_pane_argv_plans_structured_argv_without_rejoin() {
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "demo",
            &HostCall::OpenPaneArgv {
                cwd: Some("/work".into()),
                program: "codex".into(),
                args: vec!["--model".into(), "gpt 4".into()],
            },
            &[Capability::HostCallOpenPane],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::OpenPane { cwd, command } => {
                assert_eq!(cwd.as_deref(), Some("/work"));
                let command = command.expect("argv");
                assert_eq!(command.program, "codex");
                assert_eq!(command.args, ["--model", "gpt 4"]);
                let argv = spawn_argv(command);
                assert_eq!(argv.0, "codex");
                assert_eq!(argv.1, ["--model", "gpt 4"]);
                assert_ne!(argv.1.join(" "), "codex --model gpt 4");
            }
            other => panic!("expected OpenPane plan, got {other:?}"),
        }
    }

    #[test]
    fn open_pane_argv_rejects_empty_nul_and_oversize() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallOpenPane];
        for call in [
            HostCall::OpenPaneArgv {
                cwd: None,
                program: "  ".into(),
                args: vec![],
            },
            HostCall::OpenPaneArgv {
                cwd: None,
                program: "ok\0no".into(),
                args: vec![],
            },
            HostCall::OpenPaneArgv {
                cwd: None,
                program: "ok".into(),
                args: vec!["a\0b".into()],
            },
            HostCall::OpenPaneArgv {
                cwd: Some("/tmp\0".into()),
                program: "ok".into(),
                args: vec![],
            },
        ] {
            match plan_call("demo", &call, &granted, &mut limiter, 0) {
                CallPlan::Reply(HostCallResult::Error { message }) => {
                    assert!(
                        message.contains("empty")
                            || message.contains("NUL")
                            || message.contains("exceeds")
                            || message.contains("too many"),
                        "{message}"
                    );
                }
                other => panic!("expected error, got {other:?}"),
            }
        }
        let too_many: Vec<String> = (0..MAX_OPEN_ARGS + 1).map(|i| i.to_string()).collect();
        match plan_call(
            "demo",
            &HostCall::OpenPaneArgv {
                cwd: None,
                program: "ok".into(),
                args: too_many,
            },
            &granted,
            &mut limiter,
            0,
        ) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("too many"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let long: String = std::iter::repeat_n('x', MAX_OPEN_PROGRAM_CHARS + 1).collect();
        match plan_call(
            "demo",
            &HostCall::OpenPaneArgv {
                cwd: None,
                program: long,
                args: vec![],
            },
            &granted,
            &mut limiter,
            0,
        ) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("program"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let long_arg: String = std::iter::repeat_n('y', MAX_OPEN_ARG_CHARS + 1).collect();
        match plan_call(
            "demo",
            &HostCall::OpenPaneArgv {
                cwd: None,
                program: "ok".into(),
                args: vec![long_arg],
            },
            &granted,
            &mut limiter,
            0,
        ) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("arg"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_open_cwd_is_malformed_not_executed() {
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "demo",
            &HostCall::OpenPane {
                cwd: Some("  ".into()),
                command: None,
            },
            &[Capability::HostCallOpenPane],
            &mut limiter,
            0,
        );
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
    }

    #[test]
    fn required_capability_is_the_protocol_mapping() {
        assert_eq!(
            notify("t", "b").required_capability(),
            Capability::HostCallNotify
        );
        assert_eq!(
            HostCall::ListPanes.required_capability(),
            Capability::HostCallListPanes
        );
        assert_eq!(
            HostCall::ReadScreen { pane: key(1) }.required_capability(),
            Capability::HostCallReadScreen
        );
        assert_eq!(
            HostCall::OpenPane {
                cwd: None,
                command: None
            }
            .required_capability(),
            Capability::HostCallOpenPane
        );
        assert_eq!(
            HostCall::OpenPaneArgv {
                cwd: None,
                program: "codex".into(),
                args: vec!["--foo".into()],
            }
            .required_capability(),
            Capability::HostCallOpenPane
        );
        assert_eq!(
            HostCall::ScrollToRun { run_id: key(1) }.required_capability(),
            Capability::HostCallScrollToRun
        );
        assert_eq!(
            HostCall::FocusPane { pane: key(1) }.required_capability(),
            Capability::HostCallFocusPane
        );
        assert_eq!(
            HostCall::SendText {
                pane: key(1),
                text: "x".into(),
                enter: false
            }
            .required_capability(),
            Capability::HostCallSendText
        );
        assert_eq!(
            HostCall::SendKey {
                pane: key(1),
                key: "escape".into()
            }
            .required_capability(),
            Capability::HostCallSendKey
        );
        assert_eq!(
            HostCall::RequestClosePane { pane: key(1) }.required_capability(),
            Capability::HostCallRequestClosePane
        );
    }

    #[test]
    fn reply_is_produced_when_the_plugin_is_already_gone() {
        // The drain loop still plans a result for the id. Sending it may fail
        // if the session is dead; that is a lost write, not a hang.
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "dead",
            &HostCall::ListPanes,
            &[Capability::HostCallListPanes],
            &mut limiter,
            0,
        );
        assert!(matches!(plan, CallPlan::ListPanes));
    }

    #[test]
    fn scroll_to_run_plans_when_granted_and_uses_the_default_rate_limiter() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallScrollToRun];
        let call = HostCall::ScrollToRun { run_id: key(9) };
        for _ in 0..RATE_MAX_CALLS {
            match plan_call("demo", &call, &granted, &mut limiter, 1_000) {
                CallPlan::ScrollToRun { run_id } => assert_eq!(run_id, key(9)),
                other => panic!("expected ScrollToRun within budget, got {other:?}"),
            }
        }
        assert!(matches!(
            plan_call("demo", &call, &granted, &mut limiter, 1_000),
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
        assert_eq!(
            limiter.dropped_counts().get("demo").copied().unwrap_or(0),
            1
        );
    }

    fn send_text(text: &str, enter: bool) -> HostCall {
        HostCall::SendText {
            pane: key(1),
            text: text.into(),
            enter,
        }
    }

    fn send_key(name: &str) -> HostCall {
        HostCall::SendKey {
            pane: key(1),
            key: name.into(),
        }
    }

    #[test]
    fn write_terminal_does_not_imply_cross_pane_calls() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::WriteTerminal];
        for call in [
            HostCall::FocusPane { pane: key(1) },
            send_text("hi", true),
            send_key("ctrl-c"),
            HostCall::RequestClosePane { pane: key(1) },
        ] {
            match plan_call("demo", &call, &granted, &mut limiter, 0) {
                CallPlan::Reply(HostCallResult::Error { message }) => {
                    assert!(
                        message.contains("not granted"),
                        "WriteTerminal must not grant {call:?}: {message}"
                    );
                }
                other => panic!("WriteTerminal must not imply {call:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn each_cross_pane_call_needs_its_own_capability() {
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "demo",
            &send_text("hi", false),
            &[Capability::HostCallFocusPane, Capability::HostCallSendKey],
            &mut limiter,
            0,
        );
        assert!(
            matches!(plan, CallPlan::Reply(HostCallResult::Error { .. })),
            "FocusPane/SendKey must not imply SendText: {plan:?}"
        );
    }

    #[test]
    fn send_text_plans_when_granted_and_rejects_oversize() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallSendText];
        match plan_call("demo", &send_text("hi", true), &granted, &mut limiter, 0) {
            CallPlan::SendText { text, enter, pane } => {
                assert_eq!(pane, key(1));
                assert_eq!(text, "hi");
                assert!(enter);
            }
            other => panic!("expected SendText, got {other:?}"),
        }
        let long: String = std::iter::repeat_n('x', MAX_SEND_TEXT_CHARS + 1).collect();
        match plan_call("demo", &send_text(&long, false), &granted, &mut limiter, 0) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("exceeds"), "{message}");
            }
            other => panic!("oversize must be an Error, not truncation: {other:?}"),
        }
        let exact: String = std::iter::repeat_n('y', MAX_SEND_TEXT_CHARS).collect();
        assert!(matches!(
            plan_call("demo", &send_text(&exact, false), &granted, &mut limiter, 0),
            CallPlan::SendText { .. }
        ));
    }

    #[test]
    fn send_key_accepts_allowlisted_names_and_rejects_unknown() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallSendKey];
        for (i, (name, expected)) in [
            ("ctrl-c", LogicalKey::CtrlC),
            ("Ctrl+C", LogicalKey::CtrlC),
            ("escape", LogicalKey::Escape),
            ("ESC", LogicalKey::Escape),
            ("enter", LogicalKey::Enter),
            ("tab", LogicalKey::Tab),
            ("up", LogicalKey::Up),
            ("arrow-down", LogicalKey::Down),
            ("left", LogicalKey::Left),
            ("right", LogicalKey::Right),
        ]
        .into_iter()
        .enumerate()
        {
            // Spread across the rate window so the allowlist, not the limiter,
            // is the thing under test.
            let now = (i as u64) * RATE_WINDOW_MS;
            match plan_call("demo", &send_key(name), &granted, &mut limiter, now) {
                CallPlan::SendKey { key: logical, pane } => {
                    assert_eq!(pane, key(1));
                    assert_eq!(logical, expected, "name {name}");
                }
                other => panic!("expected SendKey for {name}, got {other:?}"),
            }
        }
        let mut limiter = HostCallLimiter::new();
        match plan_call("demo", &send_key("ctrl-x"), &granted, &mut limiter, 0) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("unknown key"), "{message}");
                assert!(message.contains("ctrl-c"), "{message}");
            }
            other => panic!("unknown key must be Error, got {other:?}"),
        }
        match plan_call("demo", &send_key("\\x03"), &granted, &mut limiter, 0) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("unknown key"), "{message}");
            }
            other => panic!("encoded bytes must be rejected, got {other:?}"),
        }
        match plan_call("demo", &send_key("  "), &granted, &mut limiter, 0) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("empty"), "{message}");
            }
            other => panic!("empty key must be Error, got {other:?}"),
        }
    }

    #[test]
    fn cross_pane_writes_use_the_default_rate_limiter() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallSendText];
        let call = send_text("x", false);
        for _ in 0..RATE_MAX_CALLS {
            assert!(matches!(
                plan_call("demo", &call, &granted, &mut limiter, 1_000),
                CallPlan::SendText { .. }
            ));
        }
        assert!(matches!(
            plan_call("demo", &call, &granted, &mut limiter, 1_000),
            CallPlan::Reply(HostCallResult::Error { message }) if message == "rate limited"
        ));
        assert_eq!(
            limiter.dropped_counts().get("demo").copied().unwrap_or(0),
            1
        );
    }

    #[test]
    fn request_close_pane_plans_when_granted_and_needs_its_own_capability() {
        let mut limiter = HostCallLimiter::new();
        match plan_call(
            "demo",
            &HostCall::RequestClosePane { pane: key(3) },
            &[Capability::HostCallRequestClosePane],
            &mut limiter,
            0,
        ) {
            CallPlan::RequestClosePane { pane } => assert_eq!(pane, key(3)),
            other => panic!("expected RequestClosePane, got {other:?}"),
        }
        match plan_call(
            "demo",
            &HostCall::RequestClosePane { pane: key(3) },
            &[Capability::HostCallFocusPane],
            &mut limiter,
            0,
        ) {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("not granted"), "{message}");
            }
            other => panic!("FocusPane must not imply RequestClosePane: {other:?}"),
        }
    }

    #[test]
    fn focus_pane_plans_when_granted() {
        let mut limiter = HostCallLimiter::new();
        match plan_call(
            "demo",
            &HostCall::FocusPane { pane: key(3) },
            &[Capability::HostCallFocusPane],
            &mut limiter,
            0,
        ) {
            CallPlan::FocusPane { pane } => assert_eq!(pane, key(3)),
            other => panic!("expected FocusPane, got {other:?}"),
        }
    }

    #[test]
    fn logical_key_keystroke_tokens_match_the_terminal_table() {
        assert_eq!(LogicalKey::CtrlC.keystroke_str(), "ctrl-c");
        assert_eq!(LogicalKey::Escape.keystroke_str(), "escape");
        assert_eq!(LogicalKey::Enter.keystroke_str(), "enter");
        assert_eq!(LogicalKey::Tab.keystroke_str(), "tab");
        assert_eq!(LogicalKey::Up.keystroke_str(), "up");
        assert_eq!(LogicalKey::Down.keystroke_str(), "down");
        assert_eq!(LogicalKey::Left.keystroke_str(), "left");
        assert_eq!(LogicalKey::Right.keystroke_str(), "right");
    }

    #[test]
    fn send_key_is_not_eligible_without_a_terminal_or_in_vi_mode() {
        assert!(send_key_ready(true, false).is_ok());
        match send_key_ready(false, false) {
            Err(message) => assert_eq!(message, SEND_TEXT_NO_TERMINAL),
            Ok(()) => panic!("loading pane must reject SendKey"),
        }
        match send_key_ready(true, true) {
            Err(message) => {
                assert_eq!(message, SEND_KEY_VI_MODE);
                assert!(message.contains("vi mode"), "{message}");
            }
            Ok(()) => panic!("vi mode must reject SendKey"),
        }
        match send_key_ready(false, true) {
            Err(message) => assert_eq!(message, SEND_TEXT_NO_TERMINAL),
            Ok(()) => panic!("no terminal takes priority over vi mode"),
        }
    }

    #[test]
    fn execute_list_panes_returns_the_workspace_panes() {
        let mut io = FakeWorkspace::with_terminal(key(1), "hello");
        match CallPlan::ListPanes.execute(&mut io) {
            HostCallResult::Panes { panes } => {
                assert_eq!(panes.len(), 1);
                assert_eq!(panes[0].pane, key(1));
            }
            other => panic!("expected Panes, got {other:?}"),
        }
    }

    #[test]
    fn execute_read_screen_hits_and_misses() {
        let mut io = FakeWorkspace::with_terminal(key(1), "on screen");
        match (CallPlan::ReadScreen { pane: key(1) }).execute(&mut io) {
            HostCallResult::Screen { text } => assert_eq!(text, "on screen"),
            other => panic!("expected Screen, got {other:?}"),
        }
        match (CallPlan::ReadScreen { pane: key(2) }).execute(&mut io) {
            HostCallResult::Error { message } => assert!(message.contains("not found"), "{message}"),
            other => panic!("missing pane must be an Error, got {other:?}"),
        }
    }

    #[test]
    fn execute_read_screen_caps_at_the_screen_limit() {
        let huge: String = std::iter::repeat_n('a', MAX_SCREEN_CHARS + 100).collect();
        let mut io = FakeWorkspace::with_terminal(key(1), &huge);
        match (CallPlan::ReadScreen { pane: key(1) }).execute(&mut io) {
            HostCallResult::Screen { text } => {
                assert_eq!(text.chars().count(), MAX_SCREEN_CHARS);
            }
            other => panic!("expected Screen, got {other:?}"),
        }
    }

    #[test]
    fn execute_send_text_delivers_to_the_pane_and_denies_missing() {
        let mut io = FakeWorkspace::with_terminal(key(1), "");
        let plan = CallPlan::SendText {
            pane: key(1),
            text: "ls\n".into(),
            enter: true,
        };
        assert_eq!(plan.execute(&mut io), HostCallResult::Ok);
        assert_eq!(io.sent_text, vec![(key(1), "ls\n".to_string(), true)]);

        let missing = CallPlan::SendText {
            pane: key(2),
            text: "x".into(),
            enter: false,
        };
        match missing.execute(&mut io) {
            HostCallResult::Error { message } => assert!(message.contains("not found"), "{message}"),
            other => panic!("missing pane must deny, got {other:?}"),
        }
        // The denied call left no side effect.
        assert_eq!(io.sent_text.len(), 1);
    }

    #[test]
    fn execute_focus_send_key_and_close_route_to_io() {
        let mut io = FakeWorkspace::with_terminal(key(1), "");
        assert_eq!(
            (CallPlan::FocusPane { pane: key(1) }).execute(&mut io),
            HostCallResult::Ok
        );
        assert_eq!(
            (CallPlan::SendKey {
                pane: key(1),
                key: LogicalKey::CtrlC
            })
            .execute(&mut io),
            HostCallResult::Ok
        );
        assert_eq!(
            (CallPlan::RequestClosePane { pane: key(1) }).execute(&mut io),
            HostCallResult::Ok
        );
        assert_eq!(io.focused, vec![key(1)]);
        assert_eq!(io.sent_keys, vec![(key(1), LogicalKey::CtrlC)]);
        assert_eq!(io.closed, vec![key(1)]);
    }

    #[test]
    fn execute_open_pane_forwards_cwd_and_command_and_returns_the_io_reply() {
        let mut io = FakeWorkspace::default();
        io.open_reply = Some(HostCallResult::Pane { pane: key(7) });
        let plan = CallPlan::OpenPane {
            cwd: Some("/work".into()),
            command: Some(OpenCommand {
                program: "codex".into(),
                args: vec!["--model".into()],
            }),
        };
        match plan.execute(&mut io) {
            HostCallResult::Pane { pane } => assert_eq!(pane, key(7)),
            other => panic!("expected Pane, got {other:?}"),
        }
        let (cwd, command) = io.opened.expect("open_pane called");
        assert_eq!(cwd.as_deref(), Some("/work"));
        let command = command.expect("argv");
        assert_eq!(command.program, "codex");
        assert_eq!(command.args, ["--model"]);
    }

    #[test]
    fn execute_reply_short_circuits_without_touching_io() {
        let mut io = FakeWorkspace::default();
        let plan = CallPlan::Reply(HostCallResult::Error {
            message: "capability HostCallListPanes not granted".into(),
        });
        match plan.execute(&mut io) {
            HostCallResult::Error { message } => assert!(message.contains("not granted")),
            other => panic!("Reply must pass through, got {other:?}"),
        }
        assert!(io.opened.is_none() && io.sent_text.is_empty());
    }

    #[test]
    fn execute_notify_is_never_a_workspace_call() {
        // Planners route Notify to the host side effect, not execute. If it ever
        // reaches here it must still produce a reply, not silently drop the id.
        let mut io = FakeWorkspace::default();
        let plan = CallPlan::Notify {
            title: "t".into(),
            body: "b".into(),
        };
        match plan.execute(&mut io) {
            HostCallResult::Error { message } => assert!(message.contains("notify"), "{message}"),
            other => panic!("notify must not be executed as a workspace verb: {other:?}"),
        }
    }
}

