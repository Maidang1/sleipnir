//! The resident Agents plugin session: a thin `Plugin` impl over the pure
//! `state` / `view` / `adapter` library code. The panel `PaneKey` is minted
//! once and reused, so the palette command and the strip button re-render one
//! split instead of opening many, and an event only redraws a panel that
//! already exists — an event never conjures a split the user did not ask
//! for.
//!
//! The session also owns the coordination stack: a shared
//! `agent_coordination::Registry`, the default-off Unix socket server bound
//! after hello/grants (degraded, never fatal, when the platform or the grant
//! set does not allow it), and a ~50ms tick that delivers at most one
//! adapter effect per wake.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_coordination::{AgentSessionId, Registry, Server, default_socket_path};
use sleipnir_plugin::{
    BlockId, Capability, CommandSpec, Context, EventFilter, EventKind, HostCall, HostCallResult,
    HostEvent, Invoke, Lifecycle, Manifest, Output, PaneKey, Plugin, RenderTarget, run,
};
use sleipnir_plugin_agents::adapter::{Adapter, HostCalls, missing_delivery_grants};
use sleipnir_plugin_agents::state::AgentState;
use sleipnir_plugin_agents::view::{exit_notice, panel_tree, status_tree};

/// Effect delivery cadence: one oldest pending effect per tick.
const TICK: Duration = Duration::from_millis(50);

struct Agents {
    state: AgentState,
    adapter: Adapter,
    /// Kept alive for the plugin lifetime; `None` when the socket could not
    /// be bound (unsupported platform, bind failure, missing grant).
    server: Option<Server>,
    /// Minted once so repeat renders replace one panel instead of opening many.
    panel: Option<PaneKey>,
}

impl Agents {
    fn new() -> Self {
        Self {
            state: AgentState::new(),
            adapter: Adapter::new(Registry::new()),
            server: None,
            panel: None,
        }
    }

    fn draw_status(&mut self, ctx: &mut Context<'_>) {
        let (running, exited_unseen) = self.state.summary();
        let _ = ctx.render(
            RenderTarget::Status,
            status_tree(running, exited_unseen, self.adapter.awaiting_human_count()),
        );
    }

    fn draw_panel(&mut self, ctx: &mut Context<'_>) {
        let pane = *self.panel.get_or_insert_with(PaneKey::new_v4);
        let _ = ctx.render(
            RenderTarget::Panel { pane },
            panel_tree(
                &self.state.rows(),
                &self.adapter.managed_rows(),
                &self.adapter.managed_session_rows(),
            ),
        );
    }

    /// Status always; the panel only when the user already opened it.
    fn redraw(&mut self, ctx: &mut Context<'_>) {
        self.draw_status(ctx);
        if self.panel.is_some() {
            self.draw_panel(ctx);
        }
    }

    /// Bind the coordination socket. Called from `on_hello`, after grants
    /// are known. The socket comes up only with the *full* delivery grant
    /// set — a coordinator that can launch workers but cannot interrupt or
    /// close them is a trap. Any shortfall degrades to observer-only with a
    /// clear stderr message naming the missing grants.
    fn start_coordination(&mut self, granted: &[Capability]) {
        let missing = missing_delivery_grants(granted);
        if !missing.is_empty() {
            eprintln!(
                "agents: coordination socket disabled; missing grants: {}",
                missing
                    .iter()
                    .map(|cap| format!("{cap:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return;
        }
        // Workers paste `sleipnir-agentctl --socket '<path>'` when this is
        // set; the bare command is enough for the default socket path.
        self.adapter.set_socket_override(
            std::env::var("SLEIPNIR_AGENT_CONTROL_SOCKET")
                .ok()
                .filter(|s| !s.is_empty()),
        );
        match Server::bind(self.adapter.registry().clone(), default_socket_path()) {
            Ok(server) => self.server = Some(server),
            Err(err) => eprintln!("agents: coordination socket disabled: {err}"),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Production [`HostCalls`]: maps SDK results to Ok/Err. `Ok`-class results
/// carry the ack data; anything else (denial, unknown pane, rate limit,
/// timeout) is a delivery failure.
struct CtxHost<'a, 'b>(&'a mut Context<'b>);

impl HostCalls for CtxHost<'_, '_> {
    fn open_pane_argv(
        &mut self,
        cwd: Option<String>,
        program: &str,
        args: Vec<String>,
    ) -> Result<PaneKey, String> {
        match self.0.open_pane_argv(cwd, program, args) {
            HostCallResult::Pane { pane } => Ok(pane),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected open_pane_argv result: {other:?}")),
        }
    }

    fn send_text_enter(&mut self, pane: PaneKey, text: &str) -> Result<(), String> {
        match self.0.send_text(pane, text, true) {
            HostCallResult::Ok => Ok(()),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected send_text result: {other:?}")),
        }
    }

    fn send_key(&mut self, pane: PaneKey, key: &str) -> Result<(), String> {
        match self.0.send_key(pane, key) {
            HostCallResult::Ok => Ok(()),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected send_key result: {other:?}")),
        }
    }

    fn focus_pane(&mut self, pane: PaneKey) -> Result<(), String> {
        match self.0.focus_pane(pane) {
            HostCallResult::Ok => Ok(()),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected focus_pane result: {other:?}")),
        }
    }

    fn request_close_pane(&mut self, pane: PaneKey) -> Result<(), String> {
        match self.0.request_close_pane(pane) {
            HostCallResult::Ok => Ok(()),
            HostCallResult::Error { message } => Err(message),
            other => Err(format!("unexpected request_close_pane result: {other:?}")),
        }
    }
}

impl Plugin for Agents {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "agents".into(),
            name: "Agents".into(),
            version: "0.1.0".into(),
            description:
                "Which panes run a known coding agent, conservative per-pane status, and a \
                 local coordination socket for a coordinator agent."
                    .into(),
            lifecycle: Lifecycle::Resident,
            commands: vec![CommandSpec {
                id: "open".into(),
                title: "Agents: Open panel".into(),
                description:
                    "Open the Agents panel: one row per agent pane with a conservative status."
                        .into(),
                keywords: vec![
                    "agent".into(),
                    "agents".into(),
                    "claude".into(),
                    "codex".into(),
                    "gemini".into(),
                    "opencode".into(),
                    "status".into(),
                ],
                capabilities: vec![],
            }],
        }
    }

    fn requests(&self) -> Vec<Capability> {
        vec![
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderPanel,
            Capability::RenderStatus,
            Capability::HostCallScrollToRun,
            Capability::HostCallNotify,
            Capability::HostCallOpenPane,
            Capability::HostCallFocusPane,
            Capability::HostCallSendText,
            Capability::HostCallSendKey,
            Capability::HostCallRequestClosePane,
        ]
    }

    /// Continuous observation, narrowed to exactly the facts the status model
    /// is built from. Ports are not needed and are not requested.
    fn event_filter(&self) -> EventFilter {
        EventFilter {
            panes: vec![],
            kinds: vec![
                EventKind::ForegroundChanged,
                EventKind::RunStarted,
                EventKind::RunFinished,
                EventKind::PaneFocused,
                EventKind::PaneClosed,
                EventKind::CwdChanged,
            ],
        }
    }

    fn tick_interval(&self) -> Option<Duration> {
        Some(TICK)
    }

    /// Show the strip immediately, then bring up the coordination socket
    /// (grants are known here; the server never starts without them).
    fn on_hello(&mut self, granted: &[Capability], _id: uuid::Uuid, ctx: &mut Context<'_>) {
        self.draw_status(ctx);
        self.start_coordination(granted);
    }

    /// One oldest pending adapter effect per tick, plus housekeeping
    /// (launch-detection timeout) whether or not an effect was pending.
    fn on_tick(&mut self, ctx: &mut Context<'_>) {
        let processed = {
            let mut host = CtxHost(ctx);
            self.adapter.process_one(&mut host, now_ms())
        };
        let timed_out = self.adapter.housekeeping(now_ms());
        if processed || timed_out {
            self.redraw(ctx);
        }
    }

    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        match event {
            HostEvent::ForegroundChanged { pane, agent } => {
                self.adapter
                    .foreground_changed(pane, agent.as_deref(), now_ms());
                self.state.foreground_changed(pane, agent);
                self.redraw(ctx);
            }
            HostEvent::RunStarted { run_id, pane, .. } => {
                self.state.run_started(pane, run_id);
                self.redraw(ctx);
            }
            HostEvent::RunFinished { run_id, pane, .. } => {
                // Notify only on a proven, unseen exit of a tracked session —
                // the pure `exit_notice` gate owns every other case. The
                // call is fire-and-forget: a denied or failed notify changes
                // no state and is not retried.
                let outcome = self.state.run_finished(pane, run_id);
                if let Some((title, body)) = exit_notice(&outcome) {
                    let _ = ctx.call(HostCall::Notify { title, body });
                }
                // An Exited outcome means the pinned containing run — the
                // agent process — finished. For a managed pane that closes
                // the coordination session; in-flight prompt tasks become
                // Unknown via the close, never Settled.
                if matches!(
                    outcome,
                    sleipnir_plugin_agents::state::RunFinishOutcome::Exited { .. }
                ) {
                    self.adapter.containing_run_exited(pane, now_ms());
                }
                self.redraw(ctx);
            }
            HostEvent::PaneFocused { pane } => {
                self.state.pane_focused(pane);
                self.redraw(ctx);
            }
            HostEvent::PaneClosed { pane } => {
                self.adapter.pane_closed(pane, now_ms());
                self.state.pane_closed(pane);
                self.redraw(ctx);
            }
            HostEvent::CwdChanged { pane, cwd } => {
                self.state.cwd_changed(pane, cwd);
                self.redraw(ctx);
            }
            _ => {}
        }
    }

    fn on_action(
        &mut self,
        _block_id: BlockId,
        action: &str,
        arg: Option<&str>,
        ctx: &mut Context<'_>,
    ) {
        match action {
            "open_panel" => self.draw_panel(ctx),
            "jump" => {
                let Some(run_id) = arg.and_then(|a| uuid::Uuid::parse_str(a).ok()) else {
                    return;
                };
                // Only a confirmed navigation acknowledges the row: a denied,
                // rate-limited, or unknown-run jump must not mark it seen.
                if matches!(ctx.scroll_to_run(run_id), HostCallResult::Ok) {
                    if let Some(pane) = self.state.pane_for_run(run_id) {
                        self.state.mark_seen(pane);
                    }
                }
                self.redraw(ctx);
            }
            // The human's explicit release of a human-owned managed session,
            // from the panel's Release button. Host-side, never a
            // coordinator-wire op.
            "release" => {
                let Some(session) = arg
                    .and_then(|a| uuid::Uuid::parse_str(a).ok())
                    .map(AgentSessionId::from_uuid)
                else {
                    return;
                };
                self.adapter.release_to_coordinator(session, now_ms());
                self.redraw(ctx);
            }
            // Managed-session row actions. Everything goes through the
            // registry's request path — focus included — so ownership gates
            // and the effect log stay authoritative; delivery happens on the
            // next tick. Take-over additionally queues a focus so the human
            // lands in the pane they just claimed.
            "managed_focus" => {
                let Some(session) = arg
                    .and_then(|a| uuid::Uuid::parse_str(a).ok())
                    .map(AgentSessionId::from_uuid)
                else {
                    return;
                };
                if let Err(err) = self.adapter.request_focus(session, now_ms()) {
                    eprintln!("{err}");
                }
                self.redraw(ctx);
            }
            "managed_takeover" => {
                let Some(session) = arg
                    .and_then(|a| uuid::Uuid::parse_str(a).ok())
                    .map(AgentSessionId::from_uuid)
                else {
                    return;
                };
                if let Err(err) = self.adapter.take_over(session, now_ms()) {
                    eprintln!("{err}");
                } else if let Err(err) = self.adapter.request_focus(session, now_ms()) {
                    eprintln!("{err}");
                }
                self.redraw(ctx);
            }
            "managed_interrupt" => {
                let Some(session) = arg
                    .and_then(|a| uuid::Uuid::parse_str(a).ok())
                    .map(AgentSessionId::from_uuid)
                else {
                    return;
                };
                if let Err(err) = self.adapter.request_interrupt(session, now_ms()) {
                    eprintln!("{err}");
                }
                self.redraw(ctx);
            }
            "managed_close" => {
                let Some(session) = arg
                    .and_then(|a| uuid::Uuid::parse_str(a).ok())
                    .map(AgentSessionId::from_uuid)
                else {
                    return;
                };
                if let Err(err) = self.adapter.request_close(session, now_ms()) {
                    eprintln!("{err}");
                }
                self.redraw(ctx);
            }
            _ => {}
        }
    }

    fn invoke(&mut self, req: Invoke, ctx: &mut Context<'_>) -> Result<Output, String> {
        if req.command_id == "open" {
            self.draw_panel(ctx);
        }
        Ok(Output::Ignore)
    }
}

fn main() {
    run(Agents::new());
}
