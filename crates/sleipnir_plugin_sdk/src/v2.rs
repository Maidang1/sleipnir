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

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, Write};

use plugin_protocol::v2::{self, HostMessage, MessageId, PluginMessage, versions_compatible};
use uuid::Uuid;

pub use crate::widgets::{
    Btn, Col, Row, Text, badge, bar, btn, code, code_lang, col, row, sep, spark, text,
};
pub use crate::{Invoke, Output};
pub use plugin_protocol::v2::{
    BlockId, Capability, EventFilter, EventKind, HostCall, HostCallResult, HostEvent,
    PROTOCOL_VERSION, PaneInfo, PaneKey, RenderTarget, RunId, Tone, Widget,
};
pub use plugin_protocol::{CommandSpec, InvokeContext, Lifecycle, Manifest};

use base64::Engine as _;

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
}

/// Handle given to plugin callbacks. Render is a push; [`Self::call`] waits
/// for the matching `Reply` without stalling the read loop.
pub struct Context<'a> {
    io: &'a mut dyn SessionIo,
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

    /// Plugin-initiated host call. Correlated by `id`. Intervening events are
    /// queued, not dropped, and are dispatched when this returns — so a call
    /// cannot deadlock the session against a host that sends an event first.
    pub fn call(&mut self, call: HostCall) -> HostCallResult {
        let id = self.io.next_id();
        if let Err(err) = self.io.write_plugin(&PluginMessage::Call { id, call }) {
            return HostCallResult::Error {
                message: err.to_string(),
            };
        }
        loop {
            if self.io.is_shutdown() {
                return HostCallResult::Error {
                    message: "host shutdown".into(),
                };
            }
            if let Some(result) = self.io.take_unmatched(id) {
                return result;
            }
            match self.io.read_host() {
                Ok(Some(HostMessage::Reply { id: rid, result })) if rid == id => {
                    return result;
                }
                Ok(Some(HostMessage::Reply { id: rid, result })) => {
                    self.io.store_unmatched(rid, result);
                }
                Ok(Some(HostMessage::Shutdown)) => {
                    self.io.set_shutdown();
                    return HostCallResult::Error {
                        message: "host shutdown".into(),
                    };
                }
                Ok(Some(other)) => self.io.queue(other),
                Ok(None) => {
                    return HostCallResult::Error {
                        message: "host closed".into(),
                    };
                }
                Err(err) => {
                    return HostCallResult::Error {
                        message: err.to_string(),
                    };
                }
            }
        }
    }

    /// Send an RGBA pixel buffer to the host for display in a panel.
    ///
    /// `image_id` is a stable identifier chosen by the plugin; reusing the same
    /// id replaces the previous frame. Returns the acknowledged `image_id` on
    /// success.
    ///
    /// The frame is PNG-compressed before transport: a raw 800×600 RGBA buffer
    /// base64-encodes to ~2.5 MB, which blows the host's line cap, while a PNG
    /// of a chart on a flat background is tens of KB. `data_b64` therefore
    /// carries a base64-encoded PNG, not raw RGBA.
    pub fn write_graphics(
        &mut self,
        image_id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
        pane: PaneKey,
    ) -> Result<u32, String> {
        let png = encode_png(width, height, rgba)?;
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let result = self.call(HostCall::WriteGraphics {
            image_id,
            width,
            height,
            data_b64,
            pane,
        });
        match result {
            HostCallResult::GraphicsOk { image_id } => Ok(image_id),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected result: {other:?}")),
        }
    }
}

/// PNG-encode an RGBA buffer. Returns the PNG bytes, or an error if the buffer
/// length does not match `width * height * 4`.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(format!(
            "RGBA length {} does not match {width}x{height}x4 = {expected}",
            rgba.len()
        ));
    }
    let buffer: image::RgbaImage = image::ImageBuffer::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "failed to wrap RGBA buffer".to_string())?;
    let mut png = std::io::Cursor::new(Vec::new());
    buffer
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(png.into_inner())
}

trait SessionIo {
    fn write_plugin(&mut self, msg: &PluginMessage) -> io::Result<()>;
    fn read_host(&mut self) -> io::Result<Option<HostMessage>>;
    fn next_id(&mut self) -> MessageId;
    fn take_unmatched(&mut self, id: MessageId) -> Option<HostCallResult>;
    fn store_unmatched(&mut self, id: MessageId, result: HostCallResult);
    fn queue(&mut self, msg: HostMessage);
    fn set_shutdown(&mut self);
    fn is_shutdown(&self) -> bool;
    fn granted(&self) -> &[Capability];
    fn instance_id(&self) -> Uuid;
}

struct Io<R, W> {
    reader: R,
    writer: W,
    next_id: MessageId,
    unmatched: HashMap<MessageId, HostCallResult>,
    queued: VecDeque<HostMessage>,
    shutdown: bool,
    granted: Vec<Capability>,
    instance_id: Uuid,
}

impl<R: BufRead, W: Write> SessionIo for Io<R, W> {
    fn write_plugin(&mut self, msg: &PluginMessage) -> io::Result<()> {
        write_msg(&mut self.writer, msg)
    }

    fn read_host(&mut self) -> io::Result<Option<HostMessage>> {
        read_host_line(&mut self.reader)
    }

    fn next_id(&mut self) -> MessageId {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn take_unmatched(&mut self, id: MessageId) -> Option<HostCallResult> {
        self.unmatched.remove(&id)
    }

    fn store_unmatched(&mut self, id: MessageId, result: HostCallResult) {
        self.unmatched.insert(id, result);
    }

    fn queue(&mut self, msg: HostMessage) {
        self.queued.push_back(msg);
    }

    fn set_shutdown(&mut self) {
        self.shutdown = true;
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    fn granted(&self) -> &[Capability] {
        &self.granted
    }

    fn instance_id(&self) -> Uuid {
        self.instance_id
    }
}

impl<R: BufRead, W: Write> Io<R, W> {
    fn next_msg(&mut self) -> io::Result<Option<HostMessage>> {
        if let Some(msg) = self.queued.pop_front() {
            return Ok(Some(msg));
        }
        self.read_host()
    }
}

/// Run the plugin: handshake, then serve until `Shutdown` or EOF. Call from
/// `main`.
pub fn run<P: Plugin>(plugin: P) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(err) = serve(plugin, stdin.lock(), stdout.lock()) {
        eprintln!("sleipnir-plugin: {err}");
        std::process::exit(1);
    }
}

/// Testable core of [`run`]. Never panics on host input: malformed lines after
/// handshake are skipped; a bad first line fails the handshake.
pub fn serve<P: Plugin>(mut plugin: P, reader: impl BufRead, writer: impl Write) -> io::Result<()> {
    let mut io = Io {
        reader,
        writer,
        next_id: 1,
        unmatched: HashMap::new(),
        queued: VecDeque::new(),
        shutdown: false,
        granted: Vec::new(),
        instance_id: Uuid::nil(),
    };

    handshake(&mut plugin, &mut io)?;

    while !io.shutdown {
        let Some(msg) = io.next_msg()? else {
            break;
        };
        dispatch(&mut plugin, &mut io, msg)?;
    }
    Ok(())
}

fn handshake<P: Plugin, R: BufRead, W: Write>(plugin: &mut P, io: &mut Io<R, W>) -> io::Result<()> {
    let Some(first) = read_host_line_strict(&mut io.reader)? else {
        return Ok(());
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
    if !versions_compatible(protocol_version, v2::PROTOCOL_VERSION) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "host speaks protocol {protocol_version}, plugin speaks {}",
                v2::PROTOCOL_VERSION
            ),
        ));
    }
    io.granted = granted.clone();
    io.instance_id = plugin_instance_id;
    write_msg(
        &mut io.writer,
        &PluginMessage::Ready {
            protocol_version: v2::PROTOCOL_VERSION,
            manifest: plugin.manifest(),
            requests: plugin.requests(),
            event_filter: plugin.event_filter(),
        },
    )?;
    let mut ctx = Context { io };
    plugin.on_hello(&granted, plugin_instance_id, &mut ctx);
    Ok(())
}

fn dispatch<P: Plugin>(plugin: &mut P, io: &mut dyn SessionIo, msg: HostMessage) -> io::Result<()> {
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
                    output: crate::Output::into_wire(output),
                })?,
                Err(message) => io.write_plugin(&PluginMessage::Failed { id, message })?,
            }
        }
        HostMessage::Reply { id, result } => {
            // Unsolicited: keep it so a racing call can still collect it.
            io.store_unmatched(id, result);
        }
        HostMessage::Shutdown => io.set_shutdown(),
        HostMessage::Hello { .. } => {
            // A second hello is a protocol violation; ignore defensively.
        }
    }
    Ok(())
}

fn write_msg(writer: &mut impl Write, msg: &PluginMessage) -> io::Result<()> {
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
mod tests {
    use super::*;
    use plugin_protocol::v2::{EventKind, HostCall};
    use plugin_protocol::{CommandSpec, InvokeContext, Output as ProtoOutput};
    use std::cell::RefCell;
    use std::rc::Rc;

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
        let mut out = Vec::new();
        serve(plugin, std::io::Cursor::new(input.to_string()), &mut out).unwrap();
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
        let mut out = Vec::new();
        let err = serve(
            plugin,
            std::io::Cursor::new(format!("{}\n", hello(999))),
            &mut out,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
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
}
