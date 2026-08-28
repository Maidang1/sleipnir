//! Registry of live connections, idle eviction, crash backoff.
//!
//! Connection caching is the whole point of residency: `Lifecycle::Resident`
//! reuses a handshake; `OnDemand` still tears down after one invoke so v1
//! semantics stay available through this supervisor. A crash-looping plugin is
//! not restarted forever — backoff doubles up to a ceiling, then the plugin
//! is disabled.

use super::session::Session;
use super::transport::Launcher;
use super::{
    BroadcastReport, Clock, ConnectionSnapshot, ConnectionState, Delivery, Inbound, LaunchSpec,
    PendingInvoke, SessionError, SupervisorConfig, mutex_lock,
};
use crate::PluginLifecycle;
use plugin_protocol::v2::{self, MessageId};
use plugin_protocol::{InvokeContext, Output};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Health {
    crashes: u32,
    next_restart_ms: u64,
    disabled: bool,
    last_snapshot: Option<ConnectionSnapshot>,
}

struct Inner {
    live: HashMap<String, Arc<Session>>,
    health: HashMap<String, Health>,
}

pub struct Supervisor {
    inner: Mutex<Inner>,
    /// Per-plugin lock so two `connect` calls cannot handshake the same id
    /// twice, without holding the registry lock across I/O.
    plugin_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    launcher: Arc<dyn Launcher>,
    clock: Arc<dyn Clock>,
    config: SupervisorConfig,
}

impl Supervisor {
    pub fn new(
        config: SupervisorConfig,
        launcher: Arc<dyn Launcher>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner {
                live: HashMap::new(),
                health: HashMap::new(),
            }),
            plugin_locks: Mutex::new(HashMap::new()),
            launcher,
            clock,
            config,
        }
    }

    pub fn config(&self) -> &SupervisorConfig {
        &self.config
    }

    /// Handshake if needed. Resident connections are cached; OnDemand is not.
    pub fn connect(&self, spec: &LaunchSpec) -> Result<Arc<Session>, SessionError> {
        let plug_lock = self.plugin_lock(&spec.plugin_id);
        let _guard = mutex_lock(&plug_lock);

        if spec.lifecycle == PluginLifecycle::Resident {
            if let Some(existing) = self.live_if_usable(&spec.plugin_id) {
                return Ok(existing);
            }
        } else {
            self.drop_live(&spec.plugin_id);
        }

        self.check_backoff(&spec.plugin_id)?;

        let spawned = self.launcher.launch(spec)?;
        let restart_count = self.crash_count(&spec.plugin_id);
        let session = match Session::spawn(
            spec,
            spawned,
            &self.config,
            Arc::clone(&self.clock),
            restart_count,
        ) {
            Ok(session) => session,
            Err(err) => {
                self.note_crash(&spec.plugin_id, None);
                return Err(err);
            }
        };

        if spec.lifecycle == PluginLifecycle::Resident {
            mutex_lock(&self.inner)
                .live
                .insert(spec.plugin_id.clone(), Arc::clone(&session));
        }
        Ok(session)
    }

    pub fn invoke(
        &self,
        spec: &LaunchSpec,
        command_id: &str,
        context: InvokeContext,
    ) -> Result<Output, SessionError> {
        let session = self.connect(spec)?;
        let result = session.invoke(command_id, context, self.config.request_timeout);
        if spec.lifecycle == PluginLifecycle::OnDemand {
            session.teardown(self.config.shutdown_grace);
            self.drop_live(&spec.plugin_id);
        }
        if result.is_err() && session.is_dead() {
            self.reap_dead(&spec.plugin_id);
        }
        result
    }

    pub fn begin_invoke(
        &self,
        spec: &LaunchSpec,
        command_id: &str,
        context: InvokeContext,
    ) -> Result<PendingInvoke, SessionError> {
        self.connect(spec)?.begin_invoke(command_id, context)
    }

    pub fn push_event(
        &self,
        plugin_id: &str,
        event: v2::HostEvent,
    ) -> Result<MessageId, SessionError> {
        let Some(session) = self.live_if_usable(plugin_id) else {
            return Err(SessionError::Disconnected);
        };
        match super::event_bus::fan_out(std::iter::once(session), event)
            .outcomes
            .into_iter()
            .next()
            .map(|(_, d)| d)
        {
            Some(Delivery::Delivered { id }) => Ok(id),
            Some(Delivery::Filtered) => Err(SessionError::Protocol(
                "event filtered or subscribe_events not granted".into(),
            )),
            Some(Delivery::Dropped) => Err(SessionError::Backpressure),
            Some(Delivery::Skipped) | None => Err(SessionError::Disconnected),
        }
    }

    /// Fan-out one event to every live connection. Never blocks: missing
    /// grants and full queues are recorded, not retried, and a dead plugin
    /// does not abort delivery to the rest.
    ///
    /// Per-plugin order is the enqueue order, which is this call's order
    /// relative to earlier `broadcast` calls.
    pub fn broadcast(&self, event: v2::HostEvent) -> BroadcastReport {
        let sessions: Vec<Arc<Session>> = mutex_lock(&self.inner).live.values().cloned().collect();
        super::event_bus::fan_out(sessions, event)
    }

    pub fn reply(
        &self,
        plugin_id: &str,
        id: MessageId,
        result: v2::HostCallResult,
    ) -> Result<(), SessionError> {
        self.require_live(plugin_id)?.reply(id, result)
    }

    pub fn drain_inbound(&self, plugin_id: &str) -> Vec<Inbound> {
        self.require_live(plugin_id)
            .map(|s| s.drain_inbound())
            .unwrap_or_default()
    }

    /// Drain every live connection's inbound queue. Order is plugin-id sorted
    /// so a UI poll is deterministic.
    pub fn drain_all_inbound(&self) -> Vec<(String, Inbound)> {
        let mut sessions: Vec<Arc<Session>> =
            mutex_lock(&self.inner).live.values().cloned().collect();
        sessions.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        let mut out = Vec::new();
        for session in sessions {
            for msg in session.drain_inbound() {
                out.push((session.plugin_id.clone(), msg));
            }
        }
        out
    }

    pub fn has_grant(&self, plugin_id: &str, cap: v2::Capability) -> bool {
        self.live_if_usable(plugin_id)
            .is_some_and(|s| s.has_grant(cap))
    }

    pub fn push_action(
        &self,
        plugin_id: &str,
        block_id: v2::BlockId,
        action: String,
        arg: Option<String>,
    ) -> Result<MessageId, SessionError> {
        self.require_live(plugin_id)?
            .push_action(block_id, action, arg)
    }

    pub fn snapshot(&self, plugin_id: &str) -> Option<ConnectionSnapshot> {
        let inner = mutex_lock(&self.inner);
        if let Some(session) = inner.live.get(plugin_id) {
            return Some(session.snapshot());
        }
        inner
            .health
            .get(plugin_id)
            .and_then(|h| h.last_snapshot.clone())
    }

    pub fn snapshots(&self) -> Vec<ConnectionSnapshot> {
        let inner = mutex_lock(&self.inner);
        let mut out: Vec<_> = inner.live.values().map(|s| s.snapshot()).collect();
        for (id, health) in &inner.health {
            if inner.live.contains_key(id) {
                continue;
            }
            if let Some(snap) = &health.last_snapshot {
                out.push(snap.clone());
            }
        }
        out.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        out
    }

    /// Reap dead sessions, apply backoff, evict idle residents. The host
    /// should call this on a timer; tests call it after advancing the clock.
    pub fn tick(&self) {
        let now = self.clock.now_ms();
        let idle_ms = self.config.idle.as_millis() as u64;
        let stable_ms = self.config.stable_after.as_millis() as u64;

        let ids: Vec<String> = mutex_lock(&self.inner).live.keys().cloned().collect();
        for id in ids {
            let plug_lock = self.plugin_lock(&id);
            let _guard = mutex_lock(&plug_lock);
            let session = {
                let inner = mutex_lock(&self.inner);
                inner.live.get(&id).cloned()
            };
            let Some(session) = session else {
                continue;
            };

            if session.is_dead() {
                self.reap_dead(&id);
                continue;
            }

            if now.saturating_sub(session.started_at_ms()) >= stable_ms {
                mutex_lock(&self.inner)
                    .health
                    .entry(id.clone())
                    .or_default()
                    .crashes = 0;
            }

            let idle = session.lifecycle == PluginLifecycle::Resident
                && session.in_flight() == 0
                && now.saturating_sub(session.last_activity_ms()) >= idle_ms;
            if idle {
                session.teardown(self.config.shutdown_grace);
                self.drop_live(&id);
            }
        }
    }

    pub fn shutdown(&self, plugin_id: &str) {
        let plug_lock = self.plugin_lock(plugin_id);
        let _guard = mutex_lock(&plug_lock);
        if let Some(session) = self.drop_live(plugin_id) {
            session.teardown(self.config.shutdown_grace);
        }
    }

    pub fn shutdown_all(&self) {
        let sessions: Vec<Arc<Session>> = {
            let mut inner = mutex_lock(&self.inner);
            inner.live.drain().map(|(_, s)| s).collect()
        };
        for session in sessions {
            session.teardown(self.config.shutdown_grace);
        }
    }

    fn plugin_lock(&self, id: &str) -> Arc<Mutex<()>> {
        mutex_lock(&self.plugin_locks)
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn live_if_usable(&self, id: &str) -> Option<Arc<Session>> {
        let session = mutex_lock(&self.inner).live.get(id).cloned()?;
        if session.is_dead() {
            self.reap_dead(id);
            None
        } else {
            Some(session)
        }
    }

    fn require_live(&self, id: &str) -> Result<Arc<Session>, SessionError> {
        self.live_if_usable(id).ok_or(SessionError::Disconnected)
    }

    fn drop_live(&self, id: &str) -> Option<Arc<Session>> {
        mutex_lock(&self.inner).live.remove(id)
    }

    fn crash_count(&self, id: &str) -> u32 {
        mutex_lock(&self.inner)
            .health
            .get(id)
            .map(|h| h.crashes)
            .unwrap_or(0)
    }

    fn check_backoff(&self, id: &str) -> Result<(), SessionError> {
        let inner = mutex_lock(&self.inner);
        let Some(health) = inner.health.get(id) else {
            return Ok(());
        };
        if health.disabled {
            return Err(SessionError::Disabled {
                restarts: health.crashes,
            });
        }
        let now = self.clock.now_ms();
        if now < health.next_restart_ms {
            return Err(SessionError::Backoff {
                until_ms: health.next_restart_ms,
            });
        }
        Ok(())
    }

    fn reap_dead(&self, id: &str) {
        let session = self.drop_live(id);
        if let Some(session) = session {
            let snap = session.snapshot();
            session.teardown(Duration::ZERO);
            self.note_crash(id, Some(snap));
        }
    }

    fn note_crash(&self, id: &str, snapshot: Option<ConnectionSnapshot>) {
        let now = self.clock.now_ms();
        let mut inner = mutex_lock(&self.inner);
        let health = inner.health.entry(id.to_string()).or_default();
        health.crashes = health.crashes.saturating_add(1);
        if let Some(mut snap) = snapshot {
            snap.state = ConnectionState::Dead;
            snap.restart_count = health.crashes;
            health.last_snapshot = Some(snap);
        }
        if health.crashes >= self.config.max_restarts {
            health.disabled = true;
            return;
        }
        let exp = health.crashes.saturating_sub(1).min(16);
        let delay = self
            .config
            .backoff_initial
            .saturating_mul(1u32 << exp)
            .min(self.config.backoff_ceiling);
        health.next_restart_ms = now.saturating_add(delay.as_millis() as u64);
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
