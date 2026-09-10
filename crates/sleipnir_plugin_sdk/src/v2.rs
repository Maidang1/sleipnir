//! SDK surface for protocol v2 (ADR-0016).
//!
//! v1 is request/response: the host sends `Invoke`, the plugin answers
//! `Invoked`, the process dies. v2 is a resident, multiplexed session.
//! An author implements [`Plugin`], calls [`run`], and never touches JSON
//! or the process plumbing.
//!
//! Incoming messages arrive interleaved and out of order. The serve loop
//! correlates by `id`. A [`Context::call`] that blocked the reader would
//! deadlock the moment the host sent an event before the matching `Reply`;
//! intervening messages are queued and dispatched when the call returns.

mod session;

use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use plugin_protocol::v2::{self, HostMessage, PluginMessage, host_compatible};
use session::{Io, Receive, Work};
use uuid::Uuid;

pub use crate::widgets::{
    Btn, Col, Row, Text, badge, bar, btn, code, code_lang, col, row, sep, spark, text,
};
use plugin_protocol::v2::Output as WireOutput;
pub use plugin_protocol::v2::{
    BlockId, Capability, CommandSpec, EventFilter, EventKind, HostCall, HostCallResult, HostEvent,
    InvokeContext, Lifecycle, Manifest, MessageId, PROTOCOL_VERSION, PaneInfo, PaneKey,
    RenderTarget, RunId, SceneBar, SceneCamera, SceneData, Tone, Widget,
};

/// One command invocation delivered to the plugin.
pub struct Invoke {
    pub command_id: String,
    pub context: InvokeContext,
}

/// How the plugin wants its result routed. A thin, ergonomic wrapper over the
/// wire [`WireOutput`].
pub enum Output {
    Ignore,
    Insert(String),
    Copy(String),
}

impl Output {
    pub fn insert(text: impl Into<String>) -> Self {
        Self::Insert(text.into())
    }

    pub fn copy(text: impl Into<String>) -> Self {
        Self::Copy(text.into())
    }

    pub(crate) fn into_wire(self) -> WireOutput {
        match self {
            Output::Ignore => WireOutput::Ignore,
            Output::Insert(text) => WireOutput::Insert { text },
            Output::Copy(text) => WireOutput::Copy { text },
        }
    }
}

/// The trait a v2 plugin implements.
///
/// `manifest`, `requests` and `event_filter` are asked once during the
/// handshake. Everything else is an event on a live session.
pub trait Plugin {
    /// Self-description contributed to the host. Must match `plugin.json`'s
    /// `id`; the host rejects a mismatch.
    fn manifest(&self) -> Manifest;

    /// Capabilities the plugin wants. Must be a subset of what `plugin.json`
    /// declared; the host rejects an over-request rather than silently
    /// widening the grant.
    fn requests(&self) -> Vec<Capability>;

    /// Narrows `SubscribeEvents`. Empty (the default) means "no filter".
    fn event_filter(&self) -> EventFilter {
        EventFilter::default()
    }

    /// Called once after a successful handshake, with the capabilities the
    /// host actually granted — a request is not a grant.
    fn on_hello(&mut self, granted: &[Capability], instance_id: Uuid, ctx: &mut Context<'_>) {
        let _ = (granted, instance_id, ctx);
    }

    /// A fact the app already computes (`run_ledger`, `pane_facts`).
    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        let _ = (event, ctx);
    }

    /// The user activated a `Btn` in a tree this plugin rendered.
    fn on_action(
        &mut self,
        block_id: BlockId,
        action: &str,
        arg: Option<&str>,
        ctx: &mut Context<'_>,
    ) {
        let _ = (block_id, action, arg, ctx);
    }

    /// Palette command. Errors become `Failed` on the wire; do not panic.
    fn invoke(&mut self, req: Invoke, ctx: &mut Context<'_>) -> Result<Output, String> {
        let _ = (req, ctx);
        Ok(Output::Ignore)
    }

    /// Opt-in period for [`Self::on_tick`]. `None` (the default) never wakes
    /// the serve loop for ticks. `Some(Duration::ZERO)` is treated as `None`
    /// so the loop cannot busy-spin.
    fn tick_interval(&self) -> Option<Duration> {
        None
    }

    /// Called when [`Self::tick_interval`] elapses with no pending host
    /// message. Queued inbound messages are always dispatched first.
    fn on_tick(&mut self, ctx: &mut Context<'_>) {
        let _ = ctx;
    }
}

/// Handle given to plugin callbacks. Render is a push; [`Self::call`] waits
/// for the matching `Reply` without stalling the read loop.
pub struct Context<'a> {
    io: &'a mut Io,
}

impl Context<'_> {
    /// Capabilities announced in `Hello`. A request is not a grant.
    pub fn granted(&self) -> &[Capability] {
        self.io.granted()
    }

    pub fn instance_id(&self) -> Uuid {
        self.io.instance_id()
    }

    /// Whole-tree replacement (ADR-0017). Safe to call at any time, not only
    /// as a reply to an event.
    pub fn render(&mut self, target: RenderTarget, tree: impl Into<Widget>) -> io::Result<()> {
        let id = self.io.next_id();
        self.io.write_plugin(&PluginMessage::Render {
            id,
            target,
            tree: tree.into(),
        })
    }

    /// Plugin-initiated host call with a 30-second reply deadline. Intervening
    /// events are queued and dispatched when this returns. Queue overflow
    /// ends the session rather than growing memory without a bound.
    pub fn call(&mut self, call: HostCall) -> HostCallResult {
        self.call_with_timeout(call, Duration::from_secs(30))
    }

    pub fn call_with_timeout(&mut self, call: HostCall, timeout: Duration) -> HostCallResult {
        self.io.call(call, timeout)
    }

    /// Send a 3D scene to the host for display in a panel.
    ///
    /// The host owns projection and painting: it draws the geometry as vector
    /// polygons against the panel's real pixel bounds, so the chart stays crisp
    /// at any size and the camera can move host-side without a round-trip per
    /// frame. Returns `Ok(())` when the host accepts the scene.
    pub fn draw_scene(&mut self, pane: PaneKey, scene: SceneData) -> Result<(), String> {
        match self.call(HostCall::DrawScene { pane, scene }) {
            HostCallResult::SceneOk => Ok(()),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected result: {other:?}")),
        }
    }

    /// Scroll a pane back to the output anchor of `run_id` and focus it.
    ///
    /// An inferred run (a busy-probe guess, `HostEvent::RunStarted.inferred`)
    /// has no scrollback anchor; the pane is focused instead. An unknown
    /// `run_id` returns `HostCallResult::Error`.
    pub fn scroll_to_run(&mut self, run_id: RunId) -> HostCallResult {
        self.call(HostCall::ScrollToRun { run_id })
    }

    /// Focus a terminal pane. Plugin panels and unknown keys are errors.
    /// Requires `host_call_focus_pane`; not implied by `write_terminal`.
    /// Activates the tab/pane in the owning window; does not raise a
    /// background OS window.
    pub fn focus_pane(&mut self, pane: PaneKey) -> HostCallResult {
        self.call(HostCall::FocusPane { pane })
    }

    /// Insert `text` into a specific terminal pane through the host's
    /// bracketed-paste-aware path, then optionally press Enter.
    /// Requires `host_call_send_text`; not implied by `write_terminal`.
    /// A pane with no PTY yet, and oversize text, are `Error`.
    pub fn send_text(
        &mut self,
        pane: PaneKey,
        text: impl Into<String>,
        enter: bool,
    ) -> HostCallResult {
        self.call(HostCall::SendText {
            pane,
            text: text.into(),
            enter,
        })
    }

    /// Send one allowlisted logical key (`ctrl-c`, `escape`, …) to a pane.
    /// Requires `host_call_send_key`; not implied by `write_terminal`.
    /// Terminal vi mode is `Error` (the key would be scrollback motion).
    pub fn send_key(&mut self, pane: PaneKey, key: impl Into<String>) -> HostCallResult {
        self.call(HostCall::SendKey {
            pane,
            key: key.into(),
        })
    }

    /// Ask the host to close a terminal pane via the same user-policy path
    /// as the UI close command (a busy pane may require confirmation).
    /// `Ok` means the request was accepted, not that the pane is gone.
    /// Plugin panels and unknown keys are `Error`. Requires
    /// `host_call_request_close_pane`; there is no force-close wrapper.
    pub fn request_close_pane(&mut self, pane: PaneKey) -> HostCallResult {
        self.call(HostCall::RequestClosePane { pane })
    }

    /// Open a new terminal pane with a structured argv. Requires
    /// `host_call_open_pane` (same grant as the string `OpenPane` call).
    /// `program` and `args` are passed directly to spawn; they are never
    /// joined into a shell line.
    pub fn open_pane_argv(
        &mut self,
        cwd: Option<String>,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> HostCallResult {
        self.call(HostCall::OpenPaneArgv {
            cwd,
            program: program.into(),
            args,
        })
    }
}

/// Run the plugin: handshake, then serve until `Shutdown` or EOF. Call from
/// `main`.
pub fn run<P: Plugin>(plugin: P) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(err) = serve(plugin, std::io::BufReader::new(stdin), stdout) {
        eprintln!("sleipnir-plugin: {err}");
        std::process::exit(1);
    }
}

/// Testable core of [`run`]. The owned reader runs on a separate thread so
/// reply deadlines do not depend on the host writing another line. Malformed
/// lines after handshake are skipped; a bad first line fails the handshake.
pub fn serve<P: Plugin>(
    mut plugin: P,
    mut reader: impl BufRead + Send + 'static,
    writer: impl Write + 'static,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(64);
    std::thread::Builder::new()
        .name("plugin-host-reader".into())
        .spawn(move || {
            let first = read_host_line_strict(&mut reader);
            let finished = !matches!(&first, Ok(Some(_)));
            if sender.send(first).is_err() || finished {
                return;
            }
            loop {
                let message = read_host_line(&mut reader);
                let finished = !matches!(&message, Ok(Some(_)));
                if sender.send(message).is_err() || finished {
                    return;
                }
            }
        })?;

    let mut io = Io::new(receiver, writer);
    handshake(&mut plugin, &mut io)?;

    let tick_interval = plugin.tick_interval().filter(|d| !d.is_zero());
    let mut next_tick_at = tick_interval.map(|d| std::time::Instant::now() + d);

    while !io.is_shutdown() {
        match io.next_work(next_tick_at)? {
            Work::Eof => break,
            Work::Message(msg) => dispatch(&mut plugin, &mut io, msg)?,
            Work::Tick => {
                let mut ctx = Context { io: &mut io };
                plugin.on_tick(&mut ctx);
                if let Some(d) = tick_interval {
                    next_tick_at = Some(std::time::Instant::now() + d);
                }
            }
        }
    }
    Ok(())
}

fn handshake<P: Plugin>(plugin: &mut P, io: &mut Io) -> io::Result<()> {
    let first = match io.receive(None)? {
        Receive::Message(message) => message,
        Receive::Eof => return Ok(()),
        Receive::TimedOut => unreachable!("handshake never uses a timeout"),
    };

    let HostMessage::Hello {
        protocol_version,
        granted,
        plugin_instance_id,
    } = first
    else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected hello as first message",
        ));
    };

    if !host_compatible(protocol_version, v2::PROTOCOL_VERSION) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "host speaks protocol {protocol_version}, plugin speaks {}",
                v2::PROTOCOL_VERSION
            ),
        ));
    }

    io.set_session(granted.clone(), plugin_instance_id);
    io.write_plugin(&PluginMessage::Ready {
        protocol_version: v2::PROTOCOL_VERSION,
        manifest: plugin.manifest(),
        requests: plugin.requests(),
        event_filter: plugin.event_filter(),
    })?;

    let mut ctx = Context { io };
    plugin.on_hello(&granted, plugin_instance_id, &mut ctx);
    Ok(())
}

fn dispatch<P: Plugin>(plugin: &mut P, io: &mut Io, msg: HostMessage) -> io::Result<()> {
    match msg {
        HostMessage::Event { event, .. } => {
            let mut ctx = Context { io };
            plugin.on_event(event, &mut ctx);
        }
        HostMessage::Action {
            block_id,
            action,
            arg,
            ..
        } => {
            let mut ctx = Context { io };
            plugin.on_action(block_id, &action, arg.as_deref(), &mut ctx);
        }
        HostMessage::Invoke {
            id,
            command_id,
            context,
        } => {
            let mut ctx = Context { io };
            match plugin.invoke(
                Invoke {
                    command_id,
                    context,
                },
                &mut ctx,
            ) {
                Ok(output) => io.write_plugin(&PluginMessage::Invoked {
                    id,
                    output: output.into_wire(),
                })?,
                Err(message) => io.write_plugin(&PluginMessage::Failed { id, message })?,
            }
        }
        HostMessage::Reply { id, result } => {
            io.store_unmatched(id, result);
        }
        HostMessage::Shutdown => io.set_shutdown(),
        HostMessage::Hello { .. } => {}
    }
    Ok(())
}

fn write_msg(writer: &mut dyn Write, msg: &PluginMessage) -> io::Result<()> {
    let line =
        serde_json::to_string(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// After handshake: skip malformed lines rather than dying. A noisy host
/// must not take down a resident plugin.
fn read_host_line(reader: &mut impl BufRead) -> io::Result<Option<HostMessage>> {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HostMessage>(line.trim()) {
            Ok(msg) => return Ok(Some(msg)),
            Err(err) => {
                eprintln!("sleipnir-plugin: ignoring malformed host message: {err}");
                continue;
            }
        }
    }
}

/// Handshake is strict: we cannot proceed without a real Hello.
fn read_host_line_strict(reader: &mut impl BufRead) -> io::Result<Option<HostMessage>> {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            continue;
        }
        let msg = serde_json::from_str::<HostMessage>(line.trim())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        return Ok(Some(msg));
    }
}

#[cfg(test)]
mod tests;
