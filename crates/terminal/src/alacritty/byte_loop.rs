//! PTY read loop with a scanner in front of the ANSI parser.
//!
//! Alacritty's event loop owns the `vte` processor and does not expose a hook
//! before `parser.advance`. Kitty graphics are APC sequences, and `vte` drops
//! those with no `Perform` callback, so the split has to happen here — on the
//! bytes just read — without editing `alacritty_terminal`.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::fmt::{self, Display, Formatter};
use std::io::{self, ErrorKind, Read, Write};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Instant;

use alacritty_terminal::event::{self, Event, EventListener};
use alacritty_terminal::event_loop::Msg;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::thread;
use alacritty_terminal::tty::{self, EventedPty};
use log::error;
use polling::{Event as PollingEvent, Events, PollMode, Poller};
use vte::ansi;

use crate::scanner::{Scanner, Segment};

/// Max bytes to read from the PTY before forced terminal synchronization.
/// Matches alacritty's `READ_BUFFER_SIZE`.
const READ_BUFFER_SIZE: usize = 0x10_0000;

/// Max bytes to read from the PTY while the terminal is locked.
const MAX_LOCKED_READ: usize = u16::MAX as usize;

#[cfg(unix)]
const PTY_READ_WRITE_TOKEN: usize = 0;
#[cfg(unix)]
const PTY_CHILD_EVENT_TOKEN: usize = 1;

#[cfg(windows)]
const PTY_CHILD_EVENT_TOKEN: usize = 1;
#[cfg(windows)]
const PTY_READ_WRITE_TOKEN: usize = 2;

/// Sends input, resizes, and shutdown into the byte loop.
#[derive(Clone)]
pub(super) struct ByteLoopSender {
    sender: Sender<Msg>,
    poller: Arc<Poller>,
}

impl ByteLoopSender {
    pub(super) fn send(&self, msg: Msg) -> Result<(), ByteLoopSendError> {
        self.sender.send(msg).map_err(ByteLoopSendError::send)?;
        self.poller.notify().map_err(ByteLoopSendError::io)
    }
}

#[derive(Debug)]
pub(super) struct ByteLoopSendError {
    kind: ByteLoopSendErrorKind,
}

#[derive(Debug)]
enum ByteLoopSendErrorKind {
    Io(io::Error),
    Send(mpsc::SendError<Msg>),
}

impl ByteLoopSendError {
    fn io(err: io::Error) -> Self {
        Self {
            kind: ByteLoopSendErrorKind::Io(err),
        }
    }

    fn send(err: mpsc::SendError<Msg>) -> Self {
        Self {
            kind: ByteLoopSendErrorKind::Send(err),
        }
    }
}

impl Display for ByteLoopSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ByteLoopSendErrorKind::Io(err) => err.fmt(f),
            ByteLoopSendErrorKind::Send(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ByteLoopSendError {}

struct Writing {
    source: Cow<'static, [u8]>,
    written: usize,
}

impl Writing {
    fn new(source: Cow<'static, [u8]>) -> Self {
        Self { source, written: 0 }
    }

    fn advance(&mut self, n: usize) {
        self.written += n;
    }

    fn remaining_bytes(&self) -> &[u8] {
        &self.source[self.written..]
    }

    fn finished(&self) -> bool {
        self.written >= self.source.len()
    }
}

struct LoopState {
    write_list: VecDeque<Cow<'static, [u8]>>,
    writing: Option<Writing>,
    parser: ansi::Processor,
    scanner: Scanner,
}

impl LoopState {
    fn ensure_next(&mut self) {
        if self.writing.is_none() {
            self.goto_next();
        }
    }

    fn goto_next(&mut self) {
        self.writing = self.write_list.pop_front().map(Writing::new);
    }

    fn take_current(&mut self) -> Option<Writing> {
        self.writing.take()
    }

    fn needs_write(&self) -> bool {
        self.writing.is_some() || !self.write_list.is_empty()
    }

    fn set_current(&mut self, new: Option<Writing>) {
        self.writing = new;
    }
}

impl Default for LoopState {
    fn default() -> Self {
        Self {
            write_list: VecDeque::new(),
            writing: None,
            parser: ansi::Processor::new(),
            scanner: Scanner::new(),
        }
    }
}

struct PeekableReceiver<T> {
    rx: Receiver<T>,
    peeked: Option<T>,
}

impl<T> PeekableReceiver<T> {
    fn new(rx: Receiver<T>) -> Self {
        Self { rx, peeked: None }
    }

    fn peek(&mut self) -> Option<&T> {
        if self.peeked.is_none() {
            self.peeked = self.rx.try_recv().ok();
        }
        self.peeked.as_ref()
    }

    fn recv(&mut self) -> Option<T> {
        if self.peeked.is_some() {
            self.peeked.take()
        } else {
            match self.rx.try_recv() {
                Err(TryRecvError::Disconnected) => panic!("event loop channel closed"),
                res => res.ok(),
            }
        }
    }
}

pub(super) struct ByteLoop<T, U>
where
    T: EventedPty + event::OnResize,
    U: EventListener,
{
    poll: Arc<Poller>,
    pty: T,
    rx: PeekableReceiver<Msg>,
    terminal: Arc<FairMutex<Term<U>>>,
    event_proxy: U,
    drain_on_exit: bool,
}

impl<T, U> ByteLoop<T, U>
where
    T: EventedPty + event::OnResize + Send + 'static,
    U: EventListener + Send + 'static,
{
    pub(super) fn spawn(
        terminal: Arc<FairMutex<Term<U>>>,
        event_proxy: U,
        pty: T,
        drain_on_exit: bool,
    ) -> io::Result<ByteLoopSender> {
        let (tx, rx) = mpsc::channel();
        let poll: Arc<Poller> = Poller::new()?.into();
        let sender = ByteLoopSender {
            sender: tx,
            poller: poll.clone(),
        };
        let mut loop_ = ByteLoop {
            poll,
            pty,
            rx: PeekableReceiver::new(rx),
            terminal,
            event_proxy,
            drain_on_exit,
        };
        let _thread: JoinHandle<()> = thread::spawn_named("PTY reader", move || loop_.run());
        Ok(sender)
    }

    fn run(&mut self) {
        let mut state = LoopState::default();
        let mut buf = [0u8; READ_BUFFER_SIZE];
        let poll_opts = PollMode::Level;
        let mut interest = PollingEvent::readable(0);

        if let Err(err) = unsafe { self.pty.register(&self.poll, interest, poll_opts) } {
            error!("Event loop registration error: {err}");
            return;
        }

        let mut events = Events::with_capacity(NonZeroUsize::new(1024).unwrap());

        'event_loop: loop {
            let handler = state.parser.sync_timeout();
            let timeout = handler
                .sync_timeout()
                .map(|st| st.saturating_duration_since(Instant::now()));

            events.clear();
            if let Err(err) = self.poll.wait(&mut events, timeout) {
                match err.kind() {
                    ErrorKind::Interrupted => continue,
                    _ => {
                        error!("Event loop polling error: {err}");
                        break 'event_loop;
                    }
                }
            }

            if events.is_empty() && self.rx.peek().is_none() {
                state.parser.stop_sync(&mut *self.terminal.lock());
                self.event_proxy.send_event(Event::Wakeup);
                continue;
            }

            if !self.drain_recv_channel(&mut state) {
                break;
            }

            for event in events.iter() {
                match event.key {
                    PTY_CHILD_EVENT_TOKEN => {
                        if let Some(tty::ChildEvent::Exited(status)) = self.pty.next_child_event() {
                            if let Some(status) = status {
                                self.event_proxy.send_event(Event::ChildExit(status));
                            }
                            if self.drain_on_exit {
                                let _ = self.pty_read(&mut state, &mut buf);
                            }
                            self.terminal.lock().exit();
                            self.event_proxy.send_event(Event::Wakeup);
                            break 'event_loop;
                        }
                    }
                    PTY_READ_WRITE_TOKEN => {
                        if event.is_interrupt() {
                            continue;
                        }
                        if event.readable
                            && let Err(err) = self.pty_read(&mut state, &mut buf)
                        {
                            #[cfg(target_os = "linux")]
                            if err.raw_os_error() == Some(libc::EIO) {
                                continue;
                            }
                            error!("Error reading from PTY in event loop: {err}");
                            break 'event_loop;
                        }
                        if event.writable
                            && let Err(err) = self.pty_write(&mut state)
                        {
                            error!("Error writing to PTY in event loop: {err}");
                            break 'event_loop;
                        }
                    }
                    _ => {}
                }
            }

            let needs_write = state.needs_write();
            if needs_write != interest.writable {
                interest.writable = needs_write;
                if let Err(err) = self.pty.reregister(&self.poll, interest, poll_opts) {
                    error!("Event loop reregister error: {err}");
                    break 'event_loop;
                }
            }
        }

        let _ = self.pty.deregister(&self.poll);
    }

    fn drain_recv_channel(&mut self, state: &mut LoopState) -> bool {
        while let Some(msg) = self.rx.recv() {
            match msg {
                Msg::Input(input) => state.write_list.push_back(input),
                Msg::Resize(window_size) => self.pty.on_resize(window_size),
                Msg::Shutdown => return false,
            }
        }
        true
    }

    fn pty_read(&mut self, state: &mut LoopState, buf: &mut [u8]) -> io::Result<()> {
        let mut unprocessed = 0;
        let mut processed = 0;
        let _terminal_lease = Some(self.terminal.lease());
        let mut terminal = None;

        loop {
            match self.pty.reader().read(&mut buf[unprocessed..]) {
                Ok(0) if unprocessed == 0 => break,
                Ok(got) => unprocessed += got,
                Err(err) => match err.kind() {
                    ErrorKind::Interrupted | ErrorKind::WouldBlock => {
                        if unprocessed == 0 {
                            break;
                        }
                    }
                    _ => return Err(err),
                },
            }

            let terminal = match &mut terminal {
                Some(terminal) => terminal,
                None => terminal.insert(match self.terminal.try_lock_unfair() {
                    None if unprocessed >= READ_BUFFER_SIZE => self.terminal.lock_unfair(),
                    None => continue,
                    Some(terminal) => terminal,
                }),
            };

            feed_parser(
                &mut state.parser,
                &mut state.scanner,
                terminal,
                &buf[..unprocessed],
            );

            processed += unprocessed;
            unprocessed = 0;

            if processed >= MAX_LOCKED_READ {
                break;
            }
        }

        if state.parser.sync_bytes_count() < processed && processed > 0 {
            self.event_proxy.send_event(Event::Wakeup);
        }

        Ok(())
    }

    fn pty_write(&mut self, state: &mut LoopState) -> io::Result<()> {
        state.ensure_next();

        'write_many: while let Some(mut current) = state.take_current() {
            'write_one: loop {
                match self.pty.writer().write(current.remaining_bytes()) {
                    Ok(0) => {
                        state.set_current(Some(current));
                        break 'write_many;
                    }
                    Ok(n) => {
                        current.advance(n);
                        if current.finished() {
                            state.goto_next();
                            break 'write_one;
                        }
                    }
                    Err(err) => {
                        state.set_current(Some(current));
                        match err.kind() {
                            ErrorKind::Interrupted | ErrorKind::WouldBlock => break 'write_many,
                            _ => return Err(err),
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Advance the parser over the text the scanner left, in order.
///
/// Graphics, sixel, and iTerm payloads are taken off the stream here. Their
/// handlers arrive with the static-image work; until then a payload must not
/// reach the grid as text. Synchronized-update and cell-size bytes stay in
/// the text runs, which is what the parser has always seen.
fn feed_parser<U: EventListener>(
    parser: &mut ansi::Processor,
    scanner: &mut Scanner,
    terminal: &mut Term<U>,
    bytes: &[u8],
) {
    for segment in scanner.feed(bytes) {
        if let Segment::Text(text) = segment {
            parser.advance(terminal, text);
        }
    }
}
