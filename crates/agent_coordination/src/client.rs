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
                 (start Sleipnir with its built-in Agents service enabled)"
            ),
            Self::Io(err) => write!(f, "agent-control I/O error: {err}"),
            Self::Oversize => write!(f, "server response exceeds line length cap"),
            Self::Decode(err) => write!(f, "malformed server response: {err}"),
            Self::Encode(err) => write!(f, "failed to encode request: {err}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Send one request and read framed responses on the same connection.
/// Does not consume adapter effects.
#[cfg(unix)]
pub fn call(path: &Path, req: &WireRequest) -> Result<WireResponse, ClientError> {
    use std::os::unix::net::UnixStream;

    use crate::protocol::Response;

    let mut stream = UnixStream::connect(path).map_err(|err| ClientError::Connect {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;
    let line = encode_request_line(req).map_err(ClientError::Encode)?;
    writeln!(stream, "{line}").map_err(ClientError::Io)?;
    stream.flush().map_err(ClientError::Io)?;
    let mut reader = BufReader::new(stream);
    let mut reply = read_frame(&mut reader)?;
    if reply.id != req.id && !matches!(reply.body, Response::Error { .. }) {
        return Err(ClientError::Decode("page correlation mismatch".into()));
    }
    for _ in 0..4096 {
        if !is_partial(&reply.body) {
            return Ok(reply);
        }
        let next = read_frame(&mut reader)?;
        if next.id != req.id {
            return Err(ClientError::Decode("page correlation mismatch".into()));
        }
        merge_frame(&mut reply, next)?;
    }
    Err(ClientError::Decode("too many response pages".into()))
}

#[cfg(unix)]
fn read_frame(reader: &mut BufReader<impl std::io::Read>) -> Result<WireResponse, ClientError> {
    match read_bounded_line(reader, MAX_LINE_BYTES).map_err(ClientError::Io)? {
        BoundedRead::Eof => Err(ClientError::Decode("server closed the connection".into())),
        BoundedRead::Oversize => Err(ClientError::Oversize),
        BoundedRead::Line(reply) => decode_response_line(&reply).map_err(ClientError::Decode),
    }
}

#[cfg(unix)]
fn is_partial(body: &crate::protocol::Response) -> bool {
    use crate::protocol::Response;
    match body {
        Response::Agents {
            next_offset: Some(_),
            ..
        }
        | Response::Effects {
            next_cursor: Some(_),
            ..
        }
        | Response::Wait {
            next_result_offset: Some(_),
            ..
        }
        | Response::Facts { more: true, .. } => true,
        Response::Inspect { session } => session.next_task_offset.is_some(),
        _ => false,
    }
}

#[cfg(unix)]
fn merge_frame(reply: &mut WireResponse, next: WireResponse) -> Result<(), ClientError> {
    use crate::protocol::Response;
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
            Ok(())
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
            Ok(())
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
            if after.is_some_and(|cursor| cursor <= next_cursor.unwrap_or(0)) {
                return Err(ClientError::Decode("non-advancing effect page".into()));
            }
            effects.extend(page);
            *next_cursor = after;
            Ok(())
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
            Ok(())
        }
        (
            Response::Facts {
                facts,
                next_cursor,
                missed,
                more,
            },
            Response::Facts {
                facts: page,
                next_cursor: after,
                missed: extra_missed,
                more: after_more,
            },
        ) => {
            facts.extend(page);
            *next_cursor = after;
            *missed = missed.saturating_add(extra_missed);
            *more = after_more;
            Ok(())
        }
        (_, Response::Error { message }) => {
            reply.body = Response::Error { message };
            Ok(())
        }
        _ => Err(ClientError::Decode("unexpected page response".into())),
    }
}

#[cfg(not(unix))]
pub fn call(path: &Path, req: &WireRequest) -> Result<WireResponse, ClientError> {
    let _ = (path, req);
    Err(ClientError::Unsupported)
}
