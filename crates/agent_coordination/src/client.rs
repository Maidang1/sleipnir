//! One-shot JSON-lines client for the coordination socket.

use std::path::Path;

use crate::protocol::{WireRequest, WireResponse};

#[cfg(unix)]
use std::io::{BufReader, Write};

#[cfg(unix)]
use crate::line::{BoundedRead, MAX_LINE_BYTES, read_bounded_line};
#[cfg(unix)]
use crate::protocol::{decode_response_line, encode_request_line};

#[derive(Debug)]
pub enum ClientError {
    Unsupported,
    Connect { path: String, message: String },
    Io(std::io::Error),
    Oversize,
    Decode(String),
    Encode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => {
                write!(f, "agent-control socket is not supported on this platform")
            }
            Self::Connect { path, message } => write!(
                f,
                "agent-control socket not available at {path}: {message} \
                 (the host must start the coordination server; execution still awaits an adapter)"
            ),
            Self::Io(err) => write!(f, "agent-control I/O error: {err}"),
            Self::Oversize => write!(f, "server response exceeds line length cap"),
            Self::Decode(err) => write!(f, "malformed server response: {err}"),
            Self::Encode(err) => write!(f, "failed to encode request: {err}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Send one request and read one response. Does not consume adapter effects.
#[cfg(unix)]
pub fn call(path: &Path, req: &WireRequest) -> Result<WireResponse, ClientError> {
    use crate::protocol::{Request, Response};
    let mut reply = call_page(path, req)?;
    // Preserve the CLI's complete list/inspect/effects behavior while each
    // transport frame remains bounded. Explicit page requests stay one page.
    let mut last_cursor = 0;
    let mut last_wait_offset = 0;
    for _ in 0..4096 {
        let body = match (&req.body, &reply.body) {
            (
                Request::List,
                Response::Agents {
                    next_offset: Some(offset),
                    ..
                },
            ) => Request::ListPage { offset: *offset },
            (Request::Inspect { session }, Response::Inspect { session: snapshot }) => {
                let Some(offset) = snapshot.next_task_offset else {
                    return Ok(reply);
                };
                Request::InspectPage {
                    session: *session,
                    offset,
                }
            }
            (
                Request::Effects,
                Response::Effects {
                    next_cursor: Some(cursor),
                    ..
                },
            ) => {
                if *cursor <= last_cursor {
                    return Err(ClientError::Decode("non-advancing effect page".into()));
                }
                last_cursor = *cursor;
                Request::EffectsPage { cursor: *cursor }
            }
            (
                Request::Wait { task },
                Response::Wait {
                    next_result_offset: Some(offset),
                    ..
                },
            ) => {
                if *offset <= last_wait_offset {
                    return Err(ClientError::Decode("non-advancing wait page".into()));
                }
                last_wait_offset = *offset;
                Request::WaitPage {
                    task: *task,
                    offset: *offset,
                }
            }
            _ => return Ok(reply),
        };
        let next = call_page(path, &WireRequest { id: req.id, body })?;
        if next.id != req.id {
            return Err(ClientError::Decode("page correlation mismatch".into()));
        }
        match (&mut reply.body, next.body) {
            (
                Response::Agents {
                    agents,
                    next_offset,
                },
                Response::Agents {
                    agents: page,
                    next_offset: after,
                },
            ) => {
                if after.is_some_and(|offset| offset <= next_offset.unwrap_or(0)) {
                    return Err(ClientError::Decode("non-advancing session page".into()));
                }
                agents.extend(page);
                *next_offset = after;
            }
            (Response::Inspect { session }, Response::Inspect { session: page })
                if session.session == page.session =>
            {
                if page
                    .next_task_offset
                    .is_some_and(|offset| offset <= session.next_task_offset.unwrap_or(0))
                {
                    return Err(ClientError::Decode("non-advancing task page".into()));
                }
                session.tasks.extend(page.tasks);
                session.next_task_offset = page.next_task_offset;
            }
            (
                Response::Effects {
                    effects,
                    next_cursor,
                },
                Response::Effects {
                    effects: page,
                    next_cursor: after,
                },
            ) => {
                effects.extend(page);
                *next_cursor = after;
            }
            (
                Response::Wait {
                    task,
                    status,
                    terminal,
                    result,
                    detail,
                    next_result_offset,
                },
                Response::Wait {
                    task: next_task,
                    status: next_status,
                    terminal: next_terminal,
                    result: next_result,
                    detail: next_detail,
                    next_result_offset: after,
                },
            ) => {
                if *task != next_task || *status != next_status || *terminal != next_terminal {
                    return Err(ClientError::Decode("wait page changed task state".into()));
                }
                if *detail != next_detail {
                    return Err(ClientError::Decode("wait page changed detail".into()));
                }
                let mut merged = result.take().unwrap_or_default();
                merged.push_str(next_result.as_deref().unwrap_or_default());
                *result = (!merged.is_empty()).then_some(merged);
                *next_result_offset = after;
            }
            (_, Response::Error { message }) => {
                return Ok(WireResponse {
                    id: req.id,
                    body: Response::Error { message },
                });
            }
            _ => return Err(ClientError::Decode("unexpected page response".into())),
        }
    }
    Err(ClientError::Decode("too many response pages".into()))
}

#[cfg(unix)]
fn call_page(path: &Path, req: &WireRequest) -> Result<WireResponse, ClientError> {
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(path).map_err(|err| ClientError::Connect {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    let line = encode_request_line(req).map_err(ClientError::Encode)?;
    writeln!(stream, "{line}").map_err(ClientError::Io)?;
    stream.flush().map_err(ClientError::Io)?;
    let mut reader = BufReader::new(stream);
    match read_bounded_line(&mut reader, MAX_LINE_BYTES).map_err(ClientError::Io)? {
        BoundedRead::Eof => Err(ClientError::Decode("server closed the connection".into())),
        BoundedRead::Oversize => Err(ClientError::Oversize),
        BoundedRead::Line(reply) => decode_response_line(&reply).map_err(ClientError::Decode),
    }
}

#[cfg(not(unix))]
pub fn call(path: &Path, req: &WireRequest) -> Result<WireResponse, ClientError> {
    let _ = (path, req);
    Err(ClientError::Unsupported)
}
