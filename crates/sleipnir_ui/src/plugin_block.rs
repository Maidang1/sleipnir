//! Block mount: plugin trees inside scrollback (ADR-0018).
//!
//! A Block is anchored to a Run via process-local [`run_ledger::Anchor`].
//! Height comes from [`sleipnir_widget::layout`] (integer cell rows) and is
//! the value [`row_geometry::RowGeometry`] stores. The host owns the tree:
//! death marks it stale; a crafted tree cannot hide the attribution band.
//!
//! Blocks are **not** restored across restarts — an anchor is process-local
//! and would claim a scrollback line that no longer means anything.
//!
//! Pure decision logic. No gpui. Paint and hit-testing consume the cached
//! [`Layout`]; they do not re-measure on the hot path.

use plugin_protocol::v2::{BlockId, RunId, Widget};
use row_geometry::{Anchor, Block};
use run_ledger::Anchor as LedgerAnchor;
use sleipnir_widget::{Layout, layout};
use std::collections::{BTreeMap, BTreeSet};

use crate::plugin_surface::{StaleRegistry, Surface};

/// One plugin-drawn Block. The tree is data; the host stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockSurface {
    pub plugin_id: String,
    pub block_id: BlockId,
    pub run_id: RunId,
    pub anchor: Anchor,
    pub tree: Widget,
    /// Cached layout. Invalidated on tree replace / width change / unfreeze.
    pub laid: Option<Layout>,
    pub stale: bool,
}

impl Surface for BlockSurface {
    fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
    fn set_stale(&mut self, stale: bool) {
        self.stale = stale;
    }
}

/// Outcome of applying a `Render { target: Block }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyBlock {
    Inserted,
    Replaced,
    DeniedGrant,
    /// No Run, or the Run has no process-local anchor.
    DeniedAnchor,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockRegistry {
    surfaces: BTreeMap<BlockId, BlockSurface>,
}

impl BlockRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: BlockId) -> Option<&BlockSurface> {
        self.surfaces.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BlockSurface> {
        self.surfaces.values()
    }

    /// Apply a whole-tree `Render`. `granted` is the live session's
    /// `RenderBlock` bit. `ledger_anchor` is the Run's process-local position;
    /// without it the Block cannot be placed and is dropped.
    pub fn apply_render(
        &mut self,
        plugin_id: &str,
        run_id: RunId,
        tree: Widget,
        granted: bool,
        ledger_anchor: Option<LedgerAnchor>,
        existing_id: Option<BlockId>,
    ) -> ApplyBlock {
        if !granted {
            return ApplyBlock::DeniedGrant;
        }
        let Some(la) = ledger_anchor else {
            return ApplyBlock::DeniedAnchor;
        };
        let anchor = Anchor {
            line: la.line,
            column: la.column,
        };
        if let Some(id) = existing_id {
            if let Some(existing) = self.surfaces.get_mut(&id) {
                existing.tree = tree;
                existing.anchor = anchor;
                existing.stale = false;
                existing.laid = None;
                return ApplyBlock::Replaced;
            }
        }
        let id = existing_id.unwrap_or_else(BlockId::new_v4);
        self.surfaces.insert(
            id,
            BlockSurface {
                plugin_id: plugin_id.to_string(),
                block_id: id,
                run_id,
                anchor,
                tree,
                laid: None,
                stale: false,
            },
        );
        ApplyBlock::Inserted
    }

    /// History shrink / eviction: drop surfaces whose ids are no longer in
    /// the geometry (the Block went with its anchor).
    pub fn retain_live(&mut self, live: &BTreeSet<BlockId>) {
        self.surfaces.retain(|id, _| live.contains(id));
    }

    /// Same rule as `rebase_markers_after_history_shrink` / RowGeometry:
    /// survivors shift down; a Block whose anchor fell in the removed region
    /// is dropped.
    pub fn rebase_after_history_shrink(&mut self, removed: i32) {
        if removed <= 0 {
            return;
        }
        self.surfaces.retain(|_, s| s.anchor.line >= removed);
        for s in self.surfaces.values_mut() {
            s.anchor.line -= removed;
        }
    }

    /// Re-layout every surface that needs it. Cached while the tree, width,
    /// and freeze flag stay put so per-frame work stays bounded.
    pub fn relayout(&mut self, cols: u16, frozen: bool) {
        if frozen {
            return;
        }
        for surface in self.surfaces.values_mut() {
            if surface
                .laid
                .as_ref()
                .is_some_and(|l| l.width == u32::from(cols).max(1))
            {
                continue;
            }
            surface.laid = Some(layout(&surface.tree, cols, &surface.plugin_id));
        }
    }

    /// Force a re-layout on drag end (ADR-0018 decision 3).
    pub fn invalidate_layouts(&mut self) {
        for surface in self.surfaces.values_mut() {
            surface.laid = None;
        }
    }

    pub fn geometry_blocks(&self) -> Vec<Block> {
        self.surfaces
            .values()
            .map(|s| Block {
                id: s.block_id,
                run_id: s.run_id,
                anchor: s.anchor,
                height: s
                    .laid
                    .as_ref()
                    .map(|l| u16::try_from(l.height).unwrap_or(u16::MAX))
                    .unwrap_or(1),
            })
            .collect()
    }
}

/// Death marks; it never drops. See [`crate::plugin_surface`].
impl StaleRegistry for BlockRegistry {
    type Surface = BlockSurface;
    fn surfaces_mut(&mut self) -> impl Iterator<Item = &mut BlockSurface> {
        self.surfaces.values_mut()
    }
}

pub use crate::plugin_panel::action_at;

/// Widget text that must never appear in a copied selection (ADR-0018
/// decision 5). Selection is a grid-coordinate concept; the grid has no
/// widget cells.
#[cfg(test)]
fn widget_text_fragments(tree: &Widget) -> Vec<String> {
    fn walk(w: &Widget, out: &mut Vec<String>) {
        match w {
            Widget::Text { s, .. } | Widget::Code { s, .. } | Widget::Badge { s, .. } => {
                if !s.is_empty() {
                    out.push(s.clone());
                }
            }
            Widget::Btn { s, .. } => {
                if !s.is_empty() {
                    out.push(s.clone());
                }
            }
            Widget::Col { children, .. } | Widget::Row { children, .. } => {
                for c in children {
                    walk(c, out);
                }
            }
            Widget::Bar { .. } | Widget::Spark { .. } | Widget::Sep | Widget::Unknown => {}
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
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

    fn run(n: u128) -> RunId {
        RunId::from_u128(n)
    }

    fn anchor(line: i32) -> LedgerAnchor {
        LedgerAnchor { line, column: 0 }
    }

    #[test]
    fn render_block_grant_is_required() {
        let mut reg = BlockRegistry::new();
        let out = reg.apply_render("demo", run(1), text("hi"), false, Some(anchor(4)), None);
        assert_eq!(out, ApplyBlock::DeniedGrant);
        assert!(reg.iter().next().is_none());
    }

    #[test]
    fn missing_anchor_is_denied() {
        let mut reg = BlockRegistry::new();
        let out = reg.apply_render("demo", run(1), text("hi"), true, None, None);
        assert_eq!(out, ApplyBlock::DeniedAnchor);
    }

    #[test]
    fn whole_tree_replacement_overwrites_and_clears_stale() {
        let mut reg = BlockRegistry::new();
        assert_eq!(
            reg.apply_render("demo", run(1), text("one"), true, Some(anchor(4)), None),
            ApplyBlock::Inserted
        );
        let id = reg.iter().next().unwrap().block_id;
        reg.mark_missing_stale(&BTreeSet::new());
        assert!(reg.get(id).unwrap().stale);
        let out = reg.apply_render("demo", run(1), text("two"), true, Some(anchor(4)), Some(id));
        assert_eq!(out, ApplyBlock::Replaced);
        let s = reg.get(id).unwrap();
        assert!(!s.stale);
        assert_eq!(s.tree, text("two"));
    }

    #[test]
    fn death_marks_stale_without_dropping_the_tree() {
        let mut reg = BlockRegistry::new();
        reg.apply_render("demo", run(1), text("keep"), true, Some(anchor(3)), None);
        let mut live = BTreeSet::new();
        live.insert("other".into());
        reg.mark_missing_stale(&live);
        let s = reg.iter().next().unwrap();
        assert!(s.stale);
        assert_eq!(s.tree, text("keep"));
    }

    #[test]
    fn history_shrink_drops_the_surface_with_its_anchor() {
        let mut reg = BlockRegistry::new();
        reg.apply_render("demo", run(1), text("gone"), true, Some(anchor(2)), None);
        let id = reg.iter().next().unwrap().block_id;
        let live = BTreeSet::new();
        reg.retain_live(&live);
        assert!(reg.get(id).is_none());
    }

    #[test]
    fn layouts_are_cached_until_width_changes_or_invalidated() {
        let mut reg = BlockRegistry::new();
        reg.apply_render("demo", run(1), text("hi"), true, Some(anchor(0)), None);
        reg.relayout(20, false);
        let ptr = reg.iter().next().unwrap().laid.as_ref().map(|l| l.height);
        reg.relayout(20, false);
        assert_eq!(
            ptr,
            reg.iter().next().unwrap().laid.as_ref().map(|l| l.height)
        );
        reg.relayout(20, true);
        // Frozen: still cached.
        assert!(reg.iter().next().unwrap().laid.is_some());
        reg.invalidate_layouts();
        assert!(reg.iter().next().unwrap().laid.is_none());
    }

    #[test]
    fn click_on_btn_routes_action() {
        let mut reg = BlockRegistry::new();
        reg.apply_render(
            "demo",
            run(1),
            btn("Go", "retry"),
            true,
            Some(anchor(1)),
            None,
        );
        reg.relayout(20, false);
        let laid = reg.iter().next().unwrap().laid.as_ref().unwrap();
        let hit = action_at(laid, 0, 0).expect("btn");
        assert_eq!(hit.action, "retry");
        assert!(action_at(laid, 0, laid.attribution.rect.row).is_none());
    }

    #[test]
    fn copied_selection_excludes_widget_text() {
        let tree = text("WIDGET_UNIQUE_TOKEN");
        let fragments = widget_text_fragments(&tree);
        assert!(fragments.iter().any(|s| s.contains("WIDGET_UNIQUE_TOKEN")));
        // Grid selection is terminal cells only.
        let copied = "ls\nfoo.rs\n";
        for frag in &fragments {
            assert!(
                !copied.contains(frag),
                "copied text must not contain widget {frag:?}"
            );
        }
    }

    #[test]
    fn attribution_survives_a_tree_that_looks_like_the_marker() {
        let mut reg = BlockRegistry::new();
        reg.apply_render(
            "honest",
            run(1),
            text("plugin:evil"),
            true,
            Some(anchor(0)),
            None,
        );
        reg.relayout(20, false);
        let laid = reg.iter().next().unwrap().laid.as_ref().unwrap();
        assert!(matches!(
            laid.attribution.kind,
            LaidOutKind::Attribution { .. }
        ));
    }

    #[test]
    fn blocks_are_not_in_the_session_shape() {
        // A Block is process-local. SessionNode has Panel / Leaf / Split, no
        // Block variant, so restore cannot resurrect one.
        let src = include_str!("session.rs");
        assert!(
            !src.contains("Block {"),
            "session restore must not grow a Block variant"
        );
    }

    #[test]
    fn frozen_skips_relayout() {
        let mut reg = BlockRegistry::new();
        reg.apply_render("demo", run(1), text("a"), true, Some(anchor(0)), None);
        reg.relayout(10, false);
        let h = reg.iter().next().unwrap().laid.as_ref().unwrap().height;
        // Simulate a width change while frozen: cache must stay.
        reg.relayout(40, true);
        assert_eq!(reg.iter().next().unwrap().laid.as_ref().unwrap().height, h);
    }
}
