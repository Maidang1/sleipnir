//! Server half of the SSH_ASKPASS protocol.
//!
//! One [`AskpassServer`] is created per SSH pane. It owns a private Unix
//! socket; every time `ssh` needs a secret it runs the generated askpass
//! script, which connects to that socket, sends the prompt, and prints back
//! whatever the user typed into the Sleipnir prompt.
//!
//! The listener runs on a plain OS thread (it must stay alive while `ssh`
//! blocks) and hands requests to GPUI through an `async_channel`. Answers come
//! back over a per-request `std::sync::mpsc` channel so a connection thread can
//! wait with a timeout.

use sleipnir_ssh::askpass::{
    ASKPASS_ENV, ASKPASS_REQUIRE_ENV, ASKPASS_SOCKET_ENV, AskpassMessage, MAX_MESSAGE_BYTES,
    askpass_script_path, ensure_askpass_script, socket_dir, socket_file_name,
};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::time::Duration;

/// How long a connection waits for the user to answer.
const ANSWER_GRACE: Duration = Duration::from_secs(600);
/// Per-connection I/O timeout, matching the control surface.
const CONNECTION_IO_TIMEOUT: Duration = Duration::from_secs(30);
/// Accept-loop poll interval while idle.
const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// One pending askpass request, routed to the shell that owns the pane.
pub struct AskpassRequest {
    /// The prompt text `ssh` asked for (e.g. `user@host's password: `).
    pub prompt: String,
    /// Answer channel. `Some(secret)` authenticates, `None` cancels.
    pub reply: std::sync::mpsc::SyncSender<Option<String>>,
}

/// Owns the askpass socket for one SSH pane.
///
/// On non-Unix platforms this is an inert stub: OpenSSH there has no
/// `SSH_ASKPASS`, so no listener is ever created and the terminal stays the
/// input surface.
pub struct AskpassServer {
    inner: Option<Inner>,
}

struct Inner {
    socket_path: PathBuf,
    /// Device + inode, so the socket is only unlinked if it is still ours.
    identity: (u64, u64),
    stop: async_channel::Sender<()>,
    requests: async_channel::Receiver<AskpassRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AskpassServer {
    /// Bind a fresh askpass socket and start accepting connections.
    ///
    /// Returns `None` on non-Unix platforms and on any setup failure; callers
    /// degrade to terminal-typed input rather than failing the connection.
    pub fn bind() -> Option<Self> {
        Inner::bind().map(|inner| Self { inner: Some(inner) })
    }

    /// Stream of askpass requests. Callers drain this on the GPUI thread.
    pub fn requests(&self) -> Option<async_channel::Receiver<AskpassRequest>> {
        self.inner.as_ref().map(|inner| inner.requests.clone())
    }

    /// Path of the socket this server listens on, for diagnostics.
    pub fn socket_path(&self) -> Option<&Path> {
        self.inner.as_ref().map(|inner| inner.socket_path.as_path())
    }
}

impl Drop for AskpassServer {
    fn drop(&mut self) {
        let Some(mut inner) = self.inner.take() else {
            return;
        };
        let _ = inner.stop.try_send(());
        if let Some(thread) = inner.thread.take() {
            let _ = thread.join();
        }
        remove_socket_if_matches(&inner.socket_path, inner.identity);
    }
}

impl Inner {
    #[cfg(unix)]
    fn bind() -> Option<Self> {
        use std::os::unix::fs::PermissionsExt as _;
        use std::os::unix::net::UnixListener;

        let dir = socket_dir();
        if let Err(err) = std::fs::create_dir_all(&dir) {
            log::warn!("ssh askpass: cannot create {}: {err}", dir.display());
            return None;
        }
        let permissions = std::fs::metadata(&dir).ok()?.permissions();
        if permissions.mode() & 0o777 != 0o700 {
            let mut tightened = permissions;
            tightened.set_mode(0o700);
            let _ = std::fs::set_permissions(&dir, tightened);
        }

        let socket_path = dir.join(socket_file_name());
        let listener = match UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            Err(err) => {
                log::warn!("ssh askpass: cannot bind {}: {err}", socket_path.display());
                return None;
            }
        };
        let mut permissions = std::fs::metadata(&socket_path).ok()?.permissions();
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(&socket_path, permissions);
        let metadata = std::fs::metadata(&socket_path).ok()?;
        use std::os::unix::fs::MetadataExt as _;
        let identity = (metadata.dev(), metadata.ino());

        let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
        let (requests_tx, requests_rx) = async_channel::unbounded::<AskpassRequest>();
        let thread = std::thread::Builder::new()
            .name("sleipnir-ssh-askpass".into())
            .spawn(move || accept_loop(listener, stop_rx, requests_tx))
            .ok()?;

        Some(Self {
            socket_path,
            identity,
            stop: stop_tx,
            requests: requests_rx,
            thread: Some(thread),
        })
    }

    #[cfg(not(unix))]
    fn bind() -> Option<Self> {
        None
    }
}

#[cfg(unix)]
fn accept_loop(
    listener: std::os::unix::net::UnixListener,
    stop: async_channel::Receiver<()>,
    requests: async_channel::Sender<AskpassRequest>,
) {
    if let Err(err) = listener.set_nonblocking(true) {
        log::warn!("ssh askpass: cannot make the listener non-blocking: {err}");
        return;
    }
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let requests = requests.clone();
                std::thread::Builder::new()
                    .name("sleipnir-ssh-askpass-conn".into())
                    .spawn(move || serve_connection(stream, requests))
                    .ok();
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(err) => {
                log::debug!("ssh askpass: accept failed: {err}");
                std::thread::sleep(ACCEPT_POLL);
            }
        }
    }
}

#[cfg(unix)]
fn serve_connection(
    mut stream: std::os::unix::net::UnixStream,
    requests: async_channel::Sender<AskpassRequest>,
) {
    if let Err(err) = stream.set_read_timeout(Some(CONNECTION_IO_TIMEOUT)) {
        log::debug!("ssh askpass: cannot set the read timeout: {err}");
    }
    let Some(message) = read_request(&stream) else {
        return;
    };
    let AskpassMessage::Request { prompt } = message else {
        return;
    };

    // The prompt may echo the remote user; log its length only, never content.
    log::debug!("ssh askpass: prompt of {} chars", prompt.chars().count());
    let (reply_tx, reply_rx) = sync_channel::<Option<String>>(1);
    if requests
        .try_send(AskpassRequest {
            prompt,
            reply: reply_tx,
        })
        .is_err()
    {
        let _ = stream.write_all(AskpassMessage::Cancel.to_line().as_bytes());
        let _ = stream.flush();
        return;
    }

    let answer = reply_rx.recv_timeout(ANSWER_GRACE).unwrap_or(None);
    let message = match answer {
        Some(secret) => AskpassMessage::Secret { value: secret },
        None => AskpassMessage::Cancel,
    };
    let _ = stream.write_all(message.to_line().as_bytes());
    let _ = stream.flush();
}

#[cfg(unix)]
fn read_request(stream: &std::os::unix::net::UnixStream) -> Option<AskpassMessage> {
    use std::io::Read as _;

    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                if line.len() >= MAX_MESSAGE_BYTES {
                    return None;
                }
                line.push(byte[0]);
            }
            Err(_) => return None,
        }
    }
    if line.is_empty() {
        return None;
    }
    AskpassMessage::from_line(&String::from_utf8_lossy(&line))
}

/// Unlink a socket only when it is still the file we created.
#[cfg(unix)]
fn remove_socket_if_matches(path: &Path, identity: (u64, u64)) {
    use std::os::unix::fs::MetadataExt as _;

    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if (metadata.dev(), metadata.ino()) != identity {
        return;
    }
    if let Err(err) = std::fs::remove_file(path) {
        log::debug!("ssh askpass: cannot remove {}: {err}", path.display());
    }
}

/// Environment a spawned `ssh` needs to route password prompts to Sleipnir.
pub(crate) fn askpass_env(socket_path: &Path) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();
    env.insert(
        ASKPASS_ENV.to_string(),
        askpass_script_path().to_string_lossy().into_owned(),
    );
    env.insert(ASKPASS_REQUIRE_ENV.to_string(), "force".to_string());
    env.insert(
        ASKPASS_SOCKET_ENV.to_string(),
        socket_path.to_string_lossy().into_owned(),
    );
    env
}

/// Make sure the askpass script points at the running binary.
///
/// Returns `false` when the script could not be written; the caller then falls
/// back to letting the user type the password inside the terminal.
pub(crate) fn ensure_askpass_helper(exe: &Path) -> bool {
    match ensure_askpass_script(exe) {
        Ok(path) => {
            log::debug!("ssh askpass helper ready at {}", path.display());
            true
        }
        Err(err) => {
            log::warn!("ssh askpass: cannot write the helper script: {err:#}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead as _;

    #[test]
    fn askpass_env_points_at_the_generated_helper() {
        let env = askpass_env(Path::new("/tmp/example.sock"));
        assert_eq!(
            env.get(ASKPASS_ENV).map(String::as_str),
            Some(askpass_script_path().to_string_lossy().as_ref())
        );
        assert_eq!(
            env.get(ASKPASS_REQUIRE_ENV).map(String::as_str),
            Some("force")
        );
        assert_eq!(
            env.get(ASKPASS_SOCKET_ENV).map(String::as_str),
            Some("/tmp/example.sock")
        );
    }

    #[cfg(unix)]
    #[test]
    fn server_round_trips_one_request() {
        let server = AskpassServer::bind().expect("bind askpass socket");
        let socket = server.socket_path().expect("socket path").to_path_buf();
        let requests = server.requests().expect("request stream");

        let client = std::thread::spawn(move || {
            let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
            stream
                .write_all(
                    AskpassMessage::Request {
                        prompt: "Password: ".into(),
                    }
                    .to_line()
                    .as_bytes(),
                )
                .unwrap();
            stream.flush().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream)
                .read_line(&mut line)
                .unwrap();
            line
        });

        let request = requests.recv_blocking().expect("askpass request");
        assert_eq!(request.prompt, "Password: ");
        request.reply.send(Some("hunter2".into())).unwrap();

        let line = client.join().unwrap();
        assert_eq!(
            AskpassMessage::from_line(&line),
            Some(AskpassMessage::Secret {
                value: "hunter2".into()
            })
        );
        drop(server);
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_answers_with_cancel() {
        let server = AskpassServer::bind().expect("bind askpass socket");
        let socket = server.socket_path().expect("socket path").to_path_buf();
        let requests = server.requests().expect("request stream");

        let client = std::thread::spawn(move || {
            let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
            stream
                .write_all(
                    AskpassMessage::Request {
                        prompt: "Password: ".into(),
                    }
                    .to_line()
                    .as_bytes(),
                )
                .unwrap();
            stream.flush().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream)
                .read_line(&mut line)
                .unwrap();
            line
        });

        let request = requests.recv_blocking().expect("askpass request");
        request.reply.send(None).unwrap();

        let line = client.join().unwrap();
        assert_eq!(
            AskpassMessage::from_line(&line),
            Some(AskpassMessage::Cancel)
        );
        drop(server);
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_server_removes_the_socket() {
        let server = AskpassServer::bind().expect("bind askpass socket");
        let socket = server.socket_path().expect("socket path").to_path_buf();
        assert!(socket.exists());
        drop(server);
        assert!(!socket.exists(), "socket file should be cleaned up");
    }

    #[cfg(not(unix))]
    #[test]
    fn non_unix_builds_have_no_server() {
        assert!(AskpassServer::bind().is_none());
        assert!(AskpassServer::bind().is_none());
    }
}
