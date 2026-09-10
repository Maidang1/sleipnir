//! The resident Run Ledger plugin: owns the ledger, `runs.json`, the status
//! strip, and the panel. The core only emits facts; everything user-facing
//! lives here.
//!
//! Session shape mirrors the Disk 3D example: a thin `Plugin` impl over pure
//! library code (`rows` / `state` / `view`). The panel `PaneKey` is minted
//! once and reused, so the palette command and the strip button re-render one
//! split instead of opening many, and an event only redraws a panel that
//! already exists — an event never conjures a split the user did not ask for.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sleipnir_plugin::{
    BlockId, Capability, CommandSpec, Context, EventFilter, EventKind, HostEvent, Invoke,
    Lifecycle, Manifest, Output, PaneKey, Plugin, RenderTarget, run,
};
use sleipnir_plugin_runledger::rows::rows_from_runs;
use sleipnir_plugin_runledger::state::{LedgerState, default_runs_file};
use sleipnir_plugin_runledger::view::{PanelRow, panel_tree, status_summary, status_tree};

struct RunLedger {
    state: LedgerState,
    /// Monotonic base for every `at_ms` handed to the ledger, mirroring the
    /// core's `RunLedgerGlobal::now_ms`. Durations and badge elapsed math are
    /// only ever compared within this process, so process-start-relative is
    /// exactly right.
    started_at: Instant,
    /// Minted once so repeat renders replace one panel instead of opening many.
    panel: Option<PaneKey>,
}

impl RunLedger {
    fn new() -> Self {
        Self {
            state: LedgerState::load(default_runs_file()),
            started_at: Instant::now(),
            panel: None,
        }
    }

    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    fn draw_status(&mut self, ctx: &mut Context<'_>) {
        let (failed, running) = status_summary(&self.state.ledger);
        let _ = ctx.render(RenderTarget::Status, status_tree(failed, running));
    }

    fn draw_panel(&mut self, ctx: &mut Context<'_>) {
        let pane = *self.panel.get_or_insert_with(PaneKey::new_v4);
        let rows: Vec<PanelRow> = rows_from_runs(&self.state.ledger.snapshot())
            .into_iter()
            .map(|row| PanelRow {
                host_id: self.state.host_id_for(row.id),
                row,
            })
            .collect();
        let tree = panel_tree(&rows, unix_ms(), self.state.ledger.launch_id());
        let _ = ctx.render(RenderTarget::Panel { pane }, tree);
    }

    /// Status always; the panel only when the user already opened it.
    fn redraw(&mut self, ctx: &mut Context<'_>) {
        self.draw_status(ctx);
        if self.panel.is_some() {
            self.draw_panel(ctx);
        }
    }

    fn save(&self) {
        if let Err(err) = self.state.save() {
            eprintln!("runledger: could not persist runs.json: {err}");
        }
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Plugin for RunLedger {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "runledger".into(),
            name: "Run Ledger".into(),
            version: "0.1.0".into(),
            description:
                "What ran here: every command, its outcome, and a jump back to its output.".into(),
            lifecycle: Lifecycle::Resident,
            commands: vec![CommandSpec {
                id: "open".into(),
                title: "Run Ledger: Open panel".into(),
                description:
                    "Open the Run Ledger panel: every run, grouped, with a jump back to its output."
                        .into(),
                keywords: vec![
                    "run".into(),
                    "ledger".into(),
                    "history".into(),
                    "commands".into(),
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
        ]
    }

    /// Continuous observation, narrowed to exactly the facts the ledger is
    /// built from. Ports, foreground agents and cwd drift are not needed and
    /// are not requested.
    fn event_filter(&self) -> EventFilter {
        EventFilter {
            panes: vec![],
            kinds: vec![
                EventKind::RunStarted,
                EventKind::RunFinished,
                EventKind::PaneClosed,
                EventKind::PaneFocused,
            ],
        }
    }

    /// Show the strip immediately: the badge slot and the two palette
    /// contributions should not wait for the user's first command.
    fn on_hello(&mut self, _granted: &[Capability], _id: uuid::Uuid, ctx: &mut Context<'_>) {
        self.draw_status(ctx);
    }

    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        match event {
            HostEvent::RunStarted {
                run_id,
                pane,
                command,
                cwd,
                inferred,
            } => {
                let at_ms = self.now_ms();
                self.state
                    .apply_started(run_id, pane, &command, cwd, inferred, at_ms);
                self.redraw(ctx);
            }
            HostEvent::RunFinished {
                run_id,
                exit_code,
                duration_ms,
                ..
            } => {
                self.state.apply_finished(run_id, exit_code, duration_ms);
                self.save();
                self.redraw(ctx);
            }
            // A pane that is gone will never report RunFinished; this is the
            // terminal signal for its open run.
            HostEvent::PaneClosed { pane } => {
                let at_ms = self.now_ms();
                self.state.apply_pane_closed(pane, at_ms);
                self.save();
                self.redraw(ctx);
            }
            HostEvent::PaneFocused { pane } => {
                self.state.focus(pane);
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
                let Some(host) = arg.and_then(|a| uuid::Uuid::parse_str(a).ok()) else {
                    return;
                };
                // Unknown id (stale tree, post-clear) still marks nothing but
                // the host answers Error; either way the strip is re-derived.
                let _ = ctx.scroll_to_run(host);
                self.state.mark_host_seen(host);
                self.redraw(ctx);
            }
            "clear" => {
                self.state.clear();
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
    run(RunLedger::new());
}
