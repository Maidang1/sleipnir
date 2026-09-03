//! Resident plugin supervisor (ADR-0016).
//!
//! v1 supervises a plugin as a single request/response pair and then kills it.
//! That is the right default, and [`crate::run_command`] still does it. It is
//! the wrong foundation for residency: a process that lives across invocations
//! can fill an unread stderr pipe, can interleave `Render`/`Call` with
//! `Invoked`, and can crash while callers wait. Those are harmless
//! per-invocation; they are fatal if the process is kept around.
//!
//! This module is the load-bearing fix. The host gains the supervisory duties
//! ADR-0016 names: connection caching, liveness accounting, event fan-out with
//! **backpressure**, and per-plugin teardown. A wedged, crashing, spamming, or
//! malicious plugin must never stall or unwind the terminal (ADR-0015: process
//! isolation is non-negotiable).
//!
//! Two v1 defects this supervisor exists to close:
//!
//! 1. **stderr deadlock.** v1 sets `stderr(Stdio::piped())` and never reads it.
//!    A resident plugin that logs will fill the ~64KB pipe and block on write.
//!    Stderr is drained on its own thread into a bounded ring.
//! 2. **strict request/response.** v1 `drive()` writes one message and blocks
//!    for exactly one reply. v2 is asynchronous and out-of-order. A reader
//!    thread plus a `MessageId → waiter` map routes `Invoked` to the right
//!    caller and `Render`/`Call` to inbound handlers.
//!
//! The transport is a trait so the supervisor is unit-testable without
//! spawning binaries. Nearly all tests use the in-memory impl and a manual
//! clock: no sleeps, no flaky timing.

mod event_bus;
mod session;
mod supervisor;
mod transport;

#[cfg(test)]
mod tests;

use crate::{Permission, PluginLifecycle, PluginManifest};
use plugin_protocol::v2::{self, Capability};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub use event_bus::{BroadcastReport, Delivery};
pub use session::{ConnectionSnapshot, ConnectionState, Inbound, PendingInvoke, Session};
pub use supervisor::Supervisor;
pub use transport::{
    Launcher, LineSink, LineSource, MemoryLauncher, PluginEndpoint, PluginProcess, ProcessLauncher,
    RecvLine, Spawned,
};

/// On-disk / launch description the supervisor needs. `plugin.json` remains
/// the pre-launch audit source of truth; [`LaunchSpec::declared_capabilities`]
/// is that audit, and `Ready` cannot exceed it.
#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub plugin_id: String,
    pub lifecycle: PluginLifecycle,
    pub declared_capabilities: BTreeSet<Capability>,
    pub granted: Vec<Capability>,
    pub binary: OsString,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl LaunchSpec {
    pub fn from_plugin(
        manifest: &PluginManifest,
        directory: &Path,
        granted: Vec<Capability>,
    ) -> Self {
        Self {
            plugin_id: manifest.id.clone(),
            lifecycle: manifest.lifecycle,
            declared_capabilities: declared_capabilities(manifest),
            granted,
            binary: crate::resolve_binary(directory, &manifest.binary),
            args: manifest.args.clone(),
            cwd: directory.to_path_buf(),
        }
    }
}

/// Union of plugin-level and per-command permissions in `plugin.json`, plus
/// `Resident` when the manifest declares that lifecycle. Ready.requests must
/// be a subset. Plugin-level permissions exist so a command-less resident can
/// still be audited before launch (ADR-0015 / ADR-0016).
pub fn declared_capabilities(manifest: &PluginManifest) -> BTreeSet<Capability> {
    let mut set: BTreeSet<Capability> = manifest
        .permissions
        .iter()
        .copied()
        .map(Permission::to_v2)
        .collect();
    for command in &manifest.commands {
        for permission in &command.permissions {
            set.insert(permission.to_v2());
        }
    }
    if manifest.lifecycle == PluginLifecycle::Resident {
        set.insert(Capability::Resident);
    }
    set
}

/// Tunables. Tests inject small values and a [`ManualClock`]; production uses
/// [`SupervisorConfig::default`] and [`SystemClock`].
#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    /// Shut down a resident plugin that has been unused for this long.
    pub idle: Duration,
    pub handshake_timeout: Duration,
    pub request_timeout: Duration,
    /// After `Shutdown`, wait this long before `kill`. Zero means kill at once
    /// if the process is still alive (no sleep).
    pub shutdown_grace: Duration,
    /// Protocol stdout lines longer than this fail the connection. The stream
    /// cannot be trusted once a frame overruns.
    pub max_line_bytes: usize,
    /// Last N stderr lines retained for the Monitor. Older lines are dropped.
    pub stderr_lines: usize,
    /// Host → plugin write queue. Full → [`SessionError::Backpressure`], never
    /// unbounded growth, never a blocking UI caller.
    pub write_queue_capacity: usize,
    /// Plugin → host `Render`/`Call` queue. Full → drop, never stall the reader.
    pub inbound_queue_capacity: usize,
    /// Consecutive crashes after which the plugin is not restarted.
    pub max_restarts: u32,
    pub backoff_initial: Duration,
    pub backoff_ceiling: Duration,
    /// A connection that lives this long resets the crash counter.
    pub stable_after: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(5 * 60),
            handshake_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(2),
            max_line_bytes: 256 * 1024,
            stderr_lines: 200,
            write_queue_capacity: 32,
            inbound_queue_capacity: 64,
            max_restarts: 5,
            backoff_initial: Duration::from_millis(100),
            backoff_ceiling: Duration::from_secs(30),
            stable_after: Duration::from_secs(30),
        }
    }
}

impl SupervisorConfig {
    /// Tight bounds and a short backoff ceiling so tests can drive the
    /// supervisor with a manual clock rather than wall time.
    pub fn for_tests() -> Self {
        Self {
            idle: Duration::from_millis(1_000),
            handshake_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(2),
            shutdown_grace: Duration::ZERO,
            max_line_bytes: 1024,
            stderr_lines: 5,
            write_queue_capacity: 4,
            inbound_queue_capacity: 8,
            max_restarts: 3,
            backoff_initial: Duration::from_millis(100),
            backoff_ceiling: Duration::from_millis(400),
            stable_after: Duration::from_millis(10_000),
        }
    }
}

/// Monotonic milliseconds. Injected so idle eviction and backoff are
/// deterministic under a [`ManualClock`].
pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

/// Wall-clock-anchored monotonic milliseconds. The epoch anchor is taken once
/// at construction; afterwards only [`std::time::Instant`] advances the clock,
/// so an NTP step cannot fast-forward idle eviction or backoff.
pub struct SystemClock {
    origin: std::time::Instant,
    epoch_ms: u64,
}

impl SystemClock {
    pub fn new() -> Self {
        let epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            origin: std::time::Instant::now(),
            epoch_ms,
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.epoch_ms
            .saturating_add(self.origin.elapsed().as_millis() as u64)
    }
}

/// Test clock. `advance` is the only way time passes; nothing here sleeps.
pub struct ManualClock {
    now: AtomicU64,
}

impl ManualClock {
    pub fn new(start_ms: u64) -> Self {
        Self {
            now: AtomicU64::new(start_ms),
        }
    }

    pub fn advance(&self, ms: u64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set(&self, ms: u64) {
        self.now.store(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// Failures the supervisor can surface. Never a panic: every plugin byte is
/// untrusted, and every waiter must be woken with one of these rather than
/// left hanging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    Io(String),
    Protocol(String),
    VersionMismatch { plugin: u32 },
    CapabilityExceeded { capability: Capability },
    PluginFailed(String),
    Timeout(Duration),
    Backpressure,
    Disconnected,
    Backoff { until_ms: u64 },
    Disabled { restarts: u32 },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(m) => write!(f, "plugin I/O failed: {m}"),
            Self::Protocol(m) => write!(f, "plugin protocol error: {m}"),
            Self::VersionMismatch { plugin } => write!(
                f,
                "plugin speaks protocol {plugin}, host speaks {}",
                v2::PROTOCOL_VERSION
            ),
            Self::CapabilityExceeded { capability } => {
                write!(f, "plugin requested undeclared capability {capability:?}")
            }
            Self::PluginFailed(m) => write!(f, "plugin reported failure: {m}"),
            Self::Timeout(d) => write!(f, "plugin timed out after {}ms", d.as_millis()),
            Self::Backpressure => write!(f, "plugin write queue is full"),
            Self::Disconnected => write!(f, "plugin connection closed"),
            Self::Backoff { until_ms } => {
                write!(f, "plugin restart delayed until {until_ms}ms")
            }
            Self::Disabled { restarts } => {
                write!(f, "plugin disabled after {restarts} crashes")
            }
        }
    }
}

impl std::error::Error for SessionError {}

impl From<std::io::Error> for SessionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

fn mutex_lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A panicked supervisor thread must not cascade into the terminal.
    m.lock().unwrap_or_else(|e| e.into_inner())
}
