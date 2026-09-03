//! Example resident plugin: draw a Block when a run fails, and re-draw it
//! when the user presses Retry (ADR-0016 / ADR-0017 / ADR-0018).
//!
//! SubscribeEvents is continuous observation, so the filter is narrow:
//! run lifecycle only. `RunFinished` does not carry the redacted command —
//! that arrives on `RunStarted` — which is why both kinds are requested.

use std::collections::HashMap;

use sleipnir_plugin::v2::{
    Capability, Context, EventFilter, EventKind, HostEvent, Lifecycle, Manifest, Plugin,
    RenderTarget, RunId, run,
};
use sleipnir_plugin_failed_run::{failure_tree, retried_tree};

struct RunInfo {
    command: String,
    exit_code: i32,
    duration_ms: u64,
}

struct FailedRun {
    /// Command text from `RunStarted`. The ledger redacts at capture;
    /// plugins never see the raw line.
    started: HashMap<RunId, String>,
    rendered: HashMap<RunId, RunInfo>,
}

impl FailedRun {
    fn new() -> Self {
        Self {
            started: HashMap::new(),
            rendered: HashMap::new(),
        }
    }
}

impl Plugin for FailedRun {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "failed-run".into(),
            name: "Failed Run".into(),
            version: "0.1.0".into(),
            description: "Draws a Block when a command fails, with a Retry button.".into(),
            lifecycle: Lifecycle::Resident,
            commands: vec![],
        }
    }

    fn requests(&self) -> Vec<Capability> {
        // Must be a subset of plugin.json. Resident is implied by lifecycle
        // on the host side, but asking for it keeps Ready honest.
        vec![
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderBlock,
        ]
    }

    fn event_filter(&self) -> EventFilter {
        EventFilter {
            panes: vec![],
            kinds: vec![EventKind::RunStarted, EventKind::RunFinished],
        }
    }

    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        match event {
            HostEvent::RunStarted {
                run_id, command, ..
            } => {
                self.started.insert(run_id, command);
            }
            HostEvent::RunFinished {
                run_id,
                exit_code,
                duration_ms,
                ..
            } => {
                // Drop the RunStarted entry on every finish — success or
                // failure — so `started` cannot leak completed runs.
                let command = self
                    .started
                    .remove(&run_id)
                    .unwrap_or_else(|| "<unknown>".into());
                let Some(code) = exit_code.filter(|c| *c != 0) else {
                    return;
                };
                let tree = failure_tree(&command, code, duration_ms, &run_id.to_string());
                self.rendered.insert(
                    run_id,
                    RunInfo {
                        command,
                        exit_code: code,
                        duration_ms,
                    },
                );
                let _ = ctx.render(RenderTarget::Block { anchor: run_id }, tree);
            }
            _ => {}
        }
    }

    fn on_action(
        &mut self,
        _block_id: sleipnir_plugin::v2::BlockId,
        action: &str,
        arg: Option<&str>,
        ctx: &mut Context<'_>,
    ) {
        if action != "retry" {
            return;
        }
        let Some(arg) = arg else {
            return;
        };
        let Ok(run_id) = arg.parse::<RunId>() else {
            return;
        };
        let Some(info) = self.rendered.get(&run_id) else {
            return;
        };
        let tree = retried_tree(&info.command, info.exit_code, info.duration_ms);
        let _ = ctx.render(RenderTarget::Block { anchor: run_id }, tree);
    }
}

fn main() {
    run(FailedRun::new());
}
