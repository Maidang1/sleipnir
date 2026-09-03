//! Protocol v2 draft (ADR-0016): bidirectional, multiplexed plugin RPC.
//!
//! v1 (the parent module) is request/response only: the host sends `Invoke` and
//! the plugin answers `Invoked`. v2 keeps that exchange and adds three things
//! that ADR-0016 requires:
//!
//! 1. **Correlation ids.** A resident plugin can have several events, actions
//!    and host calls in flight at once. Every message carries an `id` so a reply
//!    can be paired with its cause. This is the least visible and most easily
//!    omitted part of the v1 → v2 move.
//! 2. **Host → plugin pushes** (`Event`, `Action`) so a plugin can observe the
//!    facts layer and receive widget interactions.
//! 3. **Plugin → host pushes** (`Render`, `Call`) so a plugin can draw and can
//!    initiate host actions.
//!
//! ## Session shape (v2)
//!
//! ```text
//! host   ── Hello ──▶ plugin
//! plugin ── Ready ──▶ host
//!
//!   … any interleaving, each correlated by `id` …
//!
//! host   ── Invoke {id} ──▶ plugin      plugin ── Invoked {id} ──▶ host
//! host   ── Event  {id} ──▶ plugin      plugin ── Render  {id} ──▶ host
//! host   ── Action {id} ──▶ plugin      plugin ── Call    {id} ──▶ host
//! host   ── Reply  {id} ──▶ plugin        (answering that Call)
//!
//! host   ── Shutdown ──▶ plugin
//! ```
//!
//! Pure types, as in v1: no I/O, no process handling. Transport is unchanged —
//! line-delimited JSON, tagged unions, `snake_case`, `#[serde(default)]` on
//! additions so later fields stay backward compatible.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bumped from v1's `1`.
pub const PROTOCOL_VERSION: u32 = 2;

/// ADR-0016 §8: an external ecosystem cannot survive `host == plugin`, which
/// breaks every plugin on every host release. The host accepts the current and
/// previous versions.
pub const MIN_SUPPORTED_VERSION: u32 = PROTOCOL_VERSION - 1;

/// Correlates a request with its reply. Unique within a session per direction.
pub type MessageId = u64;

/// Identifies one plugin-rendered surface, assigned by the host.
pub type BlockId = Uuid;
pub type RunId = Uuid;
pub type PaneKey = Uuid;

/// Scrollback position of a Run or Block. Process-local — never persisted:
/// a restored anchor would claim a scrollback line that no longer means
/// anything (ADR-0018 lifecycle).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Anchor {
    /// Absolute line (`cursor.line + history_size` when the Run was recorded).
    pub line: i32,
    pub column: usize,
}

/// True when the host can speak to a plugin claiming `plugin`.
pub fn versions_compatible(host: u32, plugin: u32) -> bool {
    plugin <= host && plugin >= host.saturating_sub(1)
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

/// v1's seven permissions plus the v2 additions.
///
/// The v1 set is all "read one snapshot, when the user asked for it". The v2
/// additions are categorically stronger and are **never implied** by the v1 set
/// (ADR-0016 §4). `SubscribeEvents` in particular moves a plugin from "runs when
/// you pick it" to "watches every command you run".
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    // v1 — unchanged.
    ReadSelection,
    ReadVisibleScreen,
    ReadCwd,
    ReadTitle,
    WriteTerminal,
    Clipboard,
    Network,

    // v2 — each a tier above the above.
    /// The process keeps running between invocations.
    Resident,
    /// Continuous observation. Narrowable with `EventFilter`.
    SubscribeEvents,
    RenderBlock,
    RenderPanel,
    RenderStatus,
    HostCallNotify,
    HostCallReadScreen,
    HostCallListPanes,
    HostCallOpenPane,
    HostCallDrawScene,
}

impl Capability {
    /// Whether this capability was available in v1. Used by the host to decide
    /// which consent path applies during the compatibility window.
    pub fn is_v1(self) -> bool {
        matches!(
            self,
            Self::ReadSelection
                | Self::ReadVisibleScreen
                | Self::ReadCwd
                | Self::ReadTitle
                | Self::WriteTerminal
                | Self::Clipboard
                | Self::Network
        )
    }
}

/// The v1 ↔ v2 correspondence lives here and only here; the host and the
/// session both derive their mappings from it (the enums themselves stay
/// distinct per ADR-0015).
impl From<super::Capability> for Capability {
    fn from(cap: super::Capability) -> Self {
        match cap {
            super::Capability::ReadSelection => Self::ReadSelection,
            super::Capability::ReadVisibleScreen => Self::ReadVisibleScreen,
            super::Capability::ReadCwd => Self::ReadCwd,
            super::Capability::ReadTitle => Self::ReadTitle,
            super::Capability::WriteTerminal => Self::WriteTerminal,
            super::Capability::Clipboard => Self::Clipboard,
            super::Capability::Network => Self::Network,
        }
    }
}

/// Narrows an event subscription. An empty field means "no filter".
///
/// ADR-0016 §4 requires `subscribe_events` to be narrowable rather than
/// all-or-nothing, so a plugin that only cares about one pane cannot silently
/// observe the whole session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panes: Vec<PaneKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<EventKind>,
}

/// Discriminant of `HostEvent`, so a filter can name kinds without payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RunStarted,
    RunFinished,
    PortOpened,
    ForegroundChanged,
    CwdChanged,
    PaneFocused,
}

// ---------------------------------------------------------------------------
// Events: host → plugin
// ---------------------------------------------------------------------------

/// Facts the app already computes (`run_ledger`, `pane_facts`, `chrome/agent`).
/// v2 opens an outlet; it adds no instrumentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HostEvent {
    /// `command` is the **redacted** form. The ledger redacts at capture time
    /// (`run_ledger::redact`); plugins never see the raw command line.
    RunStarted {
        run_id: RunId,
        pane: PaneKey,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    RunFinished {
        run_id: RunId,
        pane: PaneKey,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        duration_ms: u64,
    },
    PortOpened {
        pane: PaneKey,
        pid: u32,
        addr: String,
    },
    ForegroundChanged {
        pane: PaneKey,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
    },
    CwdChanged {
        pane: PaneKey,
        cwd: String,
    },
    PaneFocused {
        pane: PaneKey,
    },
}

impl HostEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::RunStarted { .. } => EventKind::RunStarted,
            Self::RunFinished { .. } => EventKind::RunFinished,
            Self::PortOpened { .. } => EventKind::PortOpened,
            Self::ForegroundChanged { .. } => EventKind::ForegroundChanged,
            Self::CwdChanged { .. } => EventKind::CwdChanged,
            Self::PaneFocused { .. } => EventKind::PaneFocused,
        }
    }

    pub fn pane(&self) -> PaneKey {
        match self {
            Self::RunStarted { pane, .. }
            | Self::RunFinished { pane, .. }
            | Self::PortOpened { pane, .. }
            | Self::ForegroundChanged { pane, .. }
            | Self::CwdChanged { pane, .. }
            | Self::PaneFocused { pane } => *pane,
        }
    }

    /// Whether this event passes `filter`.
    pub fn matches(&self, filter: &EventFilter) -> bool {
        let pane_ok = filter.panes.is_empty() || filter.panes.contains(&self.pane());
        let kind_ok = filter.kinds.is_empty() || filter.kinds.contains(&self.kind());
        pane_ok && kind_ok
    }
}

// ---------------------------------------------------------------------------
// Host calls: plugin → host
// ---------------------------------------------------------------------------

/// Each maps to something already reachable through the control surface
/// (ADR-0011), so v2 adds no power the machine did not already expose locally —
/// it changes *who* may ask, which is why each is separately granted.
///
/// Not `Eq`: `DrawScene` carries floating-point geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum HostCall {
    Notify {
        title: String,
        body: String,
    },
    ReadScreen {
        pane: PaneKey,
    },
    ListPanes,
    OpenPane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
    /// A 3D scene the host projects and paints with vector polygons.
    ///
    /// Replaces the earlier `WriteGraphics` PNG path: the plugin sends compact
    /// geometry, and the host owns projection and painting. That keeps the chart
    /// crisp at any panel size (no bitmap scaling) and lets the host drive the
    /// camera locally without a round-trip per frame (ADR-0004/ADR-0017 bar
    /// images; this is geometry, projected host-side).
    DrawScene {
        pane: PaneKey,
        scene: SceneData,
    },
}

/// A 3D bar-chart scene in normalised, host-agnostic form.
///
/// Geometry only: no pixels, no projection. The host projects with the
/// [`SceneCamera`] against the panel's real bounds every frame, so resizing and
/// camera moves never need another message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneData {
    /// Grid columns the bars are laid out on.
    #[serde(default)]
    pub cols: u32,
    /// Grid rows the bars are laid out on.
    #[serde(default)]
    pub rows: u32,
    /// Floor plane colour, RGB.
    #[serde(default)]
    pub floor: [u8; 3],
    #[serde(default)]
    pub camera: SceneCamera,
    #[serde(default)]
    pub bars: Vec<SceneBar>,
}

/// One bar: a grid cell, a normalised height, and a colour.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SceneBar {
    /// Grid X index, `0..cols`.
    #[serde(default)]
    pub gx: u32,
    /// Grid Z index, `0..rows`.
    #[serde(default)]
    pub gz: u32,
    /// Height as a share of the tallest bar, `0.0..=1.0`.
    #[serde(default)]
    pub height: f32,
    /// Bar colour, RGB.
    #[serde(default)]
    pub color: [u8; 3],
    #[serde(default)]
    pub selected: bool,
}

/// Orthographic camera: yaw about Y, pitch about X, plus a zoom multiplier.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneCamera {
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub pitch: f32,
    #[serde(default = "default_zoom")]
    pub zoom: f32,
}

fn default_zoom() -> f32 {
    1.0
}

impl Default for SceneCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        }
    }
}

impl HostCall {
    /// The capability this call requires.
    pub fn required_capability(&self) -> Capability {
        match self {
            Self::Notify { .. } => Capability::HostCallNotify,
            Self::ReadScreen { .. } => Capability::HostCallReadScreen,
            Self::ListPanes => Capability::HostCallListPanes,
            Self::OpenPane { .. } => Capability::HostCallOpenPane,
            Self::DrawScene { .. } => Capability::HostCallDrawScene,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane: PaneKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum HostCallResult {
    Ok,
    Screen {
        text: String,
    },
    Panes {
        panes: Vec<PaneInfo>,
    },
    Pane {
        pane: PaneKey,
    },
    SceneOk,
    /// Includes the denial case: a call whose capability was not granted.
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Where a rendered tree goes. Per ADR-0017, Panel is implemented first and
/// Block last; the wire shape is fixed now so the ordering is an implementation
/// detail rather than a protocol change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum RenderTarget {
    /// Anchored to a Run, inside scrollback (ADR-0018).
    Block {
        anchor: RunId,
    },
    /// Occupies a split (ADR-0017's first mount point).
    Panel {
        pane: PaneKey,
    },
    Status,
}

impl RenderTarget {
    pub fn required_capability(&self) -> Capability {
        match self {
            Self::Block { .. } => Capability::RenderBlock,
            Self::Panel { .. } => Capability::RenderPanel,
            Self::Status => Capability::RenderStatus,
        }
    }
}

/// Semantic colour tokens (ADR-0017 constraint 1). Deliberately no hex/RGB:
/// raw colours would let plugins ignore the user's theme, and every such plugin
/// would render broken on a theme switch (ADR-0002 follows system appearance).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Fg,
    Dim,
    Accent,
    Ok,
    Warn,
    Err,
}

/// The closed v1 widget set (ADR-0017).
///
/// An unknown `t` deserializes to `Unknown` rather than failing the tree, so a
/// plugin built against a newer host degrades to a placeholder instead of
/// disappearing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Widget {
    Col {
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        children: Vec<Widget>,
    },
    Row {
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        children: Vec<Widget>,
    },
    Text {
        s: String,
        #[serde(default)]
        fg: Tone,
        #[serde(default)]
        bold: bool,
    },
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        s: String,
    },
    Badge {
        s: String,
        #[serde(default)]
        tone: Tone,
    },
    /// Progress in `0.0..=1.0`.
    Bar {
        v: f32,
    },
    Spark {
        vs: Vec<f32>,
    },
    Sep,
    /// The only interactive node (ADR-0017 constraint 3). No inputs, no drag,
    /// no focus management — text entry is what the command palette is for.
    Btn {
        s: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

/// ADR-0017 constraint 5. External authors will produce pathological trees,
/// and layout cost is paid on the UI thread and re-paid on every reflow.
pub const MAX_WIDGET_NODES: usize = 500;
pub const MAX_WIDGET_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeStats {
    pub nodes: usize,
    pub depth: usize,
}

impl TreeStats {
    pub fn within_budget(self) -> bool {
        self.nodes <= MAX_WIDGET_NODES && self.depth <= MAX_WIDGET_DEPTH
    }
}

/// Measure a tree so the host can reject or truncate it before layout.
pub fn measure(widget: &Widget) -> TreeStats {
    fn walk(w: &Widget, depth: usize) -> TreeStats {
        let children = match w {
            Widget::Col { children, .. } | Widget::Row { children, .. } => children.as_slice(),
            _ => &[],
        };
        let mut stats = TreeStats { nodes: 1, depth };
        for child in children {
            let sub = walk(child, depth + 1);
            stats.nodes += sub.nodes;
            stats.depth = stats.depth.max(sub.depth);
        }
        stats
    }
    walk(widget, 1)
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum HostMessage {
    Hello {
        protocol_version: u32,
        granted: Vec<Capability>,
        /// Identifies this run of this plugin, for the Monitor panel and logs.
        plugin_instance_id: Uuid,
    },
    Invoke {
        id: MessageId,
        command_id: String,
        context: InvokeContext,
    },
    Event {
        id: MessageId,
        event: HostEvent,
    },
    /// A user activated a `Btn` in a tree this plugin rendered.
    Action {
        id: MessageId,
        block_id: BlockId,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arg: Option<String>,
    },
    /// Answers a `PluginMessage::Call`; `id` echoes that call's id.
    Reply {
        id: MessageId,
        result: HostCallResult,
    },
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum PluginMessage {
    Ready {
        protocol_version: u32,
        manifest: Manifest,
        requests: Vec<Capability>,
        /// Only meaningful with `SubscribeEvents`.
        #[serde(default)]
        event_filter: EventFilter,
    },
    Invoked {
        id: MessageId,
        output: Output,
    },
    Failed {
        id: MessageId,
        message: String,
    },
    /// Whole-tree replacement (ADR-0017). No patch protocol in v1: bounded trees
    /// make replacement affordable, and a patch format would be a second
    /// permanent contract with its own consistency failure modes.
    Render {
        id: MessageId,
        target: RenderTarget,
        tree: Widget,
    },
    Call {
        id: MessageId,
        call: HostCall,
    },
}

// ---------------------------------------------------------------------------
// Carried over from v1, unchanged in shape
// ---------------------------------------------------------------------------

pub use super::{CommandSpec, InvokeContext, Lifecycle, Manifest, Output};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_window_accepts_current_and_previous() {
        // ADR-0016 §8: `host == plugin` would break every plugin on every host
        // release once there is an external ecosystem.
        assert!(versions_compatible(2, 2));
        assert!(versions_compatible(2, 1));
        assert!(!versions_compatible(2, 3), "never accept a future plugin");
        assert!(!versions_compatible(3, 1), "N-2 is outside the window");
    }

    #[test]
    fn v2_capabilities_are_not_implied_by_v1() {
        assert!(Capability::ReadSelection.is_v1());
        assert!(!Capability::SubscribeEvents.is_v1());
        assert!(!Capability::Resident.is_v1());
        assert!(!Capability::RenderBlock.is_v1());
    }

    #[test]
    fn correlation_id_round_trips() {
        let msg = HostMessage::Event {
            id: 42,
            event: HostEvent::RunFinished {
                run_id: Uuid::nil(),
                pane: Uuid::nil(),
                exit_code: Some(1),
                duration_ms: 1500,
            },
        };
        let line = serde_json::to_string(&msg).unwrap();
        assert!(line.contains(r#""msg":"event""#));
        assert!(line.contains(r#""event":"run_finished""#));
        assert_eq!(serde_json::from_str::<HostMessage>(&line).unwrap(), msg);
    }

    #[test]
    fn event_filter_narrows_by_pane_and_kind() {
        let pane = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let event = HostEvent::PaneFocused { pane };

        assert!(
            event.matches(&EventFilter::default()),
            "empty filter passes all"
        );
        assert!(event.matches(&EventFilter {
            panes: vec![pane],
            kinds: vec![EventKind::PaneFocused],
        }));
        assert!(!event.matches(&EventFilter {
            panes: vec![other],
            kinds: vec![],
        }));
        assert!(!event.matches(&EventFilter {
            panes: vec![],
            kinds: vec![EventKind::RunStarted],
        }));
    }

    #[test]
    fn host_calls_and_render_targets_declare_their_capability() {
        assert_eq!(
            HostCall::ListPanes.required_capability(),
            Capability::HostCallListPanes
        );
        assert_eq!(
            RenderTarget::Block {
                anchor: Uuid::nil()
            }
            .required_capability(),
            Capability::RenderBlock
        );
        assert_eq!(
            RenderTarget::Status.required_capability(),
            Capability::RenderStatus
        );
    }

    #[test]
    fn unknown_widget_degrades_instead_of_failing_the_tree() {
        // A plugin built against a newer host must not vanish entirely.
        let raw = r#"{"t":"col","children":[{"t":"text","s":"hi"},{"t":"hologram","x":1}]}"#;
        let tree: Widget = serde_json::from_str(raw).unwrap();
        let Widget::Col { children, .. } = &tree else {
            panic!("expected col");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[1], Widget::Unknown);
    }

    #[test]
    fn measure_reports_nodes_and_depth() {
        let tree = Widget::Col {
            gap: 1,
            children: vec![
                Widget::Text {
                    s: "a".into(),
                    fg: Tone::Ok,
                    bold: false,
                },
                Widget::Row {
                    gap: 0,
                    children: vec![
                        Widget::Sep,
                        Widget::Badge {
                            s: "3000".into(),
                            tone: Tone::Warn,
                        },
                    ],
                },
            ],
        };
        let stats = measure(&tree);
        assert_eq!(stats.nodes, 5);
        assert_eq!(stats.depth, 3);
        assert!(stats.within_budget());
    }

    #[test]
    fn pathological_tree_exceeds_budget() {
        // Depth guard: external authors will send these.
        let mut tree = Widget::Sep;
        for _ in 0..MAX_WIDGET_DEPTH + 5 {
            tree = Widget::Col {
                gap: 0,
                children: vec![tree],
            };
        }
        assert!(!measure(&tree).within_budget());
    }

    #[test]
    fn tone_defaults_to_fg_when_absent() {
        let w: Widget = serde_json::from_str(r#"{"t":"text","s":"x"}"#).unwrap();
        assert_eq!(
            w,
            Widget::Text {
                s: "x".into(),
                fg: Tone::Fg,
                bold: false
            }
        );
    }

    #[test]
    fn draw_scene_call_round_trips_and_declares_its_capability() {
        let call = HostCall::DrawScene {
            pane: Uuid::from_u128(3),
            scene: SceneData {
                cols: 3,
                rows: 2,
                floor: [18, 18, 22],
                camera: SceneCamera {
                    yaw: 0.7,
                    pitch: 0.42,
                    zoom: 1.25,
                },
                bars: vec![
                    SceneBar {
                        gx: 0,
                        gz: 0,
                        height: 1.0,
                        color: [40, 70, 95],
                        selected: true,
                    },
                    SceneBar {
                        gx: 1,
                        gz: 0,
                        height: 0.25,
                        color: [95, 55, 35],
                        selected: false,
                    },
                ],
            },
        };
        assert_eq!(
            call.required_capability(),
            Capability::HostCallDrawScene
        );
        let line = serde_json::to_string(&call).unwrap();
        assert!(line.contains(r#""call":"draw_scene""#));
        assert_eq!(serde_json::from_str::<HostCall>(&line).unwrap(), call);
    }

    #[test]
    fn scene_ok_result_round_trips() {
        let result = HostCallResult::SceneOk;
        let line = serde_json::to_string(&result).unwrap();
        assert!(line.contains(r#""result":"scene_ok""#));
        assert_eq!(serde_json::from_str::<HostCallResult>(&line).unwrap(), result);
    }

    #[test]
    fn scene_camera_zoom_defaults_to_one_when_absent() {
        // A camera decoded from an older/partial payload must not collapse the
        // projection with zoom 0.
        let cam: SceneCamera = serde_json::from_str(r#"{"yaw":0.1,"pitch":0.2}"#).unwrap();
        assert_eq!(cam.zoom, 1.0);
        let scene: SceneData = serde_json::from_str(r#"{"cols":1,"rows":1}"#).unwrap();
        assert_eq!(scene.camera.zoom, 1.0);
        assert!(scene.bars.is_empty());
    }
}
