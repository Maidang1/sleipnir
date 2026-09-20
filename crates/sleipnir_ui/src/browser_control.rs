//! Per-window browser-only control transport. No persistent token files.
use crate::app_shell::AppShell;
use gpui::{App, Context, Global, Task, Window};
use sleipnir_browser_control::{
    Request, Response,
    transport::{Credentials, Server},
};
use std::{collections::HashMap, sync::mpsc, time::Instant};

#[derive(Default)]
struct Connections(HashMap<u64, Credentials>);
impl Global for Connections {}
struct Job {
    request: Request,
    reply: mpsc::Sender<Response>,
    deadline: Instant,
}
pub(crate) struct WindowBridge {
    _server: Server,
    _pump: Task<()>,
}

pub(crate) fn start(window: &mut Window, cx: &mut Context<AppShell>) -> Option<WindowBridge> {
    let id = window.window_handle().window_id().as_u64();
    let (tx, rx) = async_channel::bounded::<Job>(16);
    let server = match Server::start(id, move |request, deadline| {
        let (reply, receive) = mpsc::channel();
        if tx
            .try_send(Job {
                request,
                reply,
                deadline,
            })
            .is_err()
        {
            return Response::error("busy", "Browser request queue unavailable");
        }
        receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| {
                Response::error("timeout", "Browser did not respond before deadline")
            })
    }) {
        Ok(server) => server,
        Err(error) => {
            log::error!("Browser control unavailable: {error}");
            return None;
        }
    };
    if !cx.has_global::<Connections>() {
        cx.set_global(Connections::default());
        cx.on_window_closed(|cx, id| {
            cx.global_mut::<Connections>().0.remove(&id.as_u64());
        })
        .detach();
    }
    cx.global_mut::<Connections>()
        .0
        .insert(id, server.credentials.clone());
    let pump = cx.spawn_in(window, async move |shell, cx| {
        while let Ok(job) = rx.recv().await {
            let fallback = job.reply.clone();
            if shell
                .update_in(cx, |shell, window, cx| {
                    if Instant::now() >= job.deadline {
                        let _ = job.reply.send(Response::error(
                            "timeout",
                            "Request expired before execution",
                        ));
                        return;
                    }
                    shell.browser_control_request(job.request, job.reply, job.deadline, window, cx);
                })
                .is_err()
            {
                let _ = fallback.send(Response::error("window_closed", "Target window is closed"));
                break;
            }
        }
    });
    Some(WindowBridge {
        _server: server,
        _pump: pump,
    })
}

pub(crate) fn inject_environment(
    window: u64,
    env: &mut collections::HashMap<String, String>,
    cx: &App,
) {
    use sleipnir_browser_control::{ENDPOINT_ENV, TOKEN_ENV, WINDOW_ENV};
    // Remove inherited capabilities before replacing them with this window's values.
    for key in [ENDPOINT_ENV, TOKEN_ENV, WINDOW_ENV] {
        env.remove(key);
    }
    if let Some(credentials) = cx
        .try_global::<Connections>()
        .and_then(|c| c.0.get(&window))
    {
        env.insert(ENDPOINT_ENV.into(), credentials.endpoint.to_string());
        env.insert(TOKEN_ENV.into(), credentials.token.clone());
        env.insert(WINDOW_ENV.into(), window.to_string());
    }
}
