//! Length-capped JSON-lines over loopback TCP with per-window capability tokens.
use crate::{ENDPOINT_ENV, Request, Response, TOKEN_ENV, WINDOW_ENV};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const MAX_FRAME: usize = 256 * 1024;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_CONNECTIONS: usize = 8;
#[derive(Clone)]
pub struct Credentials {
    pub endpoint: SocketAddr,
    pub token: String,
    pub window: u64,
}
impl Credentials {
    pub fn from_env() -> Result<Self, String> {
        let endpoint = std::env::var(ENDPOINT_ENV)
            .map_err(
                |_| "Launch this MCP server from a Sleipnir terminal; browser endpoint is missing",
            )?
            .parse::<SocketAddr>()
            .map_err(|_| "Invalid browser endpoint")?;
        if !endpoint.ip().is_loopback() {
            return Err("Browser endpoint must be loopback".into());
        }
        let token = std::env::var(TOKEN_ENV).map_err(|_| "Browser capability token is missing")?;
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid browser capability token".into());
        }
        let window = std::env::var(WINDOW_ENV)
            .map_err(|_| "Window binding is missing")?
            .parse()
            .map_err(|_| "Invalid window binding")?;
        Ok(Self {
            endpoint,
            token,
            window,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    request: Request,
}

pub fn read_line(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete JSON line",
                ))
            };
        }
        let end = buf
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(buf.len());
        if line.len() + end > MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON line too large",
            ));
        }
        let complete = buf[end - 1] == b'\n';
        line.extend_from_slice(&buf[..end]);
        reader.consume(end);
        if complete {
            return Ok(Some(line));
        }
    }
}
pub fn write_line(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()
}
fn token_matches(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |n, (a, b)| n | (a ^ b)) == 0
}

/// Handle one accepted connection: read a single framed request, authorize it,
/// and write one framed response. Runs on its own worker thread.
fn serve_connection(
    mut stream: TcpStream,
    token: &str,
    stop: &AtomicBool,
    window: u64,
    handler: &(impl Fn(Request, Instant) -> Response + Send + Sync),
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let response = match read_line(&mut BufReader::new(&mut stream)) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Envelope>(&bytes) {
            Ok(envelope)
                if token_matches(&envelope.token, token) && !stop.load(Ordering::Acquire) =>
            {
                if envelope.request.window().is_some_and(|id| id != window) {
                    Response::error(
                        "wrong_window",
                        "This connection is bound to a different window",
                    )
                } else if let Err(error) = envelope.request.validate() {
                    Response::error("invalid_arguments", error)
                } else {
                    handler(envelope.request, Instant::now() + REQUEST_TIMEOUT)
                }
            }
            Ok(_) => Response::error("unauthorized", "Invalid or expired browser capability"),
            Err(_) => Response::error("invalid_request", "Invalid browser request"),
        },
        _ => Response::error(
            "invalid_request",
            "Missing, oversized or incomplete browser request",
        ),
    };
    let _ = write_line(&mut stream, &response);
}

pub struct Server {
    pub credentials: Credentials,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(
        window: u64,
        handler: impl Fn(Request, Instant) -> Response + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let credentials = Credentials {
            endpoint: listener.local_addr()?,
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            window,
        };
        let token = credentials.token.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let handler = Arc::new(handler);
        let active = Arc::new(AtomicUsize::new(0));
        let join = thread::Builder::new()
            .name("browser-control".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if active.load(Ordering::Acquire) >= MAX_CONNECTIONS {
                                let _ = write_line(
                                    &mut stream,
                                    &Response::error(
                                        "busy",
                                        "Too many concurrent browser connections",
                                    ),
                                );
                                continue;
                            }
                            active.fetch_add(1, Ordering::AcqRel);
                            let worker_active = active.clone();
                            let handler = handler.clone();
                            let token = token.clone();
                            let stop = stopped.clone();
                            let spawned = thread::Builder::new()
                                .name("browser-request".into())
                                .spawn(move || {
                                    serve_connection(
                                        stream,
                                        &token,
                                        &stop,
                                        window,
                                        handler.as_ref(),
                                    );
                                    worker_active.fetch_sub(1, Ordering::AcqRel);
                                });
                            if spawned.is_err() {
                                active.fetch_sub(1, Ordering::AcqRel);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            credentials,
            stop,
            join: Some(join),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn call(credentials: &Credentials, request: Request) -> Result<Response, String> {
    request.validate()?;
    if !credentials.endpoint.ip().is_loopback() {
        return Err("Only loopback endpoints are allowed".into());
    }
    if request.window().is_some_and(|id| id != credentials.window) {
        return Err("Request does not match the bound window".into());
    }
    let mut stream = TcpStream::connect_timeout(&credentials.endpoint, Duration::from_secs(2))
        .map_err(|e| format!("Browser unavailable: {e}"))?;
    stream
        .set_read_timeout(Some(REQUEST_TIMEOUT + Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    write_line(
        &mut stream,
        &Envelope {
            token: credentials.token.clone(),
            request,
        },
    )
    .map_err(|e| e.to_string())?;
    let bytes = read_line(&mut BufReader::new(stream))
        .map_err(|e| e.to_string())?
        .ok_or("Browser closed connection")?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_socket_checks_token_and_window() {
        let server = Server::start(42, |_, _| Response::Windows { windows: vec![] }).unwrap();
        assert!(!call(&server.credentials, Request::List).unwrap().is_error());
        let mut wrong = server.credentials.clone();
        wrong.token = "0".repeat(64);
        assert!(call(&wrong, Request::List).unwrap().is_error());
        let mut wrong_window = server.credentials.clone();
        wrong_window.window = 7;
        assert!(
            call(&wrong_window, Request::Status { window: 7 })
                .unwrap()
                .is_error()
        );
        assert!(call(&server.credentials, Request::Status { window: 7 }).is_err());
        let credentials = server.credentials.clone();
        drop(server);
        assert!(call(&credentials, Request::List).is_err());
    }
    #[test]
    fn capped_lines_reject_large_and_incomplete_frames() {
        assert!(read_line(&mut &vec![b'a'; MAX_FRAME + 1][..]).is_err());
        assert!(read_line(&mut &b"{}"[..]).is_err());
        assert_eq!(
            read_line(&mut &b"{}\n"[..]).unwrap(),
            Some(b"{}\n".to_vec())
        );
    }
}
