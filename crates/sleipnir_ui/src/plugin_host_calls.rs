//! Plugin-initiated host calls (ADR-0016 §3–§4).
//!
//! A `Call` is a plugin asking the host to do something the control surface
//! already exposes locally (ADR-0011): notify, read a pane, list panes, open a
//! pane. v2 adds no new power — it changes *who* may ask, which is why each
//! verb is a separately granted capability and is **never implied** by a v1
//! read permission.
//!
//! Every `Call` id must produce exactly one `Reply`. A silent drop (denied,
//! missing pane, rate limit, dead plugin) would leave a resident plugin
//! waiting forever. Denial is [`HostCallResult::Error`], not absence.
//!
//! Pure decision logic. No gpui, no window, no process spawn. The shell
//! executes the plan and always calls `reply`.

use plugin_protocol::v2::{Capability, HostCall, HostCallResult, PaneInfo, SceneData};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::pane_tree::PaneKey;

/// Notify title cap. A plugin-controlled string is interpolated into a
/// platform notification; unbounded titles are a spam vector.
pub const MAX_NOTIFY_TITLE: usize = 80;
/// Notify body cap.
pub const MAX_NOTIFY_BODY: usize = 500;
/// Visible-screen cap. Scrollback is not included (mirrors `capture`).
pub const MAX_SCREEN_CHARS: usize = 64 * 1024;
/// OpenPane command string cap, before argv split.
pub const MAX_OPEN_COMMAND_CHARS: usize = 1024;
/// Sliding window for per-plugin host-call rate limiting.
pub const RATE_WINDOW_MS: u64 = 5_000;
/// Max accepted calls per plugin in [`RATE_WINDOW_MS`].
pub const RATE_MAX_CALLS: u32 = 10;
/// Max bars in one DrawScene call. Matches the scanner's `MAX_BARS` with
/// generous headroom; an external plugin sending more is malformed, not drawn.
pub const MAX_SCENE_BARS: usize = 256;
/// Max grid extent (cols or rows) in one DrawScene call. A bar grid larger than
/// this cannot be laid out legibly and is almost certainly a bad payload.
pub const MAX_SCENE_GRID: u32 = 64;

/// Program + argv for OpenPane. Never a shell command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// What the UI should do for one `Call`. Always ends in a reply.
///
/// Not `Eq`: `DrawScene` carries floating-point geometry.
#[derive(Clone, Debug, PartialEq)]
pub enum CallPlan {
    /// No side effect: send this result as the Reply.
    Reply(HostCallResult),
    Notify {
        title: String,
        body: String,
    },
    ReadScreen {
        pane: PaneKey,
    },
    ListPanes,
    OpenPane {
        cwd: Option<String>,
        command: Option<OpenCommand>,
    },
    DrawScene {
        pane: PaneKey,
        scene: SceneData,
    },
}

/// Per-plugin sliding-window limiter. Drops are counted so the Monitor can
/// show a resident plugin that is hammering Notify / OpenPane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostCallLimiter {
    windows: BTreeMap<String, VecDeque<u64>>,
    dropped: BTreeMap<String, u64>,
}

impl HostCallLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// True if this call is inside the budget. A rejection still requires a
    /// Reply; the UI must not drop the id.
    pub fn allow(&mut self, plugin_id: &str, now_ms: u64) -> bool {
        let q = self.windows.entry(plugin_id.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= RATE_WINDOW_MS)
        {
            q.pop_front();
        }
        if q.len() as u32 >= RATE_MAX_CALLS {
            *self.dropped.entry(plugin_id.to_string()).or_insert(0) += 1;
            return false;
        }
        q.push_back(now_ms);
        true
    }

    pub fn dropped_counts(&self) -> &BTreeMap<String, u64> {
        &self.dropped
    }
}

/// Plan one inbound `Call`. Capability is checked first so a missing grant is
/// reported as itself rather than as a rate-limit. Rate limiting then bounds
/// a granted plugin that would otherwise spam Notify or OpenPane.
pub fn plan_call(
    plugin_id: &str,
    call: &HostCall,
    granted: &[Capability],
    limiter: &mut HostCallLimiter,
    now_ms: u64,
) -> CallPlan {
    let need = call.required_capability();
    if !granted.contains(&need) {
        return CallPlan::Reply(HostCallResult::Error {
            message: format!("capability {need:?} not granted"),
        });
    }
    // DrawScene is exempt from the anti-spam limiter: it only repaints the
    // host's own surface (no external side effect like Notify / OpenPane),
    // and legitimate animations (e.g. disk3d Spin) exceed the budget.
    if !matches!(call, HostCall::DrawScene { .. }) && !limiter.allow(plugin_id, now_ms) {
        return CallPlan::Reply(HostCallResult::Error {
            message: "rate limited".into(),
        });
    }
    match call {
        HostCall::Notify { title, body } => {
            let title = cap_chars(title, MAX_NOTIFY_TITLE);
            let body = cap_chars(body, MAX_NOTIFY_BODY);
            CallPlan::Notify { title, body }
        }
        HostCall::ReadScreen { pane } => CallPlan::ReadScreen { pane: *pane },
        HostCall::ListPanes => CallPlan::ListPanes,
        HostCall::OpenPane { cwd, command } => match parse_open_pane(cwd, command) {
            Ok((cwd, command)) => CallPlan::OpenPane { cwd, command },
            Err(message) => CallPlan::Reply(HostCallResult::Error { message }),
        },
        HostCall::DrawScene { pane, scene } => match validate_scene(scene) {
            Ok(()) => CallPlan::DrawScene {
                pane: *pane,
                scene: scene.clone(),
            },
            Err(message) => CallPlan::Reply(HostCallResult::Error { message }),
        },
    }
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn parse_open_pane(
    cwd: &Option<String>,
    command: &Option<String>,
) -> Result<(Option<String>, Option<OpenCommand>), String> {
    let cwd = match cwd.as_deref() {
        None => None,
        Some(raw) if raw.trim().is_empty() => {
            return Err("cwd is empty".into());
        }
        Some(raw) => Some(raw.to_string()),
    };
    let command = match command.as_deref() {
        None => None,
        Some(raw) => Some(parse_open_command(raw)?),
    };
    Ok((cwd, command))
}

/// Split `command` into program + argv on whitespace.
///
/// No quote processing and no `sh -c`. Quote processing *is* a shell; piping
/// the plugin string through one would let OpenPane run arbitrary scripts
/// (ADR-0013: arguments are passed directly).
pub fn parse_open_command(command: &str) -> Result<OpenCommand, String> {
    if command.chars().count() > MAX_OPEN_COMMAND_CHARS {
        return Err("command exceeds length cap".into());
    }
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return Err("command is empty".into());
    };
    Ok(OpenCommand {
        program: program.to_string(),
        args: parts.map(str::to_string).collect(),
    })
}

/// Truncate visible screen text at a char boundary.
pub fn cap_screen(text: String) -> String {
    cap_chars(&text, MAX_SCREEN_CHARS)
}

/// Validate a `DrawScene` payload before the host stores it.
///
/// Cheap structural checks only: bar count and grid extent are bounded so a
/// malformed or hostile plugin cannot force the host to lay out an absurd grid,
/// and every bar must sit inside the declared grid. Geometry values (height,
/// colour) are clamped at paint time, not rejected here.
fn validate_scene(scene: &SceneData) -> Result<(), String> {
    if scene.bars.len() > MAX_SCENE_BARS {
        return Err(format!(
            "scene has {} bars, exceeds {MAX_SCENE_BARS}",
            scene.bars.len()
        ));
    }
    // An empty scene is legal (nothing to chart); a non-empty one needs a grid.
    if !scene.bars.is_empty() && (scene.cols == 0 || scene.rows == 0) {
        return Err("scene has bars but a zero-sized grid".into());
    }
    if scene.cols > MAX_SCENE_GRID || scene.rows > MAX_SCENE_GRID {
        return Err(format!(
            "scene grid {}x{} exceeds {MAX_SCENE_GRID}",
            scene.cols, scene.rows
        ));
    }
    for bar in &scene.bars {
        if bar.gx >= scene.cols || bar.gz >= scene.rows {
            return Err(format!(
                "bar at ({},{}) is outside the {}x{} grid",
                bar.gx, bar.gz, scene.cols, scene.rows
            ));
        }
    }
    Ok(())
}

/// ListPanes reports only terminal panes. A plugin Panel is not a PTY and
/// must not appear (and is not a valid ReadScreen target).
pub fn filter_listed_panes(
    panes: Vec<PaneInfo>,
    terminal_keys: &BTreeSet<PaneKey>,
) -> Vec<PaneInfo> {
    panes
        .into_iter()
        .filter(|p| terminal_keys.contains(&p.pane))
        .collect()
}

/// Classify a ReadScreen target. v1 `read_visible_screen` is not consulted
/// here — the caller already required [`Capability::HostCallReadScreen`].
pub fn read_screen_access(
    pane: PaneKey,
    terminal_keys: &BTreeSet<PaneKey>,
    panel_keys: &BTreeSet<PaneKey>,
) -> Result<(), String> {
    if panel_keys.contains(&pane) {
        return Err("pane is a plugin panel, not a terminal".into());
    }
    if !terminal_keys.contains(&pane) {
        return Err(format!("pane {pane} not found"));
    }
    Ok(())
}

pub fn error_result(message: impl Into<String>) -> HostCallResult {
    HostCallResult::Error {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn key(n: u128) -> PaneKey {
        Uuid::from_u128(n)
    }

    fn notify(title: &str, body: &str) -> HostCall {
        HostCall::Notify {
            title: title.into(),
            body: body.into(),
        }
    }

    #[test]
    fn each_call_denied_without_its_capability_is_an_error_reply() {
        let mut limiter = HostCallLimiter::new();
        let calls = [
            notify("t", "b"),
            HostCall::ReadScreen { pane: key(1) },
            HostCall::ListPanes,
            HostCall::OpenPane {
                cwd: None,
                command: None,
            },
        ];
        for call in calls {
            let plan = plan_call("demo", &call, &[], &mut limiter, 0);
            match plan {
                CallPlan::Reply(HostCallResult::Error { message }) => {
                    assert!(
                        message.contains("not granted"),
                        "denial must be an Error reply, not a drop: {message}"
                    );
                }
                other => panic!("expected Error reply, got {other:?}"),
            }
        }
    }

    #[test]
    fn every_call_id_gets_a_plan_including_denial() {
        // The drain loop maps one inbound Call to one plan. There is no
        // branch that produces "no reply".
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call("demo", &HostCall::ListPanes, &[], &mut limiter, 0);
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
    }

    #[test]
    fn read_screen_not_implied_by_v1_read_visible_screen() {
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "demo",
            &HostCall::ReadScreen { pane: key(1) },
            &[Capability::ReadVisibleScreen],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("HostCallReadScreen") || message.contains("not granted"));
            }
            other => panic!("v1 read_visible_screen must not imply ReadScreen: {other:?}"),
        }
    }

    #[test]
    fn list_panes_excludes_plugin_panel_leaves() {
        let terminal = key(1);
        let panel = key(2);
        let panes = vec![
            PaneInfo {
                pane: terminal,
                cwd: Some("/tmp".into()),
                title: Some("shell".into()),
                busy: false,
            },
            PaneInfo {
                pane: panel,
                cwd: None,
                title: Some("demo".into()),
                busy: false,
            },
        ];
        let mut keys = BTreeSet::new();
        keys.insert(terminal);
        let listed = filter_listed_panes(panes, &keys);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pane, terminal);
    }

    #[test]
    fn read_screen_on_a_panel_pane_key_fails() {
        let terminal = key(1);
        let panel = key(2);
        let mut terminals = BTreeSet::new();
        terminals.insert(terminal);
        let mut panels = BTreeSet::new();
        panels.insert(panel);
        assert!(
            read_screen_access(panel, &terminals, &panels)
                .unwrap_err()
                .contains("plugin panel")
        );
        assert!(
            read_screen_access(key(99), &terminals, &panels)
                .unwrap_err()
                .contains("not found")
        );
        assert!(read_screen_access(terminal, &terminals, &panels).is_ok());
    }

    #[test]
    fn notify_length_caps_are_enforced() {
        let mut limiter = HostCallLimiter::new();
        let title: String = std::iter::repeat_n('t', MAX_NOTIFY_TITLE + 40).collect();
        let body: String = std::iter::repeat_n('b', MAX_NOTIFY_BODY + 40).collect();
        let plan = plan_call(
            "demo",
            &notify(&title, &body),
            &[Capability::HostCallNotify],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::Notify { title, body } => {
                assert_eq!(title.chars().count(), MAX_NOTIFY_TITLE);
                assert_eq!(body.chars().count(), MAX_NOTIFY_BODY);
            }
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    #[test]
    fn screen_cap_truncates_at_char_boundary() {
        let s: String = std::iter::repeat_n('字', 10).collect();
        let capped = cap_screen(s);
        // 10 CJK chars is under the cap.
        assert_eq!(capped.chars().count(), 10);
        let huge: String = std::iter::repeat_n('a', MAX_SCREEN_CHARS + 50).collect();
        assert_eq!(cap_screen(huge).chars().count(), MAX_SCREEN_CHARS);
    }

    #[test]
    fn rate_limiting_drops_with_an_accounted_counter_and_still_replies() {
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallNotify];
        let mut accepted = 0u32;
        let mut denied = 0u32;
        for i in 0..RATE_MAX_CALLS + 5 {
            match plan_call("demo", &notify("t", "b"), &granted, &mut limiter, 1_000) {
                CallPlan::Notify { .. } => accepted += 1,
                CallPlan::Reply(HostCallResult::Error { message }) => {
                    assert_eq!(message, "rate limited");
                    denied += 1;
                }
                other => panic!("unexpected {other:?} at i={i}"),
            }
        }
        assert_eq!(accepted, RATE_MAX_CALLS);
        assert_eq!(denied, 5);
        assert_eq!(
            limiter.dropped_counts().get("demo").copied().unwrap_or(0),
            5
        );
        // Window expiry admits another call.
        match plan_call(
            "demo",
            &notify("t", "b"),
            &granted,
            &mut limiter,
            1_000 + RATE_WINDOW_MS,
        ) {
            CallPlan::Notify { .. } => {}
            other => panic!("window should have expired: {other:?}"),
        }
    }

    #[test]
    fn draw_scene_is_exempt_from_the_rate_limiter() {
        // DrawScene only repaints the host's own surface; it has no external
        // side effect like Notify / OpenPane. The anti-spam limiter must not
        // freeze a granted animation (disk3d Spin sends 22 frames in 1.8s).
        let mut limiter = HostCallLimiter::new();
        let granted = [Capability::HostCallDrawScene];
        let call = scene(1, 1, vec![bar(0, 0)]);
        for i in 0..RATE_MAX_CALLS + 12 {
            let plan = plan_call("gfx", &call, &granted, &mut limiter, 1_000);
            assert!(
                matches!(plan, CallPlan::DrawScene { .. }),
                "frame {i} must not be rate limited, got {plan:?}"
            );
        }
        // Exemption must not count towards (or inflate) the dropped counter.
        assert_eq!(limiter.dropped_counts().get("gfx").copied().unwrap_or(0), 0);
    }

    #[test]
    fn open_command_is_argv_not_a_shell_line() {
        let cmd = parse_open_command("cargo test --all").unwrap();
        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, ["test", "--all"]);
        assert!(parse_open_command("").is_err());
        let long: String = std::iter::repeat_n('x', MAX_OPEN_COMMAND_CHARS + 1).collect();
        assert!(parse_open_command(&long).is_err());
        // Quotes are literal tokens, not shell syntax.
        let quoted = parse_open_command(r#"echo "hello world""#).unwrap();
        assert_eq!(quoted.program, "echo");
        assert_eq!(quoted.args, [r#""hello"#, r#"world""#]);
    }

    #[test]
    fn empty_open_cwd_is_malformed_not_executed() {
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "demo",
            &HostCall::OpenPane {
                cwd: Some("  ".into()),
                command: None,
            },
            &[Capability::HostCallOpenPane],
            &mut limiter,
            0,
        );
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
    }

    #[test]
    fn required_capability_is_the_protocol_mapping() {
        assert_eq!(
            notify("t", "b").required_capability(),
            Capability::HostCallNotify
        );
        assert_eq!(
            HostCall::ListPanes.required_capability(),
            Capability::HostCallListPanes
        );
        assert_eq!(
            HostCall::ReadScreen { pane: key(1) }.required_capability(),
            Capability::HostCallReadScreen
        );
        assert_eq!(
            HostCall::OpenPane {
                cwd: None,
                command: None
            }
            .required_capability(),
            Capability::HostCallOpenPane
        );
    }

    #[test]
    fn reply_is_produced_when_the_plugin_is_already_gone() {
        // The drain loop still plans a result for the id. Sending it may fail
        // if the session is dead; that is a lost write, not a hang.
        let mut limiter = HostCallLimiter::new();
        let plan = plan_call(
            "dead",
            &HostCall::ListPanes,
            &[Capability::HostCallListPanes],
            &mut limiter,
            0,
        );
        assert!(matches!(plan, CallPlan::ListPanes));
    }

    fn scene(cols: u32, rows: u32, bars: Vec<plugin_protocol::v2::SceneBar>) -> HostCall {
        HostCall::DrawScene {
            pane: key(1),
            scene: SceneData {
                cols,
                rows,
                floor: [18, 18, 22],
                camera: plugin_protocol::v2::SceneCamera::default(),
                bars,
            },
        }
    }

    fn bar(gx: u32, gz: u32) -> plugin_protocol::v2::SceneBar {
        plugin_protocol::v2::SceneBar {
            gx,
            gz,
            height: 0.5,
            color: [40, 70, 95],
            selected: false,
        }
    }

    #[test]
    fn draw_scene_accepts_a_well_formed_scene() {
        let mut limiter = HostCallLimiter::new();
        let call = scene(2, 2, vec![bar(0, 0), bar(1, 1)]);
        let plan = plan_call(
            "gfx",
            &call,
            &[Capability::HostCallDrawScene],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::DrawScene { pane, scene } => {
                assert_eq!(pane, key(1));
                assert_eq!(scene.bars.len(), 2);
            }
            other => panic!("expected DrawScene, got {other:?}"),
        }
    }

    #[test]
    fn draw_scene_rejects_too_many_bars() {
        let mut limiter = HostCallLimiter::new();
        let bars: Vec<_> = (0..MAX_SCENE_BARS + 1).map(|_| bar(0, 0)).collect();
        let call = scene(MAX_SCENE_GRID, MAX_SCENE_GRID, bars);
        let plan = plan_call(
            "gfx",
            &call,
            &[Capability::HostCallDrawScene],
            &mut limiter,
            0,
        );
        match plan {
            CallPlan::Reply(HostCallResult::Error { message }) => {
                assert!(message.contains("exceeds"), "{message}");
            }
            other => panic!("expected Error reply, got {other:?}"),
        }
    }

    #[test]
    fn draw_scene_rejects_out_of_grid_bar_and_bad_dimensions() {
        let mut limiter = HostCallLimiter::new();
        // A bar outside the declared grid.
        let call = scene(1, 1, vec![bar(2, 0)]);
        let plan = plan_call(
            "gfx",
            &call,
            &[Capability::HostCallDrawScene],
            &mut limiter,
            0,
        );
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
        // Bars present but a zero-sized grid.
        let call = scene(0, 0, vec![bar(0, 0)]);
        let plan = plan_call(
            "gfx",
            &call,
            &[Capability::HostCallDrawScene],
            &mut limiter,
            0,
        );
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
        // An oversized grid.
        let call = scene(MAX_SCENE_GRID + 1, 1, vec![]);
        let plan = plan_call(
            "gfx",
            &call,
            &[Capability::HostCallDrawScene],
            &mut limiter,
            0,
        );
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
    }

    #[test]
    fn draw_scene_needs_its_capability() {
        let mut limiter = HostCallLimiter::new();
        let call = scene(1, 1, vec![bar(0, 0)]);
        let plan = plan_call("gfx", &call, &[], &mut limiter, 0);
        assert!(matches!(
            plan,
            CallPlan::Reply(HostCallResult::Error { .. })
        ));
    }
}
