use super::*;
use crate::v2::session::{Io, SharedWriter};
use plugin_protocol::v2::Output as ProtoOutput;
use plugin_protocol::v2::{CommandSpec, EventKind, HostCall, InvokeContext};
use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn reply_session(
    granted: Vec<Capability>,
    reply: HostCallResult,
) -> (
    mpsc::SyncSender<io::Result<Option<HostMessage>>>,
    Io,
    std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
) {
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(Ok(Some(HostMessage::Reply {
            id: 1,
            result: reply,
        })))
        .unwrap();
    let (writer, bytes) = SharedWriter::new();
    let mut io = Io::new(receiver, writer);
    io.set_session(granted, Uuid::nil());
    (sender, io, bytes)
}

fn written_call(bytes: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> HostCall {
    let guard = bytes.lock().unwrap();
    let line = std::str::from_utf8(&guard).unwrap().trim();
    match serde_json::from_str::<PluginMessage>(line).unwrap() {
        PluginMessage::Call { call, .. } => call,
        other => panic!("expected Call, got {other:?}"),
    }
}

#[test]
fn focus_pane_wrapper_writes_the_call_and_returns_ok() {
    let pane = Uuid::from_u128(3);
    let (_keep, mut session, bytes) =
        reply_session(vec![Capability::HostCallFocusPane], HostCallResult::Ok);
    let result = Context { io: &mut session }.focus_pane(pane);
    assert_eq!(result, HostCallResult::Ok);
    assert_eq!(written_call(&bytes), HostCall::FocusPane { pane });
}

#[test]
fn send_text_wrapper_writes_the_call_and_returns_ok() {
    let pane = Uuid::from_u128(4);
    let (_keep, mut session, bytes) =
        reply_session(vec![Capability::HostCallSendText], HostCallResult::Ok);
    let result = Context { io: &mut session }.send_text(pane, "hello", true);
    assert_eq!(result, HostCallResult::Ok);
    assert_eq!(
        written_call(&bytes),
        HostCall::SendText {
            pane,
            text: "hello".into(),
            enter: true,
        }
    );
}

#[test]
fn send_key_wrapper_writes_the_call_and_returns_ok() {
    let pane = Uuid::from_u128(5);
    let (_keep, mut session, bytes) =
        reply_session(vec![Capability::HostCallSendKey], HostCallResult::Ok);
    let result = Context { io: &mut session }.send_key(pane, "ctrl-c");
    assert_eq!(result, HostCallResult::Ok);
    assert_eq!(
        written_call(&bytes),
        HostCall::SendKey {
            pane,
            key: "ctrl-c".into(),
        }
    );
}

#[test]
fn open_pane_argv_wrapper_writes_the_call_and_returns_ok() {
    let (_keep, mut session, bytes) =
        reply_session(vec![Capability::HostCallOpenPane], HostCallResult::Ok);
    let result = Context { io: &mut session }.open_pane_argv(
        Some("/work".into()),
        "codex",
        vec!["--foo".into()],
    );
    assert_eq!(result, HostCallResult::Ok);
    assert_eq!(
        written_call(&bytes),
        HostCall::OpenPaneArgv {
            cwd: Some("/work".into()),
            program: "codex".into(),
            args: vec!["--foo".into()],
        }
    );
}

#[test]
fn request_close_pane_wrapper_writes_the_call_and_returns_ok() {
    let pane = Uuid::from_u128(6);
    let (_keep, mut session, bytes) = reply_session(
        vec![Capability::HostCallRequestClosePane],
        HostCallResult::Ok,
    );
    let result = Context { io: &mut session }.request_close_pane(pane);
    assert_eq!(result, HostCallResult::Ok);
    assert_eq!(written_call(&bytes), HostCall::RequestClosePane { pane });
}

#[test]
fn missing_host_reply_times_out_without_waiting_for_eof() {
    let (_sender, receiver) = mpsc::sync_channel(1);
    let (writer, _bytes) = SharedWriter::new();
    let mut session = Io::new(receiver, writer);
    session.set_session(vec![Capability::HostCallListPanes], Uuid::nil());
    let result = Context { io: &mut session }
        .call_with_timeout(HostCall::ListPanes, Duration::from_millis(10));
    assert!(matches!(
        result,
        HostCallResult::Error { message } if message.contains("timed out")
    ));
    assert!(!session.is_shutdown());
    session.store_unmatched(1, HostCallResult::Ok);
    assert!(session.take_unmatched(1).is_none());
}

#[test]
fn queued_events_do_not_extend_a_host_call_deadline() {
    let (sender, receiver) = mpsc::sync_channel(1);
    sender
        .send(Ok(Some(HostMessage::Event {
            id: 1,
            event: HostEvent::PaneFocused { pane: Uuid::nil() },
        })))
        .unwrap();
    let (writer, _bytes) = SharedWriter::new();
    let mut session = Io::new(receiver, writer);
    let result =
        Context { io: &mut session }.call_with_timeout(HostCall::ListPanes, Duration::ZERO);
    assert!(matches!(
        result,
        HostCallResult::Error { message } if message.contains("timed out")
    ));
}

struct ProbeState {
    events: Vec<String>,
    actions: Vec<(String, Option<String>)>,
    hellos: u32,
    call_on_finished: bool,
    render_on_finished: bool,
    last_call: Option<HostCallResult>,
    shutdown_during_call: bool,
}

struct Probe {
    state: Rc<RefCell<ProbeState>>,
}

impl Probe {
    fn new() -> (Self, Rc<RefCell<ProbeState>>) {
        let state = Rc::new(RefCell::new(ProbeState {
            events: Vec::new(),
            actions: Vec::new(),
            hellos: 0,
            call_on_finished: false,
            render_on_finished: false,
            last_call: None,
            shutdown_during_call: false,
        }));
        (
            Self {
                state: Rc::clone(&state),
            },
            state,
        )
    }
}

impl Plugin for Probe {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "probe".into(),
            name: "Probe".into(),
            version: "0.1.0".into(),
            description: String::new(),
            lifecycle: Lifecycle::Resident,
            commands: vec![CommandSpec {
                id: "noop".into(),
                title: "Noop".into(),
                description: String::new(),
                keywords: vec![],
                capabilities: vec![],
            }],
        }
    }

    fn requests(&self) -> Vec<Capability> {
        vec![
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderBlock,
            Capability::HostCallListPanes,
        ]
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            panes: vec![],
            kinds: vec![EventKind::RunFinished],
        }
    }

    fn on_hello(&mut self, _granted: &[Capability], _id: Uuid, _ctx: &mut Context<'_>) {
        self.state.borrow_mut().hellos += 1;
    }

    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        self.state
            .borrow_mut()
            .events
            .push(format!("{:?}", event.kind()));
        let call_on_finished = self.state.borrow().call_on_finished;
        let render_on_finished = self.state.borrow().render_on_finished;
        if matches!(event, HostEvent::RunFinished { .. }) {
            if render_on_finished {
                let _ = ctx.render(
                    RenderTarget::Block {
                        anchor: Uuid::nil(),
                    },
                    text("failed").tone(Tone::Err),
                );
            }
            if call_on_finished {
                let result = ctx.call(HostCall::ListPanes);
                let mut state = self.state.borrow_mut();
                if matches!(
                    result,
                    HostCallResult::Error { ref message } if message == "host shutdown"
                ) {
                    state.shutdown_during_call = true;
                }
                state.last_call = Some(result);
            }
        }
    }

    fn on_action(
        &mut self,
        _block_id: BlockId,
        action: &str,
        arg: Option<&str>,
        ctx: &mut Context<'_>,
    ) {
        self.state
            .borrow_mut()
            .actions
            .push((action.to_string(), arg.map(str::to_string)));
        let _ = ctx.render(
            RenderTarget::Block {
                anchor: Uuid::nil(),
            },
            text("retried").tone(Tone::Ok),
        );
    }
}

fn hello(version: u32) -> String {
    serde_json::to_string(&HostMessage::Hello {
        protocol_version: version,
        granted: vec![
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderBlock,
            Capability::HostCallListPanes,
        ],
        plugin_instance_id: Uuid::from_u128(7),
    })
    .unwrap()
}

fn conversation(plugin: Probe, input: &str) -> Vec<PluginMessage> {
    let (writer, bytes) = SharedWriter::new();
    serve(plugin, std::io::Cursor::new(input.to_string()), writer).unwrap();
    let out = bytes.lock().unwrap().clone();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn run_finished() -> String {
    serde_json::to_string(&HostMessage::Event {
        id: 10,
        event: HostEvent::RunFinished {
            run_id: Uuid::nil(),
            pane: Uuid::nil(),
            exit_code: Some(1),
            duration_ms: 40,
        },
    })
    .unwrap()
}

fn pane_focused() -> String {
    serde_json::to_string(&HostMessage::Event {
        id: 11,
        event: HostEvent::PaneFocused { pane: Uuid::nil() },
    })
    .unwrap()
}

#[test]
fn handshake_replies_ready_with_manifest_requests_and_filter() {
    let (plugin, state) = Probe::new();
    let msgs = conversation(plugin, &format!("{}\n", hello(v2::PROTOCOL_VERSION)));
    let PluginMessage::Ready {
        protocol_version,
        manifest,
        requests,
        event_filter,
    } = &msgs[0]
    else {
        panic!("expected ready, got {:?}", msgs[0]);
    };
    assert_eq!(*protocol_version, v2::PROTOCOL_VERSION);
    assert_eq!(manifest.id, "probe");
    assert!(requests.contains(&Capability::SubscribeEvents));
    assert_eq!(event_filter.kinds, vec![EventKind::RunFinished]);
    assert_eq!(state.borrow().hellos, 1);
}

#[test]
fn render_is_a_push_not_a_reply() {
    let (plugin, _) = Probe::new();
    plugin.state.borrow_mut().render_on_finished = true;
    let msgs = conversation(
        plugin,
        &format!("{}\n{}\n", hello(v2::PROTOCOL_VERSION), run_finished()),
    );
    assert!(
        msgs.iter()
            .any(|m| matches!(m, PluginMessage::Render { .. })),
        "Render must be sendable from on_event, got {msgs:?}"
    );
}

#[test]
fn action_callback_carries_block_id_action_and_arg() {
    let (plugin, state) = Probe::new();
    let action = serde_json::to_string(&HostMessage::Action {
        id: 3,
        block_id: Uuid::from_u128(9),
        action: "retry".into(),
        arg: Some("run-1".into()),
    })
    .unwrap();
    let msgs = conversation(
        plugin,
        &format!("{}\n{action}\n", hello(v2::PROTOCOL_VERSION)),
    );
    assert_eq!(
        state.borrow().actions,
        vec![("retry".into(), Some("run-1".into()))]
    );
    assert!(
        msgs.iter()
            .any(|m| matches!(m, PluginMessage::Render { .. }))
    );
}

#[test]
fn out_of_order_reply_is_correlated_by_id() {
    let (plugin, state) = Probe::new();
    plugin.state.borrow_mut().call_on_finished = true;
    let stray = serde_json::to_string(&HostMessage::Reply {
        id: 99,
        result: HostCallResult::Error {
            message: "not yours".into(),
        },
    })
    .unwrap();
    let reply = serde_json::to_string(&HostMessage::Reply {
        id: 1,
        result: HostCallResult::Panes { panes: vec![] },
    })
    .unwrap();
    let _msgs = conversation(
        plugin,
        &format!(
            "{}\n{}\n{stray}\n{reply}\n",
            hello(v2::PROTOCOL_VERSION),
            run_finished()
        ),
    );
    match &state.borrow().last_call {
        Some(HostCallResult::Panes { panes }) => assert!(panes.is_empty()),
        other => panic!("expected Panes result, got {other:?}"),
    }
}

#[test]
fn event_arriving_while_a_host_call_is_pending_is_not_lost() {
    let (plugin, state) = Probe::new();
    plugin.state.borrow_mut().call_on_finished = true;
    let reply = serde_json::to_string(&HostMessage::Reply {
        id: 1,
        result: HostCallResult::Ok,
    })
    .unwrap();
    let _ = conversation(
        plugin,
        &format!(
            "{}\n{}\n{}\n{reply}\n",
            hello(v2::PROTOCOL_VERSION),
            run_finished(),
            pane_focused()
        ),
    );
    let events = state.borrow().events.clone();
    assert_eq!(
        events,
        vec!["RunFinished".to_string(), "PaneFocused".to_string()],
        "the intervening event must still be delivered"
    );
    assert!(matches!(state.borrow().last_call, Some(HostCallResult::Ok)));
}

#[test]
fn shutdown_mid_flight_ends_the_call_and_the_session() {
    let (plugin, state) = Probe::new();
    plugin.state.borrow_mut().call_on_finished = true;
    let shutdown = serde_json::to_string(&HostMessage::Shutdown).unwrap();
    let extra = serde_json::to_string(&HostMessage::Event {
        id: 50,
        event: HostEvent::PaneFocused { pane: Uuid::nil() },
    })
    .unwrap();
    let msgs = conversation(
        plugin,
        &format!(
            "{}\n{}\n{shutdown}\n{extra}\n",
            hello(v2::PROTOCOL_VERSION),
            run_finished()
        ),
    );
    assert!(state.borrow().shutdown_during_call);
    assert_eq!(
        msgs.iter()
            .filter(|m| matches!(m, PluginMessage::Call { .. }))
            .count(),
        1
    );
    assert_eq!(
        state.borrow().events,
        vec!["RunFinished".to_string()],
        "nothing after Shutdown is served"
    );
}

#[test]
fn malformed_host_input_does_not_panic() {
    let (plugin, state) = Probe::new();
    let shutdown = serde_json::to_string(&HostMessage::Shutdown).unwrap();
    let msgs = conversation(
        plugin,
        &format!(
            "{}\n{{not json\n{}\n{shutdown}\n",
            hello(v2::PROTOCOL_VERSION),
            run_finished()
        ),
    );
    assert_eq!(state.borrow().events, vec!["RunFinished".to_string()]);
    assert_eq!(
        msgs.len(),
        1,
        "only Ready; garbage did not kill the session"
    );
}

#[test]
fn version_mismatch_is_an_error() {
    let (plugin, _) = Probe::new();
    let (writer, _bytes) = SharedWriter::new();
    let err = serve(
        plugin,
        std::io::Cursor::new(format!("{}\n", hello(999))),
        writer,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Unsupported);
}

struct LineFeed {
    rx: mpsc::Receiver<String>,
    leftover: Vec<u8>,
}

impl std::io::Read for LineFeed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.leftover.is_empty() {
            match self.rx.recv() {
                Ok(line) => {
                    let mut bytes = line.into_bytes();
                    if !bytes.ends_with(&[b'\n']) {
                        bytes.push(b'\n');
                    }
                    self.leftover = bytes;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = self.leftover.len().min(buf.len());
        buf[..n].copy_from_slice(&self.leftover[..n]);
        self.leftover.drain(..n);
        Ok(n)
    }
}

struct Ticker {
    interval: Option<Duration>,
    ticks: std::sync::Arc<std::sync::atomic::AtomicU32>,
    events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Plugin for Ticker {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "ticker".into(),
            name: "Ticker".into(),
            version: "0.1.0".into(),
            description: String::new(),
            lifecycle: Lifecycle::Resident,
            commands: vec![],
        }
    }

    fn requests(&self) -> Vec<Capability> {
        vec![Capability::Resident, Capability::SubscribeEvents]
    }

    fn tick_interval(&self) -> Option<Duration> {
        self.interval
    }

    fn on_tick(&mut self, _ctx: &mut Context<'_>) {
        self.ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn on_event(&mut self, event: HostEvent, _ctx: &mut Context<'_>) {
        self.events
            .lock()
            .unwrap()
            .push(format!("{:?}", event.kind()));
    }
}

fn live_serve(
    plugin: Ticker,
) -> (
    std::thread::JoinHandle<io::Result<()>>,
    mpsc::Sender<String>,
) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        serve(
            plugin,
            std::io::BufReader::new(LineFeed {
                rx,
                leftover: Vec::new(),
            }),
            std::io::sink(),
        )
    });
    (handle, tx)
}

fn wait_until(cond: impl Fn() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    cond()
}

#[test]
fn default_plugin_does_not_tick() {
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let plugin = Ticker {
        interval: None,
        ticks: std::sync::Arc::clone(&ticks),
        events,
    };
    let (handle, tx) = live_serve(plugin);
    tx.send(hello(v2::PROTOCOL_VERSION)).unwrap();
    std::thread::sleep(Duration::from_millis(25));
    assert_eq!(ticks.load(std::sync::atomic::Ordering::SeqCst), 0);
    tx.send(serde_json::to_string(&HostMessage::Shutdown).unwrap())
        .unwrap();
    drop(tx);
    handle.join().unwrap().unwrap();
    assert_eq!(ticks.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn enabled_ticks_repeat_without_busy_spin() {
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let plugin = Ticker {
        interval: Some(Duration::from_millis(5)),
        ticks: std::sync::Arc::clone(&ticks),
        events,
    };
    let started = Instant::now();
    let (handle, tx) = live_serve(plugin);
    tx.send(hello(v2::PROTOCOL_VERSION)).unwrap();
    assert!(
        wait_until(
            || ticks.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            Duration::from_millis(400)
        ),
        "expected repeated ticks, got {}",
        ticks.load(std::sync::atomic::Ordering::SeqCst)
    );
    let n = ticks.load(std::sync::atomic::Ordering::SeqCst);
    let elapsed = started.elapsed();
    assert!(
        n < 200,
        "tick loop must wait, not busy-spin: {n} ticks in {elapsed:?}"
    );
    tx.send(serde_json::to_string(&HostMessage::Shutdown).unwrap())
        .unwrap();
    drop(tx);
    handle.join().unwrap().unwrap();
}

#[test]
fn inbound_messages_are_not_starved_by_ticks() {
    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let plugin = Ticker {
        interval: Some(Duration::from_millis(5)),
        ticks: std::sync::Arc::clone(&ticks),
        events: std::sync::Arc::clone(&events),
    };
    let (handle, tx) = live_serve(plugin);
    tx.send(hello(v2::PROTOCOL_VERSION)).unwrap();
    assert!(wait_until(
        || ticks.load(std::sync::atomic::Ordering::SeqCst) >= 1,
        Duration::from_millis(400)
    ));
    tx.send(run_finished()).unwrap();
    tx.send(pane_focused()).unwrap();
    assert!(
        wait_until(
            || events.lock().unwrap().len() >= 2,
            Duration::from_millis(400)
        ),
        "ticks must not starve inbound events: {:?}",
        events.lock().unwrap()
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec!["RunFinished".to_string(), "PaneFocused".to_string()]
    );
    tx.send(serde_json::to_string(&HostMessage::Shutdown).unwrap())
        .unwrap();
    drop(tx);
    handle.join().unwrap().unwrap();
}

#[test]
fn interleaved_invoke_still_correlates() {
    let (plugin, _) = Probe::new();
    let invoke = serde_json::to_string(&HostMessage::Invoke {
        id: 42,
        command_id: "noop".into(),
        context: InvokeContext::default(),
    })
    .unwrap();
    let msgs = conversation(
        plugin,
        &format!("{}\n{invoke}\n", hello(v2::PROTOCOL_VERSION)),
    );
    let PluginMessage::Invoked {
        id,
        output: ProtoOutput::Ignore,
    } = &msgs[1]
    else {
        panic!("expected invoked, got {:?}", msgs[1]);
    };
    assert_eq!(*id, 42);
}

#[test]
fn queue_saturation_during_call_forces_shutdown_and_drops_excess_message() {
    let (sender, receiver) = mpsc::sync_channel(65);
    for id in 1..=65_u64 {
        sender
            .send(Ok(Some(HostMessage::Event {
                id,
                event: HostEvent::PaneFocused { pane: Uuid::nil() },
            })))
            .unwrap();
    }
    let (writer, _bytes) = SharedWriter::new();
    let mut session = Io::new(receiver, writer);
    let result = Context { io: &mut session }
        .call_with_timeout(HostCall::ListPanes, Duration::from_millis(50));
    assert!(matches!(
        result,
        HostCallResult::Error { message } if message == "host shutdown"
    ));
    assert!(session.is_shutdown());
    assert_eq!(session.queued_len(), 64);
}
