//! In-process Unix socket listener for ADR-0011 (`sleipnir-ctl`).

use crate::app_shell::AppShell;
use crate::run_ledger_global::RunLedgerGlobal;
use crate::TermView;
use gpui::{App, AsyncApp, Global, Task};
use run_ledger::PaneKey;
use sleipnir_ctl::{
    enabled, socket_path, wait_matches, ControlRequest, ControlResponse, PaneSnap, WaitUntil,
};
use sleipnir_settings::TerminalSettings;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Job {
    req: ControlRequest,
    reply: mpsc::Sender<ControlResponse>,
}

pub struct ControlSurface {
    stop: Option<mpsc::Sender<()>>,
    _pump: Task<()>,
}

impl Global for ControlSurface {}

pub fn init(cx: &mut App) {
    if !cx.has_global::<ControlSurface>() {
        cx.set_global(ControlSurface {
            stop: None,
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
    let running = cx.global::<ControlSurface>().stop.is_some();
    if want && !running {
        start(cx);
    } else if !want && running {
        stop(cx);
    }
}

fn start(cx: &mut App) {
    let path = socket_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(err) => {
            log::warn!("control surface bind failed ({}): {err}", path.display());
            return;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let _ = listener.set_nonblocking(true);
    let (stop_tx, stop_rx) = mpsc::channel();
    let (job_tx, job_rx) = async_channel::unbounded::<Job>();
    let sock_path = path.clone();
    std::thread::Builder::new()
        .name("sleipnir-ctl".into())
        .spawn(move || accept_loop(listener, stop_rx, job_tx, sock_path))
        .ok();
    let pump = cx.spawn(async move |cx| pump_jobs(cx, job_rx).await);
    let g = cx.global_mut::<ControlSurface>();
    g.stop = Some(stop_tx);
    g._pump = pump;
    log::info!("control surface listening on {}", path.display());
}

fn stop(cx: &mut App) {
    let g = cx.global_mut::<ControlSurface>();
    if let Some(stop) = g.stop.take() {
        let _ = stop.send(());
    }
    g._pump = Task::ready(());
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
}

fn accept_loop(
    listener: UnixListener,
    stop: mpsc::Receiver<()>,
    jobs: async_channel::Sender<Job>,
    path: std::path::PathBuf,
) {
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let jobs = jobs.clone();
                std::thread::spawn(move || handle_connection(stream, jobs));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    let _ = std::fs::remove_file(path);
}

fn handle_connection(mut stream: UnixStream, jobs: async_channel::Sender<Job>) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req = match serde_json::from_str::<ControlRequest>(trimmed) {
            Ok(req) => req,
            Err(err) => {
                let resp = ControlResponse::Error {
                    message: format!("bad request: {err}"),
                };
                let _ = writeln!(
                    stream,
                    "{}",
                    serde_json::to_string(&resp).unwrap_or_default()
                );
                continue;
            }
        };
        let timeout = match &req {
            ControlRequest::Wait { timeout_secs, .. } => timeout_secs.saturating_add(5),
            _ => 30,
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        if jobs.send_blocking(Job { req, reply: reply_tx }).is_err() {
            break;
        }
        let resp = reply_rx
            .recv_timeout(Duration::from_secs(timeout))
            .unwrap_or(ControlResponse::Error {
                message: "timeout".into(),
            });
        if writeln!(
            stream,
            "{}",
            serde_json::to_string(&resp).unwrap_or_default()
        )
        .is_err()
        {
            break;
        }
    }
}

async fn pump_jobs(cx: &mut AsyncApp, jobs: async_channel::Receiver<Job>) {
    while let Ok(job) = jobs.recv().await {
        match job.req {
            ControlRequest::Wait {
                pane,
                until,
                timeout_secs,
            } => {
                let resp = wait_until(cx, pane, until, timeout_secs).await;
                let _ = job.reply.send(resp);
            }
            other => {
                let resp = cx.update(|cx| dispatch(other, cx));
                let _ = job.reply.send(resp);
            }
        }
    }
}

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

fn wait_status(pane: PaneKey, until: WaitUntil, cx: &mut App) -> Result<bool, String> {
    let Some(view) = view_for_pane(cx, pane) else {
        return Err(format!("pane {pane} not found"));
    };
    let busy = view.read(cx).looks_busy(cx);
    let (failed, attention) = if cx.has_global::<RunLedgerGlobal>() {
        let g = cx.global::<RunLedgerGlobal>();
        (g.pane_has_failed_attention(pane), g.pane_has_attention(pane))
    } else {
        (false, false)
    };
    Ok(wait_matches(until, busy, failed, attention))
}

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

fn view_for_pane(cx: &mut App, pane: PaneKey) -> Option<gpui::Entity<TermView>> {
    collect_live_panes(cx)
        .into_iter()
        .find(|(key, _)| *key == pane)
        .map(|(_, view)| view)
}

fn collect_live_panes(cx: &mut App) -> Vec<(PaneKey, gpui::Entity<TermView>)> {
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
