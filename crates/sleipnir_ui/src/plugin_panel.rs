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
