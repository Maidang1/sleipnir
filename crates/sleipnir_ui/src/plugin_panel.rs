//! Panel mount: host-owned widget surfaces (ADR-0017).
//!
//! Panel is the first place a plugin tree appears on screen. It occupies a
//! split in `pane_tree`, so focus / zoom / tabs come free and no Block
//! coordinate math is involved. The host owns every surface: a plugin that
//! dies leaves its last tree, marked stale; a crafted tree cannot hide the
//! attribution band (`sleipnir_widget` reserved it).
//!
//! Pure decision logic. No gpui, no window. The shell calls these helpers,
//! then paints the [`sleipnir_widget::Layout`] they did not recompute.

use plugin_protocol::v2::Widget;
use sleipnir_widget::{Hit, Layout, hit_test, layout};
use uuid::Uuid;

use crate::pane_tree::PaneKey;
use crate::plugin_surface::Surface;

/// One plugin-drawn panel. The tree is data; the host stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSurface {
    pub plugin_id: String,
    pub owner_instance_id: Uuid,
    pub pane_key: PaneKey,
    /// Forwarded as `Action.block_id`. Stable for the life of this surface.
    pub surface_id: Uuid,
    pub tree: Widget,
    /// Plugin process is gone. The last tree stays, visibly marked.
    pub stale: bool,
}

impl Surface for PanelSurface {
    fn owner_instance_id(&self) -> Uuid {
        self.owner_instance_id
    }
    fn set_stale(&mut self, stale: bool) {
        self.stale = stale;
    }
}

impl PanelSurface {
    /// The plugin that owns this surface. Kept so callers holding a leaf's
    /// surface read an id the same way the old panel wrapper offered.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

/// Outcome of deciding a `Render { target: Panel }`. Pure policy: the shell
/// gathers the existing leaf (by reference), calls [`decide_panel_render`], and
/// executes the verdict. The deny matrix lives here, not in the GPUI shell, so
/// it stays unit-testable (mirrors `ApplyChrome` / `ApplyBlock`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyPanel {
    /// No existing leaf: insert a new panel leaf with `surface_id`.
    Create { surface_id: Uuid },
    /// A leaf exists for this pane: replace its surface in place. `surface_id`
    /// is the id to write — the same one when the owning instance is unchanged,
    /// a freshly minted one when a new instance reclaims a stale surface.
    Replace { surface_id: Uuid },
    /// No `RenderPanel` grant. The tree is discarded.
    DeniedGrant,
    /// `pane` is a live terminal. Rendering into it would steal the PTY.
    DeniedTerminal,
    /// Another plugin already owns this pane_key.
    DeniedOccupied,
    /// The same plugin id already has a live panel here, but from a different
    /// instance. Only a stale surface may be reclaimed by a new instance.
    DeniedOwnerInstance,
}

/// Decide how a whole-tree `Render { target: Panel }` should be applied.
///
/// Pure decision logic — no gpui, no pane tree. `existing` is the surface
/// already mounted on `pane` (if any), borrowed from the leaf. `granted` is the
/// live session's `RenderPanel` bit; `is_terminal` is true when `pane` is a PTY
/// leaf. The caller executes the returned verdict, minting `surface_id` into a
/// [`PanelSurface`] for Create/Replace.
pub fn decide_panel_render(
    existing: Option<&PanelSurface>,
    plugin_id: &str,
    instance_id: Uuid,
    is_terminal: bool,
    granted: bool,
) -> ApplyPanel {
    if !granted {
        return ApplyPanel::DeniedGrant;
    }
    if is_terminal {
        return ApplyPanel::DeniedTerminal;
    }
    match existing {
        Some(existing) if existing.plugin_id != plugin_id => ApplyPanel::DeniedOccupied,
        Some(existing) if existing.owner_instance_id != instance_id && !existing.stale => {
            ApplyPanel::DeniedOwnerInstance
        }
        Some(existing) => {
            // Stale reclaim by a new instance mints a fresh surface id so old
            // action routing cannot land on the new owner; same instance keeps
            // its id.
            let surface_id = if existing.owner_instance_id != instance_id {
                Uuid::new_v4()
            } else {
                existing.surface_id
            };
            ApplyPanel::Replace { surface_id }
        }
        None => ApplyPanel::Create {
            surface_id: Uuid::new_v4(),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelAction {
    pub action: String,
    pub arg: Option<String>,
}

/// Hit-test a laid-out tree at a cell. Only `Btn` produces an action.
pub fn action_at(laid: &Layout, col: u32, row: u32) -> Option<PanelAction> {
    match hit_test(laid, sleipnir_widget::CellPos { col, row }) {
        Hit::Btn { action, arg } => Some(PanelAction {
            action: action.to_string(),
            arg: arg.map(str::to_string),
        }),
        Hit::Miss => None,
    }
}

/// Lay out `surface.tree` through the shared crate. Mount points must not
/// reimplement wrap / budget / attribution.
pub fn layout_surface(surface: &PanelSurface, cols: u16) -> Layout {
    layout(&surface.tree, cols, &surface.plugin_id)
}

/// A tab is a workspace of shells (ADR-0001). Panels occupy a split of that
/// workspace; they are not themselves a workspace. Closing the last terminal
/// while a panel remains would leave a tab that looks alive but has no PTY
/// to type into — so the tab closes, taking its guest panels with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabClosePolicy {
    KeepTab,
    CloseTab,
}

pub fn tab_close_policy(terminals_remaining: usize) -> TabClosePolicy {
    if terminals_remaining == 0 {
        TabClosePolicy::CloseTab
    } else {
        TabClosePolicy::KeepTab
    }
}

/// Columns that fit in `pixel_width` given the terminal's cell width.
pub fn cols_from_pixels(pixel_width: f32, cell_width: f32) -> u16 {
    if !pixel_width.is_finite() || !cell_width.is_finite() || cell_width <= 0.0 {
        return 1;
    }
    let cols = (pixel_width / cell_width).floor();
    if !cols.is_finite() || cols < 1.0 {
        1
    } else if cols > u16::MAX as f32 {
        u16::MAX
    } else {
        cols as u16
    }
}

pub fn cell_from_pixels(
    local_x: f32,
    local_y: f32,
    cell_width: f32,
    line_height: f32,
) -> sleipnir_widget::CellPos {
    let col = if cell_width > 0.0 && local_x.is_finite() {
        (local_x / cell_width).floor().max(0.0) as u32
    } else {
        0
    };
    let row = if line_height > 0.0 && local_y.is_finite() {
        (local_y / line_height).floor().max(0.0) as u32
    } else {
        0
    };
    sleipnir_widget::CellPos { col, row }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_protocol::v2::Tone;
    use sleipnir_widget::LaidOutKind;

    fn text(s: &str) -> Widget {
        Widget::Text {
            s: s.into(),
            fg: Tone::Fg,
            bold: false,
        }
    }

    fn btn(s: &str, action: &str) -> Widget {
        Widget::Btn {
            s: s.into(),
            action: action.into(),
            arg: None,
        }
    }

    fn key(n: u128) -> PaneKey {
        Uuid::from_u128(n)
    }

    /// Apply a `decide_panel_render` verdict to an in-memory surface slot,
    /// mirroring what the shell does to the tree leaf. Returns the verdict so a
    /// test can assert on it. Denials leave the slot untouched.
    fn apply(
        slot: &mut Option<PanelSurface>,
        plugin_id: &str,
        instance_id: Uuid,
        pane: PaneKey,
        tree: Widget,
        is_terminal: bool,
        granted: bool,
    ) -> ApplyPanel {
        let out = decide_panel_render(slot.as_ref(), plugin_id, instance_id, is_terminal, granted);
        match out {
            ApplyPanel::Create { surface_id } | ApplyPanel::Replace { surface_id } => {
                *slot = Some(PanelSurface {
                    plugin_id: plugin_id.to_string(),
                    owner_instance_id: instance_id,
                    pane_key: pane,
                    surface_id,
                    tree,
                    stale: false,
                });
            }
            ApplyPanel::DeniedGrant
            | ApplyPanel::DeniedTerminal
            | ApplyPanel::DeniedOccupied
            | ApplyPanel::DeniedOwnerInstance => {}
        }
        out
    }

    #[test]
    fn render_panel_grant_is_required() {
        let mut slot = None;
        let out = apply(
            &mut slot,
            "demo",
            Uuid::nil(),
            key(1),
            text("hi"),
            false,
            false,
        );
        assert_eq!(out, ApplyPanel::DeniedGrant);
        assert!(slot.is_none());
    }

    #[test]
    fn render_will_not_steal_a_terminal_pane() {
        let mut slot = None;
        let out = apply(
            &mut slot,
            "demo",
            Uuid::nil(),
            key(7),
            text("hi"),
            true,
            true,
        );
        assert_eq!(out, ApplyPanel::DeniedTerminal);
        assert!(slot.is_none());
    }

    #[test]
    fn whole_tree_replacement_overwrites_and_clears_stale() {
        let mut slot = None;
        assert!(matches!(
            apply(
                &mut slot,
                "demo",
                Uuid::from_u128(1),
                key(1),
                text("one"),
                false,
                true
            ),
            ApplyPanel::Create { .. }
        ));
        let original_surface_id = slot.as_ref().unwrap().surface_id;
        // Owner instance dies -> the sync pass marks it stale.
        slot.as_mut().unwrap().stale = true;
        let out = apply(
            &mut slot,
            "demo",
            Uuid::from_u128(2),
            key(1),
            text("two"),
            false,
            true,
        );
        let surface = slot.as_ref().unwrap();
        assert_eq!(
            out,
            ApplyPanel::Replace {
                surface_id: surface.surface_id
            }
        );
        assert!(!surface.stale);
        assert_eq!(surface.owner_instance_id, Uuid::from_u128(2));
        assert_eq!(surface.tree, text("two"));
        assert_ne!(
            surface.surface_id, original_surface_id,
            "stale reclaim must mint a fresh surface id"
        );
    }

    #[test]
    fn another_plugin_cannot_occupy_an_existing_panel() {
        let mut slot = None;
        apply(&mut slot, "a", Uuid::nil(), key(1), text("a"), false, true);
        let out = apply(&mut slot, "b", Uuid::nil(), key(1), text("b"), false, true);
        assert_eq!(out, ApplyPanel::DeniedOccupied);
        assert_eq!(slot.as_ref().unwrap().plugin_id, "a");
    }

    #[test]
    fn same_plugin_live_different_instance_cannot_take_panel() {
        let mut slot = None;
        apply(
            &mut slot,
            "demo",
            Uuid::from_u128(1),
            key(1),
            text("one"),
            false,
            true,
        );
        let original = slot.clone();
        let out = apply(
            &mut slot,
            "demo",
            Uuid::from_u128(2),
            key(1),
            text("two"),
            false,
            true,
        );
        assert_eq!(out, ApplyPanel::DeniedOwnerInstance);
        assert_eq!(slot, original);
    }

    #[test]
    fn same_instance_keeps_its_surface_id_on_replace() {
        let mut slot = None;
        apply(
            &mut slot,
            "demo",
            Uuid::from_u128(1),
            key(1),
            text("one"),
            false,
            true,
        );
        let id = slot.as_ref().unwrap().surface_id;
        let out = apply(
            &mut slot,
            "demo",
            Uuid::from_u128(1),
            key(1),
            text("two"),
            false,
            true,
        );
        assert_eq!(out, ApplyPanel::Replace { surface_id: id });
        assert_eq!(slot.as_ref().unwrap().tree, text("two"));
    }

    #[test]
    fn death_marks_stale_without_dropping_the_tree() {
        // The pure decision does not itself mark stale; the sync pass does. This
        // pins that a marked-stale surface still holds its tree and remains
        // reclaimable (see whole_tree_replacement_overwrites_and_clears_stale).
        let mut slot = None;
        apply(
            &mut slot,
            "demo",
            Uuid::from_u128(10),
            key(1),
            text("keep"),
            false,
            true,
        );
        slot.as_mut().unwrap().stale = true;
        let surface = slot.as_ref().unwrap();
        assert!(surface.stale);
        assert_eq!(surface.tree, text("keep"));
    }

    #[test]
    fn hit_test_routes_btn_to_action() {
        let surface = PanelSurface {
            plugin_id: "demo".into(),
            owner_instance_id: Uuid::nil(),
            pane_key: key(1),
            surface_id: Uuid::nil(),
            tree: btn("Go", "retry"),
            stale: false,
        };
        let laid = layout_surface(&surface, 20);
        let hit = action_at(&laid, 0, 0).expect("btn");
        assert_eq!(hit.action, "retry");
        assert!(action_at(&laid, 0, laid.attribution.rect.row).is_none());
    }

    #[test]
    fn layout_is_the_shared_crate_and_keeps_attribution() {
        let surface = PanelSurface {
            plugin_id: "honest".into(),
            owner_instance_id: Uuid::nil(),
            pane_key: key(1),
            surface_id: Uuid::nil(),
            tree: text("plugin:evil"),
            stale: false,
        };
        let laid = layout_surface(&surface, 20);
        assert!(matches!(
            laid.attribution.kind,
            LaidOutKind::Attribution { .. }
        ));
        let LaidOutKind::Attribution { plugin_id, .. } = &laid.attribution.kind else {
            panic!();
        };
        assert_eq!(plugin_id, "honest");
    }

    #[test]
    fn last_terminal_closes_the_tab() {
        assert_eq!(tab_close_policy(0), TabClosePolicy::CloseTab);
        assert_eq!(tab_close_policy(1), TabClosePolicy::KeepTab);
    }

    #[test]
    fn cols_from_pixels_never_zero_or_panic() {
        assert_eq!(cols_from_pixels(80.0, 8.0), 10);
        assert_eq!(cols_from_pixels(0.0, 8.0), 1);
        assert_eq!(cols_from_pixels(80.0, 0.0), 1);
        assert_eq!(cols_from_pixels(f32::NAN, 8.0), 1);
        assert_eq!(cols_from_pixels(80.0, f32::INFINITY), 1);
    }
}
