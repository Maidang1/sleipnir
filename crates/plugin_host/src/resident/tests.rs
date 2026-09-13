//! Deterministic supervisor tests. The plugin is an in-memory endpoint driven
//! from a helper thread; time is a [`ManualClock`]. Nothing here sleeps.

use super::*;
use crate::PluginLifecycle;
use plugin_protocol::v2::{
    self, Capability, EventFilter, EventKind, HostCall, HostEvent, HostMessage, PluginMessage,
    RenderTarget, Widget,
};
use plugin_protocol::v2::{CommandSpec, InvokeContext, Lifecycle, Manifest, Output};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct Env {
    sup: Supervisor,
    ep_rx: Arc<Mutex<mpsc::Receiver<PluginEndpoint>>>,
    clock: Arc<ManualClock>,
}

impl Env {
    fn new() -> Self {
        Self::with_config(SupervisorConfig::for_tests())
    }

    fn with_config(config: SupervisorConfig) -> Self {
        let (launcher, ep_rx) = MemoryLauncher::pair();
        let clock = Arc::new(ManualClock::new(1_000));
        let sup = Supervisor::new(
            config,
            Arc::new(launcher),
            Arc::clone(&clock) as Arc<dyn Clock>,
        );
        Self {
            sup,
            ep_rx: Arc::new(Mutex::new(ep_rx)),
            clock,
        }
    }

    fn with_pipe_cap(config: SupervisorConfig, pipe_cap: usize) -> Self {
        let (launcher, ep_rx) = MemoryLauncher::pair_with_cap(pipe_cap);
        let clock = Arc::new(ManualClock::new(1_000));
        let sup = Supervisor::new(
            config,
            Arc::new(launcher),
            Arc::clone(&clock) as Arc<dyn Clock>,
        );
        Self {
            sup,
            ep_rx: Arc::new(Mutex::new(ep_rx)),
            clock,
        }
    }

    fn spawn_plugin(
        &self,
        plugin: impl FnOnce(&mut PluginEndpoint) + Send + 'static,
    ) -> JoinHandle<()> {
        let rx = Arc::clone(&self.ep_rx);
        thread::spawn(move || {
            let mut ep = mutex_lock(&rx).recv().expect("plugin launched");
            plugin(&mut ep);
        })
    }

    fn spawn_loop(&self, plugin: impl Fn(&mut PluginEndpoint) + Send + 'static) -> JoinHandle<()> {
        let rx = Arc::clone(&self.ep_rx);
        thread::spawn(move || {
            while let Ok(mut ep) = mutex_lock(&rx).recv() {
                plugin(&mut ep);
            }
        })
    }
}

fn spec() -> LaunchSpec {
    LaunchSpec {
        plugin_id: "demo".into(),
        keep_alive: false,
        lifecycle: PluginLifecycle::Resident,
        declared_capabilities: BTreeSet::from([
            Capability::ReadCwd,
            Capability::Resident,
            Capability::RenderStatus,
            Capability::HostCallListPanes,
        ]),
        granted: vec![Capability::ReadCwd],
        binary: "demo".into(),
        args: vec![],
        cwd: PathBuf::from("."),
    }
}

fn ready(version: u32, requests: Vec<Capability>) -> PluginMessage {
    PluginMessage::Ready {
        protocol_version: version,
        manifest: Manifest {
            id: "demo".into(),
            name: "Demo".into(),
            version: "1".into(),
            description: String::new(),
            lifecycle: Lifecycle::Resident,
            commands: vec![CommandSpec {
                id: "run".into(),
                title: "Run".into(),
                description: String::new(),
                keywords: vec![],
                capabilities: vec![plugin_protocol::v2::Capability::ReadCwd],
            }],
        },
        requests,
        event_filter: v2::EventFilter::default(),
    }
}

fn good_ready() -> PluginMessage {
    ready(
        v2::PROTOCOL_VERSION,
        vec![Capability::ReadCwd, Capability::Resident],
    )
}

fn invoked(id: v2::MessageId) -> PluginMessage {
    PluginMessage::Invoked {
        id,
        output: Output::Ignore,
    }
}

fn handshake_and_echo(ep: &mut PluginEndpoint) {
    ep.handshake(&good_ready()).expect("handshake");
    serve_until_eof(ep);
}

fn serve_until_eof(ep: &mut PluginEndpoint) {
    while let Ok(msg) = ep.recv() {
        match msg {
            HostMessage::Invoke { id, .. } => {
                let _ = ep.send(&invoked(id));
            }
            HostMessage::Shutdown | HostMessage::Hello { .. } => break,
            _ => {}
        }
    }
}

fn event_spec(id: &str) -> LaunchSpec {
    let mut spec = spec();
    spec.plugin_id = id.into();
    spec.declared_capabilities
        .insert(Capability::SubscribeEvents);
    spec.granted.push(Capability::SubscribeEvents);
    spec
}

fn ready_events(id: &str, filter: EventFilter) -> PluginMessage {
    let mut msg = ready(
        v2::PROTOCOL_VERSION,
        vec![
            Capability::ReadCwd,
            Capability::Resident,
            Capability::SubscribeEvents,
        ],
    );
    if let PluginMessage::Ready {
        manifest,
        event_filter,
        ..
    } = &mut msg
    {
        manifest.id = id.into();
        *event_filter = filter;
    }
    msg
}

fn pane(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}

fn run_started(p: uuid::Uuid, command: &str) -> HostEvent {
    HostEvent::RunStarted {
        run_id: uuid::Uuid::from_u128(p.as_u128() + 100),
        pane: p,
        command: command.into(),
        cwd: Some("/tmp".into()),
        inferred: false,
    }
}

fn run_finished(p: uuid::Uuid) -> HostEvent {
    HostEvent::RunFinished {
        run_id: uuid::Uuid::from_u128(p.as_u128() + 100),
        pane: p,
        exit_code: Some(0),
        duration_ms: 12,
    }
}

fn handshake_events(ep: &mut PluginEndpoint, id: &str, filter: EventFilter) {
    ep.handshake(&ready_events(id, filter)).expect("handshake");
}

fn collect_until_invoke(ep: &mut PluginEndpoint, sink: &std::sync::mpsc::Sender<HostEvent>) {
    while let Ok(msg) = ep.recv() {
        match msg {
            HostMessage::Event { event, .. } => {
                let _ = sink.send(event);
            }
            HostMessage::Invoke { id, .. } => {
                let _ = ep.send(&invoked(id));
            }
            HostMessage::Shutdown | HostMessage::Hello { .. } => break,
            _ => {}
        }
    }
}

fn wait_until(mut pred: impl FnMut() -> bool, what: &str) {
    for _ in 0..1_000_000 {
        if pred() {
            return;
        }
        thread::yield_now();
    }
    panic!("{what} never became true");
}

fn wait_until_value<T>(mut value: impl FnMut() -> Option<T>, what: &str) -> T {
    for _ in 0..1_000_000 {
        if let Some(value) = value() {
            return value;
        }
        thread::yield_now();
    }
    panic!("{what} never became true");
}

#[test]
fn handshake_success() {
    let env = Env::new();
    let plugin = env.spawn_plugin(handshake_and_echo);
    let session = env.sup.connect(&spec()).expect("connect");
    let snap = session.snapshot();
    assert_eq!(snap.plugin_id, "demo");
    assert_eq!(snap.state, ConnectionState::Live);
    assert_eq!(snap.in_flight, 0);
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn version_outside_window_is_rejected() {
    let env = Env::new();
    for version in [0_u32, 3] {
        let plugin = env.spawn_plugin(move |ep| {
            let _ = ep.handshake(&ready(version, vec![Capability::ReadCwd]));
        });
        let err = env.sup.connect(&spec()).unwrap_err();
        assert!(
            matches!(err, SessionError::VersionMismatch { plugin } if plugin == version),
            "version {version}: {err:?}"
        );
        let _ = plugin.join();
        env.clock.advance(10_000);
    }
}

#[test]
fn version_from_the_removed_v1_dialect_is_rejected() {
    // The v1 dialect is gone: its frames carry no correlation ids, so a v1
    // handshake must be refused, not accepted into a session that cannot
    // interoperate. The N/N-1 window (ADR-0016 §8) reopens when v3 lands and
    // v2 stays implemented.
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        let _ = ep.handshake(&ready(1, vec![Capability::ReadCwd]));
        serve_until_eof(ep);
    });
    let err = env.sup.connect(&spec()).unwrap_err();
    assert!(
        matches!(err, SessionError::VersionMismatch { plugin } if plugin == 1),
        "v1 wire must be rejected: {err:?}"
    );
    let _ = plugin.join();
}

#[test]
fn ready_requesting_more_than_declared_is_rejected() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        let _ = ep.handshake(&ready(
            v2::PROTOCOL_VERSION,
            vec![Capability::ReadCwd, Capability::SubscribeEvents],
        ));
    });
    let err = env.sup.connect(&spec()).unwrap_err();
    assert_eq!(
        err,
        SessionError::CapabilityExceeded {
            capability: Capability::SubscribeEvents
        }
    );
    let _ = plugin.join();
}

#[test]
fn correlation_routes_out_of_order_replies() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id: a, .. } = ep.recv().unwrap() else {
            panic!("expected first invoke");
        };
        let HostMessage::Invoke { id: b, .. } = ep.recv().unwrap() else {
            panic!("expected second invoke");
        };
        // Reply newest first: the whole point of MessageId routing.
        ep.send(&invoked(b)).unwrap();
        ep.send(&invoked(a)).unwrap();
        serve_until_eof(ep);
    });

    let spec = spec();
    let pa = env
        .sup
        .begin_invoke(&spec, "one", InvokeContext::default())
        .unwrap();
    let pb = env
        .sup
        .begin_invoke(&spec, "two", InvokeContext::default())
        .unwrap();
    assert_ne!(pa.id(), pb.id());
    assert_eq!(pb.wait(Duration::from_secs(2)).unwrap(), Output::Ignore);
    assert_eq!(pa.wait(Duration::from_secs(2)).unwrap(), Output::Ignore);
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn unsolicited_render_is_routed() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        // Same stdout stream: Render then Invoked. Waiting on Invoked is a
        // happens-before for the Render having been routed.
        ep.send(&PluginMessage::Render {
            id: 7,
            target: RenderTarget::Status,
            tree: Widget::Sep,
        })
        .unwrap();
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    let spec = spec();
    env.sup
        .invoke(&spec, "run", InvokeContext::default())
        .unwrap();
    let inbound = env.sup.drain_inbound("demo");
    assert_eq!(inbound.len(), 1);
    match &inbound[0] {
        Inbound::Render { id, target, tree } => {
            assert_eq!(*id, 7);
            assert_eq!(*target, RenderTarget::Status);
            assert_eq!(*tree, Widget::Sep);
        }
        other => panic!("expected Render, got {other:?}"),
    }
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn unsolicited_call_is_routed() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&PluginMessage::Call {
            id: 11,
            call: HostCall::ListPanes,
        })
        .unwrap();
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    match env.sup.drain_inbound("demo").as_slice() {
        [Inbound::Call { id, call }] => {
            assert_eq!(*id, 11);
            assert_eq!(*call, HostCall::ListPanes);
        }
        other => panic!("expected Call, got {other:?}"),
    }
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn invocation_outputs_require_grants_for_both_lifecycles() {
    for lifecycle in [PluginLifecycle::Resident, PluginLifecycle::OnDemand] {
        for (output, capability) in [
            (
                Output::Insert {
                    text: "command\n".into(),
                },
                Capability::WriteTerminal,
            ),
            (
                Output::Copy {
                    text: "clipboard".into(),
                },
                Capability::Clipboard,
            ),
        ] {
            for allowed in [false, true] {
                let env = Env::new();
                let mut launch = spec();
                launch.lifecycle = lifecycle;
                launch.declared_capabilities.insert(capability);
                if allowed {
                    launch.granted.push(capability);
                }
                let response = output.clone();
                let plugin = env.spawn_plugin(move |endpoint| {
                    endpoint.handshake(&good_ready()).unwrap();
                    let HostMessage::Invoke { id, .. } = endpoint.recv().unwrap() else {
                        panic!("expected invocation");
                    };
                    endpoint
                        .send(&PluginMessage::Invoked {
                            id,
                            output: response,
                        })
                        .unwrap();
                    serve_until_eof(endpoint);
                });
                let result = env.sup.invoke(&launch, "run", InvokeContext::default());
                env.sup.shutdown("demo");
                plugin.join().unwrap();
                if allowed {
                    assert_eq!(result.unwrap(), output);
                } else {
                    assert!(
                        result.is_err(),
                        "ungranted {capability:?} output was accepted"
                    );
                }
            }
        }
    }
}

#[test]
fn inbound_overflow_replies_to_calls_instead_of_dropping_them() {
    let mut config = SupervisorConfig::for_tests();
    config.inbound_queue_capacity = 1;
    let env = Env::with_config(config);
    let (reply_tx, reply_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |endpoint| {
        endpoint.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = endpoint.recv().unwrap() else {
            panic!("expected invocation");
        };
        for call_id in [11, 12] {
            endpoint
                .send(&PluginMessage::Call {
                    id: call_id,
                    call: HostCall::ListPanes,
                })
                .unwrap();
        }
        endpoint.send(&invoked(id)).unwrap();
        while let Ok(message) = endpoint.recv() {
            if matches!(message, HostMessage::Shutdown) {
                break;
            }
            reply_tx.send(message).unwrap();
        }
    });
    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    let reply = reply_rx.recv_timeout(Duration::from_secs(2));
    let inbound = env.sup.drain_inbound("demo");
    env.sup.shutdown("demo");
    plugin.join().unwrap();
    assert!(matches!(
        reply,
        Ok(HostMessage::Reply {
            id: 12,
            result: v2::HostCallResult::Error { .. }
        })
    ));
    assert!(matches!(inbound.as_slice(), [Inbound::Call { id: 11, .. }]));
}

#[test]
fn render_bursts_keep_the_latest_tree_for_each_target() {
    let mut config = SupervisorConfig::for_tests();
    config.inbound_queue_capacity = 1;
    let env = Env::with_config(config);
    let plugin = env.spawn_plugin(|endpoint| {
        endpoint.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = endpoint.recv().unwrap() else {
            panic!("expected invocation");
        };
        for render_id in [11, 12, 13] {
            endpoint
                .send(&PluginMessage::Render {
                    id: render_id,
                    target: RenderTarget::Status,
                    tree: Widget::Sep,
                })
                .unwrap();
        }
        endpoint.send(&invoked(id)).unwrap();
        serve_until_eof(endpoint);
    });
    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    let inbound = env.sup.drain_inbound("demo");
    env.sup.shutdown("demo");
    plugin.join().unwrap();
    assert!(matches!(
        inbound.as_slice(),
        [Inbound::Render { id: 13, .. }]
    ));
}

#[test]
fn closing_supervisor_revokes_live_and_future_connections() {
    let env = Env::new();
    let plugin = env.spawn_plugin(handshake_and_echo);
    let session = env.sup.connect(&spec()).unwrap();
    let instance_id = session.instance_id();
    env.sup.close();
    assert!(session.is_dead());
    assert!(!env.sup.has_grant("demo", Capability::ReadCwd));
    assert!(
        !env.sup
            .has_grant_for_instance(instance_id, Capability::ReadCwd)
    );
    assert!(matches!(
        env.sup.connect(&spec()),
        Err(SessionError::Disconnected)
    ));
    env.sup.shutdown_all();
    plugin.join().unwrap();
}

#[test]
fn closing_supervisor_cancels_a_handshake_already_in_flight() {
    let env = Env::new();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |endpoint| {
        assert!(matches!(
            endpoint.recv().unwrap(),
            HostMessage::Hello { .. }
        ));
        started_tx.send(()).unwrap();
        finish_rx.recv().unwrap();
        endpoint.send(&good_ready()).unwrap();
        serve_until_eof(endpoint);
    });
    thread::scope(|scope| {
        let connecting = scope.spawn(|| env.sup.connect(&spec()));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        env.sup.close();
        finish_tx.send(()).unwrap();
        assert!(matches!(
            connecting.join().unwrap(),
            Err(SessionError::Disconnected)
        ));
    });
    assert!(env.sup.snapshots().is_empty());
    plugin.join().unwrap();
}

#[test]
fn shutdown_all_waits_for_an_unregistered_child_to_finish_teardown() {
    let env = Env::new();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |endpoint| {
        assert!(matches!(
            endpoint.recv().unwrap(),
            HostMessage::Hello { .. }
        ));
        started_tx.send(()).unwrap();
        finish_rx.recv().unwrap();
        endpoint.send(&good_ready()).unwrap();
        serve_until_eof(endpoint);
    });
    thread::scope(|scope| {
        let connecting = scope.spawn(|| env.sup.connect(&spec()));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        env.sup.close();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let supervisor = &env.sup;
        let stopping = scope.spawn(move || {
            supervisor.shutdown_all();
            shutdown_tx.send(()).unwrap();
        });
        // The handshake timeout is two seconds. Shutdown must not return just
        // because there is not yet a registered active session.
        assert!(matches!(
            shutdown_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        finish_tx.send(()).unwrap();
        assert!(matches!(
            connecting.join().unwrap(),
            Err(SessionError::Disconnected)
        ));
        stopping.join().unwrap();
        shutdown_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    });
    plugin.join().unwrap();
    assert!(env.sup.snapshots().is_empty());
}

#[test]
fn monitor_and_grant_reads_do_not_wait_for_process_teardown() {
    struct SlowProcess {
        inner: Box<dyn PluginProcess>,
        entered: mpsc::Sender<()>,
        release: Option<mpsc::Receiver<()>>,
    }
    impl PluginProcess for SlowProcess {
        fn pid(&self) -> Option<u32> {
            self.inner.pid()
        }
        fn cancel_io(&mut self) -> std::io::Result<()> {
            self.inner.cancel_io()
        }
        fn kill(&mut self) -> std::io::Result<()> {
            self.inner.kill()
        }
        fn wait_timeout(&mut self, timeout: Duration) -> bool {
            if !timeout.is_zero() {
                if let Some(release) = self.release.take() {
                    self.entered.send(()).unwrap();
                    release.recv().unwrap();
                }
            }
            self.inner.wait_timeout(timeout)
        }
    }
    struct SlowLauncher {
        inner: MemoryLauncher,
        entered: mpsc::Sender<()>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
    }
    impl Launcher for SlowLauncher {
        fn launch(&self, launch: &LaunchSpec) -> Result<Spawned, SessionError> {
            let mut spawned = self.inner.launch(launch)?;
            spawned.process = Box::new(SlowProcess {
                inner: spawned.process,
                entered: self.entered.clone(),
                release: mutex_lock(&self.release).take(),
            });
            Ok(spawned)
        }
    }
    let (memory, endpoints) = MemoryLauncher::pair();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let launcher = SlowLauncher {
        inner: memory,
        entered: entered_tx,
        release: Mutex::new(Some(release_rx)),
    };
    let mut config = SupervisorConfig::for_tests();
    config.idle = Duration::ZERO;
    config.shutdown_grace = Duration::from_secs(1);
    let supervisor = Supervisor::new(
        config,
        Arc::new(launcher),
        Arc::new(ManualClock::new(1_000)),
    );
    let plugin = thread::spawn(move || handshake_and_echo(&mut endpoints.recv().unwrap()));
    supervisor.connect(&spec()).unwrap();
    thread::scope(|scope| {
        let cleanup = scope.spawn(|| supervisor.tick());
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (monitor_tx, monitor_rx) = mpsc::channel();
        let supervisor = &supervisor;
        scope.spawn(move || {
            let snapshots = supervisor.snapshots();
            let granted = supervisor
                .live_instances()
                .first()
                .is_some_and(|(_, instance_id)| {
                    supervisor.has_grant_for_instance(*instance_id, Capability::ReadCwd)
                });
            monitor_tx.send((snapshots, granted)).unwrap();
        });
        let observed = monitor_rx.recv_timeout(Duration::from_secs(2));
        release_tx.send(()).unwrap();
        cleanup.join().unwrap();
        let (snapshots, granted) = observed.expect("UI reads must not wait for process exit");
        assert!(!granted);
        assert_eq!(snapshots[0].state, ConnectionState::ShuttingDown);
    });
    plugin.join().unwrap();
}

#[test]
fn render_coalescing_does_not_cross_a_host_call() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|endpoint| {
        endpoint.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = endpoint.recv().unwrap() else {
            panic!("expected invocation");
        };
        endpoint
            .send(&PluginMessage::Render {
                id: 11,
                target: RenderTarget::Status,
                tree: Widget::Sep,
            })
            .unwrap();
        endpoint
            .send(&PluginMessage::Call {
                id: 12,
                call: HostCall::ListPanes,
            })
            .unwrap();
        endpoint
            .send(&PluginMessage::Render {
                id: 13,
                target: RenderTarget::Status,
                tree: Widget::Sep,
            })
            .unwrap();
        endpoint.send(&invoked(id)).unwrap();
        serve_until_eof(endpoint);
    });
    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    let inbound = env.sup.drain_inbound("demo");
    env.sup.shutdown("demo");
    plugin.join().unwrap();
    assert!(matches!(
        inbound.as_slice(),
        [
            Inbound::Render { id: 11, .. },
            Inbound::Call { id: 12, .. },
            Inbound::Render { id: 13, .. }
        ]
    ));
}

#[test]
fn undeliverable_replies_disconnect_instead_of_leaving_calls_waiting() {
    let env = Env::with_pipe_cap(SupervisorConfig::for_tests(), 1);
    let (release_tx, release_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |endpoint| {
        endpoint.handshake(&good_ready()).unwrap();
        release_rx.recv().unwrap();
    });
    let session = env.sup.connect(&spec()).unwrap();
    let mut backpressured = false;
    for id in 1..100 {
        if session.reply(id, v2::HostCallResult::Ok).is_err() {
            backpressured = true;
            break;
        }
    }
    let disconnected = session.is_dead();
    release_tx.send(()).unwrap();
    env.sup.shutdown("demo");
    plugin.join().unwrap();
    assert!(backpressured);
    assert!(disconnected);
}

#[test]
fn reply_to_unknown_id_is_ignored() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        ep.send(&invoked(999_999)).unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn duplicate_reply_is_ignored() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&invoked(id)).unwrap();
        ep.send(&invoked(id)).unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected second invoke");
        };
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    let spec = spec();
    env.sup
        .invoke(&spec, "first", InvokeContext::default())
        .unwrap();
    env.sup
        .invoke(&spec, "second", InvokeContext::default())
        .unwrap();
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn plugin_death_wakes_pending_waiters() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        // Drop the endpoint without replying: the plugin "crashed".
    });

    let pending = env
        .sup
        .begin_invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    let err = pending.wait(Duration::from_secs(2)).unwrap_err();
    assert_eq!(err, SessionError::Disconnected);
    let _ = plugin.join();
}

#[test]
fn reply_after_death_does_not_hang() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    env.sup.shutdown("demo");
    let err = env
        .sup
        .reply(
            "demo",
            1,
            v2::HostCallResult::Error {
                message: "late".into(),
            },
        )
        .unwrap_err();
    assert_eq!(err, SessionError::Disconnected);
    let _ = plugin.join();
}

#[test]
fn restart_backoff_caps_at_ceiling_then_disables() {
    let mut config = SupervisorConfig::for_tests();
    config.max_restarts = 5;
    config.backoff_initial = Duration::from_millis(100);
    config.backoff_ceiling = Duration::from_millis(400);
    let env = Env::with_config(config);
    let plugin = env.spawn_loop(|ep| {
        let _ = ep.handshake(&ready(99, vec![Capability::ReadCwd]));
    });

    let spec = spec();
    let mut delays = Vec::new();
    for _ in 0..4 {
        let err = env.sup.connect(&spec).unwrap_err();
        assert!(matches!(err, SessionError::VersionMismatch { plugin: 99 }));
        match env.sup.connect(&spec).unwrap_err() {
            SessionError::Backoff { until_ms } => {
                delays.push(until_ms.saturating_sub(env.clock.now_ms()));
                env.clock.set(until_ms);
            }
            other => panic!("expected Backoff, got {other:?}"),
        }
    }
    assert_eq!(delays, vec![100, 200, 400, 400], "doubles then caps");

    env.clock.advance(400);
    let err = env.sup.connect(&spec).unwrap_err();
    assert!(matches!(err, SessionError::VersionMismatch { plugin: 99 }));
    let err = env.sup.connect(&spec).unwrap_err();
    assert_eq!(err, SessionError::Disabled { restarts: 5 });

    drop(env.sup);
    let _ = plugin.join();
}

#[test]
fn host_pinned_service_survives_idle_but_can_still_be_stopped() {
    let env = Env::new();
    let plugin = env.spawn_plugin(handshake_and_echo);
    let mut launch = spec();
    launch.keep_alive = true;
    let session = env.sup.connect(&launch).unwrap();
    let instance = session.instance_id();
    env.clock.advance(600_000);
    env.sup.tick();
    assert_eq!(
        env.sup.snapshot("demo").unwrap().state,
        ConnectionState::Live
    );
    assert!(
        env.sup
            .has_grant_for_instance(instance, Capability::ReadCwd)
    );
    env.sup.shutdown("demo");
    assert!(env.sup.live_instances().is_empty());
    plugin.join().unwrap();
}

#[test]
fn idle_eviction_shuts_down_resident_plugin() {
    let env = Env::new();
    let plugin = env.spawn_plugin(handshake_and_echo);
    let session = env.sup.connect(&spec()).unwrap();
    let instance_id = session.instance_id();
    assert!(env.sup.snapshot("demo").is_some());
    assert!(
        env.sup
            .has_grant_for_instance(instance_id, Capability::ReadCwd)
    );
    env.clock.advance(1_000);
    env.sup.tick();
    assert!(
        env.sup.snapshot("demo").is_none()
            || env
                .sup
                .snapshot("demo")
                .is_some_and(|s| s.state != ConnectionState::Live)
    );
    assert!(
        !env.sup
            .has_grant_for_instance(instance_id, Capability::ReadCwd)
    );
    assert!(env.sup.live_instances().is_empty());
    // A fresh connect after eviction must handshake again.
    env.clock.advance(10_000);
    let plugin2 = env.spawn_plugin(handshake_and_echo);
    env.sup.connect(&spec()).expect("relaunch after idle");
    env.sup.shutdown("demo");
    let _ = plugin.join();
    let _ = plugin2.join();
}

#[test]
fn resident_reuses_connection_ondemand_does_not() {
    let env = Env::new();
    let hellos = Arc::new(Mutex::new(0_u32));
    let hellos_p = Arc::clone(&hellos);
    let plugin = env.spawn_loop(move |ep| {
        if let Ok(HostMessage::Hello { .. }) = ep.recv() {
            *mutex_lock(&hellos_p) += 1;
            let _ = ep.send(&good_ready());
            serve_until_eof(ep);
        }
    });

    let mut resident = spec();
    resident.lifecycle = PluginLifecycle::Resident;
    env.sup.connect(&resident).unwrap();
    env.sup.connect(&resident).unwrap();
    assert_eq!(*mutex_lock(&hellos), 1, "resident caches the handshake");

    env.sup.shutdown("demo");
    env.clock.advance(10_000);

    let mut on_demand = spec();
    on_demand.lifecycle = PluginLifecycle::OnDemand;
    // OnDemand is not cached: dropping the session (or invoke's teardown)
    // must launch a new process next time.
    drop(env.sup.connect(&on_demand).unwrap());
    drop(env.sup.connect(&on_demand).unwrap());
    assert_eq!(*mutex_lock(&hellos), 3, "on-demand launches every time");

    drop(env.sup);
    let _ = plugin.join();
}

#[test]
fn concurrent_ondemand_calls_and_replies_route_by_exact_session_identity() {
    let env = Env::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (reply_tx, reply_rx) = mpsc::channel();
    let spawn = |label: &'static str| {
        let ready_tx = ready_tx.clone();
        let reply_tx = reply_tx.clone();
        env.spawn_plugin(move |ep| {
            ep.handshake(&good_ready()).unwrap();
            let HostMessage::Invoke { id: invoke_id, .. } = ep.recv().unwrap() else {
                panic!("expected invoke");
            };
            let call_id = if label == "a" { 101 } else { 202 };
            ep.send(&PluginMessage::Call {
                id: call_id,
                call: HostCall::ListPanes,
            })
            .unwrap();
            ready_tx.send(label).unwrap();
            let HostMessage::Reply { id, result } = ep.recv().unwrap() else {
                panic!("expected reply");
            };
            reply_tx.send((label, id, result)).unwrap();
            ep.send(&invoked(invoke_id)).unwrap();
            serve_until_eof(ep);
        })
    };
    let plugin_a = spawn("a");
    let plugin_b = spawn("b");

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;

    let pending_a = env
        .sup
        .begin_invoke(&spec, "run-a", InvokeContext::default())
        .unwrap();
    let pending_b = env
        .sup
        .begin_invoke(&spec, "run-b", InvokeContext::default())
        .unwrap();

    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let mut inbound = Vec::new();
    wait_until(
        || {
            inbound.extend(env.sup.drain_all_inbound());
            inbound.len() == 2
        },
        "expected one host call from each on-demand session",
    );
    assert_eq!(inbound.len(), 2, "expected one host call from each invoke");
    assert!(
        inbound.iter().all(|envelope| envelope.plugin_id == "demo"
            && matches!(
                envelope.message,
                Inbound::Call {
                    call: HostCall::ListPanes,
                    ..
                }
            )),
        "unexpected inbound envelopes: {inbound:?}"
    );
    assert_ne!(inbound[0].instance_id, inbound[1].instance_id);

    for envelope in &inbound {
        let Inbound::Call { id, .. } = envelope.message else {
            panic!("expected call envelope");
        };
        env.sup
            .reply_to_instance(
                envelope.instance_id,
                id,
                v2::HostCallResult::Error {
                    message: format!("reply-{}", id),
                },
            )
            .unwrap();
    }

    assert_eq!(
        pending_a.wait(Duration::from_secs(2)).unwrap(),
        Output::Ignore
    );
    assert_eq!(
        pending_b.wait(Duration::from_secs(2)).unwrap(),
        Output::Ignore
    );
    wait_until(
        || env.sup.live_instances().is_empty(),
        "on-demand sessions should unregister after invoke completion",
    );

    let mut replies = vec![
        reply_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        reply_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
    ];
    replies.sort_by_key(|(label, _, _)| *label);
    assert!(matches!(
        replies.as_slice(),
        [
            ("a", 101, v2::HostCallResult::Error { .. }),
            ("b", 202, v2::HostCallResult::Error { .. })
        ]
    ));

    plugin_a.join().unwrap();
    plugin_b.join().unwrap();
}

#[test]
fn on_demand_grants_are_checked_per_active_instance() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&ready(
            v2::PROTOCOL_VERSION,
            vec![Capability::ReadCwd, Capability::HostCallListPanes],
        ))
        .unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&PluginMessage::Call {
            id: 77,
            call: HostCall::ListPanes,
        })
        .unwrap();
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    spec.declared_capabilities
        .insert(Capability::HostCallListPanes);
    spec.granted = vec![Capability::ReadCwd, Capability::HostCallListPanes];

    let pending = env
        .sup
        .begin_invoke(&spec, "run", InvokeContext::default())
        .unwrap();
    let envelope = loop {
        if let Some(envelope) = env.sup.drain_all_inbound().into_iter().next() {
            break envelope;
        }
        thread::yield_now();
    };
    assert!(matches!(
        envelope.message,
        Inbound::Call {
            id: 77,
            call: HostCall::ListPanes,
        }
    ));
    assert!(
        env.sup
            .has_grant_for_instance(envelope.instance_id, Capability::HostCallListPanes)
    );
    assert!(!env.sup.has_grant("demo", Capability::HostCallListPanes));

    drop(envelope);
    assert_eq!(
        pending.wait(Duration::from_secs(2)).unwrap(),
        Output::Ignore
    );
    plugin.join().unwrap();
}

#[test]
fn shutdown_all_cancels_all_active_ondemand_sessions() {
    let env = Env::new();
    let (invoked_tx, invoked_rx) = mpsc::channel();
    let plugin_a = env.spawn_plugin({
        let invoked_tx = invoked_tx.clone();
        move |ep| {
            ep.handshake(&good_ready()).unwrap();
            let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
                panic!("expected invoke");
            };
            invoked_tx.send("a").unwrap();
            serve_until_eof(ep);
        }
    });
    let plugin_b = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        invoked_tx.send("b").unwrap();
        serve_until_eof(ep);
    });

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    let pending_a = env
        .sup
        .begin_invoke(&spec, "run-a", InvokeContext::default())
        .unwrap();
    let pending_b = env
        .sup
        .begin_invoke(&spec, "run-b", InvokeContext::default())
        .unwrap();

    let observed = vec![
        invoked_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        invoked_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
    ];
    assert!(
        observed.contains(&"a") && observed.contains(&"b"),
        "expected both plugin endpoints to observe invoke before shutdown_all: {observed:?}"
    );
    wait_until(
        || env.sup.live_instances().len() == 2,
        "both on-demand sessions should be active before shutdown_all",
    );
    env.sup.shutdown_all();
    assert_eq!(
        pending_a.wait(Duration::from_secs(2)).unwrap_err(),
        SessionError::Disconnected
    );
    assert_eq!(
        pending_b.wait(Duration::from_secs(2)).unwrap_err(),
        SessionError::Disconnected
    );
    assert!(env.sup.live_instances().is_empty());

    plugin_a.join().unwrap();
    plugin_b.join().unwrap();
}

#[test]
fn close_rejects_late_active_registration_from_inflight_ondemand_connect() {
    let env = Env::new();
    let (started_tx, started_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |endpoint| {
        assert!(matches!(
            endpoint.recv().unwrap(),
            HostMessage::Hello { .. }
        ));
        started_tx.send(()).unwrap();
        finish_rx.recv().unwrap();
        endpoint.send(&good_ready()).unwrap();
        serve_until_eof(endpoint);
    });

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    thread::scope(|scope| {
        let pending = scope.spawn(|| env.sup.begin_invoke(&spec, "run", InvokeContext::default()));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        env.sup.close();
        finish_tx.send(()).unwrap();
        assert!(matches!(
            pending.join().unwrap(),
            Err(SessionError::Disconnected)
        ));
    });
    assert!(env.sup.live_instances().is_empty());
    assert!(env.sup.drain_all_inbound().is_empty());
    plugin.join().unwrap();
}

#[test]
fn disconnect_cancels_all_active_sessions_for_plugin_id() {
    let env = Env::new();
    let (invoked_tx, invoked_rx) = mpsc::channel();
    let plugin_a = env.spawn_plugin({
        let invoked_tx = invoked_tx.clone();
        move |ep| {
            ep.handshake(&good_ready()).unwrap();
            let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
                panic!("expected invoke");
            };
            invoked_tx.send("a").unwrap();
            serve_until_eof(ep);
        }
    });
    let plugin_b = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        invoked_tx.send("b").unwrap();
        serve_until_eof(ep);
    });

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    let pending_a = env
        .sup
        .begin_invoke(&spec, "run-a", InvokeContext::default())
        .unwrap();
    let pending_b = env
        .sup
        .begin_invoke(&spec, "run-b", InvokeContext::default())
        .unwrap();

    let observed = vec![
        invoked_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        invoked_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
    ];
    assert!(
        observed.contains(&"a") && observed.contains(&"b"),
        "expected both plugin endpoints to observe invoke before disconnect: {observed:?}"
    );
    wait_until(
        || env.sup.live_instances().len() == 2,
        "both on-demand sessions should be active before disconnect",
    );
    let sessions = env.sup.disconnect("demo");
    assert_eq!(sessions.len(), 2);
    for session in &sessions {
        session.teardown(env.sup.config().shutdown_grace);
    }
    assert!(env.sup.live_instances().is_empty());
    assert_eq!(
        pending_a.wait(Duration::from_secs(2)).unwrap_err(),
        SessionError::Disconnected
    );
    assert_eq!(
        pending_b.wait(Duration::from_secs(2)).unwrap_err(),
        SessionError::Disconnected
    );

    plugin_a.join().unwrap();
    plugin_b.join().unwrap();
}

#[test]
fn same_plugin_same_host_call_id_does_not_swap_replies_between_instances() {
    let env = Env::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (reply_tx, reply_rx) = mpsc::channel();
    let spawn = |label: &'static str| {
        let ready_tx = ready_tx.clone();
        let reply_tx = reply_tx.clone();
        env.spawn_plugin(move |ep| {
            ep.handshake(&good_ready()).unwrap();
            let HostMessage::Invoke { id: invoke_id, .. } = ep.recv().unwrap() else {
                panic!("expected invoke");
            };
            ep.send(&PluginMessage::Call {
                id: 500,
                call: HostCall::ListPanes,
            })
            .unwrap();
            ready_tx.send(label).unwrap();
            let HostMessage::Reply { id, result } = ep.recv().unwrap() else {
                panic!("expected reply");
            };
            reply_tx.send((label, id, result)).unwrap();
            ep.send(&invoked(invoke_id)).unwrap();
            serve_until_eof(ep);
        })
    };
    let plugin_a = spawn("a");
    let plugin_b = spawn("b");

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    let pending_a = env
        .sup
        .begin_invoke(&spec, "run-a", InvokeContext::default())
        .unwrap();
    let pending_b = env
        .sup
        .begin_invoke(&spec, "run-b", InvokeContext::default())
        .unwrap();

    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let mut inbound = Vec::new();
    wait_until(
        || {
            inbound.extend(env.sup.drain_all_inbound());
            inbound.len() == 2
        },
        "expected one host call from each on-demand session",
    );
    assert_eq!(inbound.len(), 2);
    assert!(inbound.iter().all(|envelope| matches!(
        envelope.message,
        Inbound::Call {
            id: 500,
            call: HostCall::ListPanes,
        }
    )));
    assert_ne!(inbound[0].instance_id, inbound[1].instance_id);

    for envelope in &inbound {
        env.sup
            .reply_to_instance(
                envelope.instance_id,
                500,
                v2::HostCallResult::Error {
                    message: format!("reply-{}", envelope.instance_id),
                },
            )
            .unwrap();
    }

    assert_eq!(
        pending_a.wait(Duration::from_secs(2)).unwrap(),
        Output::Ignore
    );
    assert_eq!(
        pending_b.wait(Duration::from_secs(2)).unwrap(),
        Output::Ignore
    );

    let mut replies = vec![
        reply_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        reply_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
    ];
    replies.sort_by_key(|(label, _, _)| *label);
    assert!(matches!(
        replies.as_slice(),
        [
            ("a", 500, v2::HostCallResult::Error { .. }),
            ("b", 500, v2::HostCallResult::Error { .. })
        ]
    ));

    plugin_a.join().unwrap();
    plugin_b.join().unwrap();
}

#[test]
fn garbage_line_does_not_panic_and_session_continues() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        ep.send_raw("NOT JSON {{{").unwrap();
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    let snap = env.sup.snapshot("demo").unwrap();
    assert!(snap.malformed_lines >= 1);
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn oversized_line_fails_the_connection_without_panic() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let huge = "x".repeat(2048);
        let _ = ep.send_raw(&huge);
        // Connection should die; further recv may EOF.
        let _ = ep.recv();
    });
    env.sup.connect(&spec()).unwrap();

    wait_until(
        || {
            env.sup
                .snapshot("demo")
                .is_some_and(|s| s.state == ConnectionState::Dead || s.malformed_lines >= 1)
                || env.sup.snapshot("demo").is_none()
        },
        "oversized line should fail the session",
    );
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn stderr_ring_keeps_last_n_and_never_blocks() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        for i in 0..20 {
            ep.write_stderr(&format!("line-{i}")).unwrap();
        }
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    env.sup
        .invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    wait_until(
        || {
            env.sup
                .snapshot("demo")
                .is_some_and(|s| s.stderr.last().is_some_and(|l| l == "line-19"))
        },
        "stderr drain should keep the last line",
    );
    let stderr = env.sup.snapshot("demo").unwrap().stderr;
    assert_eq!(stderr.len(), 5, "ring retains last N");
    assert_eq!(
        stderr,
        vec!["line-15", "line-16", "line-17", "line-18", "line-19"]
    );
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn graceful_shutdown_delivers_shutdown_then_forced_kill_unblocks() {
    let env = Env::new();
    let (seen_tx, seen_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        loop {
            match ep.recv() {
                Ok(HostMessage::Shutdown) => {
                    let _ = seen_tx.send("shutdown");
                    break;
                }
                Ok(HostMessage::Invoke { id, .. }) => {
                    let _ = ep.send(&invoked(id));
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = seen_tx.send("eof");
                    break;
                }
            }
        }
    });

    let session = env.sup.connect(&spec()).unwrap();
    session.request_shutdown().unwrap();
    let seen = seen_rx.recv().expect("plugin should observe shutdown");
    assert_eq!(seen, "shutdown");
    env.sup.shutdown("demo");
    let _ = plugin.join();

    // Forced path: the plugin ignores Shutdown and only leaves on EOF/kill.
    let env = Env::new();
    let (seen_tx, seen_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        loop {
            match ep.recv() {
                Ok(HostMessage::Shutdown) => {
                    let _ = seen_tx.send("ignored-shutdown");
                    // Keep running — this is the wedged-plugin case.
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = seen_tx.send("killed");
                    break;
                }
            }
        }
    });
    env.sup.connect(&spec()).unwrap();
    env.sup.shutdown("demo");
    let mut events = Vec::new();
    while let Ok(ev) = seen_rx.recv() {
        events.push(ev);
        if ev == "killed" {
            break;
        }
    }
    assert!(
        events.contains(&"killed"),
        "kill must unblock a plugin that ignores Shutdown: {events:?}"
    );
    let _ = plugin.join();
}

#[test]
fn write_backpressure_does_not_stall_the_caller() {
    let mut config = SupervisorConfig::for_tests();
    config.write_queue_capacity = 2;
    let env = Env::with_pipe_cap(config, 1);
    let (hold_tx, hold_rx) = mpsc::channel::<()>();
    let plugin = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        // Do not read: the host pipe must fill. Stay alive until released.
        let _ = hold_rx.recv();
    });

    let spec = spec();
    env.sup.connect(&spec).unwrap();
    let mut saw_backpressure = false;
    for _ in 0..32 {
        match env.sup.begin_invoke(&spec, "run", InvokeContext::default()) {
            Ok(_) => {}
            Err(SessionError::Backpressure) => {
                saw_backpressure = true;
                break;
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        saw_backpressure,
        "a plugin that will not read must backpressure"
    );
    let _ = hold_tx.send(());
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn snapshots_report_inflight_and_restarts() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        serve_until_eof(ep);
    });

    let pending = env
        .sup
        .begin_invoke(&spec(), "run", InvokeContext::default())
        .unwrap();
    wait_until(
        || env.sup.snapshot("demo").is_some_and(|s| s.in_flight == 1),
        "in-flight should count the pending invoke",
    );
    drop(pending);
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn dropping_pending_invoke_removes_waiter_without_stopping_resident() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        let HostMessage::Invoke { id, .. } = ep.recv().unwrap() else {
            panic!("expected second invoke");
        };
        ep.send(&invoked(id)).unwrap();
        serve_until_eof(ep);
    });

    let spec = spec();
    let session = env.sup.connect(&spec).unwrap();
    let pending = env
        .sup
        .begin_invoke(&spec, "run", InvokeContext::default())
        .unwrap();
    wait_until(
        || env.sup.snapshot("demo").is_some_and(|s| s.in_flight == 1),
        "resident invoke should be registered as in-flight",
    );
    drop(pending);
    wait_until(
        || env.sup.snapshot("demo").is_some_and(|s| s.in_flight == 0),
        "dropping the waiter must remove it from the session",
    );
    assert!(!session.is_dead(), "resident session must stay alive");

    env.sup
        .invoke(&spec, "run-again", InvokeContext::default())
        .unwrap();
    env.sup.shutdown("demo");
    plugin.join().unwrap();
}

#[test]
fn dropping_pending_invoke_cleans_up_ondemand_session() {
    let env = Env::new();
    let plugin = env.spawn_plugin(|ep| {
        ep.handshake(&good_ready()).unwrap();
        match ep.recv() {
            Ok(HostMessage::Invoke { .. }) | Err(_) => {}
            Ok(other) => panic!("expected invoke, got {other:?}"),
        }
    });

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    let pending = env
        .sup
        .begin_invoke(&spec, "run", InvokeContext::default())
        .unwrap();
    wait_until(
        || env.sup.live_instances().len() == 1,
        "on-demand invoke should register an active session",
    );
    drop(pending);
    wait_until(
        || env.sup.live_instances().is_empty(),
        "dropping an on-demand waiter must clean up the active session",
    );

    plugin.join().unwrap();
}

#[test]
fn snapshots_hold_at_most_one_entry_per_plugin_id() {
    // Load-bearing for the shell's staleness sweep
    // (`app_shell/plugins.rs::poll_plugin_inbound`): it derives the live set
    // from these snapshots and marks every surface whose plugin is absent
    // from it. That single sweep is only equivalent to also marking each
    // non-live snapshot individually if a plugin_id cannot appear twice --
    // otherwise a plugin could be reported both Live and Dead in one batch,
    // land in `live`, and the sweep would spare surfaces it should mark.
    //
    // `snapshots()` unions `live` and `health`, both keyed by plugin_id, and
    // skips health entries already in `live`. This pins that union with one
    // dead and one live plugin present at the same time.
    let env = Env::new();
    let dead = env.spawn_plugin(|ep| {
        handshake_events(ep, "dead", EventFilter::default());
    });
    env.sup.connect(&event_spec("dead")).unwrap();
    wait_until(
        || {
            env.sup
                .snapshot("dead")
                .is_some_and(|s| s.state == ConnectionState::Dead)
        },
        "plugin drop should mark the session dead",
    );
    let live_plugin = env.spawn_plugin(|ep| {
        handshake_events(ep, "live", EventFilter::default());
        serve_until_eof(ep);
    });
    env.sup.connect(&event_spec("live")).unwrap();

    let snaps = env.sup.snapshots();
    let ids: Vec<&String> = snaps.iter().map(|s| &s.plugin_id).collect();
    let unique: BTreeSet<&&String> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "duplicate plugin_id in {ids:?}");

    // The batch must actually contain both states, or the assertion above is
    // vacuous and proves nothing about the union.
    let live_set: BTreeSet<&String> = snaps
        .iter()
        .filter(|s| s.state == ConnectionState::Live)
        .map(|s| &s.plugin_id)
        .collect();
    assert!(
        live_set.contains(&"live".to_string()),
        "expected the live plugin in {snaps:?}"
    );
    assert!(
        !live_set.contains(&"dead".to_string()),
        "a dead plugin must not appear in the live set: {snaps:?}"
    );

    env.sup.shutdown_all();
    let _ = dead.join();
    let _ = live_plugin.join();
}

#[test]
fn snapshots_include_active_ondemand_instance_while_it_is_alive() {
    let env = Env::new();
    let (invoked_tx, invoked_rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
            panic!("expected invoke");
        };
        invoked_tx.send(()).unwrap();
        serve_until_eof(ep);
    });

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    let pending = env
        .sup
        .begin_invoke(&spec, "run", InvokeContext::default())
        .unwrap();

    invoked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let instance_id = wait_until_value(
        || {
            env.sup
                .live_instances()
                .into_iter()
                .next()
                .map(|(_, instance_id)| instance_id)
        },
        "on-demand instance should become active",
    );
    let snaps = env.sup.snapshots();
    assert!(
        snaps
            .iter()
            .any(|snap| snap.instance_id == instance_id && snap.state == ConnectionState::Live),
        "expected live on-demand instance in snapshots: {snaps:?}"
    );

    drop(pending);
    wait_until(
        || env.sup.live_instances().is_empty(),
        "dropping on-demand waiter should unregister the active instance",
    );
    plugin.join().unwrap();
}

#[test]
fn push_action_targets_exact_active_instance_for_ondemand_plugin() {
    let env = Env::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    let spawn = |label: &'static str| {
        let ready_tx = ready_tx.clone();
        let event_tx = event_tx.clone();
        env.spawn_plugin(move |ep| {
            let HostMessage::Hello {
                plugin_instance_id, ..
            } = ep.handshake(&good_ready()).unwrap()
            else {
                panic!("expected hello");
            };
            let HostMessage::Invoke { .. } = ep.recv().unwrap() else {
                panic!("expected invoke");
            };
            ready_tx.send((label, plugin_instance_id)).unwrap();
            loop {
                match ep.recv() {
                    Ok(HostMessage::Action {
                        block_id,
                        action,
                        arg,
                        ..
                    }) => {
                        event_tx
                            .send((label, plugin_instance_id, Some((block_id, action, arg))))
                            .unwrap();
                    }
                    Ok(HostMessage::Shutdown) | Err(_) => {
                        event_tx.send((label, plugin_instance_id, None)).unwrap();
                        break;
                    }
                    Ok(other) => panic!("expected action or shutdown, got {other:?}"),
                }
            }
        })
    };
    let plugin_a = spawn("a");
    let plugin_b = spawn("b");

    let mut spec = spec();
    spec.lifecycle = PluginLifecycle::OnDemand;
    let pending_a = env
        .sup
        .begin_invoke(&spec, "run-a", InvokeContext::default())
        .unwrap();
    let pending_b = env
        .sup
        .begin_invoke(&spec, "run-b", InvokeContext::default())
        .unwrap();

    let mut ready = [
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
    ];
    ready.sort_by_key(|(label, _)| *label);
    assert_ne!(ready[0].1, ready[1].1);

    let target_block = uuid::Uuid::from_u128(77);
    env.sup
        .push_action_to_instance(ready[1].1, target_block, "go".into(), Some("now".into()))
        .unwrap();

    let (label, instance_id, payload) = event_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (block_id, action, arg) =
        payload.expect("action must arrive before invocation is canceled");
    let mut action_events = vec![(label, instance_id, block_id, action, arg)];
    drop(pending_a);
    drop(pending_b);
    let mut closes = 0usize;
    while closes < 2 {
        let (label, instance_id, payload) = event_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        match payload {
            Some((block_id, action, arg)) => {
                action_events.push((label, instance_id, block_id, action, arg));
            }
            None => closes += 1,
        }
    }
    assert_eq!(
        action_events.len(),
        1,
        "unexpected action events: {action_events:?}"
    );
    let delivered = &action_events[0];
    assert_eq!(delivered.0, "b");
    assert_eq!(delivered.1, ready[1].1);
    assert_eq!(delivered.2, target_block);
    assert_eq!(delivered.3, "go");
    assert_eq!(delivered.4.as_deref(), Some("now"));
    plugin_a.join().unwrap();
    plugin_b.join().unwrap();
}

#[test]
fn declared_capabilities_include_resident_from_lifecycle() {
    let manifest = crate::PluginManifest {
        id: "x".into(),
        name: "X".into(),
        version: "1".into(),
        api_version: crate::PLUGIN_API_VERSION,
        description: String::new(),
        enabled: true,
        lifecycle: PluginLifecycle::Resident,
        binary: "x".into(),
        args: vec![],
        permissions: BTreeSet::new(),
        commands: vec![crate::PluginCommand {
            id: "run".into(),
            title: "Run".into(),
            description: String::new(),
            keywords: vec![],
            permissions: BTreeSet::from([crate::Permission::ReadCwd]),
            timeout_secs: None,
        }],
    };
    let caps = declared_capabilities(&manifest);
    assert!(caps.contains(&Capability::ReadCwd));
    assert!(caps.contains(&Capability::Resident));
    assert!(!caps.contains(&Capability::SubscribeEvents));
}

#[test]
fn declared_capabilities_include_plugin_level_v2_permissions() {
    // Ready.requests is checked against this set. If plugin.json cannot
    // name subscribe_events / render_block, a v2 plugin cannot load.
    let manifest = crate::PluginManifest {
        id: "demo".into(),
        name: "Demo".into(),
        version: "1".into(),
        api_version: crate::PLUGIN_API_VERSION,
        description: String::new(),
        enabled: true,
        lifecycle: PluginLifecycle::Resident,
        binary: "x".into(),
        args: vec![],
        permissions: BTreeSet::from([
            crate::Permission::SubscribeEvents,
            crate::Permission::RenderBlock,
            crate::Permission::ReadCwd,
        ]),
        commands: vec![],
    };
    let caps = declared_capabilities(&manifest);
    assert!(caps.contains(&Capability::SubscribeEvents));
    assert!(caps.contains(&Capability::RenderBlock));
    assert!(caps.contains(&Capability::ReadCwd));
    assert!(caps.contains(&Capability::Resident));
    assert!(!caps.contains(&Capability::RenderPanel));
}

fn connect_subscriber(
    env: &Env,
    id: &str,
    filter: EventFilter,
    sink: mpsc::Sender<HostEvent>,
) -> JoinHandle<()> {
    let id_owned = id.to_string();
    let plugin = env.spawn_plugin(move |ep| {
        handshake_events(ep, &id_owned, filter);
        collect_until_invoke(ep, &sink);
    });
    env.sup
        .connect(&event_spec(id))
        .expect("connect subscriber");
    plugin
}

fn barrier_invoke(env: &Env, id: &str) {
    env.sup
        .invoke(&event_spec(id), "run", InvokeContext::default())
        .expect("barrier invoke");
}

#[test]
fn plugin_without_subscribe_events_receives_nothing() {
    let env = Env::new();
    let (tx, rx) = mpsc::channel();
    let plugin = env.spawn_plugin(move |ep| {
        ep.handshake(&good_ready()).unwrap();
        collect_until_invoke(ep, &tx);
    });
    env.sup.connect(&spec()).unwrap();
    let report = env.sup.broadcast(run_started(pane(1), "echo hi"));
    assert_eq!(report.delivered(), 0);
    assert_eq!(report.filtered(), 1);
    barrier_invoke(&env, "demo");
    assert!(
        rx.try_recv().is_err(),
        "a plugin without SubscribeEvents must never see a HostEvent"
    );
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn empty_filter_receives_everything() {
    let env = Env::new();
    let (tx, rx) = mpsc::channel();
    let plugin = connect_subscriber(&env, "demo", EventFilter::default(), tx);
    env.sup.broadcast(run_started(pane(1), "echo a"));
    env.sup.broadcast(run_finished(pane(1)));
    env.sup.broadcast(HostEvent::PaneFocused { pane: pane(2) });
    barrier_invoke(&env, "demo");
    let kinds: Vec<_> = rx.try_iter().map(|e| e.kind()).collect();
    assert_eq!(
        kinds,
        [
            EventKind::RunStarted,
            EventKind::RunFinished,
            EventKind::PaneFocused
        ]
    );
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn pane_filter_excludes_other_panes() {
    let env = Env::new();
    let (tx, rx) = mpsc::channel();
    let filter = EventFilter {
        panes: vec![pane(1)],
        kinds: vec![],
    };
    let plugin = connect_subscriber(&env, "demo", filter, tx);
    env.sup.broadcast(run_started(pane(1), "echo a"));
    env.sup.broadcast(run_started(pane(2), "echo b"));
    barrier_invoke(&env, "demo");
    let panes: Vec<_> = rx.try_iter().map(|e| e.pane()).collect();
    assert_eq!(panes, [pane(1)]);
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn kind_filter_excludes_other_kinds() {
    let env = Env::new();
    let (tx, rx) = mpsc::channel();
    let filter = EventFilter {
        panes: vec![],
        kinds: vec![EventKind::RunFinished],
    };
    let plugin = connect_subscriber(&env, "demo", filter, tx);
    env.sup.broadcast(run_started(pane(1), "echo a"));
    env.sup.broadcast(run_finished(pane(1)));
    barrier_invoke(&env, "demo");
    let kinds: Vec<_> = rx.try_iter().map(|e| e.kind()).collect();
    assert_eq!(kinds, [EventKind::RunFinished]);
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn multiple_subscribers_each_get_their_filtered_view() {
    let env = Env::new();
    let (tx_a, rx_a) = mpsc::channel();
    let (tx_b, rx_b) = mpsc::channel();
    let a = connect_subscriber(
        &env,
        "alpha",
        EventFilter {
            panes: vec![pane(1)],
            kinds: vec![],
        },
        tx_a,
    );
    let b = connect_subscriber(
        &env,
        "beta",
        EventFilter {
            panes: vec![],
            kinds: vec![EventKind::RunFinished],
        },
        tx_b,
    );
    env.sup.broadcast(run_started(pane(1), "echo a"));
    env.sup.broadcast(run_started(pane(2), "echo b"));
    env.sup.broadcast(run_finished(pane(2)));
    barrier_invoke(&env, "alpha");
    barrier_invoke(&env, "beta");
    let a_kinds: Vec<_> = rx_a.try_iter().map(|e| e.kind()).collect();
    let b_kinds: Vec<_> = rx_b.try_iter().map(|e| e.kind()).collect();
    assert_eq!(a_kinds, [EventKind::RunStarted]);
    assert_eq!(b_kinds, [EventKind::RunFinished]);
    env.sup.shutdown_all();
    let _ = a.join();
    let _ = b.join();
}

#[test]
fn per_plugin_order_is_preserved_under_interleaved_emissions() {
    let env = Env::new();
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sink = collected.clone();
    let plugin = env.spawn_plugin(move |ep| {
        handshake_events(ep, "demo", EventFilter::default());
        while let Ok(msg) = ep.recv() {
            match msg {
                HostMessage::Event { event, .. } => mutex_lock(&sink).push(event),
                HostMessage::Shutdown | HostMessage::Hello { .. } => break,
                _ => {}
            }
        }
    });
    env.sup.connect(&event_spec("demo")).unwrap();
    let a = pane(1);
    let b = pane(2);
    env.sup.broadcast(run_started(a, "echo a"));
    env.sup.broadcast(run_started(b, "echo b"));
    env.sup.broadcast(run_finished(a));
    env.sup.broadcast(run_finished(b));
    wait_until(
        || mutex_lock(&collected).len() == 4,
        "all four events should arrive",
    );
    let kinds: Vec<_> = mutex_lock(&collected)
        .iter()
        .map(|e| (e.pane(), e.kind()))
        .collect();
    assert_eq!(
        kinds,
        [
            (a, EventKind::RunStarted),
            (b, EventKind::RunStarted),
            (a, EventKind::RunFinished),
            (b, EventKind::RunFinished),
        ],
        "a plugin must not observe RunFinished before the RunStarted that was broadcast first"
    );
    env.sup.shutdown("demo");
    let _ = plugin.join();
}

#[test]
fn wedged_subscriber_drops_without_blocking_others() {
    let mut config = SupervisorConfig::for_tests();
    config.write_queue_capacity = 2;
    let env = Env::with_pipe_cap(config, 1);
    let (hold_tx, hold_rx) = mpsc::channel::<()>();
    let wedged = env.spawn_plugin(move |ep| {
        handshake_events(ep, "wedged", EventFilter::default());
        let _ = hold_rx.recv();
    });
    env.sup.connect(&event_spec("wedged")).unwrap();

    let (tx, rx) = mpsc::channel();
    let healthy = connect_subscriber(&env, "healthy", EventFilter::default(), tx);

    let mut dropped = 0;
    for i in 0..24 {
        let report = env
            .sup
            .broadcast(run_started(pane(1), &format!("echo {i}")));
        dropped += report.dropped();
        rx.recv_timeout(Duration::from_secs(2)).expect(
            "healthy subscriber must receive each event without waiting for the wedged one",
        );
    }
    assert!(dropped > 0, "a full queue must drop, not grow");
    let snap = env.sup.snapshot("wedged").unwrap();
    assert!(
        snap.events_dropped > 0,
        "Monitor must see events_dropped: {snap:?}"
    );

    let _ = hold_tx.send(());
    env.sup.shutdown_all();
    let _ = wedged.join();
    let _ = healthy.join();
}

#[test]
fn dead_connection_is_skipped_without_aborting_healthy_plugins() {
    let env = Env::new();
    let dead = env.spawn_plugin(|ep| {
        handshake_events(ep, "dead", EventFilter::default());
    });
    env.sup.connect(&event_spec("dead")).unwrap();
    wait_until(
        || {
            env.sup
                .snapshot("dead")
                .is_some_and(|s| s.state == ConnectionState::Dead)
        },
        "plugin drop should mark the session dead",
    );

    let (tx, rx) = mpsc::channel();
    let healthy = connect_subscriber(&env, "healthy", EventFilter::default(), tx);
    let report = env.sup.broadcast(run_started(pane(1), "echo hi"));
    assert!(
        report.skipped() >= 1 || report.filtered() >= 1 || report.delivered() >= 1,
        "broadcast must not panic on a dead peer: {report:?}"
    );
    barrier_invoke(&env, "healthy");
    assert_eq!(rx.try_iter().count(), 1);
    env.sup.shutdown_all();
    let _ = dead.join();
    let _ = healthy.join();
}

#[test]
fn secret_bearing_command_does_not_reach_the_plugin_verbatim() {
    let env = Env::new();
    let (tx, rx) = mpsc::channel();
    let plugin = connect_subscriber(&env, "demo", EventFilter::default(), tx);
    let secret = "AWS_SECRET_ACCESS_KEY=supersecret aws s3 ls";
    env.sup.broadcast(run_started(pane(1), secret));
    barrier_invoke(&env, "demo");
    let HostEvent::RunStarted { command, .. } = rx.try_recv().expect("event") else {
        panic!("expected RunStarted");
    };
    assert!(
        !command.contains("supersecret"),
        "plugins must never see the raw command line: {command}"
    );
    env.sup.shutdown("demo");
    let _ = plugin.join();
}
