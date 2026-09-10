//! One live plugin connection: writer, reader, stderr drain, waiters.
//!
//! Isolation rules (ADR-0015 / ADR-0016):
//! - stderr is drained on its own thread into a ring of the last N lines, so a
//!   chatty plugin cannot fill the pipe and block.
//! - host writes go through a bounded queue; a plugin that will not read
//!   surfaces [`SessionError::Backpressure`] instead of growing memory or
//!   stalling the caller.
//! - inbound renders coalesce within a call-free batch; excess calls receive
//!   an error reply. An undeliverable reply disconnects the session.
//! - every waiter has a timeout; plugin death fails every pending waiter.
//! - no panic on plugin input: malformed JSON, unknown ids, duplicates, and
//!   oversized frames are errors.

use super::event_bus::Delivery;
use super::transport::{LineSink, LineSource, PluginProcess, RecvLine, Spawned};
use super::{Clock, LaunchSpec, SessionError, SupervisorConfig, mutex_lock};
use plugin_protocol::v2::{
    self, Capability, EventFilter, HostCall, HostMessage, MessageId, PluginMessage, RenderTarget,
    Widget,
};
use plugin_protocol::v2::{InvokeContext, Manifest, Output};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TrySendError};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

enum WriteCmd {
    Line(String),
    Shutdown,
}

enum HandshakeSlot {
    Waiting(mpsc::Sender<Result<ReadyInfo, SessionError>>),
    Done,
}

struct ReadyInfo {
    protocol_version: u32,
    manifest: Manifest,
    requests: Vec<Capability>,
    event_filter: EventFilter,
}

struct Waiter {
    tx: mpsc::Sender<Result<Output, SessionError>>,
}

/// Plugin-initiated messages the host must handle (draw, call).
#[derive(Debug, Clone)]
pub enum Inbound {
    Render {
        id: MessageId,
        target: RenderTarget,
        tree: Widget,
    },
    Call {
        id: MessageId,
        call: HostCall,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Live,
    Dead,
    ShuttingDown,
}

/// Monitor-facing snapshot. Cheap to clone; stderr is the last N lines.
#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    pub plugin_id: String,
    pub instance_id: Uuid,
    pub pid: Option<u32>,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
    pub in_flight: usize,
    pub restart_count: u32,
    pub stderr: Vec<String>,
    pub state: ConnectionState,
    pub inbound_dropped: u64,
    pub malformed_lines: u64,
    /// Host → plugin events dropped because the write queue was full.
    pub events_dropped: u64,
}

/// A live RPC session with one plugin process.
pub struct Session {
    pub(crate) plugin_id: String,
    instance_id: Uuid,
    pub(crate) lifecycle: crate::PluginLifecycle,
    clock: Arc<dyn Clock>,
    write_tx: Mutex<Option<mpsc::SyncSender<WriteCmd>>>,
    pending: Mutex<HashMap<MessageId, Waiter>>,
    next_id: AtomicU64,
    inbound: Mutex<VecDeque<Inbound>>,
    inbound_cap: usize,
    inbound_dropped: AtomicU64,
    malformed: AtomicU64,
    events_dropped: AtomicU64,
    /// Capabilities announced in Hello. The event bus gates on this set, not
    /// on what the plugin *asked* for — a request is not a grant.
    granted: BTreeSet<Capability>,
    event_filter: Mutex<EventFilter>,
    stderr: Mutex<VecDeque<String>>,
    stderr_cap: usize,
    started_at_ms: u64,
    last_activity_ms: AtomicU64,
    pub(crate) restart_count: u32,
    process: Mutex<Box<dyn PluginProcess>>,
    pid: Option<u32>,
    shutdown: AtomicBool,
    dead: AtomicBool,
    handshake: Mutex<HandshakeSlot>,
    threads: Mutex<IoThreads>,
}

struct IoThreads {
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
}

/// Handle returned by [`Session::begin_invoke`]. `wait` is the only blocking
/// point, and it is always bounded.
pub struct PendingInvoke {
    id: MessageId,
    rx: mpsc::Receiver<Result<Output, SessionError>>,
    session: Arc<Session>,
    cleanup_on_drop: Option<Arc<dyn Fn(Uuid) + Send + Sync>>,
}

impl PendingInvoke {
    pub fn id(&self) -> MessageId {
        self.id
    }

    pub fn wait(self, timeout: Duration) -> Result<Output, SessionError> {
        let result = match self.rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                mutex_lock(&self.session.pending).remove(&self.id);
                Err(SessionError::Timeout(timeout))
            }
            Err(RecvTimeoutError::Disconnected) => Err(SessionError::Disconnected),
        };
        result
    }

    pub(crate) fn with_cleanup(mut self, cleanup: Arc<dyn Fn(Uuid) + Send + Sync>) -> Self {
        self.cleanup_on_drop = Some(cleanup);
        self
    }
}

impl Drop for PendingInvoke {
    fn drop(&mut self) {
        mutex_lock(&self.session.pending).remove(&self.id);
        if let Some(cleanup) = &self.cleanup_on_drop {
            cleanup(self.session.instance_id());
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("plugin_id", &self.plugin_id)
            .field("instance_id", &self.instance_id)
            .field("dead", &self.dead.load(Ordering::SeqCst))
            .finish()
    }
}

impl Session {
    pub(crate) fn spawn(
        spec: &LaunchSpec,
        spawned: Spawned,
        config: &SupervisorConfig,
        clock: Arc<dyn Clock>,
        restart_count: u32,
    ) -> Result<Arc<Self>, SessionError> {
        let instance_id = Uuid::new_v4();
        let now = clock.now_ms();
        let (write_tx, write_rx) = mpsc::sync_channel(config.write_queue_capacity.max(1));
        let (hs_tx, hs_rx) = mpsc::channel();

        let session = Arc::new(Session {
            plugin_id: spec.plugin_id.clone(),
            instance_id,
            lifecycle: spec.lifecycle,
            clock: clock.clone(),
            write_tx: Mutex::new(Some(write_tx)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            inbound: Mutex::new(VecDeque::new()),
            inbound_cap: config.inbound_queue_capacity.max(1),
            inbound_dropped: AtomicU64::new(0),
            malformed: AtomicU64::new(0),
            events_dropped: AtomicU64::new(0),
            granted: spec.granted.iter().copied().collect(),
            event_filter: Mutex::new(EventFilter::default()),
            stderr: Mutex::new(VecDeque::new()),
            stderr_cap: config.stderr_lines.max(1),
            started_at_ms: now,
            last_activity_ms: AtomicU64::new(now),
            restart_count,
            pid: spawned.process.pid(),
            process: Mutex::new(spawned.process),
            shutdown: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            handshake: Mutex::new(HandshakeSlot::Waiting(hs_tx)),
            threads: Mutex::new(IoThreads {
                reader: None,
                writer: None,
                stderr: None,
            }),
        });

        // Stderr drain MUST start before we write Hello. A plugin that logs on
        // startup would otherwise fill the pipe during handshake.
        // I/O threads hold Weak, not Arc: a dropped OnDemand session must be
        // able to run Drop/teardown while they are blocked in recv.
        let weak = Arc::downgrade(&session);

        let stderr_src = spawned.stderr;
        let stderr_session = weak.clone();
        let stderr_max = config.max_line_bytes;
        let stderr_thread = spawn_named(&format!("plugin-{}-stderr", spec.plugin_id), move || {
            stderr_loop(stderr_src, stderr_session, stderr_max)
        });

        let reader_src = spawned.stdout;
        let reader_session = weak;
        let max_line = config.max_line_bytes;
        let reader_thread = spawn_named(&format!("plugin-{}-reader", spec.plugin_id), move || {
            reader_loop(reader_src, reader_session, max_line)
        });

        let writer_sink = spawned.stdin;
        let writer_session = Arc::downgrade(&session);
        let writer_thread = spawn_named(&format!("plugin-{}-writer", spec.plugin_id), move || {
            writer_loop(writer_sink, write_rx, writer_session)
        });

        {
            let mut threads = mutex_lock(&session.threads);
            threads.stderr = Some(stderr_thread);
            threads.reader = Some(reader_thread);
            threads.writer = Some(writer_thread);
        }

        let hello = HostMessage::Hello {
            protocol_version: v2::PROTOCOL_VERSION,
            granted: spec.granted.clone(),
            plugin_instance_id: instance_id,
        };
        session.enqueue_msg(&hello)?;

        let ready = match hs_rx.recv_timeout(config.handshake_timeout) {
            Ok(Ok(info)) => info,
            Ok(Err(err)) => {
                session.teardown(Duration::ZERO);
                return Err(err);
            }
            Err(RecvTimeoutError::Timeout) => {
                session.teardown(Duration::ZERO);
                return Err(SessionError::Timeout(config.handshake_timeout));
            }
            Err(RecvTimeoutError::Disconnected) => {
                session.teardown(Duration::ZERO);
                return Err(SessionError::Disconnected);
            }
        };

        if let Err(err) = validate_ready(spec, &ready) {
            session.teardown(Duration::ZERO);
            return Err(err);
        }
        *mutex_lock(&session.event_filter) = ready.event_filter;

        Ok(session)
    }

    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    /// Capabilities announced in Hello. A request is not a grant: this is
    /// what the event bus and mount points must consult.
    pub fn has_grant(&self, cap: Capability) -> bool {
        self.granted.contains(&cap)
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub fn last_activity_ms(&self) -> u64 {
        self.last_activity_ms.load(Ordering::SeqCst)
    }

    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    pub fn in_flight(&self) -> usize {
        mutex_lock(&self.pending).len()
    }

    pub fn invoke(
        self: &Arc<Self>,
        command_id: &str,
        context: InvokeContext,
        timeout: Duration,
    ) -> Result<Output, SessionError> {
        self.begin_invoke(command_id, context)?.wait(timeout)
    }

    pub fn begin_invoke(
        self: &Arc<Self>,
        command_id: &str,
        context: InvokeContext,
    ) -> Result<PendingInvoke, SessionError> {
        self.ensure_live()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        mutex_lock(&self.pending).insert(id, Waiter { tx });
        let msg = HostMessage::Invoke {
            id,
            command_id: command_id.to_string(),
            context,
        };
        if let Err(err) = self.enqueue_msg(&msg) {
            mutex_lock(&self.pending).remove(&id);
            return Err(err);
        }
        self.touch();
        Ok(PendingInvoke {
            id,
            rx,
            session: Arc::clone(self),
            cleanup_on_drop: None,
        })
    }

    /// Offer one event to this connection. Never blocks.
    ///
    /// `RunStarted.command` is redacted with `run_ledger::redact` here — the
    /// only wire choke point — so a missed redact at the capture site cannot
    /// leak a secret onto the wire. A plugin without
    /// [`Capability::SubscribeEvents`] is Filtered, not Delivered — that is the
    /// security property of the event path.
    pub fn receive_event(&self, event: &v2::HostEvent) -> Delivery {
        let event = redact_run_started(event.clone());
        if self.is_dead() || self.is_shutting_down() {
            return Delivery::Skipped;
        }
        if !self.granted.contains(&Capability::SubscribeEvents) {
            return Delivery::Filtered;
        }
        let filter = mutex_lock(&self.event_filter);
        if !event.matches(&filter) {
            return Delivery::Filtered;
        }
        drop(filter);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        match self.enqueue_msg(&HostMessage::Event { id, event }) {
            Ok(()) => {
                self.touch();
                Delivery::Delivered { id }
            }
            Err(SessionError::Backpressure) => {
                self.events_dropped.fetch_add(1, Ordering::Relaxed);
                Delivery::Dropped
            }
            Err(_) => Delivery::Skipped,
        }
    }

    pub fn push_action(
        &self,
        block_id: v2::BlockId,
        action: String,
        arg: Option<String>,
    ) -> Result<MessageId, SessionError> {
        self.ensure_live()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.enqueue_msg(&HostMessage::Action {
            id,
            block_id,
            action,
            arg,
        })?;
        self.touch();
        Ok(id)
    }

    pub fn reply(
        self: &Arc<Self>,
        id: MessageId,
        result: v2::HostCallResult,
    ) -> Result<(), SessionError> {
        self.ensure_live()?;
        if let Err(error) = self.enqueue_msg(&HostMessage::Reply { id, result }) {
            self.disconnect();
            return Err(error);
        }
        self.touch();
        Ok(())
    }

    pub fn drain_inbound(&self) -> Vec<Inbound> {
        mutex_lock(&self.inbound).drain(..).collect()
    }

    pub fn snapshot(&self) -> ConnectionSnapshot {
        let state = if self.shutdown.load(Ordering::SeqCst) {
            ConnectionState::ShuttingDown
        } else if self.dead.load(Ordering::SeqCst) {
            ConnectionState::Dead
        } else {
            ConnectionState::Live
        };
        ConnectionSnapshot {
            plugin_id: self.plugin_id.clone(),
            instance_id: self.instance_id,
            pid: self.pid,
            started_at_ms: self.started_at_ms,
            last_activity_ms: self.last_activity_ms.load(Ordering::SeqCst),
            in_flight: mutex_lock(&self.pending).len(),
            restart_count: self.restart_count,
            stderr: mutex_lock(&self.stderr).iter().cloned().collect(),
            state,
            inbound_dropped: self.inbound_dropped.load(Ordering::Relaxed),
            malformed_lines: self.malformed.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
        }
    }

    /// Ask the plugin to exit. Does not kill; [`teardown`] follows this with a
    /// grace period and then `kill` so a plugin that ignores Shutdown cannot
    /// leak a process.
    pub fn request_shutdown(&self) -> Result<(), SessionError> {
        self.enqueue(WriteCmd::Shutdown)
    }

    pub(crate) fn cancel(&self) {
        self.mark_dead(SessionError::Disconnected);
    }

    /// Shutdown, grace, kill, reap, join. Idempotent. Pending waiters get
    /// [`SessionError::Disconnected`] rather than hanging.
    pub fn teardown(&self, grace: Duration) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.enqueue(WriteCmd::Shutdown);
        if let Some(tx) = mutex_lock(&self.write_tx).take() {
            drop(tx);
        }
        {
            let mut proc = mutex_lock(&self.process);
            if !grace.is_zero() {
                let _ = proc.wait_timeout(grace);
            }
            let _ = proc.cancel_io();
            let _ = proc.kill();
            let _ = proc.wait_timeout(Duration::ZERO);
        }
        self.mark_dead(SessionError::Disconnected);
        let mut threads = mutex_lock(&self.threads);
        join_thread(threads.stderr.take());
        join_thread(threads.reader.take());
        join_thread(threads.writer.take());
    }

    fn ensure_live(&self) -> Result<(), SessionError> {
        if self.dead.load(Ordering::SeqCst) || self.shutdown.load(Ordering::SeqCst) {
            Err(SessionError::Disconnected)
        } else {
            Ok(())
        }
    }

    fn touch(&self) {
        self.last_activity_ms
            .store(self.clock.now_ms(), Ordering::SeqCst);
    }

    fn enqueue_msg(&self, msg: &HostMessage) -> Result<(), SessionError> {
        let line = serde_json::to_string(msg).map_err(|e| SessionError::Protocol(e.to_string()))?;
        self.enqueue(WriteCmd::Line(line))
    }

    fn enqueue(&self, cmd: WriteCmd) -> Result<(), SessionError> {
        let guard = mutex_lock(&self.write_tx);
        let Some(tx) = guard.as_ref() else {
            return Err(SessionError::Disconnected);
        };
        match tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(SessionError::Backpressure),
            Err(TrySendError::Disconnected(_)) => Err(SessionError::Disconnected),
        }
    }

    fn fail_waiters(&self, err: SessionError) {
        let waiters: Vec<Waiter> = mutex_lock(&self.pending).drain().map(|(_, w)| w).collect();
        for waiter in waiters {
            let _ = waiter.tx.send(Err(err.clone()));
        }
    }

    fn fail_handshake(&self, err: SessionError) {
        let mut slot = mutex_lock(&self.handshake);
        if let HandshakeSlot::Waiting(tx) = std::mem::replace(&mut *slot, HandshakeSlot::Done) {
            let _ = tx.send(Err(err));
        }
    }

    fn mark_dead(&self, err: SessionError) {
        if self.dead.swap(true, Ordering::SeqCst) {
            return;
        }
        self.fail_handshake(err.clone());
        self.fail_waiters(err);
    }

    fn complete_waiter(&self, id: MessageId, result: Result<Output, SessionError>) {
        let result = result.and_then(|output| {
            if let Some(capability) = output.required_capability() {
                if !self.has_grant(capability) {
                    return Err(SessionError::CapabilityDenied { capability });
                }
            }
            Ok(output)
        });
        let waiter = mutex_lock(&self.pending).remove(&id);
        match waiter {
            Some(waiter) => {
                let _ = waiter.tx.send(result);
                self.touch();
            }
            None => {
                // Unknown or duplicate id: ignore. A malicious plugin must not
                // be able to panic the host by inventing or repeating ids.
            }
        }
    }

    fn push_inbound(self: &Arc<Self>, msg: Inbound) {
        let mut q = mutex_lock(&self.inbound);
        if let Inbound::Render { target, .. } = &msg {
            let start = q
                .iter()
                .rposition(|queued| matches!(queued, Inbound::Call { .. }))
                .map_or(0, |index| index + 1);
            if let Some(index) = q.iter().skip(start).position(|queued| {
                matches!(queued, Inbound::Render { target: existing, .. } if existing == target)
            }) {
                q[start + index] = msg;
                self.touch();
                return;
            }
        }
        if q.len() >= self.inbound_cap {
            self.inbound_dropped.fetch_add(1, Ordering::Relaxed);
            drop(q);
            if let Inbound::Call { id, .. } = msg {
                let _ = self.reply(
                    id,
                    v2::HostCallResult::Error {
                        message: "host inbound queue is full; retry later".into(),
                    },
                );
            }
            return;
        }
        q.push_back(msg);
        self.touch();
    }

    fn on_line(self: &Arc<Self>, line: &str) {
        let msg: PluginMessage = match serde_json::from_str(line) {
            Ok(msg) => msg,
            Err(err) => {
                self.malformed.fetch_add(1, Ordering::Relaxed);
                log::warn!("plugin {} sent garbage (ignored): {err}", self.plugin_id);
                self.fail_handshake(SessionError::Protocol(format!("garbage: {err}")));
                return;
            }
        };
        self.route(msg);
    }

    fn route(self: &Arc<Self>, msg: PluginMessage) {
        match msg {
            PluginMessage::Ready {
                protocol_version,
                manifest,
                requests,
                event_filter,
            } => {
                let mut slot = mutex_lock(&self.handshake);
                match std::mem::replace(&mut *slot, HandshakeSlot::Done) {
                    HandshakeSlot::Waiting(tx) => {
                        let _ = tx.send(Ok(ReadyInfo {
                            protocol_version,
                            manifest,
                            requests,
                            event_filter,
                        }));
                    }
                    HandshakeSlot::Done => {
                        // Unsolicited Ready after handshake: ignore.
                    }
                }
            }
            PluginMessage::Invoked { id, output } => self.complete_waiter(id, Ok(output)),
            PluginMessage::Failed { id, message } => {
                self.complete_waiter(id, Err(SessionError::PluginFailed(message)))
            }
            PluginMessage::Render { id, target, tree } => {
                self.push_inbound(Inbound::Render { id, target, tree })
            }
            PluginMessage::Call { id, call } => self.push_inbound(Inbound::Call { id, call }),
        }
    }

    fn on_eof(&self) {
        self.mark_dead(SessionError::Disconnected);
    }

    fn disconnect(self: &Arc<Self>) {
        if self.dead.swap(true, Ordering::SeqCst) {
            return;
        }
        self.fail_handshake(SessionError::Disconnected);
        self.fail_waiters(SessionError::Disconnected);
        let session = Arc::clone(self);
        std::thread::spawn(move || session.teardown(Duration::ZERO));
    }

    fn on_oversized(&self) {
        self.malformed.fetch_add(1, Ordering::Relaxed);
        self.mark_dead(SessionError::Protocol("oversized line".into()));
    }

    fn push_stderr(&self, line: String) {
        let mut ring = mutex_lock(&self.stderr);
        if ring.len() >= self.stderr_cap {
            ring.pop_front();
        }
        ring.push_back(line);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Last strong ref is gone. Tear down so OnDemand (which is not in the
        // supervisor cache) cannot leak threads or a child process.
        self.teardown(Duration::ZERO);
    }
}

fn validate_ready(spec: &LaunchSpec, ready: &ReadyInfo) -> Result<(), SessionError> {
    // ADR-0016 §8's window covers only dialects the host still implements.
    // v1 was removed, so today this is equality with v2::PROTOCOL_VERSION;
    // it widens to N-1 when a v3 lands and v2 stays implemented.
    if !v2::versions_compatible(v2::PROTOCOL_VERSION, ready.protocol_version) {
        return Err(SessionError::VersionMismatch {
            plugin: ready.protocol_version,
        });
    }
    if ready.manifest.id != spec.plugin_id {
        return Err(SessionError::Protocol(format!(
            "ready id {} does not match plugin.json {}",
            ready.manifest.id, spec.plugin_id
        )));
    }
    for cap in &ready.requests {
        if !spec.declared_capabilities.contains(cap) {
            return Err(SessionError::CapabilityExceeded { capability: *cap });
        }
    }
    // Command-level capabilities in Ready.manifest still cannot exceed what
    // plugin.json declared.
    for command in &ready.manifest.commands {
        for cap in &command.capabilities {
            if !spec.declared_capabilities.contains(cap) {
                return Err(SessionError::CapabilityExceeded { capability: *cap });
            }
        }
    }
    Ok(())
}

fn redact_run_started(event: v2::HostEvent) -> v2::HostEvent {
    match event {
        v2::HostEvent::RunStarted {
            run_id,
            pane,
            command,
            cwd,
            inferred,
        } => v2::HostEvent::RunStarted {
            run_id,
            pane,
            command: run_ledger::redact_command(&command),
            cwd,
            inferred,
        },
        other => other,
    }
}

fn writer_loop(mut sink: Box<dyn LineSink>, rx: mpsc::Receiver<WriteCmd>, session: Weak<Session>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            WriteCmd::Line(line) => {
                if sink.send_line(&line).is_err() {
                    if let Some(session) = session.upgrade() {
                        session.mark_dead(SessionError::Disconnected);
                    }
                    break;
                }
            }
            WriteCmd::Shutdown => {
                if let Ok(line) = serde_json::to_string(&HostMessage::Shutdown) {
                    if sink.send_line(&line).is_err() {
                        if let Some(session) = session.upgrade() {
                            session.mark_dead(SessionError::Disconnected);
                        }
                    }
                }
                break;
            }
        }
    }
}

fn reader_loop(mut src: Box<dyn LineSource>, session: Weak<Session>, max_bytes: usize) {
    loop {
        match src.recv_line(max_bytes) {
            Ok(RecvLine::Line(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Some(session) = session.upgrade() else {
                    break;
                };
                session.on_line(trimmed);
            }
            Ok(RecvLine::Eof) => {
                if let Some(session) = session.upgrade() {
                    session.on_eof();
                }
                break;
            }
            Ok(RecvLine::Oversized) => {
                if let Some(session) = session.upgrade() {
                    session.on_oversized();
                }
                break;
            }
            Err(_) => {
                if let Some(session) = session.upgrade() {
                    session.on_eof();
                }
                break;
            }
        }
    }
}

fn stderr_loop(mut src: Box<dyn LineSource>, session: Weak<Session>, max_bytes: usize) {
    loop {
        match src.recv_line(max_bytes) {
            Ok(RecvLine::Line(line)) => {
                let Some(session) = session.upgrade() else {
                    break;
                };
                session.push_stderr(line);
            }
            Ok(RecvLine::Oversized) => {
                if let Some(session) = session.upgrade() {
                    session.push_stderr("<truncated>".into());
                }
            }
            Ok(RecvLine::Eof) | Err(_) => break,
        }
    }
}

fn spawn_named<F>(name: &str, f: F) -> JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    thread::Builder::new()
        .name(name.to_string())
        .spawn(f)
        .expect("failed to spawn plugin I/O thread")
}

fn join_thread(handle: Option<JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod worker_error_tests {
    use super::*;
    use crate::PluginLifecycle;
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;

    struct BrokenSink;

    impl LineSink for BrokenSink {
        fn send_line(&mut self, _line: &str) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "simulated sink failure",
            ))
        }
    }

    struct IdleSource;

    impl LineSource for IdleSource {
        fn recv_line(&mut self, _max_bytes: usize) -> std::io::Result<RecvLine> {
            Ok(RecvLine::Eof)
        }
    }

    struct FakeProcess {
        dead: bool,
        calls: Arc<Mutex<Vec<&'static str>>>,
        graceful_exit: bool,
    }

    impl PluginProcess for FakeProcess {
        fn pid(&self) -> Option<u32> {
            None
        }

        fn cancel_io(&mut self) -> std::io::Result<()> {
            self.calls.lock().unwrap().push("cancel_io");
            self.dead = true;
            Ok(())
        }

        fn kill(&mut self) -> std::io::Result<()> {
            self.calls.lock().unwrap().push("kill");
            self.dead = true;
            Ok(())
        }

        fn wait_timeout(&mut self, timeout: Duration) -> bool {
            if timeout.is_zero() {
                self.calls.lock().unwrap().push("wait_zero");
                return self.dead;
            }
            self.calls.lock().unwrap().push("wait_grace");
            if self.graceful_exit {
                self.dead = true;
            }
            self.graceful_exit
        }
    }

    fn test_spec() -> LaunchSpec {
        LaunchSpec {
            plugin_id: "demo".into(),
            lifecycle: PluginLifecycle::Resident,
            declared_capabilities: BTreeSet::new(),
            granted: vec![],
            binary: "demo".into(),
            args: vec![],
            cwd: PathBuf::from("."),
        }
    }

    #[test]
    fn writer_failure_marks_session_dead_and_wakes_waiter() {
        let clock = Arc::new(super::super::ManualClock::new(1_000)) as Arc<dyn Clock>;
        let spawned = Spawned {
            stdin: Box::new(BrokenSink),
            stdout: Box::new(IdleSource),
            stderr: Box::new(IdleSource),
            process: Box::new(FakeProcess {
                dead: false,
                calls: Arc::new(Mutex::new(Vec::new())),
                graceful_exit: false,
            }),
        };
        let session = Session::spawn(
            &test_spec(),
            spawned,
            &SupervisorConfig::for_tests(),
            clock,
            0,
        )
        .unwrap_err();
        assert_eq!(session, SessionError::Disconnected);
    }

    #[derive(Clone, Default)]
    struct WireState {
        lines: Arc<Mutex<Vec<String>>>,
        shutdown_sent: Arc<(Mutex<bool>, Condvar)>,
        closed: Arc<(Mutex<bool>, Condvar)>,
    }

    impl WireState {
        fn wait_for_shutdown(&self, timeout: Duration) -> bool {
            let (lock, cv) = &*self.shutdown_sent;
            let sent = lock.lock().unwrap();
            let (sent, _) = cv
                .wait_timeout_while(sent, timeout, |sent| !*sent)
                .unwrap_or_else(|e| e.into_inner());
            *sent
        }

        fn close(&self) {
            let (lock, cv) = &*self.closed;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }

        fn wait_until_closed(&self) {
            let (lock, cv) = &*self.closed;
            let closed = lock.lock().unwrap();
            let _guard = cv
                .wait_while(closed, |closed| !*closed)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    struct RecordingSink {
        wire: WireState,
    }

    impl LineSink for RecordingSink {
        fn send_line(&mut self, line: &str) -> std::io::Result<()> {
            self.wire.lines.lock().unwrap().push(line.to_string());
            if line == serde_json::to_string(&HostMessage::Shutdown).unwrap() {
                let (lock, cv) = &*self.wire.shutdown_sent;
                *lock.lock().unwrap() = true;
                cv.notify_all();
            }
            Ok(())
        }
    }

    struct ScriptedSource {
        ready: Option<String>,
        wire: WireState,
    }

    impl LineSource for ScriptedSource {
        fn recv_line(&mut self, _max_bytes: usize) -> std::io::Result<RecvLine> {
            if let Some(line) = self.ready.take() {
                return Ok(RecvLine::Line(line));
            }
            self.wire.wait_until_closed();
            Ok(RecvLine::Eof)
        }
    }

    struct OrderingProcess {
        dead: bool,
        calls: Arc<Mutex<Vec<&'static str>>>,
        wire: WireState,
        graceful_exit: bool,
    }

    impl PluginProcess for OrderingProcess {
        fn pid(&self) -> Option<u32> {
            None
        }

        fn cancel_io(&mut self) -> std::io::Result<()> {
            self.calls.lock().unwrap().push("cancel_io");
            self.wire.close();
            self.dead = true;
            Ok(())
        }

        fn kill(&mut self) -> std::io::Result<()> {
            self.calls.lock().unwrap().push("kill");
            self.wire.close();
            self.dead = true;
            Ok(())
        }

        fn wait_timeout(&mut self, timeout: Duration) -> bool {
            if timeout.is_zero() {
                self.calls.lock().unwrap().push("wait_zero");
                return self.dead;
            }
            self.calls.lock().unwrap().push("wait_grace");
            let saw_shutdown = self.wire.wait_for_shutdown(timeout);
            assert!(
                saw_shutdown,
                "grace wait should observe Shutdown before teardown cleanup"
            );
            if self.graceful_exit {
                self.dead = true;
            }
            self.graceful_exit
        }
    }

    fn ready_line() -> String {
        serde_json::to_string(&PluginMessage::Ready {
            protocol_version: v2::PROTOCOL_VERSION,
            manifest: plugin_protocol::v2::Manifest {
                id: "demo".into(),
                name: "Demo".into(),
                version: "1".into(),
                description: String::new(),
                lifecycle: plugin_protocol::v2::Lifecycle::Resident,
                commands: vec![],
            },
            requests: vec![],
            event_filter: plugin_protocol::v2::EventFilter::default(),
        })
        .unwrap()
    }

    #[test]
    fn teardown_waits_for_shutdown_write_before_cancel_when_grace_is_positive() {
        let clock = Arc::new(super::super::ManualClock::new(1_000)) as Arc<dyn Clock>;
        let wire = WireState::default();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let spawned = Spawned {
            stdin: Box::new(RecordingSink { wire: wire.clone() }),
            stdout: Box::new(ScriptedSource {
                ready: Some(ready_line()),
                wire: wire.clone(),
            }),
            stderr: Box::new(ScriptedSource {
                ready: None,
                wire: wire.clone(),
            }),
            process: Box::new(OrderingProcess {
                dead: false,
                calls: calls.clone(),
                wire: wire.clone(),
                graceful_exit: true,
            }),
        };
        let session = Session::spawn(
            &test_spec(),
            spawned,
            &SupervisorConfig::for_tests(),
            clock,
            0,
        )
        .expect("session should spawn");

        let teardown_session = Arc::clone(&session);
        let teardown = thread::spawn(move || teardown_session.teardown(Duration::from_millis(1)));
        teardown.join().unwrap();

        let lines = wire.lines.lock().unwrap().clone();
        assert_eq!(lines.len(), 2, "expected Hello then Shutdown");
        assert_eq!(
            lines[1],
            serde_json::to_string(&HostMessage::Shutdown).expect("serialize shutdown")
        );
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &["wait_grace", "cancel_io", "kill", "wait_zero"],
            "positive grace must flush Shutdown before cancel, then always clean owned process state"
        );
        drop(session);
    }

    #[test]
    fn teardown_zero_grace_cancels_immediately() {
        let clock = Arc::new(super::super::ManualClock::new(1_000)) as Arc<dyn Clock>;
        let wire = WireState::default();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let spawned = Spawned {
            stdin: Box::new(RecordingSink { wire: wire.clone() }),
            stdout: Box::new(ScriptedSource {
                ready: Some(ready_line()),
                wire: wire.clone(),
            }),
            stderr: Box::new(ScriptedSource {
                ready: None,
                wire: wire.clone(),
            }),
            process: Box::new(OrderingProcess {
                dead: false,
                calls: calls.clone(),
                wire,
                graceful_exit: false,
            }),
        };
        let session = Session::spawn(
            &test_spec(),
            spawned,
            &SupervisorConfig::for_tests(),
            clock,
            0,
        )
        .expect("session should spawn");
        session.teardown(Duration::ZERO);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &["cancel_io", "kill", "wait_zero"],
            "zero grace must skip the grace wait and clean immediately"
        );
    }
}

#[cfg(test)]
mod redact_tests {
    use super::redact_run_started;
    use plugin_protocol::v2::HostEvent;
    use uuid::Uuid;

    #[test]
    fn run_started_command_is_redacted_before_anyone_sees_it() {
        let event = HostEvent::RunStarted {
            run_id: Uuid::nil(),
            pane: Uuid::nil(),
            command: "AWS_SECRET_ACCESS_KEY=supersecret aws s3 ls".into(),
            cwd: None,
            inferred: true,
        };
        let HostEvent::RunStarted {
            command, inferred, ..
        } = redact_run_started(event)
        else {
            panic!("expected RunStarted");
        };
        assert!(
            !command.contains("supersecret"),
            "raw secret must not survive redact: {command}"
        );
        assert!(inferred, "redaction must not rewrite the provenance flag");
    }
}
