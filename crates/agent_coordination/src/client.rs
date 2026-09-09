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
