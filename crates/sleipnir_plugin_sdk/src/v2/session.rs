use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::sync::mpsc;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plugin_protocol::v2::{
    Capability, HostCall, HostCallResult, HostMessage, MessageId, PluginMessage,
};
use uuid::Uuid;

use super::write_msg;

const MAX_UNMATCHED_REPLIES: usize = 64;
const MAX_QUEUED_MESSAGES: usize = 64;

pub(super) enum Receive {
    Message(HostMessage),
    TimedOut,
    Eof,
}

pub(super) enum Work {
    Message(HostMessage),
    Tick,
    Eof,
}

pub(super) struct Io {
    reader: mpsc::Receiver<io::Result<Option<HostMessage>>>,
    writer: Box<dyn Write>,
    next_id: MessageId,
    unmatched: HashMap<MessageId, HostCallResult>,
    queued: VecDeque<HostMessage>,
    shutdown: bool,
    granted: Vec<Capability>,
    instance_id: Uuid,
}

impl Io {
    pub(super) fn new(
        reader: mpsc::Receiver<io::Result<Option<HostMessage>>>,
        writer: impl Write + 'static,
    ) -> Self {
        Self {
            reader,
            writer: Box::new(writer),
            next_id: 1,
            unmatched: HashMap::new(),
            queued: VecDeque::new(),
            shutdown: false,
            granted: Vec::new(),
            instance_id: Uuid::nil(),
        }
    }

    pub(super) fn set_session(&mut self, granted: Vec<Capability>, instance_id: Uuid) {
        self.granted = granted;
        self.instance_id = instance_id;
    }

    pub(super) fn granted(&self) -> &[Capability] {
        &self.granted
    }

    pub(super) fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    pub(super) fn next_id(&mut self) -> MessageId {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub(super) fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    pub(super) fn set_shutdown(&mut self) {
        self.shutdown = true;
    }

    pub(super) fn write_plugin(&mut self, msg: &PluginMessage) -> io::Result<()> {
        write_msg(self.writer.as_mut(), msg)
    }

    pub(super) fn call(&mut self, call: HostCall, timeout: Duration) -> HostCallResult {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let id = self.next_id();

        if let Err(err) = self.write_plugin(&PluginMessage::Call { id, call }) {
            return HostCallResult::Error {
                message: err.to_string(),
            };
        }

        loop {
            if self.shutdown {
                return HostCallResult::Error {
                    message: "host shutdown".into(),
                };
            }

            if let Some(result) = self.take_unmatched(id) {
                return result;
            }

            match self.receive_matching_reply(id, Some(deadline)) {
                Ok(Some(result)) => return result,
                Ok(None) => continue,
                Err(err) => {
                    return HostCallResult::Error {
                        message: err.to_string(),
                    };
                }
            }
        }
    }

    pub(super) fn next_work(&mut self, next_tick_at: Option<Instant>) -> io::Result<Work> {
        if let Some(msg) = self.queued.pop_front() {
            return Ok(Work::Message(msg));
        }

        match self.reader.try_recv() {
            Ok(Ok(Some(msg))) => return Ok(Work::Message(msg)),
            Ok(Ok(None)) => return Ok(Work::Eof),
            Ok(Err(err)) => return Err(err),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(Work::Eof),
            Err(mpsc::TryRecvError::Empty) => {}
        }

        let Some(deadline) = next_tick_at else {
            return match self.receive(None)? {
                Receive::Message(msg) => Ok(Work::Message(msg)),
                Receive::TimedOut => unreachable!("receive(None) cannot time out"),
                Receive::Eof => Ok(Work::Eof),
            };
        };

        if deadline.saturating_duration_since(Instant::now()).is_zero() {
            return Ok(Work::Tick);
        }

        match self.receive(Some(deadline))? {
            Receive::Message(msg) => Ok(Work::Message(msg)),
            Receive::TimedOut => Ok(Work::Tick),
            Receive::Eof => Ok(Work::Eof),
        }
    }

    pub(super) fn receive(&mut self, deadline: Option<Instant>) -> io::Result<Receive> {
        let received = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Ok(Receive::TimedOut);
                }
                self.reader.recv_timeout(remaining)
            }
            None => self
                .reader
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };

        match received {
            Ok(Ok(Some(message))) => Ok(Receive::Message(message)),
            Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Receive::Eof),
            Ok(Err(err)) => Err(err),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(Receive::TimedOut),
        }
    }

    pub(super) fn store_unmatched(&mut self, id: MessageId, result: HostCallResult) {
        if id >= self.next_id && self.unmatched.len() < MAX_UNMATCHED_REPLIES {
            self.unmatched.insert(id, result);
        }
    }

    pub(super) fn take_unmatched(&mut self, id: MessageId) -> Option<HostCallResult> {
        self.unmatched.remove(&id)
    }

    #[cfg(test)]
    pub(super) fn queued_len(&self) -> usize {
        self.queued.len()
    }

    fn receive_matching_reply(
        &mut self,
        waiting_for: MessageId,
        deadline: Option<Instant>,
    ) -> io::Result<Option<HostCallResult>> {
        match self.receive(deadline)? {
            Receive::Message(HostMessage::Reply { id, result }) if id == waiting_for => {
                Ok(Some(result))
            }
            Receive::Message(HostMessage::Reply { id, result }) => {
                self.store_unmatched(id, result);
                Ok(None)
            }
            Receive::Message(HostMessage::Shutdown) => {
                self.shutdown = true;
                Ok(None)
            }
            Receive::Message(message) => {
                self.queue(message);
                Ok(None)
            }
            Receive::TimedOut => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "host call timed out",
            )),
            Receive::Eof => Err(io::Error::new(io::ErrorKind::BrokenPipe, "host closed")),
        }
    }

    fn queue(&mut self, msg: HostMessage) {
        if self.queued.len() >= MAX_QUEUED_MESSAGES {
            self.shutdown = true;
        } else {
            self.queued.push_back(msg);
        }
    }
}

#[cfg(test)]
pub(super) struct SharedWriter(pub(super) Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl SharedWriter {
    pub(super) fn new() -> (Self, Arc<Mutex<Vec<u8>>>) {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        (Self(Arc::clone(&bytes)), bytes)
    }
}

#[cfg(test)]
impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
