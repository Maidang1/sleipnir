//! `sleipnir-agentctl` argument parsing and wait-poll policy.
//!
//! The binary speaks JSON-lines to the local coordination server. Launch and
//! prompt are **accepted and queued**; execution awaits an adapter this CLI
//! does not provide. There is no approve command.
//!
//! `prompt-wait` / `launch-wait` are client-side compositions of those
//! accepted ops plus `wait`. They do not add protocol operations.

use std::path::PathBuf;
use std::time::Duration;

use agent_coordination::{
    AgentKind, AgentSessionId, CoordinationTaskId, Request, Response, TaskStatus, WireResponse,
};
use serde::Serialize;
use uuid::Uuid;

mod cli;
pub use cli::run_cli;

pub const AWAIT_ADAPTER_NOTE: &str = "note: accepted and queued; execution awaits an adapter (this CLI does not launch or prompt an agent)";

/// I/O, protocol, or correlation-id mismatch.
pub const EXIT_ERROR: u8 = 1;
/// Usage / unknown command.
pub const EXIT_USAGE: u8 = 2;
/// `wait` ended in `failed_delivery`.
pub const EXIT_FAILED_DELIVERY: u8 = 3;
/// `wait` ended in `unknown`.
pub const EXIT_UNKNOWN: u8 = 4;
/// `wait` client-side timeout while still non-terminal.
pub const EXIT_TIMEOUT: u8 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub socket: Option<PathBuf>,
    pub command: Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    List,
    Launch {
        kind: AgentKind,
        cwd: String,
        name: Option<String>,
        args: Vec<String>,
    },
    /// `launch` then poll `wait` until the launch task is observed or terminal.
    /// Does not send a prompt.
    LaunchWait {
        kind: AgentKind,
        cwd: String,
        name: Option<String>,
        args: Vec<String>,
        timeout_ms: u64,
    },
    Prompt {
        session: AgentSessionId,
        text: String,
    },
    /// `prompt` then poll `wait` until the prompt task is terminal.
    PromptWait {
        session: AgentSessionId,
        text: String,
        timeout_ms: u64,
    },
    Wait {
        task: CoordinationTaskId,
        timeout_ms: u64,
    },
    Interrupt {
        session: AgentSessionId,
    },
    Focus {
        session: AgentSessionId,
    },
    Inspect {
        session: AgentSessionId,
    },
    HumanTakeover {
        session: AgentSessionId,
    },
    Close {
        session: AgentSessionId,
    },
    Effects,
    Facts {
        cursor: u64,
    },
    ReportRunning {
        task: CoordinationTaskId,
    },
    ReportAwaitingHuman {
        task: CoordinationTaskId,
        detail: Option<String>,
        stdin: bool,
    },
    ReportResult {
        task: CoordinationTaskId,
        text: String,
        stdin: bool,
    },
    ReportSessionClosed {
        session: AgentSessionId,
    },
}

impl Command {
    pub fn to_request(&self) -> Request {
        match self {
            Self::List => Request::List,
            Self::Launch {
                kind,
                cwd,
                name,
                args,
            }
            | Self::LaunchWait {
                kind,
                cwd,
                name,
                args,
                ..
            } => Request::Launch {
                kind: *kind,
                cwd: cwd.clone(),
                name: name.clone(),
                args: args.clone(),
            },
            Self::Prompt { session, text } | Self::PromptWait { session, text, .. } => {
                Request::Prompt {
                    session: *session,
                    text: text.clone(),
                }
            }
            Self::Wait { task, .. } => Request::Wait { task: *task },
            Self::Interrupt { session } => Request::Interrupt { session: *session },
            Self::Focus { session } => Request::Focus { session: *session },
            Self::Inspect { session } => Request::Inspect { session: *session },
            Self::HumanTakeover { session } => Request::HumanTakeover { session: *session },
            Self::Close { session } => Request::Close { session: *session },
            Self::Effects => Request::Effects,
            Self::Facts { cursor } => Request::Facts { cursor: *cursor },
            Self::ReportRunning { task } => Request::ReportRunning { task: *task },
            Self::ReportAwaitingHuman { task, detail, .. } => Request::ReportAwaitingHuman {
                task: *task,
                detail: detail.clone(),
            },
            Self::ReportResult { task, text, .. } => Request::ReportResult {
                task: *task,
                text: text.clone(),
            },
            Self::ReportSessionClosed { session } => {
                Request::ReportSessionClosed { session: *session }
            }
        }
    }

    pub fn queues_adapter_work(&self) -> bool {
        matches!(
            self,
            Self::Launch { .. }
                | Self::LaunchWait { .. }
                | Self::Prompt { .. }
                | Self::PromptWait { .. }
                | Self::Interrupt { .. }
                | Self::Focus { .. }
                | Self::Close { .. }
        )
    }

    pub fn is_worker_report(&self) -> bool {
        matches!(
            self,
            Self::ReportRunning { .. }
                | Self::ReportAwaitingHuman { .. }
                | Self::ReportResult { .. }
                | Self::ReportSessionClosed { .. }
        )
    }

    pub fn needs_stdin(&self) -> bool {
        matches!(
            self,
            Self::ReportAwaitingHuman { stdin: true, .. } | Self::ReportResult { stdin: true, .. }
        )
    }

    /// Direct `wait`, or a convenience command that waits after an accepted op.
    pub fn is_wait_workflow(&self) -> bool {
        matches!(
            self,
            Self::Wait { .. } | Self::PromptWait { .. } | Self::LaunchWait { .. }
        )
    }

    /// Client-side wait already known from the command (plain `wait` only).
    pub fn direct_wait(&self) -> Option<WaitFollowup> {
        match self {
            Self::Wait { task, timeout_ms } => Some(WaitFollowup {
                task: *task,
                session: None,
                timeout_ms: *timeout_ms,
                until_process: false,
                op: OP_WAIT,
            }),
            _ => None,
        }
    }
}

pub fn apply_stdin(command: Command, input: String) -> Result<Command, ParseError> {
    match command {
        Command::ReportResult {
            task, stdin: true, ..
        } => Ok(Command::ReportResult {
            task,
            text: input,
            stdin: false,
        }),
        Command::ReportAwaitingHuman {
            task, stdin: true, ..
        } => Ok(Command::ReportAwaitingHuman {
            task,
            detail: if input.trim().is_empty() {
                None
            } else {
                Some(input)
            },
            stdin: false,
        }),
        other => Ok(other),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn usage() -> &'static str {
    "usage: sleipnir-agentctl [--socket PATH] <command> [args]\n\
     built-in: sleipnir agentctl [--socket PATH] <command> [args]\n\
     commands:\n\
       list\n\
       launch <kind> <cwd> [--name NAME] [-- ARG...]\n\
       launch-wait <kind> <cwd> [--name NAME] [--timeout-ms N] [-- ARG...]\n\
       prompt <session> <text...>\n\
       prompt-wait <session> [--timeout-ms N] <text...>\n\
       wait <task> [--timeout-ms N]\n\
       interrupt <session>\n\
       focus <session>\n\
       inspect <session>\n\
       human-takeover <session>\n\
       close <session>\n\
       effects\n\
       facts [cursor]\n\
       report-running <task>\n\
       report-awaiting-human <task> [detail...] [--stdin]\n\
       report-result <task> [text...] [--stdin]\n\
       report-session-closed <session>\n\
     Launch and prompt are accepted and queued; execution awaits an adapter.\n\
     launch-wait / prompt-wait compose those ops with wait on the client;\n\
     launch-wait does not prompt. --timeout-ms 0 (default) is one snapshot.\n\
     report-* commands are local worker self-reports; any same-user socket\n\
     client can spoof them. They are not approval answers and have no success flag.\n\
     There is no approve command.\n\
     exit codes: 0 settled, launch-wait process observed, or non-terminal snapshot;\n\
     1 I/O/protocol/correlation; 2 usage; 3 wait failed_delivery; 4 wait unknown;\n\
     5 wait timed out."
}

pub fn parse_args<I, S>(args: I) -> Result<Parsed, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    if args.is_empty() {
        return Err(usage_err("missing command"));
    }

    let mut socket = None;
    while args.first().map(String::as_str) == Some("--socket") {
        args.remove(0);
        let path = args
            .first()
            .cloned()
            .ok_or_else(|| usage_err("--socket requires a path"))?;
        args.remove(0);
        socket = Some(PathBuf::from(path));
    }

    let cmd = args
        .first()
        .cloned()
        .ok_or_else(|| usage_err("missing command"))?;
    let rest = &args[1..];
    let command = match cmd.as_str() {
        "help" | "--help" | "-h" => {
            return Err(ParseError {
                message: usage().to_string(),
            });
        }
        "approve" | "deny" | "answer_approval" => {
            return Err(ParseError {
                message: format!(
                    "{cmd} is not a protocol operation; native approvals stay in the visible pane"
                ),
            });
        }
        "list" => {
            reject_extra("list", rest)?;
            Command::List
        }
        "launch" => parse_launch(rest)?,
        "launch-wait" | "launch_wait" => parse_launch_wait(rest)?,
        "prompt" => parse_prompt(rest)?,
        "prompt-wait" | "prompt_wait" => parse_prompt_wait(rest)?,
        "wait" => parse_wait(rest)?,
        "interrupt" => Command::Interrupt {
            session: parse_session(one("interrupt", rest)?)?,
        },
        "focus" => Command::Focus {
            session: parse_session(one("focus", rest)?)?,
        },
        "inspect" => Command::Inspect {
            session: parse_session(one("inspect", rest)?)?,
        },
        "human-takeover" | "human_takeover" => Command::HumanTakeover {
            session: parse_session(one("human-takeover", rest)?)?,
        },
        "close" => Command::Close {
            session: parse_session(one("close", rest)?)?,
        },
        "effects" => {
            reject_extra("effects", rest)?;
            Command::Effects
        }
        "facts" => parse_facts(rest)?,
        "report-running" | "report_running" => Command::ReportRunning {
            task: parse_task(one("report-running", rest)?)?,
        },
        "report-awaiting-human" | "report_awaiting_human" => parse_report_awaiting(rest)?,
        "report-result" | "report_result" => parse_report_result(rest)?,
        "report-session-closed" | "report_session_closed" => Command::ReportSessionClosed {
            session: parse_session(one("report-session-closed", rest)?)?,
        },
        other => {
            return Err(ParseError {
                message: format!("unknown command {other}\n{}", usage()),
            });
        }
    };
    Ok(Parsed { socket, command })
}

fn usage_err(message: &str) -> ParseError {
    ParseError {
        message: format!("{message}\n{}", usage()),
    }
}

fn reject_extra(cmd: &str, rest: &[String]) -> Result<(), ParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(usage_err(&format!("{cmd} takes no arguments")))
    }
}

fn one<'a>(cmd: &str, rest: &'a [String]) -> Result<&'a str, ParseError> {
    match rest {
        [value] => Ok(value.as_str()),
        [] => Err(usage_err(&format!("{cmd} requires an id"))),
        _ => Err(usage_err(&format!("{cmd} takes a single id"))),
    }
}

fn parse_kind(s: &str) -> Result<AgentKind, ParseError> {
    s.parse().map_err(|message| ParseError { message })
}

fn parse_session(s: &str) -> Result<AgentSessionId, ParseError> {
    Uuid::parse_str(s)
        .map(AgentSessionId::from_uuid)
        .map_err(|_| ParseError {
            message: format!("session id is not a UUID: {s}"),
        })
}

fn parse_task(s: &str) -> Result<CoordinationTaskId, ParseError> {
    Uuid::parse_str(s)
        .map(CoordinationTaskId::from_uuid)
        .map_err(|_| ParseError {
            message: format!("task id is not a UUID: {s}"),
        })
}

struct LaunchSpec {
    kind: AgentKind,
    cwd: String,
    name: Option<String>,
    args: Vec<String>,
    timeout_ms: u64,
}

fn parse_launch(rest: &[String]) -> Result<Command, ParseError> {
    let spec = parse_launch_spec("launch", rest, false)?;
    Ok(Command::Launch {
        kind: spec.kind,
        cwd: spec.cwd,
        name: spec.name,
        args: spec.args,
    })
}

fn parse_launch_wait(rest: &[String]) -> Result<Command, ParseError> {
    let spec = parse_launch_spec("launch-wait", rest, true)?;
    Ok(Command::LaunchWait {
        kind: spec.kind,
        cwd: spec.cwd,
        name: spec.name,
        args: spec.args,
        timeout_ms: spec.timeout_ms,
    })
}

fn parse_launch_spec(
    cmd: &str,
    rest: &[String],
    allow_timeout: bool,
) -> Result<LaunchSpec, ParseError> {
    if rest.len() < 2 {
        return Err(usage_err(&format!("{cmd} requires <kind> <cwd>")));
    }
    let kind = parse_kind(&rest[0])?;
    let cwd = rest[1].clone();
    let mut name = None;
    let mut args = Vec::new();
    let mut timeout_ms = 0;
    let mut seen_timeout = false;
    let mut i = 2;
    while i < rest.len() {
        match rest[i].as_str() {
            "--" => {
                args.extend(rest[i + 1..].iter().cloned());
                break;
            }
            "--name" => {
                i += 1;
                let value = rest
                    .get(i)
                    .ok_or_else(|| usage_err("--name requires a value"))?;
                name = Some(value.clone());
            }
            "--timeout-ms" if allow_timeout => {
                if seen_timeout {
                    return Err(usage_err("duplicate --timeout-ms"));
                }
                i += 1;
                timeout_ms = parse_timeout_ms(
                    rest.get(i)
                        .ok_or_else(|| usage_err("--timeout-ms requires an integer"))?,
                )?;
                seen_timeout = true;
            }
            other => {
                let flags = if allow_timeout {
                    "--name NAME, --timeout-ms N, or -- ARG..."
                } else {
                    "--name NAME or -- ARG..."
                };
                return Err(usage_err(&format!(
                    "unexpected {cmd} argument {other:?}; use {flags}"
                )));
            }
        }
        i += 1;
    }
    Ok(LaunchSpec {
        kind,
        cwd,
        name,
        args,
        timeout_ms,
    })
}

fn parse_prompt(rest: &[String]) -> Result<Command, ParseError> {
    let (session, text, _) = parse_prompt_spec("prompt", rest, false)?;
    Ok(Command::Prompt { session, text })
}

fn parse_prompt_wait(rest: &[String]) -> Result<Command, ParseError> {
    let (session, text, timeout_ms) = parse_prompt_spec("prompt-wait", rest, true)?;
    Ok(Command::PromptWait {
        session,
        text,
        timeout_ms,
    })
}

fn parse_prompt_spec(
    cmd: &str,
    rest: &[String],
    allow_timeout: bool,
) -> Result<(AgentSessionId, String, u64), ParseError> {
    if rest.is_empty() {
        return Err(usage_err(&format!("{cmd} requires <session> <text...>")));
    }
    let session = parse_session(&rest[0])?;
    let (words, timeout_ms) = if allow_timeout {
        extract_timeout_ms(&rest[1..])?
    } else {
        (rest[1..].to_vec(), 0)
    };
    let text = words.join(" ");
    if text.trim().is_empty() {
        return Err(usage_err(&format!("{cmd} requires <session> <text...>")));
    }
    Ok((session, text, timeout_ms))
}

fn parse_wait(rest: &[String]) -> Result<Command, ParseError> {
    if rest.is_empty() {
        return Err(usage_err("wait requires <task>"));
    }
    let task = parse_task(&rest[0])?;
    let (extra, timeout_ms) = extract_timeout_ms(&rest[1..])?;
    if let Some(other) = extra.first() {
        return Err(usage_err(&format!("unexpected wait argument {other:?}")));
    }
    Ok(Command::Wait { task, timeout_ms })
}

fn parse_timeout_ms(raw: &str) -> Result<u64, ParseError> {
    raw.parse::<u64>().map_err(|_| ParseError {
        message: format!("timeout-ms must be an integer: {raw}"),
    })
}

/// Pull `--timeout-ms N` out of a token list. Duplicate flags are an error.
/// Remaining words keep their order so prompt text and wait extras stay intact.
fn extract_timeout_ms(args: &[String]) -> Result<(Vec<String>, u64), ParseError> {
    let mut timeout_ms = 0;
    let mut seen = false;
    let mut words = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--timeout-ms" {
            if seen {
                return Err(usage_err("duplicate --timeout-ms"));
            }
            i += 1;
            timeout_ms = parse_timeout_ms(
                args.get(i)
                    .ok_or_else(|| usage_err("--timeout-ms requires an integer"))?,
            )?;
            seen = true;
            i += 1;
        } else {
            words.push(args[i].clone());
            i += 1;
        }
    }
    Ok((words, timeout_ms))
}

fn take_stdin_flag(args: &[String]) -> Result<(Vec<String>, bool), ParseError> {
    let mut stdin = false;
    let mut words = Vec::new();
    for arg in args {
        if arg == "--stdin" {
            if stdin {
                return Err(usage_err("duplicate --stdin"));
            }
            stdin = true;
        } else {
            words.push(arg.clone());
        }
    }
    Ok((words, stdin))
}

fn parse_report_awaiting(rest: &[String]) -> Result<Command, ParseError> {
    let (words, stdin) = take_stdin_flag(rest)?;
    if words.is_empty() {
        return Err(usage_err("report-awaiting-human requires <task>"));
    }
    let task = parse_task(&words[0])?;
    let extra = &words[1..];
    if stdin && !extra.is_empty() {
        return Err(usage_err("do not mix --stdin with inline text"));
    }
    let detail = if extra.is_empty() {
        None
    } else {
        Some(extra.join(" "))
    };
    Ok(Command::ReportAwaitingHuman {
        task,
        detail,
        stdin,
    })
}

fn parse_report_result(rest: &[String]) -> Result<Command, ParseError> {
    let (words, stdin) = take_stdin_flag(rest)?;
    if words.is_empty() {
        return Err(usage_err("report-result requires <task>"));
    }
    let task = parse_task(&words[0])?;
    let extra = &words[1..];
    if stdin && !extra.is_empty() {
        return Err(usage_err("do not mix --stdin with inline text"));
    }
    Ok(Command::ReportResult {
        task,
        text: extra.join(" "),
        stdin,
    })
}

fn parse_facts(rest: &[String]) -> Result<Command, ParseError> {
    match rest {
        [] => Ok(Command::Facts { cursor: 0 }),
        [cursor] => {
            let cursor = cursor.parse::<u64>().map_err(|_| ParseError {
                message: format!("facts cursor must be an integer: {cursor}"),
            })?;
            Ok(Command::Facts { cursor })
        }
        _ => Err(usage_err("facts takes at most a cursor")),
    }
}

/// Client-side wait policy. `timeout_ms == 0` is a single snapshot, not an
/// immediate timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitPoll {
    Stop,
    TimedOut,
    Continue,
}

pub fn wait_poll(terminal: bool, elapsed: Duration, timeout_ms: u64) -> WaitPoll {
    if terminal {
        WaitPoll::Stop
    } else if timeout_ms == 0 {
        WaitPoll::Stop
    } else if elapsed >= Duration::from_millis(timeout_ms) {
        WaitPoll::TimedOut
    } else {
        WaitPoll::Continue
    }
}

/// Exit status for a finished `wait`. `timed_out` wins; a non-terminal
/// snapshot is 0; `failed_delivery` is 3; `unknown` is 4; `settled` is 0.
pub fn wait_exit_code(status: TaskStatus, terminal: bool, timed_out: bool) -> u8 {
    if timed_out {
        return EXIT_TIMEOUT;
    }
    if !terminal {
        return 0;
    }
    match status {
        TaskStatus::FailedDelivery => EXIT_FAILED_DELIVERY,
        TaskStatus::Unknown => EXIT_UNKNOWN,
        _ => 0,
    }
}

/// `None` when ids match. Oversize/malformed server replies may carry `id: 0`.
pub fn correlation_mismatch(req_id: u64, resp: &WireResponse) -> Option<String> {
    if resp.id == req_id {
        None
    } else {
        Some(format!(
            "response id {} does not match request id {req_id}",
            resp.id
        ))
    }
}

pub const OP_WAIT: &str = "wait";
pub const OP_PROMPT_WAIT: &str = "prompt_wait";
pub const OP_LAUNCH_WAIT: &str = "launch_wait";

/// Client-side plan: poll `wait` for this task after an accepted op (or directly).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitFollowup {
    pub task: CoordinationTaskId,
    pub session: Option<AgentSessionId>,
    pub timeout_ms: u64,
    /// `launch-wait`: stop when the process is observed (`running` /
    /// `awaiting_human`) or the task is terminal. Not a prompt.
    pub until_process: bool,
    pub op: &'static str,
}

impl WaitFollowup {
    pub fn wait_request(&self) -> Request {
        Request::Wait { task: self.task }
    }
}

/// One wait JSON object. Convenience commands print only the final snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WaitSnapshot {
    pub id: u64,
    pub op: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<AgentSessionId>,
    pub task: CoordinationTaskId,
    pub status: TaskStatus,
    pub terminal: bool,
    pub timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitDecision {
    Continue(WaitSnapshot),
    Done { snapshot: WaitSnapshot, exit: u8 },
}

/// Build the wait plan from `prompt_accepted` / `launch_accepted`.
pub fn followup_from_accepted(command: &Command, body: &Response) -> Result<WaitFollowup, String> {
    match (command, body) {
        (
            Command::PromptWait {
                session,
                timeout_ms,
                ..
            },
            Response::PromptAccepted { task },
        ) => Ok(WaitFollowup {
            task: *task,
            session: Some(*session),
            timeout_ms: *timeout_ms,
            until_process: false,
            op: OP_PROMPT_WAIT,
        }),
        (Command::LaunchWait { timeout_ms, .. }, Response::LaunchAccepted { session, task }) => {
            Ok(WaitFollowup {
                task: *task,
                session: Some(*session),
                timeout_ms: *timeout_ms,
                until_process: true,
                op: OP_LAUNCH_WAIT,
            })
        }
        (_, Response::Error { message }) => Err(message.clone()),
        (Command::PromptWait { .. }, other) => {
            Err(format!("expected prompt_accepted, got {other:?}"))
        }
        (Command::LaunchWait { .. }, other) => {
            Err(format!("expected launch_accepted, got {other:?}"))
        }
        _ => Err("command does not wait on an accepted task".into()),
    }
}

/// `launch-wait` treats process observation as done; `prompt-wait` / `wait`
/// require a terminal status (or a timeout-ms 0 snapshot via [`wait_poll`]).
pub fn wait_target_reached(until_process: bool, status: TaskStatus, terminal: bool) -> bool {
    if terminal {
        true
    } else if until_process {
        matches!(status, TaskStatus::Running | TaskStatus::AwaitingHuman)
    } else {
        false
    }
}

/// Correlate, then decide whether to keep polling. Protocol `Error` bodies
/// are returned as `Err` so the caller can print the original JSON.
pub fn interpret_wait(
    req_id: u64,
    resp: &WireResponse,
    followup: &WaitFollowup,
    elapsed: Duration,
) -> Result<WaitDecision, String> {
    if let Some(message) = correlation_mismatch(req_id, resp) {
        return Err(message);
    }
    match &resp.body {
        Response::Error { message } => Err(message.clone()),
        Response::Wait {
            task,
            status,
            terminal,
            result,
            detail,
            next_result_offset: _,
        } => {
            if *task != followup.task {
                return Err(format!(
                    "wait task {task:?} does not match follow-up task {:?}",
                    followup.task
                ));
            }
            let reached = wait_target_reached(followup.until_process, *status, *terminal);
            let poll = wait_poll(reached, elapsed, followup.timeout_ms);
            let timed_out = poll == WaitPoll::TimedOut;
            let snapshot = WaitSnapshot {
                id: resp.id,
                op: followup.op,
                session: followup.session,
                task: *task,
                status: *status,
                terminal: *terminal,
                timed_out,
                result: result.clone(),
                detail: detail.clone(),
            };
            match poll {
                WaitPoll::Continue => Ok(WaitDecision::Continue(snapshot)),
                WaitPoll::Stop | WaitPoll::TimedOut => Ok(WaitDecision::Done {
                    exit: wait_exit_code(*status, *terminal, timed_out),
                    snapshot,
                }),
            }
        }
        other => Err(format!("unexpected wait response: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> String {
        Uuid::from_u128(1).to_string()
    }

    fn tid() -> String {
        Uuid::from_u128(2).to_string()
    }

    #[test]
    fn list_and_diagnostics() {
        assert_eq!(parse_args(["list"]).unwrap().command, Command::List);
        assert_eq!(parse_args(["effects"]).unwrap().command, Command::Effects);
        assert_eq!(
            parse_args(["facts"]).unwrap().command,
            Command::Facts { cursor: 0 }
        );
        assert_eq!(
            parse_args(["facts", "9"]).unwrap().command,
            Command::Facts { cursor: 9 }
        );
    }

    #[test]
    fn launch_parses_kind_cwd_name_and_argv() {
        let parsed = parse_args([
            "launch", "codex", "/work", "--name", "w1", "--", "--foo", "bar",
        ])
        .unwrap();
        match parsed.command {
            Command::Launch {
                kind,
                cwd,
                name,
                args,
            } => {
                assert_eq!(kind, AgentKind::Codex);
                assert_eq!(cwd, "/work");
                assert_eq!(name.as_deref(), Some("w1"));
                assert_eq!(args, vec!["--foo", "bar"]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_args(["launch", "cursor", "/work"]), Err(_)));
    }

    #[test]
    fn prompt_joins_remaining_words() {
        let parsed = parse_args(["prompt", &sid(), "implement", "the", "tests"]).unwrap();
        match parsed.command {
            Command::Prompt { text, .. } => assert_eq!(text, "implement the tests"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wait_default_is_a_single_snapshot() {
        let parsed = parse_args(["wait", &tid()]).unwrap();
        match parsed.command {
            Command::Wait { timeout_ms, .. } => assert_eq!(timeout_ms, 0),
            other => panic!("{other:?}"),
        }
        let parsed = parse_args(["wait", &tid(), "--timeout-ms", "1500"]).unwrap();
        match parsed.command {
            Command::Wait { timeout_ms, .. } => assert_eq!(timeout_ms, 1500),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn session_commands_parse_uuids() {
        let id = sid();
        for (args, expect_human) in [
            (vec!["interrupt", id.as_str()], false),
            (vec!["focus", id.as_str()], false),
            (vec!["inspect", id.as_str()], false),
            (vec!["human-takeover", id.as_str()], true),
            (vec!["close", id.as_str()], false),
        ] {
            let command = parse_args(args).unwrap().command;
            match command {
                Command::HumanTakeover { .. } => assert!(expect_human),
                Command::Interrupt { .. }
                | Command::Focus { .. }
                | Command::Inspect { .. }
                | Command::Close { .. } => assert!(!expect_human),
                other => panic!("{other:?}"),
            }
        }
        assert!(parse_args(["inspect", "not-a-uuid"]).is_err());
    }

    #[test]
    fn socket_flag_is_stripped_before_the_command() {
        let parsed = parse_args(["--socket", "/tmp/x.sock", "list"]).unwrap();
        assert_eq!(parsed.socket, Some(PathBuf::from("/tmp/x.sock")));
        assert_eq!(parsed.command, Command::List);
    }

    #[test]
    fn approve_is_rejected_with_a_useful_error() {
        let err = parse_args(["approve"]).unwrap_err();
        assert!(err.message.contains("not a protocol operation"));
        assert!(!err.message.contains("usage:"));
        let err = parse_args(["deny"]).unwrap_err();
        assert!(err.message.contains("not a protocol operation"));
    }

    #[test]
    fn unknown_and_empty_print_usage() {
        let err = parse_args::<[&str; 0], _>([]).unwrap_err();
        assert!(err.message.contains("usage:"));
        let err = parse_args(["nope"]).unwrap_err();
        assert!(err.message.contains("unknown command"));
        assert!(err.message.contains("usage:"));
    }

    #[test]
    fn to_request_does_not_claim_execution() {
        let launch = parse_args(["launch", "gemini", "/repo"]).unwrap();
        assert!(matches!(
            launch.command.to_request(),
            Request::Launch { .. }
        ));
        assert!(launch.command.queues_adapter_work());
        assert!(!Command::List.queues_adapter_work());
        assert!(
            !Command::Inspect {
                session: AgentSessionId::from_uuid(Uuid::from_u128(1))
            }
            .queues_adapter_work()
        );
    }

    #[test]
    fn wait_poll_zero_timeout_is_a_snapshot() {
        assert_eq!(
            wait_poll(false, Duration::from_millis(0), 0),
            WaitPoll::Stop
        );
        assert_eq!(
            wait_poll(true, Duration::from_millis(0), 5_000),
            WaitPoll::Stop
        );
        assert_eq!(
            wait_poll(false, Duration::from_millis(0), 5_000),
            WaitPoll::Continue
        );
        assert_eq!(
            wait_poll(false, Duration::from_millis(5_000), 5_000),
            WaitPoll::TimedOut
        );
    }

    #[test]
    fn wait_exit_classifies_terminal_statuses() {
        assert_eq!(wait_exit_code(TaskStatus::Settled, true, false), 0);
        assert_eq!(
            wait_exit_code(TaskStatus::FailedDelivery, true, false),
            EXIT_FAILED_DELIVERY
        );
        assert_eq!(
            wait_exit_code(TaskStatus::Unknown, true, false),
            EXIT_UNKNOWN
        );
        assert_eq!(
            wait_exit_code(TaskStatus::Dispatching, false, true),
            EXIT_TIMEOUT
        );
        assert_eq!(wait_exit_code(TaskStatus::Dispatching, false, false), 0);
        assert_eq!(
            wait_exit_code(TaskStatus::FailedDelivery, true, true),
            EXIT_TIMEOUT
        );
    }

    #[test]
    fn correlation_helper_rejects_mismatched_ids() {
        let resp = agent_coordination::WireResponse {
            id: 0,
            body: agent_coordination::Response::Error {
                message: "request exceeds line length cap".into(),
            },
        };
        assert!(correlation_mismatch(1, &resp).is_some());
        let ok = agent_coordination::WireResponse {
            id: 7,
            body: agent_coordination::Response::Agents {
                agents: vec![],
                next_offset: None,
            },
        };
        assert!(correlation_mismatch(7, &ok).is_none());
    }

    #[test]
    fn report_commands_parse_and_map_to_requests() {
        let task = tid();
        let session = sid();
        let running = parse_args(["report-running", &task]).unwrap();
        assert!(running.command.is_worker_report());
        assert!(!running.command.queues_adapter_work());
        assert!(matches!(
            running.command.to_request(),
            Request::ReportRunning { .. }
        ));
        let awaiting = parse_args(["report-awaiting-human", &task, "please", "review"]).unwrap();
        match awaiting.command.to_request() {
            Request::ReportAwaitingHuman { detail, .. } => {
                assert_eq!(detail.as_deref(), Some("please review"));
            }
            other => panic!("{other:?}"),
        }
        let result = parse_args(["report-result", &task, "files", "written"]).unwrap();
        match result.command.to_request() {
            Request::ReportResult { text, .. } => assert_eq!(text, "files written"),
            other => panic!("{other:?}"),
        }
        let closed = parse_args(["report-session-closed", &session]).unwrap();
        assert!(matches!(
            closed.command.to_request(),
            Request::ReportSessionClosed { .. }
        ));
    }

    #[test]
    fn report_stdin_flag_and_apply_stdin_helper() {
        let task = tid();
        let parsed = parse_args(["report-result", &task, "--stdin"]).unwrap();
        assert!(parsed.command.needs_stdin());
        let filled = apply_stdin(parsed.command, "from stdin".into()).unwrap();
        match filled.to_request() {
            Request::ReportResult { text, .. } => assert_eq!(text, "from stdin"),
            other => panic!("{other:?}"),
        }
        assert!(parse_args(["report-result", &task, "--stdin", "nope"]).is_err());
        let awaiting = parse_args(["report-awaiting-human", &task, "--stdin"]).unwrap();
        let filled = apply_stdin(awaiting.command, "   ".into()).unwrap();
        match filled {
            Command::ReportAwaitingHuman { detail, stdin, .. } => {
                assert!(detail.is_none());
                assert!(!stdin);
            }
            other => panic!("{other:?}"),
        }
        let awaiting = parse_args(["report-awaiting-human", &task, "--stdin"]).unwrap();
        let filled = apply_stdin(awaiting.command, "native dialog".into()).unwrap();
        match filled.to_request() {
            Request::ReportAwaitingHuman { detail, .. } => {
                assert_eq!(detail.as_deref(), Some("native dialog"));
            }
            other => panic!("{other:?}"),
        }
    }

    fn task_id(n: u128) -> CoordinationTaskId {
        CoordinationTaskId::from_uuid(Uuid::from_u128(n))
    }

    fn session_id(n: u128) -> AgentSessionId {
        AgentSessionId::from_uuid(Uuid::from_u128(n))
    }

    fn wait_resp(
        id: u64,
        task: CoordinationTaskId,
        status: TaskStatus,
        result: Option<&str>,
        detail: Option<&str>,
    ) -> WireResponse {
        WireResponse {
            id,
            body: Response::Wait {
                task,
                status,
                terminal: status.is_terminal(),
                result: result.map(str::to_string),
                detail: detail.map(str::to_string),
                next_result_offset: None,
            },
        }
    }

    #[test]
    fn prompt_wait_parses_timeout_before_or_after_text() {
        let session = sid();
        let before = parse_args([
            "prompt-wait",
            &session,
            "--timeout-ms",
            "1500",
            "implement",
            "the",
            "tests",
        ])
        .unwrap();
        match &before.command {
            Command::PromptWait {
                text, timeout_ms, ..
            } => {
                assert_eq!(text, "implement the tests");
                assert_eq!(*timeout_ms, 1500);
            }
            other => panic!("{other:?}"),
        }
        let after = parse_args([
            "prompt-wait",
            &session,
            "implement",
            "the",
            "tests",
            "--timeout-ms",
            "1500",
        ])
        .unwrap();
        assert_eq!(before.command, after.command);
        assert!(matches!(
            before.command.to_request(),
            Request::Prompt { .. }
        ));
        assert!(before.command.queues_adapter_work());
        assert!(before.command.is_wait_workflow());
        assert!(parse_args(["prompt-wait", &session, "--timeout-ms", "5"]).is_err());
        assert!(parse_args(["launch", "codex", "/work", "--timeout-ms", "5"]).is_err());
    }

    #[test]
    fn launch_wait_parses_flag_order_and_does_not_prompt() {
        let named_first = parse_args([
            "launch-wait",
            "codex",
            "/work",
            "--name",
            "w1",
            "--timeout-ms",
            "2000",
            "--",
            "--foo",
        ])
        .unwrap();
        let timeout_first = parse_args([
            "launch-wait",
            "codex",
            "/work",
            "--timeout-ms",
            "2000",
            "--name",
            "w1",
            "--",
            "--foo",
        ])
        .unwrap();
        assert_eq!(named_first.command, timeout_first.command);
        match named_first.command.to_request() {
            Request::Launch {
                kind,
                cwd,
                name,
                args,
            } => {
                assert_eq!(kind, AgentKind::Codex);
                assert_eq!(cwd, "/work");
                assert_eq!(name.as_deref(), Some("w1"));
                assert_eq!(args, vec!["--foo"]);
            }
            other => panic!("{other:?}"),
        }
        assert!(!matches!(
            named_first.command.to_request(),
            Request::Prompt { .. }
        ));
        match parse_args(["launch-wait", "codex", "/work", "--", "--timeout-ms", "9"])
            .unwrap()
            .command
        {
            Command::LaunchWait {
                args, timeout_ms, ..
            } => {
                assert_eq!(timeout_ms, 0);
                assert_eq!(args, vec!["--timeout-ms", "9"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn followup_uses_accepted_ids_and_launch_wait_does_not_auto_prompt() {
        let prompt = parse_args(["prompt-wait", &sid(), "--timeout-ms", "5", "go"])
            .unwrap()
            .command;
        let prompt_task = task_id(9);
        let follow =
            followup_from_accepted(&prompt, &Response::PromptAccepted { task: prompt_task })
                .unwrap();
        assert_eq!(follow.task, prompt_task);
        assert_eq!(follow.session, Some(session_id(1)));
        assert!(!follow.until_process);
        assert_eq!(follow.op, OP_PROMPT_WAIT);
        assert_eq!(follow.wait_request(), Request::Wait { task: prompt_task });

        let launch = parse_args(["launch-wait", "gemini", "/repo", "--timeout-ms", "40"])
            .unwrap()
            .command;
        let launch_task = task_id(8);
        let launch_session = session_id(7);
        let follow = followup_from_accepted(
            &launch,
            &Response::LaunchAccepted {
                session: launch_session,
                task: launch_task,
            },
        )
        .unwrap();
        assert_eq!(follow.task, launch_task);
        assert_eq!(follow.session, Some(launch_session));
        assert!(follow.until_process);
        assert_eq!(follow.wait_request(), Request::Wait { task: launch_task });
        assert!(!matches!(follow.wait_request(), Request::Prompt { .. }));
        assert!(
            followup_from_accepted(&launch, &Response::PromptAccepted { task: launch_task })
                .is_err()
        );
        assert!(
            followup_from_accepted(
                &Command::List,
                &Response::Agents {
                    agents: vec![],
                    next_offset: None
                }
            )
            .is_err()
        );
    }

    #[test]
    fn launch_wait_stops_on_process_observation_prompt_wait_does_not() {
        assert!(wait_target_reached(true, TaskStatus::Running, false));
        assert!(wait_target_reached(true, TaskStatus::AwaitingHuman, false));
        assert!(wait_target_reached(true, TaskStatus::FailedDelivery, true));
        assert!(wait_target_reached(true, TaskStatus::Unknown, true));
        assert!(!wait_target_reached(true, TaskStatus::Dispatching, false));
        assert!(!wait_target_reached(false, TaskStatus::Running, false));
        assert!(wait_target_reached(false, TaskStatus::Settled, true));
    }

    #[test]
    fn interpret_wait_propagates_result_and_failure_exit_codes() {
        let task = task_id(2);
        let follow = WaitFollowup {
            task,
            session: Some(session_id(1)),
            timeout_ms: 5_000,
            until_process: false,
            op: OP_PROMPT_WAIT,
        };
        let done = interpret_wait(
            2,
            &wait_resp(
                2,
                task,
                TaskStatus::Settled,
                Some("files written"),
                Some("note"),
            ),
            &follow,
            Duration::from_millis(10),
        )
        .unwrap();
        match done {
            WaitDecision::Done { snapshot, exit } => {
                assert_eq!(exit, 0);
                assert_eq!(snapshot.result.as_deref(), Some("files written"));
                assert_eq!(snapshot.detail.as_deref(), Some("note"));
                assert!(!snapshot.timed_out);
                let json = serde_json::to_string(&snapshot).unwrap();
                assert!(json.contains("\"result\":\"files written\""));
                assert!(json.contains("\"detail\":\"note\""));
                assert!(json.contains("prompt_wait"));
            }
            other => panic!("{other:?}"),
        }

        let failed = interpret_wait(
            3,
            &wait_resp(3, task, TaskStatus::FailedDelivery, None, None),
            &follow,
            Duration::from_millis(10),
        )
        .unwrap();
        match failed {
            WaitDecision::Done { exit, .. } => assert_eq!(exit, EXIT_FAILED_DELIVERY),
            other => panic!("{other:?}"),
        }

        let unknown = interpret_wait(
            4,
            &wait_resp(4, task, TaskStatus::Unknown, None, None),
            &follow,
            Duration::from_millis(10),
        )
        .unwrap();
        match unknown {
            WaitDecision::Done { exit, .. } => assert_eq!(exit, EXIT_UNKNOWN),
            other => panic!("{other:?}"),
        }

        let timed = interpret_wait(
            5,
            &wait_resp(5, task, TaskStatus::Dispatching, None, None),
            &follow,
            Duration::from_millis(5_000),
        )
        .unwrap();
        match timed {
            WaitDecision::Done { snapshot, exit, .. } => {
                assert_eq!(exit, EXIT_TIMEOUT);
                assert!(snapshot.timed_out);
            }
            other => panic!("{other:?}"),
        }

        assert!(
            interpret_wait(
                9,
                &wait_resp(8, task, TaskStatus::Settled, None, None),
                &follow,
                Duration::from_millis(0),
            )
            .unwrap_err()
            .contains("does not match request id")
        );

        let launch = WaitFollowup {
            until_process: true,
            op: OP_LAUNCH_WAIT,
            timeout_ms: 5_000,
            ..follow
        };
        match interpret_wait(
            6,
            &wait_resp(6, task, TaskStatus::Running, None, None),
            &launch,
            Duration::from_millis(20),
        )
        .unwrap()
        {
            WaitDecision::Done { snapshot, exit } => {
                assert_eq!(exit, 0);
                assert!(!snapshot.terminal);
                assert_eq!(snapshot.status, TaskStatus::Running);
            }
            other => panic!("{other:?}"),
        }
        match interpret_wait(
            7,
            &wait_resp(7, task, TaskStatus::Dispatching, None, None),
            &launch,
            Duration::from_millis(0),
        )
        .unwrap()
        {
            WaitDecision::Continue(_) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn convenience_commands_are_not_approvals() {
        let err = parse_args(["approve"]).unwrap_err();
        assert!(err.message.contains("not a protocol operation"));
        assert!(usage().contains("There is no approve command."));
        assert!(
            !usage()
                .lines()
                .any(|line| line.trim().starts_with("approve"))
        );
        let prompt_wait = parse_args(["prompt-wait", &sid(), "please review"]).unwrap();
        assert!(!matches!(
            prompt_wait.command.to_request(),
            Request::ReportAwaitingHuman { .. }
        ));
        assert!(parse_args(["prompt-wait", "approve", "x"]).is_err());
    }
}
