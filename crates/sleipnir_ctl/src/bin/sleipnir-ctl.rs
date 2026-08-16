//! Default-off control client (ADR-0011). Does not start a listener.

use sleipnir_ctl::{socket_path, ControlRequest, ControlResponse};
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(op) = args.next() else {
        eprintln!("usage: sleipnir-ctl ls | capture <pane> | send <pane> <text> | wait <pane> free|failed|attention");
        std::process::exit(2);
    };
    let req = match op.as_str() {
        "ls" => ControlRequest::Ls,
        "capture" => {
            let pane = args.next().expect("pane uuid");
            ControlRequest::Capture {
                pane: pane.parse().expect("uuid"),
            }
        }
        "send" => {
            let pane = args.next().expect("pane uuid");
            let text = args.collect::<Vec<_>>().join(" ");
            ControlRequest::Send {
                pane: pane.parse().expect("uuid"),
                text,
                enter: true,
            }
        }
        "wait" => {
            let pane = args.next().expect("pane uuid");
            let until = args.next().unwrap_or_else(|| "free".into());
            ControlRequest::Wait {
                pane: pane.parse().expect("uuid"),
                until: match until.as_str() {
                    "failed" => sleipnir_ctl::WaitUntil::Failed,
                    "attention" => sleipnir_ctl::WaitUntil::Attention,
                    _ => sleipnir_ctl::WaitUntil::Free,
                },
                timeout_secs: 60,
            }
        }
        _ => {
            eprintln!("unknown op {op}");
            std::process::exit(2);
        }
    };
    let path = socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("control socket not available ({path:?}): {err}");
            eprintln!("enable with control_surface: true or SLEIPNIR_CONTROL=1");
            std::process::exit(1);
        }
    };
    let line = serde_json::to_string(&req).expect("json");
    writeln!(stream, "{line}").ok();
    let mut reply = String::new();
    std::io::BufReader::new(&stream).read_line(&mut reply).ok();
    match serde_json::from_str::<ControlResponse>(reply.trim()) {
        Ok(resp) => println!("{}", serde_json::to_string_pretty(&resp).unwrap_or(reply)),
        Err(_) => print!("{reply}"),
    }
}
