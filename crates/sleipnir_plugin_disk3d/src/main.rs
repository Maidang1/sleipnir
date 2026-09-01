//! The resident plugin: owns the session, the camera and the current scan.
//!
//! Surface choice is deliberate. A Panel occupies a split, so it has the room a
//! chart needs and it survives focus changes — unlike a Block, which is pinned
//! to one finished run in scrollback. The panel opens on the palette command and
//! then follows the working directory: `cd` somewhere and the chart is for
//! *that* directory.
//!
//! Two protocol facts shape the code:
//!
//! - The panel's real pixel size is never sent to the plugin, so the surface
//!   size is assumed and the tree is clamped to the node budget in `view`
//!   rather than relying on host truncation.
//! - `RenderTarget::Panel` needs a `PaneKey` the host does not yet own; the host
//!   creates the split on first render for an unknown key. The key is minted
//!   once and reused, so every later render replaces that same panel in place.

use std::path::PathBuf;

use sleipnir_plugin::v2::{
    BlockId, Capability, Context, EventFilter, EventKind, HostEvent, Invoke, Lifecycle, Manifest,
    Output, PaneKey, Plugin, RenderTarget, run,
};
use sleipnir_plugin_disk3d::{
    PITCH_STEP, SPIN_STEP, View, YAW_STEP, ZOOM_STEP, render, scan::scan,
};

/// Assumed panel size. The host does not report the split's cell size, so the
/// tree is built for a typical half-window split; `view::render` auto-fits the
/// projection and clamps the node count, so being wrong costs framing, never
/// correctness.
const ASSUMED_COLS: u16 = 78;
const ASSUMED_ROWS: u16 = 26;

/// Frames drawn by one "Spin ½ turn" press.
///
/// Animation runs inline on the serve thread: the SDK writes to a locked stdout
/// held for the whole session, so a second thread cannot render, and a
/// long-running spin would stop the plugin answering events. A bounded sweep
/// keeps the plugin responsive and still shows the geometry turning.
const SPIN_FRAMES: usize = 22;

struct Disk3d {
    view: View,
    /// Minted once so repeat renders replace one panel instead of opening many.
    panel: Option<PaneKey>,
    /// Latest cwd from the event stream; the chart follows it.
    cwd: Option<PathBuf>,
}

impl Disk3d {
    fn new() -> Self {
        Self {
            view: View::new(scan(&std::env::current_dir().unwrap_or_default())),
            panel: None,
            cwd: None,
        }
    }

    fn target(&mut self) -> RenderTarget {
        let pane = *self.panel.get_or_insert_with(uuid::Uuid::new_v4);
        RenderTarget::Panel { pane }
    }

    fn draw(&mut self, ctx: &mut Context<'_>) {
        let target = self.target();
        let tree = render(&self.view, ASSUMED_COLS, ASSUMED_ROWS);
        let _ = ctx.render(target, tree);
    }

    /// Rescan the directory the chart is currently about.
    fn rescan(&mut self) {
        let root = self
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        self.view.replace_scan(scan(&root));
    }
}

impl Plugin for Disk3d {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "disk3d".into(),
            name: "Disk 3D".into(),
            version: "0.1.0".into(),
            description: "Shows where disk space went as a rotatable 3D chart.".into(),
            lifecycle: Lifecycle::Resident,
            commands: vec![sleipnir_plugin::v2::CommandSpec {
                id: "open".into(),
                title: "Disk 3D: Chart This Directory".into(),
                description: "Open a 3D disk-usage chart for the working directory.".into(),
                keywords: vec![
                    "disk".into(),
                    "usage".into(),
                    "3d".into(),
                    "size".into(),
                    "du".into(),
                ],
                capabilities: vec![],
            }],
        }
    }

    fn requests(&self) -> Vec<Capability> {
        vec![
            Capability::Resident,
            Capability::RenderPanel,
            Capability::SubscribeEvents,
            Capability::ReadCwd,
        ]
    }

    /// `SubscribeEvents` is continuous observation, so the filter is as narrow
    /// as the feature allows: directory changes only. Run contents, ports and
    /// focus are not needed and are not requested.
    fn event_filter(&self) -> EventFilter {
        EventFilter {
            panes: vec![],
            kinds: vec![EventKind::CwdChanged],
        }
    }

    fn invoke(&mut self, req: Invoke, ctx: &mut Context<'_>) -> Result<Output, String> {
        if req.command_id != "open" {
            return Ok(Output::Ignore);
        }
        // The palette supplies cwd for the active pane; prefer it over the
        // plugin process's own directory.
        if let Some(cwd) = req.context.cwd.as_deref() {
            self.cwd = Some(PathBuf::from(cwd));
        }
        self.rescan();
        self.draw(ctx);
        Ok(Output::Ignore)
    }

    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        let HostEvent::CwdChanged { cwd, .. } = event else {
            return;
        };
        let next = PathBuf::from(&cwd);
        if self.cwd.as_ref() == Some(&next) {
            return;
        }
        self.cwd = Some(next);
        // Only redraw a panel that already exists: an event must not conjure a
        // split the user never asked for.
        if self.panel.is_some() {
            self.rescan();
            self.draw(ctx);
        }
    }

    fn on_action(
        &mut self,
        _block_id: BlockId,
        action: &str,
        _arg: Option<&str>,
        ctx: &mut Context<'_>,
    ) {
        match action {
            "yaw-" => self.view.yaw_by(-YAW_STEP),
            "yaw+" => self.view.yaw_by(YAW_STEP),
            "pitch+" => self.view.pitch_by(PITCH_STEP),
            "pitch-" => self.view.pitch_by(-PITCH_STEP),
            "zoom+" => self.view.zoom_by(ZOOM_STEP),
            "zoom-" => self.view.zoom_by(1.0 / ZOOM_STEP),
            "next" => self.view.select_next(),
            "rescan" => self.rescan(),
            "spin" => {
                for _ in 0..SPIN_FRAMES {
                    self.view.yaw_by(SPIN_STEP);
                    self.draw(ctx);
                    std::thread::sleep(std::time::Duration::from_millis(
                        sleipnir_plugin_disk3d::SPIN_INTERVAL_MS,
                    ));
                }
                return;
            }
            _ => return,
        }
        self.draw(ctx);
    }
}

fn main() {
    run(Disk3d::new());
}
