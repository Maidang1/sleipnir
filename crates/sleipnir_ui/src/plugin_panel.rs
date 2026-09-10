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

use plugin_protocol::v2::{SceneData, Widget};
use sleipnir_widget::{Hit, Layout, hit_test, layout};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::pane_tree::PaneKey;
use crate::plugin_surface::{StaleRegistry, Surface};

/// A 3D scene sent by a plugin, projected and painted host-side in the panel.
/// Mirrors [`plugin_protocol::v2::SceneData`]; the host stores it verbatim and
/// owns projection so the picture stays crisp at any panel size.
pub type PanelScene = SceneData;

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
    /// 3D scene from the plugin, projected and painted below the chrome.
    pub scene: Option<PanelScene>,
}

impl Surface for PanelSurface {
    fn owner_instance_id(&self) -> Uuid {
        self.owner_instance_id
    }
    fn set_stale(&mut self, stale: bool) {
        self.stale = stale;
    }
}

/// Outcome of applying a `Render { target: Panel }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyPanel {
    /// Insert a new leaf. The caller performs the pane_tree split.
    Create { pane_key: PaneKey },
    /// Same pane, new tree. Elm-style whole-tree replacement (ADR-0017).
    Replace { pane_key: PaneKey },
    /// No `RenderPanel` grant. The tree is discarded.
    DeniedGrant,
    /// `pane` is a live terminal. Rendering into it would steal the PTY.
    DeniedTerminal,
    /// Another plugin already owns this pane_key.
    DeniedOccupied,
    /// The same plugin id already has a live panel here, but from a different
    /// plugin instance. Only a stale surface may be reclaimed by a new
    /// instance.
    DeniedOwnerInstance,
}

/// Host-side registry of panel surfaces, keyed by [`PaneKey`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelRegistry {
    surfaces: BTreeMap<PaneKey, PanelSurface>,
}

impl PanelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_surface(&mut self, surface: PanelSurface) {
        self.surfaces.insert(surface.pane_key, surface);
    }

    pub fn insert_surfaces(&mut self, surfaces: impl IntoIterator<Item = PanelSurface>) {
        for surface in surfaces {
            self.insert_surface(surface);
        }
    }

    pub fn get(&self, pane: PaneKey) -> Option<&PanelSurface> {
        self.surfaces.get(&pane)
    }

    pub fn clone_surfaces(&self, keys: impl IntoIterator<Item = PaneKey>) -> Vec<PanelSurface> {
        keys.into_iter()
            .filter_map(|key| self.surfaces.get(&key).cloned())
            .collect()
    }

    pub fn remove(&mut self, pane: PaneKey) -> Option<PanelSurface> {
        self.surfaces.remove(&pane)
    }

    pub fn remove_all(&mut self, keys: impl IntoIterator<Item = PaneKey>) {
        for key in keys {
            self.surfaces.remove(&key);
        }
    }

    /// Apply a whole-tree `Render`. `granted` is the live session's
    /// `RenderPanel` bit (same source the event bus uses for
    /// `SubscribeEvents`). `terminal_panes` are PTY leaves — never overwritten.
    pub fn apply_render(
        &mut self,
        plugin_id: &str,
        owner_instance_id: Uuid,
        pane: PaneKey,
        tree: Widget,
        granted: bool,
        terminal_panes: &BTreeSet<PaneKey>,
    ) -> ApplyPanel {
        if !granted {
            return ApplyPanel::DeniedGrant;
        }
        if terminal_panes.contains(&pane) {
            return ApplyPanel::DeniedTerminal;
        }
        match self.surfaces.get_mut(&pane) {
            Some(existing) if existing.plugin_id != plugin_id => ApplyPanel::DeniedOccupied,
            Some(existing)
                if existing.owner_instance_id != owner_instance_id && !existing.stale =>
            {
                ApplyPanel::DeniedOwnerInstance
            }
            Some(existing) => {
                existing.tree = tree;
                if existing.owner_instance_id != owner_instance_id {
                    existing.owner_instance_id = owner_instance_id;
                    existing.surface_id = Uuid::new_v4();
                    existing.scene = None;
                }
                existing.stale = false;
                ApplyPanel::Replace { pane_key: pane }
            }
            None => {
                self.surfaces.insert(
                    pane,
                    PanelSurface {
                        plugin_id: plugin_id.to_string(),
                        owner_instance_id,
                        pane_key: pane,
                        surface_id: Uuid::new_v4(),
                        tree,
                        stale: false,
                        scene: None,
                    },
                );
                ApplyPanel::Create { pane_key: pane }
            }
        }
    }

    /// Store a 3D scene on a panel surface owned by `plugin_id`. Returns `true`
    /// if the scene was accepted.
    pub fn set_scene(
        &mut self,
        pane: PaneKey,
        plugin_id: &str,
        owner_instance_id: Uuid,
        scene: PanelScene,
    ) -> bool {
        match self.surfaces.get_mut(&pane) {
            Some(surface)
                if !surface.stale
                    && surface.plugin_id == plugin_id
                    && surface.owner_instance_id == owner_instance_id =>
            {
                surface.scene = Some(scene);
                true
            }
            _ => false,
        }
    }

    /// Update just the camera on an existing scene. Used for host-driven camera
    /// moves that must not wait for the plugin to resend geometry. Returns
    /// `true` if a scene was present to update.
    pub fn set_scene_camera(
        &mut self,
        pane: PaneKey,
        camera: plugin_protocol::v2::SceneCamera,
    ) -> bool {
        match self.surfaces.get_mut(&pane) {
            Some(surface) => match surface.scene.as_mut() {
                Some(scene) => {
                    scene.camera = camera;
                    true
                }
                None => false,
            },
            None => false,
        }
    }

    /// Read the current scene on `pane`, if any. The interactive camera reads
    /// this to seed a drag before mutating it.
    pub fn scene(&self, pane: PaneKey) -> Option<&PanelScene> {
        self.surfaces.get(&pane).and_then(|s| s.scene.as_ref())
    }
}

/// Death marks; it never drops. See [`crate::plugin_surface`].
impl StaleRegistry for PanelRegistry {
    type Surface = PanelSurface;
    fn surfaces_mut(&mut self) -> impl Iterator<Item = &mut PanelSurface> {
        self.surfaces.values_mut()
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

/// Default placeholder tree so a Create that races layout still paints
/// attribution. Not used as a protocol default.
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
    fn render_panel_grant_is_required() {
        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        let out = reg.apply_render("demo", Uuid::nil(), key(1), text("hi"), false, &terminals);
        assert_eq!(out, ApplyPanel::DeniedGrant);
        assert!(reg.get(key(1)).is_none());
    }

    #[test]
    fn render_will_not_steal_a_terminal_pane() {
        let mut reg = PanelRegistry::new();
        let mut terminals = BTreeSet::new();
        terminals.insert(key(7));
        let out = reg.apply_render("demo", Uuid::nil(), key(7), text("hi"), true, &terminals);
        assert_eq!(out, ApplyPanel::DeniedTerminal);
        assert!(reg.get(key(7)).is_none());
    }

    #[test]
    fn whole_tree_replacement_overwrites_and_clears_stale() {
        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        assert!(matches!(
            reg.apply_render(
                "demo",
                Uuid::from_u128(1),
                key(1),
                text("one"),
                true,
                &terminals
            ),
            ApplyPanel::Create { .. }
        ));
        let original_surface_id = reg.get(key(1)).unwrap().surface_id;
        reg.mark_missing_stale(&BTreeSet::new());
        assert!(reg.get(key(1)).unwrap().stale);
        let out = reg.apply_render(
            "demo",
            Uuid::from_u128(2),
            key(1),
            text("two"),
            true,
            &terminals,
        );
        assert_eq!(out, ApplyPanel::Replace { pane_key: key(1) });
        let surface = reg.get(key(1)).unwrap();
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
        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        reg.apply_render("a", Uuid::nil(), key(1), text("a"), true, &terminals);
        let out = reg.apply_render("b", Uuid::nil(), key(1), text("b"), true, &terminals);
        assert_eq!(out, ApplyPanel::DeniedOccupied);
        assert_eq!(reg.get(key(1)).unwrap().plugin_id, "a");
    }

    #[test]
    fn same_plugin_live_different_instance_cannot_take_panel() {
        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        reg.apply_render(
            "demo",
            Uuid::from_u128(1),
            key(1),
            text("one"),
            true,
            &terminals,
        );
        let original = reg.get(key(1)).unwrap().clone();
        let out = reg.apply_render(
            "demo",
            Uuid::from_u128(2),
            key(1),
            text("two"),
            true,
            &terminals,
        );
        assert_eq!(out, ApplyPanel::DeniedOwnerInstance);
        assert_eq!(reg.get(key(1)), Some(&original));
    }

    #[test]
    fn death_marks_stale_without_dropping_the_tree() {
        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        reg.apply_render(
            "demo",
            Uuid::from_u128(10),
            key(1),
            text("keep"),
            true,
            &terminals,
        );
        let mut live = BTreeSet::new();
        live.insert(Uuid::from_u128(11));
        reg.mark_missing_stale(&live);
        let surface = reg.get(key(1)).unwrap();
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
            scene: None,
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
            scene: None,
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

    #[test]
    fn set_scene_requires_owner_and_camera_update_is_in_place() {
        use plugin_protocol::v2::{SceneBar, SceneCamera, SceneData};
        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        let owner = Uuid::from_u128(1);
        reg.apply_render("demo", owner, key(1), text("hi"), true, &terminals);
        let scene = SceneData {
            cols: 1,
            rows: 1,
            floor: [1, 2, 3],
            camera: SceneCamera {
                yaw: 0.1,
                pitch: 0.2,
                zoom: 1.0,
            },
            bars: vec![SceneBar {
                gx: 0,
                gz: 0,
                height: 1.0,
                color: [9, 9, 9],
                selected: true,
            }],
        };
        // A different plugin cannot write into this surface.
        assert!(!reg.set_scene(key(1), "other", owner, scene.clone()));
        assert!(!reg.set_scene(key(1), "demo", Uuid::from_u128(2), scene.clone()));
        assert!(reg.set_scene(key(1), "demo", owner, scene));
        assert!(reg.scene(key(1)).is_some());
        // Camera-only update keeps the geometry and just moves the view.
        let cam = SceneCamera {
            yaw: 1.0,
            pitch: 0.5,
            zoom: 2.0,
        };
        assert!(reg.set_scene_camera(key(1), cam));
        let updated_scene = reg.scene(key(1)).unwrap().clone();
        assert_eq!(updated_scene.camera, cam);
        assert_eq!(updated_scene.bars.len(), 1);
        // No scene on a fresh pane means no camera to move.
        reg.apply_render("demo", Uuid::nil(), key(2), text("x"), true, &terminals);
        assert!(!reg.set_scene_camera(key(2), cam));
        reg.mark_missing_stale(&BTreeSet::new());
        assert!(
            !reg.set_scene(key(1), "demo", owner, updated_scene),
            "stale surfaces must not accept a new scene"
        );
    }

    #[test]
    fn stale_reclaim_clears_old_scene() {
        use plugin_protocol::v2::{SceneBar, SceneCamera, SceneData};

        let mut reg = PanelRegistry::new();
        let terminals = BTreeSet::new();
        let first_owner = Uuid::from_u128(1);
        let second_owner = Uuid::from_u128(2);
        reg.apply_render("demo", first_owner, key(1), text("one"), true, &terminals);
        assert!(reg.set_scene(
            key(1),
            "demo",
            first_owner,
            SceneData {
                cols: 1,
                rows: 1,
                floor: [0, 0, 0],
                camera: SceneCamera {
                    yaw: 0.1,
                    pitch: 0.2,
                    zoom: 1.0,
                },
                bars: vec![SceneBar {
                    gx: 0,
                    gz: 0,
                    height: 1.0,
                    color: [1, 2, 3],
                    selected: false,
                }],
            }
        ));
        reg.mark_missing_stale(&BTreeSet::new());
        assert_eq!(
            reg.apply_render("demo", second_owner, key(1), text("two"), true, &terminals),
            ApplyPanel::Replace { pane_key: key(1) }
        );
        let surface = reg.get(key(1)).unwrap();
        assert_eq!(surface.owner_instance_id, second_owner);
        assert!(
            surface.scene.is_none(),
            "new owner must not inherit old scene"
        );
    }

    #[test]
    fn clone_and_insert_surfaces_preserve_surface_identity_and_scene() {
        use plugin_protocol::v2::{SceneBar, SceneCamera, SceneData};

        let mut source = PanelRegistry::new();
        let mut target = PanelRegistry::new();
        let terminals = BTreeSet::new();
        source.apply_render(
            "demo",
            Uuid::from_u128(1),
            key(1),
            text("one"),
            true,
            &terminals,
        );
        source.apply_render(
            "demo",
            Uuid::from_u128(2),
            key(2),
            text("two"),
            true,
            &terminals,
        );
        source.set_scene(
            key(1),
            "demo",
            Uuid::from_u128(1),
            SceneData {
                cols: 1,
                rows: 1,
                floor: [0, 0, 0],
                camera: SceneCamera {
                    yaw: 0.2,
                    pitch: 0.3,
                    zoom: 1.4,
                },
                bars: vec![SceneBar {
                    gx: 1,
                    gz: 2,
                    height: 3.0,
                    color: [4, 5, 6],
                    selected: false,
                }],
            },
        );
        let original = source.get(key(1)).expect("source surface").clone();

        let moved = source.clone_surfaces([key(1)]);
        assert_eq!(moved, vec![original.clone()]);

        target.insert_surfaces(moved);
        assert_eq!(target.get(key(1)), Some(&original));
        assert!(target.get(key(2)).is_none());
    }
}
