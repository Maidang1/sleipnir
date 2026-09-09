//! JSON-lines client for the local agent-coordination server.
//! Does not start a listener, spawn an agent, or consume adapter effects.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use agent_coordination::{
    Response, WireRequest, WireResponse, call, default_socket_path, encode_response_line,
};
use sleipnir_agentctl::{
    AWAIT_ADAPTER_NOTE, Command, EXIT_ERROR, EXIT_USAGE, Parsed, WaitDecision, WaitFollowup,
    apply_stdin, correlation_mismatch, followup_from_accepted, interpret_wait, parse_args,
};

fn main() -> ExitCode {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match run(parsed) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run(parsed: Parsed) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let path = parsed.socket.unwrap_or_else(default_socket_path);
    if let Some(followup) = parsed.command.direct_wait() {
        return wait_on(&path, followup, 1, true);
    }
    if matches!(
        parsed.command,
        Command::PromptWait { .. } | Command::LaunchWait { .. }
    ) {
        return accepted_then_wait(&path, &parsed.command);
    }
    let mut command = parsed.command;
    if command.needs_stdin() {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        command = apply_stdin(command, buf)?;
    }
    let req = WireRequest {
        id: 1,
        body: command.to_request(),
    };
    let resp = call(&path, &req)?;
    if let Some(message) = correlation_mismatch(req.id, &resp) {
        print_json(&resp)?;
        return Err(message.into());
    }
    print_json(&resp)?;
    if !matches!(resp.body, Response::Error { .. }) {
        if command.queues_adapter_work() {
            eprintln!("{AWAIT_ADAPTER_NOTE}");
        } else if command.is_worker_report() {
            eprintln!(
                "note: recorded a local worker self-report; any same-user socket client can spoof this"
            );
        }
    }
    Ok(match resp.body {
        Response::Error { .. } => ExitCode::from(EXIT_ERROR),
        _ => ExitCode::SUCCESS,
    })
}

fn accepted_then_wait(
    path: &Path,
    command: &Command,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let req = WireRequest {
        id: 1,
        body: command.to_request(),
    };
    let resp = call(path, &req)?;
    if let Some(message) = correlation_mismatch(req.id, &resp) {
        print_json(&resp)?;
        return Err(message.into());
    }
    if matches!(resp.body, Response::Error { .. }) {
        print_json(&resp)?;
        return Ok(ExitCode::from(EXIT_ERROR));
    }
    let followup = followup_from_accepted(command, &resp.body)?;
    wait_on(path, followup, 2, false)
}

fn wait_on(
    path: &Path,
    followup: WaitFollowup,
    mut id: u64,
    print_each: bool,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let start = Instant::now();
    loop {
        let req_id = id;
        id = id.saturating_add(1);
        let resp = call(
            path,
            &WireRequest {
                id: req_id,
                body: followup.wait_request(),
            },
        )?;
        if matches!(resp.body, Response::Error { .. }) {
            if let Some(message) = correlation_mismatch(req_id, &resp) {
                print_json(&resp)?;
                return Err(message.into());
            }
            print_json(&resp)?;
            return Ok(ExitCode::from(EXIT_ERROR));
        }
        match interpret_wait(req_id, &resp, &followup, start.elapsed())? {
            WaitDecision::Continue(snapshot) => {
                if print_each {
                    println!("{}", serde_json::to_string(&snapshot)?);
                }
                thread::sleep(Duration::from_millis(50));
            }
            WaitDecision::Done { snapshot, exit } => {
                println!("{}", serde_json::to_string(&snapshot)?);
                if snapshot.timed_out {
                    eprintln!(
                        "timed out; execution awaits an adapter (status still {:?})",
                        snapshot.status
                    );
                } else if !snapshot.terminal && !followup.until_process {
                    eprintln!("note: snapshot only; execution awaits an adapter");
                } else if followup.until_process && !snapshot.terminal {
                    eprintln!(
                        "note: launch task observed ({:?}); this CLI does not prompt",
                        snapshot.status
                    );
                }
                return Ok(ExitCode::from(exit));
            }
        }
    }
}

fn print_json(resp: &WireResponse) -> io::Result<()> {
    let line = encode_response_line(resp).map_err(io::Error::other)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{line}")
}
