//! Byte-pipe abstraction for the supervisor.
//!
//! The supervisor must not be coupled to `std::process::Child`. If the only
//! way to test it is to launch binaries, it is not a solid foundation: races,
//! PATH, and OS scheduling leak into every assertion. A [`Launcher`] produces
//! framed line ends plus a [`PluginProcess`]; tests inject [`MemoryLauncher`],
//! production uses [`ProcessLauncher`].

use super::{LaunchSpec, SessionError};
use plugin_protocol::v2;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// One framed line, or a terminal condition. Oversized is distinct from a
/// parse error: the frame itself overran, so the stream may be desynchronized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecvLine {
    Line(String),
    Eof,
    Oversized,
}

pub trait LineSink: Send {
    fn send_line(&mut self, line: &str) -> io::Result<()>;
}

pub trait LineSource: Send {
    fn recv_line(&mut self, max_bytes: usize) -> io::Result<RecvLine>;
}

/// The OS process (or its in-memory stand-in). `kill` MUST unblock any
/// in-flight `send_line` / `recv_line` on the associated pipes, otherwise
/// teardown joins hang and a wedged plugin stalls the host.
pub trait PluginProcess: Send {
    fn pid(&self) -> Option<u32>;
    fn is_alive(&self) -> bool;
    fn kill(&mut self) -> io::Result<()>;
    /// Wait up to `timeout` for exit. The in-memory impl never sleeps: it
    /// returns immediately whether the stand-in is already dead.
    fn wait_timeout(&mut self, timeout: Duration) -> bool;
}

pub struct Spawned {
    pub stdin: Box<dyn LineSink>,
    pub stdout: Box<dyn LineSource>,
    pub stderr: Box<dyn LineSource>,
    pub process: Box<dyn PluginProcess>,
}

pub trait Launcher: Send + Sync {
    fn launch(&self, spec: &LaunchSpec) -> Result<Spawned, SessionError>;
}

// ---------------------------------------------------------------------------
// OS process transport
// ---------------------------------------------------------------------------

pub struct ProcessLauncher;

impl Launcher for ProcessLauncher {
    fn launch(&self, spec: &LaunchSpec) -> Result<Spawned, SessionError> {
        let mut child = Command::new(&spec.binary)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("SLEIPNIR_PLUGIN_ID", &spec.plugin_id)
            .env(
                "SLEIPNIR_PLUGIN_API_VERSION",
                v2::PROTOCOL_VERSION.to_string(),
            )
            .spawn()
            .map_err(|e| SessionError::Io(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SessionError::Protocol("plugin has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SessionError::Protocol("plugin has no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SessionError::Protocol("plugin has no stderr".into()))?;

        Ok(Spawned {
            stdin: Box::new(PipeSink { inner: stdin }),
            stdout: Box::new(PipeSource {
                inner: BufReader::new(stdout),
            }),
            stderr: Box::new(PipeSource {
                inner: BufReader::new(stderr),
            }),
            process: Box::new(ChildProcess(child)),
        })
    }
}

struct PipeSink<W: Write + Send> {
    inner: W,
}

impl<W: Write + Send> LineSink for PipeSink<W> {
    fn send_line(&mut self, line: &str) -> io::Result<()> {
        self.inner.write_all(line.as_bytes())?;
        self.inner.write_all(b"\n")?;
        self.inner.flush()
    }
}

struct PipeSource<R: Read + Send> {
    inner: BufReader<R>,
}

impl<R: Read + Send> LineSource for PipeSource<R> {
    fn recv_line(&mut self, max_bytes: usize) -> io::Result<RecvLine> {
        read_line_limited(&mut self.inner, max_bytes)
    }
}

/// Cap the frame *while reading* so a plugin that never sends a newline
/// cannot grow host memory without bound.
pub(crate) fn read_line_limited<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<RecvLine> {
    let mut buf = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if buf.is_empty() {
                return Ok(RecvLine::Eof);
            }
            return Ok(RecvLine::Line(String::from_utf8_lossy(&buf).into_owned()));
        }
        if let Some(i) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..i]);
            reader.consume(i + 1);
            if buf.len() > max_bytes {
                return Ok(RecvLine::Oversized);
            }
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(RecvLine::Line(String::from_utf8_lossy(&buf).into_owned()));
        }
        if buf.len().saturating_add(available.len()) > max_bytes {
            let n = available.len();
            reader.consume(n);
            let mut rest = Vec::new();
            let _ = reader.read_until(b'\n', &mut rest);
            return Ok(RecvLine::Oversized);
        }
        buf.extend_from_slice(available);
        let n = available.len();
        reader.consume(n);
    }
}

struct ChildProcess(Child);

impl PluginProcess for ChildProcess {
    fn pid(&self) -> Option<u32> {
        Some(self.0.id())
    }

    fn is_alive(&self) -> bool {
        // `try_wait` needs `&mut self`; we cannot call it here. The supervisor
        // observes death from the reader EOF path, which is the source of
        // truth for "the plugin is gone".
        true
    }

    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    fn wait_timeout(&mut self, timeout: Duration) -> bool {
        if timeout.is_zero() {
            return matches!(self.0.try_wait(), Ok(Some(_)));
        }
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) if std::time::Instant::now() >= deadline => return false,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => return false,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory transport
// ---------------------------------------------------------------------------

/// Bounded, close-able line pipe. `close` wakes every blocked send/recv so
/// teardown never waits on wall time.
struct PipeCore {
    inner: Mutex<PipeInner>,
    cv: Condvar,
}

struct PipeInner {
    lines: VecDeque<String>,
    cap: usize,
    closed: bool,
}

#[derive(Clone)]
pub struct MemorySink {
    core: Arc<PipeCore>,
}

#[derive(Clone)]
pub struct MemorySource {
    core: Arc<PipeCore>,
}

fn memory_pipe(cap: usize) -> (MemorySink, MemorySource) {
    let core = Arc::new(PipeCore {
        inner: Mutex::new(PipeInner {
            lines: VecDeque::new(),
            cap: cap.max(1),
            closed: false,
        }),
        cv: Condvar::new(),
    });
    (MemorySink { core: core.clone() }, MemorySource { core })
}

fn lock_pipe(core: &PipeCore) -> std::sync::MutexGuard<'_, PipeInner> {
    core.inner.lock().unwrap_or_else(|e| e.into_inner())
}

impl PipeCore {
    fn close(&self) {
        let mut inner = lock_pipe(self);
        inner.closed = true;
        self.cv.notify_all();
    }
}

impl LineSink for MemorySink {
    fn send_line(&mut self, line: &str) -> io::Result<()> {
        let mut inner = lock_pipe(&self.core);
        while inner.lines.len() >= inner.cap && !inner.closed {
            inner = self.core.cv.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        if inner.closed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "memory pipe closed",
            ));
        }
        inner.lines.push_back(line.to_string());
        self.core.cv.notify_all();
        Ok(())
    }
}

impl LineSource for MemorySource {
    fn recv_line(&mut self, max_bytes: usize) -> io::Result<RecvLine> {
        let mut inner = lock_pipe(&self.core);
        while inner.lines.is_empty() && !inner.closed {
            inner = self.core.cv.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        match inner.lines.pop_front() {
            Some(line) => {
                self.core.cv.notify_all();
                if line.len() > max_bytes {
                    Ok(RecvLine::Oversized)
                } else {
                    Ok(RecvLine::Line(line))
                }
            }
            None => Ok(RecvLine::Eof),
        }
    }
}

/// Test-side handle of a launched in-memory plugin. Dropping it is a crash:
/// stdout/stderr close, the reader sees EOF, pending waiters are woken.
pub struct PluginEndpoint {
    stdin: MemorySource,
    stdout: MemorySink,
    stderr: MemorySink,
    alive: Arc<AtomicBool>,
    /// Held so Drop can close the host-side pipes too (forced kill).
    cores: Vec<Arc<PipeCore>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointClosed;

impl PluginEndpoint {
    /// Block until the host writes a message. EOF → [`EndpointClosed`].
    pub fn recv(&mut self) -> Result<v2::HostMessage, EndpointClosed> {
        loop {
            match self.stdin.recv_line(usize::MAX) {
                Ok(RecvLine::Line(line)) if line.trim().is_empty() => continue,
                Ok(RecvLine::Line(line)) => {
                    return serde_json::from_str(line.trim()).map_err(|_| EndpointClosed);
                }
                Ok(RecvLine::Eof | RecvLine::Oversized) | Err(_) => return Err(EndpointClosed),
            }
        }
    }

    pub fn send(&mut self, msg: &v2::PluginMessage) -> Result<(), EndpointClosed> {
        let line = serde_json::to_string(msg).map_err(|_| EndpointClosed)?;
        self.send_raw(&line)
    }

    pub fn send_raw(&mut self, line: &str) -> Result<(), EndpointClosed> {
        self.stdout.send_line(line).map_err(|_| EndpointClosed)
    }

    pub fn write_stderr(&mut self, line: &str) -> Result<(), EndpointClosed> {
        self.stderr.send_line(line).map_err(|_| EndpointClosed)
    }

    /// Answer Hello with `ready` and return the Hello the host sent.
    pub fn handshake(
        &mut self,
        ready: &v2::PluginMessage,
    ) -> Result<v2::HostMessage, EndpointClosed> {
        let hello = self.recv()?;
        self.send(ready)?;
        Ok(hello)
    }
}

impl Drop for PluginEndpoint {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        for core in &self.cores {
            core.close();
        }
    }
}

struct MemoryProcess {
    alive: Arc<AtomicBool>,
    cores: Vec<Arc<PipeCore>>,
}

impl PluginProcess for MemoryProcess {
    fn pid(&self) -> Option<u32> {
        None
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn kill(&mut self) -> io::Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        for core in &self.cores {
            core.close();
        }
        Ok(())
    }

    fn wait_timeout(&mut self, _timeout: Duration) -> bool {
        !self.alive.load(Ordering::SeqCst)
    }
}

/// Hands each [`launch`](Launcher::launch) to the test via a channel.
pub struct MemoryLauncher {
    tx: Mutex<mpsc::Sender<PluginEndpoint>>,
    /// Protocol pipe capacity. Small values let tests fill the pipe and
    /// observe backpressure; production in-memory use is tests-only.
    pipe_cap: usize,
}

impl MemoryLauncher {
    pub fn pair() -> (Self, mpsc::Receiver<PluginEndpoint>) {
        Self::pair_with_cap(32)
    }

    pub fn pair_with_cap(pipe_cap: usize) -> (Self, mpsc::Receiver<PluginEndpoint>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                tx: Mutex::new(tx),
                pipe_cap: pipe_cap.max(1),
            },
            rx,
        )
    }
}

impl Launcher for MemoryLauncher {
    fn launch(&self, _spec: &LaunchSpec) -> Result<Spawned, SessionError> {
        let (host_stdin, plugin_stdin) = memory_pipe(self.pipe_cap);
        let (plugin_stdout, host_stdout) = memory_pipe(self.pipe_cap);
        // Unbounded-in-practice: a logging plugin must never block on stderr.
        let (plugin_stderr, host_stderr) = memory_pipe(usize::MAX / 4);

        let alive = Arc::new(AtomicBool::new(true));
        let cores = vec![
            host_stdin.core.clone(),
            plugin_stdout.core.clone(),
            plugin_stderr.core.clone(),
        ];

        let endpoint = PluginEndpoint {
            stdin: plugin_stdin,
            stdout: plugin_stdout,
            stderr: plugin_stderr,
            alive: alive.clone(),
            cores: cores.clone(),
        };

        self.tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(endpoint)
            .map_err(|_| SessionError::Io("test endpoint receiver dropped".into()))?;

        Ok(Spawned {
            stdin: Box::new(host_stdin),
            stdout: Box::new(host_stdout),
            stderr: Box::new(host_stderr),
            process: Box::new(MemoryProcess { alive, cores }),
        })
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn read_line_limited_caps_and_resyncs() {
        let mut cur = Cursor::new(b"ok\nthis-is-too-long\nnext\n");
        assert_eq!(
            read_line_limited(&mut cur, 8).unwrap(),
            RecvLine::Line("ok".into())
        );
        assert_eq!(read_line_limited(&mut cur, 8).unwrap(), RecvLine::Oversized);
        assert_eq!(
            read_line_limited(&mut cur, 8).unwrap(),
            RecvLine::Line("next".into())
        );
    }

    #[test]
    fn close_unblocks_recv() {
        let (sink, mut source) = memory_pipe(4);
        sink.core.close();
        assert_eq!(source.recv_line(64).unwrap(), RecvLine::Eof);
    }
}
