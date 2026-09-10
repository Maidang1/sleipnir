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
#[cfg(windows)]
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
#[cfg(not(windows))]
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
#[cfg(windows)]
use windows::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS,
    SetHandleInformation, WAIT_OBJECT_0,
};
#[cfg(windows)]
use windows::Win32::Security::SECURITY_ATTRIBUTES;
#[cfg(windows)]
use windows::Win32::System::IO::CancelSynchronousIo;
#[cfg(windows)]
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
#[cfg(windows)]
use windows::Win32::System::Pipes::CreatePipe;
#[cfg(windows)]
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetCurrentThread,
    InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_CREATION_FLAGS,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
#[cfg(windows)]
use windows::core::{PCWSTR, PWSTR};

const IO_POLL_INTERVAL: Duration = Duration::from_millis(10);

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

/// The OS process (or its in-memory stand-in).
///
/// Teardown needs two distinct operations:
/// - [`PluginProcess::cancel_io`] MUST unblock any in-flight `send_line` /
///   `recv_line` on the associated pipes so worker-thread joins stay bounded
///   even if the direct child already exited but a descendant still owns a
///   pipe.
/// - [`PluginProcess::kill`] MUST perform best-effort owned-process cleanup
///   for plugins that ignore `Shutdown` and for descendant processes still
///   retained by the launch boundary.
pub trait PluginProcess: Send {
    fn pid(&self) -> Option<u32>;
    fn cancel_io(&mut self) -> io::Result<()>;
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
        #[cfg(windows)]
        {
            return launch_windows(spec);
        }

        #[cfg(not(windows))]
        {
            let mut command = Command::new(&spec.binary);
            command
                .args(&spec.args)
                .current_dir(&spec.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env("SLEIPNIR_PLUGIN_ID", &spec.plugin_id)
                .env(
                    "SLEIPNIR_PLUGIN_API_VERSION",
                    v2::PROTOCOL_VERSION.to_string(),
                );
            #[cfg(unix)]
            command.process_group(0);

            let mut child = command
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

            #[cfg(unix)]
            {
                set_nonblocking(stdin.as_raw_fd())?;
                set_nonblocking(stdout.as_raw_fd())?;
                set_nonblocking(stderr.as_raw_fd())?;
            }

            let cancel = Arc::new(AtomicBool::new(false));

            Ok(Spawned {
                stdin: Box::new(PipeSink::new(stdin, cancel.clone())),
                stdout: Box::new(PipeSource::new(stdout, cancel.clone())),
                stderr: Box::new(PipeSource::new(stderr, cancel.clone())),
                process: Box::new(ChildProcess::new(child, cancel)),
            })
        }
    }
}

struct PipeSink<W: Write + Send> {
    inner: W,
    cancel: Arc<AtomicBool>,
    #[cfg(windows)]
    io_thread: Arc<ThreadHandleSlot>,
}

impl<W: Write + Send> PipeSink<W> {
    #[cfg(not(windows))]
    fn new(inner: W, cancel: Arc<AtomicBool>) -> Self {
        Self { inner, cancel }
    }

    #[cfg(windows)]
    fn new(inner: W, cancel: Arc<AtomicBool>, io_thread: Arc<ThreadHandleSlot>) -> Self {
        Self {
            inner,
            cancel,
            io_thread,
        }
    }
}

impl<W: Write + Send> LineSink for PipeSink<W> {
    fn send_line(&mut self, line: &str) -> io::Result<()> {
        #[cfg(windows)]
        self.io_thread.register_current_thread()?;

        let mut frame = Vec::with_capacity(line.len() + 1);
        frame.extend_from_slice(line.as_bytes());
        frame.push(b'\n');
        write_all_cancellable(&mut self.inner, &frame, &self.cancel)?;
        flush_cancellable(&mut self.inner, &self.cancel)
    }
}

struct PipeSource<R: Read + Send> {
    inner: R,
    cancel: Arc<AtomicBool>,
    buffered: Vec<u8>,
    #[cfg(windows)]
    io_thread: Arc<ThreadHandleSlot>,
}

impl<R: Read + Send> PipeSource<R> {
    #[cfg(not(windows))]
    fn new(inner: R, cancel: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            cancel,
            buffered: Vec::new(),
        }
    }

    #[cfg(windows)]
    fn new(inner: R, cancel: Arc<AtomicBool>, io_thread: Arc<ThreadHandleSlot>) -> Self {
        Self {
            inner,
            cancel,
            buffered: Vec::new(),
            io_thread,
        }
    }

    fn pop_buffered_line(&mut self, max_bytes: usize) -> Option<RecvLine> {
        let newline = self.buffered.iter().position(|&b| b == b'\n')?;
        let mut line = self.buffered.drain(..=newline).collect::<Vec<_>>();
        debug_assert_eq!(line.pop(), Some(b'\n'));
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.len() > max_bytes {
            return Some(RecvLine::Oversized);
        }
        Some(RecvLine::Line(String::from_utf8_lossy(&line).into_owned()))
    }
}

impl<R: Read + Send> LineSource for PipeSource<R> {
    fn recv_line(&mut self, max_bytes: usize) -> io::Result<RecvLine> {
        #[cfg(windows)]
        self.io_thread.register_current_thread()?;

        loop {
            if let Some(line) = self.pop_buffered_line(max_bytes) {
                return Ok(line);
            }
            if self.buffered.len() > max_bytes {
                return Ok(RecvLine::Oversized);
            }
            if self.cancel.load(Ordering::SeqCst) {
                return Ok(RecvLine::Eof);
            }

            let mut chunk = [0_u8; 4096];
            match self.inner.read(&mut chunk) {
                Ok(0) => {
                    if self.buffered.is_empty() {
                        return Ok(RecvLine::Eof);
                    }
                    let mut line = std::mem::take(&mut self.buffered);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.len() > max_bytes {
                        return Ok(RecvLine::Oversized);
                    }
                    return Ok(RecvLine::Line(String::from_utf8_lossy(&line).into_owned()));
                }
                Ok(n) => {
                    self.buffered.extend_from_slice(&chunk[..n]);
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                #[cfg(windows)]
                Err(err) if is_cancelled_io(&err, &self.cancel) => {
                    return Ok(RecvLine::Eof);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    if self.cancel.load(Ordering::SeqCst) {
                        return Ok(RecvLine::Eof);
                    }
                    std::thread::sleep(IO_POLL_INTERVAL);
                }
                Err(err) => return Err(err),
            }
        }
    }
}

fn write_all_cancellable<W: Write>(
    writer: &mut W,
    mut buf: &[u8],
    cancel: &AtomicBool,
) -> io::Result<()> {
    while !buf.is_empty() {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_io_error());
        }
        match writer.write(buf) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "plugin pipe closed while writing",
                ));
            }
            Ok(n) => buf = &buf[n..],
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            #[cfg(windows)]
            Err(err) if is_cancelled_io(&err, cancel) => return Err(cancelled_io_error()),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if cancel.load(Ordering::SeqCst) {
                    return Err(cancelled_io_error());
                }
                std::thread::sleep(IO_POLL_INTERVAL);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn flush_cancellable<W: Write>(writer: &mut W, cancel: &AtomicBool) -> io::Result<()> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled_io_error());
        }
        match writer.flush() {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            #[cfg(windows)]
            Err(err) if is_cancelled_io(&err, cancel) => return Err(cancelled_io_error()),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if cancel.load(Ordering::SeqCst) {
                    return Err(cancelled_io_error());
                }
                std::thread::sleep(IO_POLL_INTERVAL);
            }
            Err(err) => return Err(err),
        }
    }
}

fn cancelled_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "plugin pipe cancelled")
}

#[cfg(windows)]
fn is_cancelled_io(err: &io::Error, cancel: &AtomicBool) -> bool {
    cancel.load(Ordering::SeqCst) && err.raw_os_error() == Some(995)
}

/// Cap the frame while reading. Oversized returns immediately because the
/// caller closes the session; there is no resync/discard path here.
#[cfg_attr(not(test), allow(dead_code))]
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
            return Ok(RecvLine::Oversized);
        }
        buf.extend_from_slice(available);
        let n = available.len();
        reader.consume(n);
    }
}

struct ChildProcess {
    #[cfg(not(windows))]
    child: Child,
    #[cfg(windows)]
    process: OwnedHandle,
    #[cfg(windows)]
    job: Option<OwnedHandle>,
    #[cfg(windows)]
    pid: u32,
    cancel: Arc<AtomicBool>,
    #[cfg(windows)]
    io_threads: Vec<Arc<ThreadHandleSlot>>,
}

impl ChildProcess {
    #[cfg(not(windows))]
    fn new(child: Child, cancel: Arc<AtomicBool>) -> Self {
        Self { child, cancel }
    }

    #[cfg(windows)]
    fn new(
        process: OwnedHandle,
        job: OwnedHandle,
        pid: u32,
        cancel: Arc<AtomicBool>,
        io_threads: Vec<Arc<ThreadHandleSlot>>,
    ) -> Self {
        Self {
            process,
            job: Some(job),
            pid,
            cancel,
            io_threads,
        }
    }

    #[cfg(unix)]
    fn kill_owned_group(&mut self) -> io::Result<()> {
        let Some(pid) = self.pid() else {
            return Ok(());
        };
        let rc = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(err)
    }

    #[cfg(unix)]
    fn reap_direct_child_once(&mut self) -> io::Result<()> {
        match self.child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => self.child.kill().or_else(ignore_known_exited_child_error),
            Err(err) => ignore_known_exited_child_error(err),
        }
    }
}

#[cfg(unix)]
fn ignore_known_exited_child_error(err: io::Error) -> io::Result<()> {
    match err.kind() {
        io::ErrorKind::InvalidInput => Ok(()),
        _ if err.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        _ => Err(err),
    }
}

impl PluginProcess for ChildProcess {
    fn pid(&self) -> Option<u32> {
        #[cfg(not(windows))]
        {
            Some(self.child.id())
        }
        #[cfg(windows)]
        {
            Some(self.pid)
        }
    }

    fn cancel_io(&mut self) -> io::Result<()> {
        self.cancel.store(true, Ordering::SeqCst);

        #[cfg(unix)]
        {
            return Ok(());
        }

        #[cfg(windows)]
        {
            for thread in &self.io_threads {
                thread.cancel();
            }
            return Ok(());
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        let _ = self.cancel_io();

        #[cfg(unix)]
        {
            self.kill_owned_group()?;
            return self.reap_direct_child_once();
        }

        #[cfg(windows)]
        {
            if let Some(job) = self.job.take() {
                drop(job);
            }
            for thread in &self.io_threads {
                thread.cancel();
            }
            return Ok(());
        }
    }

    fn wait_timeout(&mut self, timeout: Duration) -> bool {
        #[cfg(windows)]
        {
            let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
            return unsafe {
                WaitForSingleObject(HANDLE(self.process.as_raw_handle()), timeout_ms)
                    == WAIT_OBJECT_0
            };
        }

        #[cfg(not(windows))]
        {
            if timeout.is_zero() {
                return matches!(self.child.try_wait(), Ok(Some(_)));
            }
            let deadline = std::time::Instant::now() + timeout;
            loop {
                match self.child.try_wait() {
                    Ok(Some(_)) => return true,
                    Ok(None) if std::time::Instant::now() >= deadline => return false,
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => return false,
                }
            }
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
#[derive(Default)]
struct ThreadHandleSlot {
    handle: Mutex<Option<OwnedHandle>>,
    cancel_requested: AtomicBool,
}

#[cfg(windows)]
impl ThreadHandleSlot {
    fn register_current_thread(&self) -> io::Result<()> {
        let mut guard = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return Ok(());
        }
        let mut dup = HANDLE::default();
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                GetCurrentThread(),
                GetCurrentProcess(),
                &mut dup,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        }
        .map_err(windows_io_error)?;
        *guard = Some(unsafe { OwnedHandle::from_raw_handle(dup.0) });
        if self.cancel_requested.load(Ordering::SeqCst) {
            if let Some(handle) = guard.as_ref() {
                let _ = unsafe { CancelSynchronousIo(HANDLE(handle.as_raw_handle())) };
            }
        }
        Ok(())
    }

    fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        let guard = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = guard.as_ref() {
            let _ = unsafe { CancelSynchronousIo(HANDLE(handle.as_raw_handle())) };
        }
    }
}

#[cfg(windows)]
fn launch_windows(spec: &LaunchSpec) -> Result<Spawned, SessionError> {
    let stdin_pipe = PipeEnds::new_stdin()?;
    let stdout_pipe = PipeEnds::new_stdout()?;
    let stderr_pipe = PipeEnds::new_stdout()?;
    let job = create_kill_on_close_job()?;
    let child = spawn_suspended_windows_process(
        spec,
        stdin_pipe.child_end,
        stdout_pipe.child_end,
        stderr_pipe.child_end,
    )?;
    let child_guard = SuspendedWindowsChildGuard::new(child);
    assign_process_to_job(
        &job,
        child_guard
            .process
            .as_ref()
            .expect("suspended child process missing"),
    )?;
    resume_thread(
        child_guard
            .thread
            .as_ref()
            .expect("suspended child thread missing"),
    )?;
    let child = child_guard.disarm();

    let writer_thread = Arc::new(ThreadHandleSlot::default());
    let stdout_thread = Arc::new(ThreadHandleSlot::default());
    let stderr_thread = Arc::new(ThreadHandleSlot::default());
    let cancel = Arc::new(AtomicBool::new(false));

    Ok(Spawned {
        stdin: Box::new(PipeSink::new(
            File::from(stdin_pipe.parent_end),
            cancel.clone(),
            writer_thread.clone(),
        )),
        stdout: Box::new(PipeSource::new(
            File::from(stdout_pipe.parent_end),
            cancel.clone(),
            stdout_thread.clone(),
        )),
        stderr: Box::new(PipeSource::new(
            File::from(stderr_pipe.parent_end),
            cancel.clone(),
            stderr_thread.clone(),
        )),
        process: Box::new(ChildProcess::new(
            child.process,
            job,
            child.pid,
            cancel,
            vec![writer_thread, stdout_thread, stderr_thread],
        )),
    })
}

#[cfg(windows)]
struct PipeEnds {
    parent_end: OwnedHandle,
    child_end: OwnedHandle,
}

#[cfg(windows)]
impl PipeEnds {
    fn stdin_pair() -> Result<Self, SessionError> {
        let mut read = HANDLE::default();
        let mut write = HANDLE::default();
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: true.into(),
        };
        unsafe { CreatePipe(&mut read, &mut write, Some(&mut sa), 0) }
            .map_err(windows_session_error)?;
        Ok(Self {
            parent_end: unsafe { OwnedHandle::from_raw_handle(write.0) },
            child_end: unsafe { OwnedHandle::from_raw_handle(read.0) },
        })
    }

    fn stdout_pair() -> Result<Self, SessionError> {
        let mut read = HANDLE::default();
        let mut write = HANDLE::default();
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: true.into(),
        };
        unsafe { CreatePipe(&mut read, &mut write, Some(&mut sa), 0) }
            .map_err(windows_session_error)?;
        Ok(Self {
            parent_end: unsafe { OwnedHandle::from_raw_handle(read.0) },
            child_end: unsafe { OwnedHandle::from_raw_handle(write.0) },
        })
    }
}

#[cfg(windows)]
impl PipeEnds {
    fn new_stdin() -> Result<Self, SessionError> {
        let pair = Self::stdin_pair()?;
        clear_inherit(HANDLE(pair.parent_end.as_raw_handle()))
            .map_err(|err| SessionError::Io(err.to_string()))?;
        Ok(pair)
    }

    fn new_stdout() -> Result<Self, SessionError> {
        let pair = Self::stdout_pair()?;
        clear_inherit(HANDLE(pair.parent_end.as_raw_handle()))
            .map_err(|err| SessionError::Io(err.to_string()))?;
        Ok(pair)
    }
}

#[cfg(windows)]
fn clear_inherit(handle: HANDLE) -> io::Result<()> {
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
        .map_err(windows_io_error)
}

#[cfg(windows)]
struct InheritHandleList {
    buffer: Box<[u8]>,
    list: windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
}

#[cfg(windows)]
impl InheritHandleList {
    fn new(handles: &[HANDLE]) -> Result<Self, SessionError> {
        let mut size = 0usize;
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, Some(0), &mut size) };
        if size == 0 {
            return Err(SessionError::Io(
                "InitializeProcThreadAttributeList size query returned zero".into(),
            ));
        }

        let mut buffer = vec![0_u8; size].into_boxed_slice();
        let list = windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST(
            buffer.as_mut_ptr().cast(),
        );
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, Some(0), &mut size) }
            .map_err(windows_session_error)?;
        unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(windows_session_error)?;

        Ok(Self { buffer, list })
    }
}

#[cfg(windows)]
impl Drop for InheritHandleList {
    fn drop(&mut self) {
        let _ = self.buffer.len();
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

#[cfg(windows)]
struct SpawnedWindowsChild {
    process: OwnedHandle,
    thread: OwnedHandle,
    pid: u32,
}

#[cfg(windows)]
struct SuspendedWindowsChildGuard {
    process: Option<OwnedHandle>,
    thread: Option<OwnedHandle>,
    pid: u32,
    armed: bool,
}

#[cfg(windows)]
impl SuspendedWindowsChildGuard {
    fn new(child: SpawnedWindowsChild) -> Self {
        Self {
            process: Some(child.process),
            thread: Some(child.thread),
            pid: child.pid,
            armed: true,
        }
    }

    fn disarm(mut self) -> SpawnedWindowsChild {
        self.armed = false;
        SpawnedWindowsChild {
            process: self
                .process
                .take()
                .expect("suspended child process missing"),
            thread: self.thread.take().expect("suspended child thread missing"),
            pid: self.pid,
        }
    }
}

#[cfg(windows)]
impl Drop for SuspendedWindowsChildGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(process) = self.process.as_ref() {
            let _ = unsafe { TerminateProcess(HANDLE(process.as_raw_handle()), 1) };
            let _ = unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 5_000) };
        }
    }
}

#[cfg(windows)]
fn spawn_suspended_windows_process(
    spec: &LaunchSpec,
    child_stdin: OwnedHandle,
    child_stdout: OwnedHandle,
    child_stderr: OwnedHandle,
) -> Result<SpawnedWindowsChild, SessionError> {
    let application = wide_null(OsStr::new(&spec.binary));
    let mut command_line = build_command_line(OsStr::new(&spec.binary), &spec.args);
    let cwd = wide_null(spec.cwd.as_os_str());
    let env = build_environment_block(spec);
    let inherit_handles = [
        HANDLE(child_stdin.as_raw_handle()),
        HANDLE(child_stdout.as_raw_handle()),
        HANDLE(child_stderr.as_raw_handle()),
    ];
    let attribute_list = InheritHandleList::new(&inherit_handles)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherit_handles[0];
    startup.StartupInfo.hStdOutput = inherit_handles[1];
    startup.StartupInfo.hStdError = inherit_handles[2];
    startup.lpAttributeList = attribute_list.list;
    let mut process_info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            PROCESS_CREATION_FLAGS(
                CREATE_SUSPENDED.0 | CREATE_UNICODE_ENVIRONMENT.0 | EXTENDED_STARTUPINFO_PRESENT.0,
            ),
            Some(env.as_ptr().cast()),
            PCWSTR(cwd.as_ptr()),
            &startup.StartupInfo,
            &mut process_info,
        )
    }
    .map_err(windows_session_error)?;
    Ok(SpawnedWindowsChild {
        process: unsafe { OwnedHandle::from_raw_handle(process_info.hProcess.0) },
        thread: unsafe { OwnedHandle::from_raw_handle(process_info.hThread.0) },
        pid: process_info.dwProcessId,
    })
}

#[cfg(windows)]
fn create_kill_on_close_job() -> Result<OwnedHandle, SessionError> {
    let job = unsafe { CreateJobObjectW(None, None) }.map_err(windows_session_error)?;
    let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            HANDLE(job.as_raw_handle()),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(windows_session_error)?;
    Ok(job)
}

#[cfg(windows)]
fn assign_process_to_job(job: &OwnedHandle, process: &OwnedHandle) -> Result<(), SessionError> {
    unsafe {
        AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process.as_raw_handle()))
    }
    .map_err(windows_session_error)
}

#[cfg(windows)]
fn resume_thread(thread: &OwnedHandle) -> Result<(), SessionError> {
    let resumed = unsafe { ResumeThread(HANDLE(thread.as_raw_handle())) };
    if resumed == u32::MAX {
        return Err(SessionError::Io(io::Error::last_os_error().to_string()));
    }
    Ok(())
}

#[cfg(windows)]
fn build_environment_block(spec: &LaunchSpec) -> Vec<u16> {
    let mut entries: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    entries.push((
        OsString::from("SLEIPNIR_PLUGIN_ID"),
        OsString::from(&spec.plugin_id),
    ));
    entries.push((
        OsString::from("SLEIPNIR_PLUGIN_API_VERSION"),
        OsString::from(v2::PROTOCOL_VERSION.to_string()),
    ));
    entries.sort_by(|(ak, _), (bk, _)| {
        ak.to_string_lossy()
            .to_ascii_lowercase()
            .cmp(&bk.to_string_lossy().to_ascii_lowercase())
    });
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend(key.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(windows)]
fn build_command_line(program: &OsStr, args: &[String]) -> Vec<u16> {
    let mut cmd = quote_windows_arg(program);
    for arg in args {
        cmd.push(' ' as u16);
        cmd.extend(quote_windows_arg(OsStr::new(arg)));
    }
    cmd.push(0);
    cmd
}

#[cfg(windows)]
fn quote_windows_arg(arg: &OsStr) -> Vec<u16> {
    let wide: Vec<u16> = arg.encode_wide().collect();
    let needs_quotes = wide.is_empty()
        || wide
            .iter()
            .any(|ch| *ch == b' ' as u16 || *ch == b'\t' as u16 || *ch == b'"' as u16);
    if !needs_quotes {
        return wide;
    }

    let mut out = Vec::with_capacity(wide.len() + 2);
    out.push(b'"' as u16);
    let mut slashes = 0usize;
    for ch in wide {
        if ch == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if ch == b'"' as u16 {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            out.push(ch);
            slashes = 0;
            continue;
        }
        if slashes > 0 {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            slashes = 0;
        }
        out.push(ch);
    }
    if slashes > 0 {
        out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    }
    out.push(b'"' as u16);
    out
}

#[cfg(windows)]
fn wide_null(value: &OsStr) -> Vec<u16> {
    let mut wide: Vec<u16> = value.encode_wide().collect();
    wide.push(0);
    wide
}

#[cfg(windows)]
fn windows_io_error(_: windows::core::Error) -> io::Error {
    io::Error::last_os_error()
}

#[cfg(windows)]
fn windows_session_error(_: windows::core::Error) -> SessionError {
    SessionError::Io(io::Error::last_os_error().to_string())
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

    fn cancel_io(&mut self) -> io::Result<()> {
        for core in &self.cores {
            core.close();
        }
        Ok(())
    }

    fn kill(&mut self) -> io::Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        self.cancel_io()
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
    use crate::PluginLifecycle;
    use std::collections::BTreeSet;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    #[cfg(unix)]
    use std::time::Instant;

    #[test]
    fn read_line_limited_reports_oversized_without_discarding() {
        let mut cur = Cursor::new(b"ok\nthis-is-too-long\nnext\n");
        assert_eq!(
            read_line_limited(&mut cur, 8).unwrap(),
            RecvLine::Line("ok".into())
        );
        assert_eq!(read_line_limited(&mut cur, 8).unwrap(), RecvLine::Oversized);
    }

    #[test]
    fn close_unblocks_recv() {
        let (sink, mut source) = memory_pipe(4);
        sink.core.close();
        assert_eq!(source.recv_line(64).unwrap(), RecvLine::Eof);
    }

    #[cfg(unix)]
    #[test]
    fn kill_terminates_owned_process_group_and_unblocks_reader() {
        let launcher = ProcessLauncher;
        let spec = LaunchSpec {
            plugin_id: "demo".into(),
            lifecycle: PluginLifecycle::Resident,
            declared_capabilities: BTreeSet::new(),
            granted: vec![],
            binary: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30 & wait".into()],
            cwd: PathBuf::from("/"),
        };

        let spawned = launcher.launch(&spec).expect("spawn shell");
        let mut stdout = spawned.stdout;
        let mut process = spawned.process;
        let (done_tx, done_rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let result = stdout.recv_line(1024);
            let _ = done_tx.send(result);
        });

        process.kill().expect("kill process group");
        assert!(
            process.wait_timeout(Duration::from_secs(2)),
            "child should exit after group kill"
        );
        let recv = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader should unblock")
            .expect("reader should not error after kill");
        assert_eq!(recv, RecvLine::Eof);
        reader.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn kill_cleans_descendant_after_parent_exit_when_descendant_keeps_pipe_open() {
        let launcher = ProcessLauncher;
        let marker = format!(
            "/tmp/sleipnir-plugin-host-descendant-{}",
            std::process::id()
        );
        let _ = std::fs::remove_file(&marker);
        let spec = LaunchSpec {
            plugin_id: "demo".into(),
            lifecycle: PluginLifecycle::Resident,
            declared_capabilities: BTreeSet::new(),
            granted: vec![],
            binary: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "sleep 30 & child=$!; printf '%s\\n' \"$child\"; printf . > {}; exit 0",
                    marker
                ),
            ],
            cwd: PathBuf::from("/"),
        };

        let spawned = launcher.launch(&spec).expect("spawn shell");
        let mut stdout = spawned.stdout;
        let mut process = spawned.process;
        let child_pid = match stdout.recv_line(1024).expect("read descendant pid") {
            RecvLine::Line(line) => line.trim().parse::<i32>().expect("pid line"),
            other => panic!("expected descendant pid line, got {other:?}"),
        };
        let (done_tx, done_rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let result = stdout.recv_line(1024);
            let _ = done_tx.send(result);
        });

        assert!(
            process.wait_timeout(Duration::from_secs(2)),
            "shell parent should exit cleanly"
        );
        process.cancel_io().expect("cancel pipe io");
        let recv = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader should unblock after pipe cancellation")
            .expect("reader should not error after pipe cancellation");
        assert_eq!(recv, RecvLine::Eof);
        process.kill().expect("cleanup descendant process group");
        assert!(
            process.wait_timeout(Duration::from_secs(2)),
            "direct child should stay reaped after owned-group cleanup"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let rc = unsafe { libc::kill(child_pid, 0) };
            if rc != 0 {
                let err = io::Error::last_os_error();
                assert_eq!(
                    err.raw_os_error(),
                    Some(libc::ESRCH),
                    "unexpected kill(0) status while checking descendant {child_pid}: {err}"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant {child_pid} still alive after owned-group cleanup"
            );
            std::thread::yield_now();
        }
        reader.join().unwrap();
        let _ = std::fs::remove_file(marker);
    }
}
