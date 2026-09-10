//! Local Unix JSON-lines coordination server.
//!
//! Accepts a shared [`Registry`]. Does **not** consume adapter effects or
//! fabricate an adapter. Windows builds compile with [`ServerError::Unsupported`].

use std::path::{Path, PathBuf};

use crate::registry::Registry;

/// Max concurrent client handler threads. Extra connects are dropped.
pub const MAX_CLIENTS: usize = 16;
/// Idle read timeout per connection. A client that never writes is closed.
pub const CLIENT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug)]
pub enum ServerError {
    Unsupported,
    /// A live server is already bound at this path. The existing listener
    /// was not unlinked.
    AlreadyRunning {
        path: PathBuf,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => {
                write!(f, "agent-control socket is not supported on this platform")
            }
            Self::AlreadyRunning { path } => write!(
                f,
                "agent-control socket already in use at {} (not hijacking a live server)",
                path.display()
            ),
            Self::Io(err) => write!(f, "agent-control socket error: {err}"),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<std::io::Error> for ServerError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// `$SLEIPNIR_AGENT_CONTROL_SOCKET` if set and non-empty, else
/// `~/.config/sleipnir/agent-control.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("SLEIPNIR_AGENT_CONTROL_SOCKET") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if cfg!(windows) {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("sleipnir")
            .join("agent-control.sock")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/sleipnir/agent-control.sock")
    }
}

pub struct Server {
    path: PathBuf,
    /// `(dev, ino)` of the socket we bound. Drop unlinks only when the path
    /// still names this inode, so a replacement listener is not stolen.
    #[cfg(unix)]
    identity: Option<(u64, u64)>,
    #[cfg(unix)]
    stop: Option<std::sync::mpsc::Sender<()>>,
    #[cfg(unix)]
    accept: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Bind `path`, chmod 0600 on Unix, and serve `registry` to concurrent
    /// clients. Does not start an adapter consumer.
    ///
    /// If `path` already has a live server, returns [`ServerError::AlreadyRunning`]
    /// without unlinking. A stale (non-listening) path is removed and reused.
    pub fn bind(registry: Registry, path: impl AsRef<Path>) -> Result<Self, ServerError> {
        #[cfg(not(unix))]
        {
            let _ = (registry, path);
            return Err(ServerError::Unsupported);
        }
        #[cfg(unix)]
        {
            bind_unix(registry, path.as_ref())
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stop accepting, join the accept thread, and unlink the socket.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            if let Some(tx) = self.stop.take() {
                let _ = tx.send(());
            }
            if let Some(handle) = self.accept.take() {
                let _ = handle.join();
            }
            unlink_if_ours(&self.path, self.identity);
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn bind_unix(registry: Registry, path: &Path) -> Result<Server, ServerError> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::thread;

    let path = path.to_path_buf();
    if let Some(dir) = path.parent() {
        prepare_socket_parent(dir)?;
    }
    if path_has_live_server(&path) {
        return Err(ServerError::AlreadyRunning { path });
    }
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let identity = socket_identity(&path);
    listener.set_nonblocking(true)?;
    let (stop_tx, stop_rx) = mpsc::channel();
    match thread::Builder::new()
        .name("sleipnir-agent-control".into())
        .spawn(move || accept_loop(listener, stop_rx, registry))
    {
        Ok(accept) => Ok(Server {
            path,
            identity,
            stop: Some(stop_tx),
            accept: Some(accept),
        }),
        Err(err) => {
            unlink_if_ours(&path, identity);
            Err(ServerError::Io(err))
        }
    }
}

/// Tighten `…/sleipnir/` to 0700. Other parents (e.g. `/tmp` in tests) are
/// created but not chmod'd.
#[cfg(unix)]
fn prepare_socket_parent(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir)?;
    if dir.file_name().is_some_and(|name| name == "sleipnir") {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// True when connecting succeeds: a process is accepting on `path`.
#[cfg(unix)]
fn path_has_live_server(path: &Path) -> bool {
    use std::os::unix::net::UnixStream;

    UnixStream::connect(path).is_ok()
}

#[cfg(unix)]
fn socket_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;

    std::fs::metadata(path)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

#[cfg(unix)]
fn unlink_if_ours(path: &Path, identity: Option<(u64, u64)>) {
    let Some(identity) = identity else {
        return;
    };
    if socket_identity(path) == Some(identity) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
fn accept_loop(
    listener: std::os::unix::net::UnixListener,
    stop: std::sync::mpsc::Receiver<()>,
    registry: Registry,
) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    let live = Arc::new(AtomicUsize::new(0));
    let mut backoff_ms = 10u64;
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                backoff_ms = 10;
                let prev = live.fetch_add(1, Ordering::SeqCst);
                if prev >= MAX_CLIENTS {
                    live.fetch_sub(1, Ordering::SeqCst);
                    drop(stream);
                    continue;
                }
                let registry = registry.clone();
                let live = Arc::clone(&live);
                let _ = thread::Builder::new()
                    .name("sleipnir-agent-control-client".into())
                    .spawn(move || {
                        handle_connection(stream, registry);
                        live.fetch_sub(1, Ordering::SeqCst);
                    });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionReset
                ) => {}
            Err(err) => {
                eprintln!("agent-control accept error (retrying): {err}");
                thread::sleep(Duration::from_millis(backoff_ms));
                backoff_ms = (backoff_ms.saturating_mul(2)).min(500);
            }
        }
    }
}

#[cfg(unix)]
fn handle_connection(stream: std::os::unix::net::UnixStream, registry: Registry) {
    use std::io::{BufReader, Write};

    use crate::line::{BoundedRead, MAX_LINE_BYTES, drain_until_newline, read_bounded_line};
    use crate::protocol::{decode_request_line, encode_response_line};

    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_READ_TIMEOUT));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut writer = stream;
    loop {
        match read_bounded_line(&mut reader, MAX_LINE_BYTES) {
            Ok(BoundedRead::Eof) => break,
            Ok(BoundedRead::Oversize) => {
                drain_until_newline(&mut reader);
                let _ = writeln!(
                    writer,
                    "{}",
                    error_json(0, "request exceeds line length cap")
                );
                break;
            }
            Ok(BoundedRead::Line(line)) => {
                if line.trim().is_empty() {
                    continue;
                }
                let reply = match decode_request_line(&line) {
                    Ok(req) => {
                        let resp = registry.handle(req, now_ms());
                        match encode_response_line(&resp) {
                            Ok(line) if line.len() <= MAX_LINE_BYTES => line,
                            Ok(_) => error_json(
                                resp.id,
                                "response exceeds line length cap; result is retained without truncation",
                            ),
                            Err(err) => error_json(resp.id, &err),
                        }
                    }
                    Err(err) => error_json(salvage_id(&line), &format!("malformed request: {err}")),
                };
                let reply = if reply.len() > MAX_LINE_BYTES {
                    error_json(salvage_id(&line), "response exceeds line length cap")
                } else {
                    reply
                };
                if writeln!(writer, "{reply}").is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(unix)]
fn salvage_id(line: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_u64()))
        .unwrap_or(0)
}

#[cfg(unix)]
fn error_json(id: u64, message: &str) -> String {
    use crate::protocol::{Response, WireResponse, encode_response_line};

    encode_response_line(&WireResponse {
        id,
        body: Response::Error {
            message: message.to_string(),
        },
    })
    .unwrap_or_else(|_| format!(r#"{{"id":{id},"op":"error","message":"failed to encode error"}}"#))
}

#[cfg(unix)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;
    use crate::registry::Registry;

    #[cfg(unix)]
    use crate::protocol::{EffectBody, Request, Response, TaskStatus, WireRequest};
    #[cfg(unix)]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    fn test_sock() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        PathBuf::from(format!(
            "/tmp/sac-{}-{}.sock",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[cfg(unix)]
    fn abs(path: &str) -> String {
        if cfg!(windows) {
            if let Some(rest) = path.strip_prefix('/') {
                format!(r"C:\{}", rest.replace('/', "\\"))
            } else {
                path.to_string()
            }
        } else {
            path.to_string()
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn bind_reports_unsupported() {
        assert!(matches!(
            Server::bind(Registry::new(), "x").unwrap_err(),
            ServerError::Unsupported
        ));
    }

    #[test]
    fn default_path_uses_agent_control_sock_name() {
        if std::env::var("SLEIPNIR_AGENT_CONTROL_SOCKET")
            .ok()
            .filter(|s| !s.is_empty())
            .is_some()
        {
            return;
        }
        let path = default_socket_path();
        assert!(path.ends_with("agent-control.sock"), "{}", path.display());
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[cfg(unix)]
    fn req(id: u64, body: Request) -> WireRequest {
        WireRequest { id, body }
    }

    #[cfg(unix)]
    #[test]
    fn round_trip_list_and_launch_await_adapter() {
        use crate::call;

        let path = test_sock();
        let server = Server::bind(Registry::new(), &path).unwrap();
        let list = call(&path, &req(1, Request::List)).unwrap();
        assert_eq!(list.id, 1);
        match list.body {
            Response::Agents { agents, .. } => assert!(agents.is_empty()),
            other => panic!("{other:?}"),
        }
        let launched = call(
            &path,
            &req(
                2,
                Request::Launch {
                    kind: crate::protocol::AgentKind::Codex,
                    cwd: abs("/work"),
                    name: Some("w".into()),
                    args: vec![],
                },
            ),
        )
        .unwrap();
        let (session, task) = match launched.body {
            Response::LaunchAccepted { session, task } => (session, task),
            other => panic!("must accept, not execute: {other:?}"),
        };
        let waited = call(&path, &req(3, Request::Wait { task })).unwrap();
        match waited.body {
            Response::Wait {
                status, terminal, ..
            } => {
                assert_eq!(status, TaskStatus::Dispatching);
                assert!(!terminal);
            }
            other => panic!("{other:?}"),
        }
        let effects = call(&path, &req(4, Request::Effects)).unwrap();
        match effects.body {
            Response::Effects { effects, .. } => {
                assert!(
                    effects
                        .iter()
                        .any(|e| matches!(e.body, EffectBody::LaunchRequested { .. })),
                    "launch stays queued until an adapter acks"
                );
            }
            other => panic!("{other:?}"),
        }
        let inspect = call(&path, &req(5, Request::Inspect { session })).unwrap();
        assert!(matches!(inspect.body, Response::Inspect { .. }));
        assert!(matches!(
            call(&path, &req(6, Request::ReportRunning { task }))
                .unwrap()
                .body,
            Response::ReportedRunning { .. }
        ));
        let result = call(
            &path,
            &req(
                7,
                Request::ReportResult {
                    task,
                    text: "worker done".into(),
                },
            ),
        )
        .unwrap();
        assert!(matches!(result.body, Response::ReportedResult { .. }));
        match call(&path, &req(8, Request::Wait { task })).unwrap().body {
            Response::Wait {
                status,
                terminal,
                result,
                ..
            } => {
                assert_eq!(status, TaskStatus::Settled);
                assert!(terminal);
                assert_eq!(result.as_deref(), Some("worker done"));
            }
            other => panic!("{other:?}"),
        }
        server.stop();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn large_results_are_retrieved_by_wait_not_session_summaries() {
        use crate::call;
        let registry = Registry::new();
        let path = test_sock();
        let server = Server::bind(registry, &path).unwrap();
        let session = match call(
            &path,
            &req(
                1,
                Request::Launch {
                    kind: crate::AgentKind::Codex,
                    cwd: "/tmp".into(),
                    name: None,
                    args: vec![],
                },
            ),
        )
        .unwrap()
        .body
        {
            Response::LaunchAccepted { session, task } => {
                call(
                    &path,
                    &req(
                        2,
                        Request::ReportResult {
                            task,
                            text: "x".repeat(40 * 1024),
                        },
                    ),
                )
                .unwrap();
                session
            }
            other => panic!("{other:?}"),
        };
        let task = match call(
            &path,
            &req(
                3,
                Request::Prompt {
                    session,
                    text: "next".into(),
                },
            ),
        )
        .unwrap()
        .body
        {
            Response::PromptAccepted { task } => task,
            other => panic!("{other:?}"),
        };
        call(
            &path,
            &req(
                4,
                Request::ReportResult {
                    task,
                    text: "y".repeat(40 * 1024),
                },
            ),
        )
        .unwrap();
        for request in [Request::Inspect { session }, Request::List] {
            let reply = call(&path, &req(5, request))
                .expect("summaries must fit the official client's frame cap");
            let encoded = crate::encode_response_line(&reply).unwrap();
            assert!(
                !encoded.contains("yyyyyyyyyy"),
                "summary must omit the result payload"
            );
        }
        match call(&path, &req(6, Request::Wait { task })).unwrap().body {
            Response::Wait {
                result,
                next_result_offset,
                ..
            } => {
                assert_eq!(result.unwrap(), "y".repeat(40 * 1024));
                assert!(next_result_offset.is_none());
            }
            other => panic!("{other:?}"),
        }
        server.stop();
    }

    #[cfg(unix)]
    #[test]
    fn official_client_collects_all_summary_and_diagnostic_pages() {
        let registry = Registry::new();
        for i in 0..crate::registry::MAX_SESSIONS {
            registry.handle(
                req(
                    i as u64,
                    Request::Launch {
                        kind: crate::AgentKind::Codex,
                        cwd: format!("/{}", "界".repeat(1023)),
                        name: None,
                        args: vec![],
                    },
                ),
                i as u64,
            );
        }
        let path = test_sock();
        let server = Server::bind(registry, &path).unwrap();
        match crate::call(&path, &req(1, Request::List)).unwrap().body {
            Response::Agents {
                agents,
                next_offset,
            } => {
                assert_eq!(agents.len(), crate::registry::MAX_SESSIONS);
                assert!(next_offset.is_none());
            }
            other => panic!("{other:?}"),
        }
        match crate::call(&path, &req(2, Request::Effects)).unwrap().body {
            Response::Effects {
                effects,
                next_cursor,
            } => {
                assert_eq!(effects.len(), crate::registry::MAX_SESSIONS);
                assert!(next_cursor.is_none());
            }
            other => panic!("{other:?}"),
        }
        server.stop();
    }

    #[cfg(unix)]
    #[test]
    fn malformed_request_error_also_obeys_response_byte_cap() {
        use std::io::{BufReader, Write};
        use std::os::unix::net::UnixStream;
        let path = test_sock();
        let server = Server::bind(Registry::new(), &path).unwrap();
        let mut socket = UnixStream::connect(&path).unwrap();
        let request = format!(
            "{{\"id\":7,\"op\":\"{}\"}}",
            "x".repeat(crate::MAX_LINE_BYTES - 20)
        );
        assert!(request.len() <= crate::MAX_LINE_BYTES);
        writeln!(socket, "{request}").unwrap();
        match crate::line::read_bounded_line(&mut BufReader::new(socket), crate::MAX_LINE_BYTES)
            .unwrap()
        {
            crate::line::BoundedRead::Line(line) => assert!(matches!(
                crate::decode_response_line(&line).unwrap().body,
                Response::Error { .. }
            )),
            other => panic!("error itself must be bounded: {other:?}"),
        }
        server.stop();
    }

    #[cfg(unix)]
    #[test]
    fn oversized_wait_results_are_retrieved_losslessly_in_unicode_pages() {
        use crate::call;
        let registry = Registry::new();
        let task = match registry
            .handle(
                req(
                    1,
                    Request::Launch {
                        kind: crate::AgentKind::Codex,
                        cwd: "/tmp".into(),
                        name: None,
                        args: vec![],
                    },
                ),
                0,
            )
            .body
        {
            Response::LaunchAccepted { task, .. } => task,
            other => panic!("{other:?}"),
        };
        let payload = "界".repeat(30 * 1024);
        registry
            .apply(
                crate::AdapterUpdate::TaskResult {
                    task,
                    text: payload.clone(),
                },
                1,
            )
            .unwrap();
        let path = test_sock();
        let server = Server::bind(registry.clone(), &path).unwrap();
        match call(&path, &req(7, Request::Wait { task }))
            .expect("paged wait response")
            .body
        {
            Response::Wait {
                result,
                detail,
                next_result_offset,
                ..
            } => {
                assert_eq!(result, Some(payload));
                assert!(detail.is_none());
                assert!(next_result_offset.is_none());
            }
            other => panic!("{other:?}"),
        }
        match registry.handle(req(8, Request::Wait { task }), 2).body {
            Response::Wait {
                result,
                next_result_offset,
                ..
            } => {
                assert!(result.is_some());
                assert!(next_result_offset.is_some());
            }
            other => panic!("{other:?}"),
        }
        server.stop();
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_clients_share_the_registry() {
        use crate::call;
        use std::thread;

        let path = test_sock();
        let server = Server::bind(Registry::new(), &path).unwrap();
        let launched = call(
            &path,
            &req(
                1,
                Request::Launch {
                    kind: crate::protocol::AgentKind::Claude,
                    cwd: abs("/tmp"),
                    name: None,
                    args: vec![],
                },
            ),
        )
        .unwrap();
        assert!(matches!(launched.body, Response::LaunchAccepted { .. }));
        let mut handles = Vec::new();
        for i in 0..8 {
            let path = path.clone();
            handles.push(thread::spawn(move || {
                let resp = call(&path, &req(10 + i, Request::List)).unwrap();
                match resp.body {
                    Response::Agents { agents, .. } => assert_eq!(agents.len(), 1),
                    other => panic!("{other:?}"),
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        drop(server);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn oversize_and_malformed_requests_return_errors() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        use crate::line::MAX_LINE_BYTES;
        use crate::protocol::decode_response_line;

        let path = test_sock();
        let server = Server::bind(Registry::new(), &path).unwrap();

        let mut oversize = UnixStream::connect(&path).unwrap();
        let mut huge = vec![b'x'; MAX_LINE_BYTES + 8];
        huge.push(b'\n');
        let _ = oversize.write_all(&huge);
        let _ = oversize.flush();
        let mut reply = String::new();
        BufReader::new(oversize).read_line(&mut reply).unwrap();
        let resp = decode_response_line(&reply).unwrap();
        match resp.body {
            Response::Error { message } => {
                assert!(message.contains("line length"), "{message}");
            }
            other => panic!("{other:?}"),
        }

        let mut bad = UnixStream::connect(&path).unwrap();
        writeln!(bad, r#"{{"id":7,"op":"not_a_real_op"}}"#).unwrap();
        let mut reply = String::new();
        BufReader::new(bad).read_line(&mut reply).unwrap();
        let resp = decode_response_line(&reply).unwrap();
        assert_eq!(resp.id, 7);
        match resp.body {
            Response::Error { message } => {
                assert!(message.contains("malformed"), "{message}");
            }
            other => panic!("{other:?}"),
        }

        let mut garbage = UnixStream::connect(&path).unwrap();
        writeln!(garbage, "not json").unwrap();
        let mut reply = String::new();
        BufReader::new(garbage).read_line(&mut reply).unwrap();
        let resp = decode_response_line(&reply).unwrap();
        match resp.body {
            Response::Error { message } => {
                assert!(message.contains("malformed"), "{message}");
            }
            other => panic!("{other:?}"),
        }

        server.stop();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn socket_is_mode_600() {
        use std::os::unix::fs::PermissionsExt;

        let path = test_sock();
        let server = Server::bind(Registry::new(), &path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        drop(server);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn live_socket_is_not_hijacked() {
        use crate::call;

        let path = test_sock();
        let first = Server::bind(Registry::new(), &path).unwrap();
        match Server::bind(Registry::new(), &path) {
            Err(ServerError::AlreadyRunning { path: p }) => assert_eq!(p, path),
            Err(err) => panic!("expected AlreadyRunning, got {err}"),
            Ok(_) => panic!("expected AlreadyRunning, got a second listener"),
        }
        let list = call(&path, &req(1, Request::List)).unwrap();
        assert_eq!(list.id, 1);
        first.stop();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_path_is_reused() {
        use std::os::unix::net::UnixListener;

        use crate::call;

        let path = test_sock();
        let leftover = UnixListener::bind(&path).unwrap();
        drop(leftover);
        assert!(path.exists(), "dropping the listener leaves the path");
        let server = Server::bind(Registry::new(), &path).unwrap();
        let list = call(&path, &req(1, Request::List)).unwrap();
        assert_eq!(list.id, 1);
        server.stop();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn drop_does_not_unlink_a_replacement_socket() {
        use crate::call;

        let path = test_sock();
        let first = Server::bind(Registry::new(), &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let second = Server::bind(Registry::new(), &path).unwrap();
        drop(first);
        assert!(
            path.exists(),
            "first server must not unlink the replacement"
        );
        let list = call(&path, &req(1, Request::List)).unwrap();
        assert_eq!(list.id, 1);
        second.stop();
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn sleipnir_parent_dir_is_mode_700() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir()
            .join(format!("sac-parent-{}", std::process::id()))
            .join("sleipnir");
        let path = dir.join("agent-control.sock");
        let server = Server::bind(Registry::new(), &path).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        drop(server);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn extra_clients_beyond_the_cap_are_dropped() {
        use std::os::unix::net::UnixStream;
        use std::thread;
        use std::time::Duration;

        use crate::call;

        let path = test_sock();
        let server = Server::bind(Registry::new(), &path).unwrap();
        let mut held = Vec::new();
        for _ in 0..MAX_CLIENTS {
            held.push(UnixStream::connect(&path).unwrap());
        }
        thread::sleep(Duration::from_millis(50));
        let extra = call(&path, &req(1, Request::List));
        assert!(
            extra.is_err(),
            "over-cap client must not be served: {extra:?}"
        );
        drop(held);
        thread::sleep(Duration::from_millis(30));
        let list = call(&path, &req(2, Request::List)).unwrap();
        assert_eq!(list.id, 2);
        server.stop();
    }
}
