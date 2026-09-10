//! In-process Unix socket listener for ADR-0011 (`sleipnir-ctl`).

use gpui::{App, Global, Task};
use sleipnir_ctl::enabled;
use sleipnir_settings::TerminalSettings;
use std::io;
use std::sync::mpsc;

use crate::TermView;
use crate::app_shell::AppShell;
use run_ledger::PaneKey;

#[cfg(unix)]
use crate::run_ledger_global::RunLedgerGlobal;
#[cfg(unix)]
use gpui::AsyncApp;
#[cfg(unix)]
use sleipnir_ctl::{
    ControlRequest, ControlResponse, PaneSnap, WaitUntil, socket_path, wait_matches,
};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::os::unix::{fs::FileTypeExt, fs::MetadataExt};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
struct Job {
    req: ControlRequest,
    reply: mpsc::Sender<ControlResponse>,
}

#[cfg(unix)]
const MAX_PENDING_WAITS: usize = 64;
#[cfg(unix)]
const CONNECTION_IO_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const MAX_REQUEST_LINE_BYTES: usize = 1024 * 1024;

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SocketIdentity {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
#[derive(Debug)]
struct ListenerHandle {
    stop: mpsc::Sender<()>,
    join: std::thread::JoinHandle<()>,
}

#[cfg(unix)]
impl ListenerHandle {
    fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.join.join();
    }
}

pub struct ControlSurface {
    #[cfg(unix)]
    listener: Option<ListenerHandle>,
    _pump: Task<()>,
}

impl Global for ControlSurface {}

pub fn init(cx: &mut App) {
    if !cx.has_global::<ControlSurface>() {
        cx.set_global(ControlSurface {
            #[cfg(unix)]
            listener: None,
            _pump: Task::ready(()),
        });
    }
    reload(cx);
}

pub fn reload(cx: &mut App) {
    if !cx.has_global::<ControlSurface>() {
        init(cx);
        return;
    }
    let want = enabled(TerminalSettings::get_global(cx).control_surface);
    #[cfg(unix)]
    let running = cx.global::<ControlSurface>().listener.is_some();
    #[cfg(not(unix))]
    let running = false;
    if want && !running {
        start(cx);
    } else if !want && running {
        stop(cx);
    }
}

fn start(cx: &mut App) {
    #[cfg(not(unix))]
    {
        let _ = cx;
        log::warn!("control surface is not supported on this platform");
        return;
    }
    #[cfg(unix)]
    start_unix(cx);
}

#[cfg(unix)]
fn start_unix(cx: &mut App) {
    let path = socket_path();
    let (job_tx, job_rx) = async_channel::unbounded::<Job>();
    let listener = match spawn_listener(&path, job_tx) {
        Ok(listener) => listener,
        Err(err) => {
            log::warn!("control surface bind failed ({}): {err}", path.display());
            return;
        }
    };
    let pump = cx.spawn(async move |cx| pump_jobs(cx, job_rx).await);
    let g = cx.global_mut::<ControlSurface>();
    g.listener = Some(listener);
    g._pump = pump;
    log::info!("control surface listening on {}", path.display());
}

fn stop(cx: &mut App) {
    #[cfg(unix)]
    let listener = {
        let g = cx.global_mut::<ControlSurface>();
        let listener = g.listener.take();
        g._pump = Task::ready(());
        listener
    };
    #[cfg(not(unix))]
    {
        cx.global_mut::<ControlSurface>()._pump = Task::ready(());
    }
    #[cfg(unix)]
    if let Some(listener) = listener {
        listener.stop();
    }
}

#[cfg(unix)]
fn accept_loop(
    listener: UnixListener,
    stop: mpsc::Receiver<()>,
    jobs: async_channel::Sender<Job>,
    path: std::path::PathBuf,
    identity: SocketIdentity,
) {
    loop {
        match stop.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let jobs = jobs.clone();
                let _ = std::thread::Builder::new()
                    .name("sleipnir-ctl-conn".into())
                    .spawn(move || handle_connection(stream, jobs));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    let _ = remove_socket_if_matches(&path, Some(identity));
}

#[cfg(unix)]
fn handle_connection(mut stream: UnixStream, jobs: async_channel::Sender<Job>) {
    let _ = stream.set_read_timeout(Some(CONNECTION_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONNECTION_IO_TIMEOUT));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    loop {
        let line = match read_capped_line(&mut reader, MAX_REQUEST_LINE_BYTES) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(ReadLineError::TooLong) => {
                let _ = write_json_line(
                    &mut stream,
                    &ControlResponse::Error {
                        message: format!(
                            "bad request: line exceeds {MAX_REQUEST_LINE_BYTES} bytes"
                        ),
                    },
                );
                break;
            }
            Err(ReadLineError::Io(err)) if err.kind() == io::ErrorKind::TimedOut => break,
            Err(ReadLineError::Io(_)) => break,
            Err(ReadLineError::InvalidUtf8(err)) => {
                let _ = write_json_line(
                    &mut stream,
                    &ControlResponse::Error {
                        message: format!("bad request: {err}"),
                    },
                );
                continue;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req = match serde_json::from_str::<ControlRequest>(trimmed) {
            Ok(req) => req,
            Err(err) => {
                let _ = write_json_line(
                    &mut stream,
                    &ControlResponse::Error {
                        message: format!("bad request: {err}"),
                    },
                );
                continue;
            }
        };
        let timeout = match &req {
            ControlRequest::Wait { timeout_secs, .. } => timeout_secs.saturating_add(5),
            _ => 30,
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        if jobs
            .send_blocking(Job {
                req,
                reply: reply_tx,
            })
            .is_err()
        {
            break;
        }
        let resp = reply_rx
            .recv_timeout(Duration::from_secs(timeout))
            .unwrap_or(ControlResponse::Error {
                message: "timeout".into(),
            });
        if write_json_line(&mut stream, &resp).is_err() {
            break;
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
enum ReadLineError {
    Io(io::Error),
    InvalidUtf8(std::string::FromUtf8Error),
    TooLong,
}

#[cfg(unix)]
fn read_capped_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, ReadLineError> {
    let mut bytes = Vec::new();
    let read = reader
        .by_ref()
        .take((max_bytes + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(ReadLineError::Io)?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > max_bytes {
        return Err(ReadLineError::TooLong);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(ReadLineError::InvalidUtf8)
}

#[cfg(unix)]
fn write_json_line(stream: &mut UnixStream, response: &ControlResponse) -> io::Result<()> {
    writeln!(
        stream,
        "{}",
        serde_json::to_string(response).unwrap_or_default()
    )
}

#[cfg(unix)]
fn spawn_listener(
    path: &std::path::Path,
    jobs: async_channel::Sender<Job>,
) -> io::Result<ListenerHandle> {
    prepare_socket_path(path)?;
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    listener.set_nonblocking(true)?;
    let identity = socket_identity(path)?;
    let (stop_tx, stop_rx) = mpsc::channel();
    let listener_path = path.to_path_buf();
    let join = match std::thread::Builder::new()
        .name("sleipnir-ctl".into())
        .spawn(move || accept_loop(listener, stop_rx, jobs, listener_path, identity))
    {
        Ok(join) => join,
        Err(err) => {
            let _ = remove_socket_if_matches(path, Some(identity));
            return Err(err);
        }
    };
    Ok(ListenerHandle {
        stop: stop_tx,
        join,
    })
}

#[cfg(unix)]
fn prepare_socket_path(path: &std::path::Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "refusing to replace non-socket control surface path: {}",
                path.display()
            ),
        ));
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("control surface already active at {}", path.display()),
        )),
        Err(err) if stale_socket_connect_error(&err) => {
            remove_socket_if_matches(path, None)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn stale_socket_connect_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound | io::ErrorKind::ConnectionReset
    )
}

#[cfg(unix)]
fn socket_identity(path: &std::path::Path) -> io::Result<SocketIdentity> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(SocketIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
    })
}

#[cfg(unix)]
fn remove_socket_if_matches(
    path: &std::path::Path,
    expected: Option<SocketIdentity>,
) -> io::Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if !metadata.file_type().is_socket() {
        return Ok(false);
    }
    if let Some(expected) = expected {
        let actual = SocketIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        };
        if actual != expected {
            return Ok(false);
        }
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

#[cfg(unix)]
async fn pump_jobs(cx: &mut AsyncApp, jobs: async_channel::Receiver<Job>) {
    serve_jobs(jobs, |req| {
        let mut cx = cx.clone();
        async move {
            match req {
                ControlRequest::Wait {
                    pane,
                    until,
                    timeout_secs,
                } => wait_until(&mut cx, pane, until, timeout_secs).await,
                other => cx.update(|cx| dispatch(other, cx)),
            }
        }
    })
    .await;
}

/// The same request queue is used by every socket connection. Keep its
/// scheduling independent of GPUI so concurrent clients can be tested without
/// a native window or a running shell.
#[cfg(unix)]
async fn serve_jobs<F, R>(jobs: async_channel::Receiver<Job>, mut respond: F)
where
    F: FnMut(ControlRequest) -> R,
    R: std::future::Future<Output = ControlResponse>,
{
    use futures::{FutureExt as _, StreamExt as _, stream::FuturesUnordered};

    // Owned by the pump, not detached: disabling the control surface drops
    // every pending wait along with the receiver. Ordinary requests remain
    // ordered, but never queue behind a wait from another connection.
    let mut waits = FuturesUnordered::new();
    loop {
        futures::select_biased! {
            _ = waits.select_next_some() => {},
            job = jobs.recv().fuse() => {
                let Ok(job) = job else { break };
                if matches!(job.req, ControlRequest::Wait { .. }) {
                    if waits.len() >= MAX_PENDING_WAITS {
                        let _ = job.reply.send(ControlResponse::Error {
                            message: "too many pending wait requests".into(),
                        });
                        continue;
                    }
                    let response = respond(job.req);
                    waits.push(async move {
                        let _ = job.reply.send(response.await);
                    });
                } else {
                    let _ = job.reply.send(respond(job.req).await);
                }
            }
        }
    }
}

#[cfg(unix)]
async fn wait_until(
    cx: &mut AsyncApp,
    pane: PaneKey,
    until: WaitUntil,
    timeout_secs: u64,
) -> ControlResponse {
    let start = Instant::now();
    loop {
        match cx.update(|cx| wait_status(pane, until, cx)) {
            Ok(true) => return ControlResponse::Wait,
            Ok(false) => {}
            Err(message) => return ControlResponse::Error { message },
        }
        if start.elapsed().as_secs() >= timeout_secs {
            return ControlResponse::Error {
                message: "timeout".into(),
            };
        }
        cx.background_executor()
            .timer(Duration::from_millis(50))
            .await;
    }
}

#[cfg(unix)]
fn dispatch(req: ControlRequest, cx: &mut App) -> ControlResponse {
    match req {
        ControlRequest::Ls => ControlResponse::Ls {
            panes: list_panes(cx),
        },
        ControlRequest::Capture { pane } => match view_for_pane(cx, pane) {
            Some(view) => ControlResponse::Capture {
                text: view.read(cx).visible_screen_text(cx),
            },
            None => ControlResponse::Error {
                message: format!("pane {pane} not found"),
            },
        },
        ControlRequest::Send { pane, text, enter } => match view_for_pane(cx, pane) {
            Some(view) => {
                let mut bytes = text.into_bytes();
                if enter {
                    bytes.push(b'\r');
                }
                view.update(cx, |v, cx| v.input_bytes(bytes, cx));
                ControlResponse::Send
            }
            None => ControlResponse::Error {
                message: format!("pane {pane} not found"),
            },
        },
        ControlRequest::Wait { .. } => ControlResponse::Error {
            message: "wait handled asynchronously".into(),
        },
    }
}

#[cfg(unix)]
fn wait_status(pane: PaneKey, until: WaitUntil, cx: &mut App) -> Result<bool, String> {
    let Some(view) = view_for_pane(cx, pane) else {
        return Err(format!("pane {pane} not found"));
    };
    let busy = view.read(cx).looks_busy(cx);
    let (failed, attention) = if cx.has_global::<RunLedgerGlobal>() {
        let g = cx.global::<RunLedgerGlobal>();
        (
            g.pane_has_failed_attention(pane),
            g.pane_has_attention(pane),
        )
    } else {
        (false, false)
    };
    Ok(wait_matches(until, busy, failed, attention))
}

#[cfg(unix)]
fn list_panes(cx: &mut App) -> Vec<PaneSnap> {
    collect_live_panes(cx)
        .into_iter()
        .map(|(pane, view)| PaneSnap {
            pane,
            cwd: view
                .read(cx)
                .working_directory(cx)
                .map(|p| p.to_string_lossy().into_owned()),
            busy: view.read(cx).looks_busy(cx),
            title: Some(view.read(cx).title().to_string()),
        })
        .collect()
}

#[cfg(unix)]
fn view_for_pane(cx: &mut App, pane: PaneKey) -> Option<gpui::Entity<TermView>> {
    live_terminal_panes(cx)
        .into_iter()
        .find(|(key, _)| *key == pane)
        .map(|(_, view)| view)
}

/// Terminal (PTY) panes across every window. Plugin Panel leaves are already
/// excluded by [`AppShell::all_live_panes`]. Host calls and `sleipnir-ctl ls`
/// share this walk so they cannot drift into two enumerations.
pub(crate) fn live_terminal_panes(cx: &mut App) -> Vec<(PaneKey, gpui::Entity<TermView>)> {
    let mut out = Vec::new();
    for handle in cx.windows() {
        let Some(handle) = handle.downcast::<AppShell>() else {
            continue;
        };
        let Ok(panes) = handle.update(cx, |shell, _window, _cx| shell.all_live_panes()) else {
            continue;
        };
        out.extend(panes);
    }
    out
}

#[cfg(unix)]
fn collect_live_panes(cx: &mut App) -> Vec<(PaneKey, gpui::Entity<TermView>)> {
    live_terminal_panes(cx)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures::FutureExt as _;
    use std::pin::pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn enqueue(
        jobs: &async_channel::Sender<Job>,
        req: ControlRequest,
    ) -> mpsc::Receiver<ControlResponse> {
        let (reply, receiver) = mpsc::channel();
        jobs.try_send(Job { req, reply }).unwrap();
        receiver
    }

    fn temp_socket_path(name: &str) -> std::path::PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        std::path::PathBuf::from("/tmp").join(format!(
            "slctl-{name}-{}-{unique}.sock",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn cleanup_test_path(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }

    #[test]
    fn waiting_client_does_not_block_send_capture_or_list() {
        let (jobs, receiver) = async_channel::unbounded();
        let (finished, completion) = async_channel::bounded(1);
        let pane = PaneKey::new_v4();
        let wait = enqueue(
            &jobs,
            ControlRequest::Wait {
                pane,
                until: WaitUntil::Free,
                timeout_secs: 60,
            },
        );
        let mut pump = pin!(serve_jobs(receiver, |req| {
            let completion = completion.clone();
            async move {
                match req {
                    ControlRequest::Wait { .. } => {
                        completion.recv().await.unwrap();
                        ControlResponse::Wait
                    }
                    ControlRequest::Send { .. } => ControlResponse::Send,
                    ControlRequest::Capture { .. } => ControlResponse::Capture {
                        text: "still responsive".into(),
                    },
                    ControlRequest::Ls => ControlResponse::Ls { panes: vec![] },
                }
            }
        }));
        assert!(pump.as_mut().now_or_never().is_none());

        let send = enqueue(
            &jobs,
            ControlRequest::Send {
                pane,
                text: "exit".into(),
                enter: true,
            },
        );
        let capture = enqueue(&jobs, ControlRequest::Capture { pane });
        let list = enqueue(&jobs, ControlRequest::Ls);
        assert!(pump.as_mut().now_or_never().is_none());
        assert_eq!(send.try_recv(), Ok(ControlResponse::Send));
        assert_eq!(
            capture.try_recv(),
            Ok(ControlResponse::Capture {
                text: "still responsive".into(),
            })
        );
        assert_eq!(list.try_recv(), Ok(ControlResponse::Ls { panes: vec![] }));
        assert_eq!(wait.try_recv(), Err(mpsc::TryRecvError::Empty));

        finished.try_send(()).unwrap();
        assert!(pump.as_mut().now_or_never().is_none());
        assert_eq!(wait.try_recv(), Ok(ControlResponse::Wait));
        drop(jobs);
        assert!(pump.as_mut().now_or_never().is_some());
    }

    #[test]
    fn waits_complete_independently_and_preserve_error_replies() {
        let (jobs, receiver) = async_channel::unbounded();
        let (complete, completion) = async_channel::bounded(1);
        let pane = PaneKey::new_v4();
        let first = enqueue(
            &jobs,
            ControlRequest::Wait {
                pane,
                until: WaitUntil::Free,
                timeout_secs: 60,
            },
        );
        let mut pump = pin!(serve_jobs(receiver, |req| {
            let completion = completion.clone();
            async move {
                if matches!(
                    req,
                    ControlRequest::Wait {
                        timeout_secs: 0,
                        ..
                    }
                ) {
                    return ControlResponse::Error {
                        message: "timeout".into(),
                    };
                }
                completion.recv().await.unwrap();
                ControlResponse::Wait
            }
        }));
        assert!(pump.as_mut().now_or_never().is_none());
        let second = enqueue(
            &jobs,
            ControlRequest::Wait {
                pane,
                until: WaitUntil::Failed,
                timeout_secs: 0,
            },
        );
        assert!(pump.as_mut().now_or_never().is_none());
        assert_eq!(
            second.try_recv(),
            Ok(ControlResponse::Error {
                message: "timeout".into()
            })
        );
        assert_eq!(first.try_recv(), Err(mpsc::TryRecvError::Empty));
        complete.try_send(()).unwrap();
        assert!(pump.as_mut().now_or_never().is_none());
        assert_eq!(first.try_recv(), Ok(ControlResponse::Wait));
    }

    #[test]
    fn wait_limit_does_not_block_ordinary_requests_and_shutdown_cancels_waits() {
        let (jobs, receiver) = async_channel::unbounded();
        let mut pending = Vec::new();
        for _ in 0..MAX_PENDING_WAITS {
            pending.push(enqueue(
                &jobs,
                ControlRequest::Wait {
                    pane: PaneKey::new_v4(),
                    until: WaitUntil::Free,
                    timeout_secs: 60,
                },
            ));
        }
        let mut pump = Box::pin(serve_jobs(receiver, |req| async move {
            match req {
                ControlRequest::Wait { .. } => std::future::pending().await,
                _ => ControlResponse::Ls { panes: vec![] },
            }
        }));
        assert!(pump.as_mut().now_or_never().is_none());
        let overflow = enqueue(
            &jobs,
            ControlRequest::Wait {
                pane: PaneKey::new_v4(),
                until: WaitUntil::Free,
                timeout_secs: 60,
            },
        );
        let list = enqueue(&jobs, ControlRequest::Ls);
        assert!(pump.as_mut().now_or_never().is_none());
        assert_eq!(
            overflow.try_recv(),
            Ok(ControlResponse::Error {
                message: "too many pending wait requests".into(),
            })
        );
        assert_eq!(list.try_recv(), Ok(ControlResponse::Ls { panes: vec![] }));
        drop(pump);
        assert!(jobs.is_closed());
        for reply in pending {
            assert_eq!(reply.try_recv(), Err(mpsc::TryRecvError::Disconnected));
        }
    }

    #[test]
    fn separate_socket_clients_can_send_while_one_waits() {
        let (jobs, receiver) = async_channel::unbounded();
        let (started, wait_started) = mpsc::channel();
        let (finished, completion) = async_channel::bounded(1);
        let pane = PaneKey::new_v4();
        let (mut waiting_client, waiting_host) = UnixStream::pair().unwrap();
        let (mut sending_client, sending_host) = UnixStream::pair().unwrap();
        // Timeouts are failure guards only; channels establish all ordering.
        for stream in [&waiting_client, &sending_client] {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
        }
        let waiting_jobs = jobs.clone();
        let waiting_server =
            std::thread::spawn(move || handle_connection(waiting_host, waiting_jobs));
        let sending_server = std::thread::spawn(move || handle_connection(sending_host, jobs));
        let waiting = std::thread::spawn(move || {
            let request = ControlRequest::Wait {
                pane,
                until: WaitUntil::Free,
                timeout_secs: 60,
            };
            writeln!(
                waiting_client,
                "{}",
                serde_json::to_string(&request).unwrap()
            )
            .unwrap();
            let mut line = String::new();
            BufReader::new(waiting_client).read_line(&mut line).unwrap();
            assert_eq!(
                serde_json::from_str::<ControlResponse>(&line).unwrap(),
                ControlResponse::Wait
            );
        });
        let sending = std::thread::spawn(move || {
            wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
            let request = ControlRequest::Send {
                pane,
                text: "exit".into(),
                enter: true,
            };
            writeln!(
                sending_client,
                "{}",
                serde_json::to_string(&request).unwrap()
            )
            .unwrap();
            let mut line = String::new();
            let result = BufReader::new(sending_client).read_line(&mut line);
            // Release the waiter even if a regression timed out this client.
            finished.send_blocking(()).unwrap();
            result.unwrap();
            assert_eq!(
                serde_json::from_str::<ControlResponse>(&line).unwrap(),
                ControlResponse::Send
            );
        });
        futures::executor::block_on(serve_jobs(receiver, |request| {
            let completion = completion.clone();
            let started = started.clone();
            async move {
                match request {
                    ControlRequest::Wait { .. } => {
                        started.send(()).unwrap();
                        completion.recv().await.unwrap();
                        ControlResponse::Wait
                    }
                    ControlRequest::Send { .. } => ControlResponse::Send,
                    _ => unreachable!(),
                }
            }
        }));
        waiting.join().unwrap();
        sending.join().unwrap();
        waiting_server.join().unwrap();
        sending_server.join().unwrap();
    }

    #[test]
    fn start_reclaims_stale_socket_and_stop_does_not_unlink_rebound_path() {
        let path = temp_socket_path("restart-safe");
        let stale = UnixListener::bind(&path).unwrap();
        drop(stale);

        let (jobs, _receiver) = async_channel::unbounded();
        let listener = spawn_listener(&path, jobs).unwrap();
        // Reclaim is proven functionally: the stale listener is gone, so a
        // successful connect means the path is owned by the new listener.
        // Do not compare (dev, ino) socket identities here — Linux
        // filesystems may immediately recycle the unlinked inode number.
        UnixStream::connect(&path).unwrap();

        listener.stop();
        assert!(!path.exists());

        let replacement = UnixListener::bind(&path).unwrap();
        assert!(path.exists());
        std::thread::sleep(Duration::from_millis(150));
        assert!(path.exists());
        drop(replacement);

        cleanup_test_path(&path);
    }

    #[test]
    fn live_listener_refusal_preserves_existing_socket() {
        let path = temp_socket_path("live-refusal");
        let existing = UnixListener::bind(&path).unwrap();
        let existing_identity = socket_identity(&path).unwrap();

        let (jobs, _receiver) = async_channel::unbounded();
        let err = spawn_listener(&path, jobs).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(socket_identity(&path).unwrap(), existing_identity);

        drop(existing);
        let _ = remove_socket_if_matches(&path, Some(existing_identity));
        cleanup_test_path(&path);
    }

    #[test]
    fn start_refuses_to_replace_unrelated_file() {
        let path = temp_socket_path("preserve-file");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, b"not a socket").unwrap();

        let (jobs, _receiver) = async_channel::unbounded();
        let err = spawn_listener(&path, jobs).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");

        cleanup_test_path(&path);
    }

    #[test]
    fn oversized_request_line_returns_error() {
        let (jobs, _receiver) = async_channel::unbounded();
        let (mut client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let server_thread = std::thread::spawn(move || handle_connection(server, jobs));
        let oversized = "x".repeat(MAX_REQUEST_LINE_BYTES + 1);
        writeln!(client, "{oversized}").unwrap();

        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        let response = serde_json::from_str::<ControlResponse>(&line).unwrap();
        assert_eq!(
            response,
            ControlResponse::Error {
                message: format!("bad request: line exceeds {MAX_REQUEST_LINE_BYTES} bytes"),
            }
        );

        server_thread.join().unwrap();
    }
}
