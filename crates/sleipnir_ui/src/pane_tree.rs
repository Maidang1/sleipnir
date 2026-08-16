//! Recursive pane tree for split terminals (ADR-0001).
//!
//! A `Tab`'s content is a `PaneNode`: interior nodes are `Split`s (an axis + a
//! ratio) and leaves are `Pane`s (one `TermView` = one PTY). A brand-new Tab is
//! a single `Leaf`, so the no-split case is the degenerate tree.

use gpui::{Bounds, Entity, Pixels};
use uuid::Uuid;

use crate::TermView;

/// Stable identity for a leaf pane within a tab.
pub type PaneId = u64;
/// Stable identity for a Pane across restarts; persisted in `session.json`.
pub type PaneKey = Uuid;

/// Split orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// Children are placed left | right (a vertical divider). `⌘D`.
    Horizontal,
    /// Children are placed top / bottom (a horizontal divider). `⌘⇧D`.
    Vertical,
}

/// Focus-movement direction for `⌘⌥`+arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// A node in a tab's pane tree.
#[derive(Clone)]
pub enum PaneNode {
    Leaf {
        id: PaneId,
        pane_key: PaneKey,
        view: Entity<TermView>,
    },
    Split {
        axis: SplitAxis,
        /// Fraction of the space given to `first` (0.0..1.0).
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

/// Minimum fraction a child of a split may shrink to, so a pane never vanishes
/// under a divider drag.
pub const MIN_RATIO: f32 = 0.1;

impl PaneNode {
    pub fn leaf(id: PaneId, view: Entity<TermView>) -> Self {
        Self::leaf_with_key(id, Uuid::new_v4(), view)
    }

    pub fn leaf_with_key(id: PaneId, pane_key: PaneKey, view: Entity<TermView>) -> Self {
        PaneNode::Leaf { id, pane_key, view }
    }

    /// Number of leaf panes in this subtree.
    pub fn leaf_count(&self) -> usize {
        match self {
            PaneNode::Leaf { .. } => 1,
            PaneNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Collect `(PaneId, &Entity<TermView>)` for every leaf, in tree order.
    pub fn leaves<'a>(&'a self, out: &mut Vec<(PaneId, &'a Entity<TermView>)>) {
        match self {
            PaneNode::Leaf { id, view, .. } => out.push((*id, view)),
            PaneNode::Split { first, second, .. } => {
                first.leaves(out);
                second.leaves(out);
            }
        }
    }

    pub fn pane_key_for_view(&self, view: &Entity<TermView>) -> Option<PaneKey> {
        match self {
            PaneNode::Leaf { pane_key, view: leaf, .. } if leaf == view => Some(*pane_key),
            PaneNode::Split { first, second, .. } => first
                .pane_key_for_view(view)
                .or_else(|| second.pane_key_for_view(view)),
            PaneNode::Leaf { .. } => None,
        }
    }

    pub fn pane_id_for_key(&self, key: PaneKey) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { id, pane_key, .. } if *pane_key == key => Some(*id),
            PaneNode::Split { first, second, .. } => first
                .pane_id_for_key(key)
                .or_else(|| second.pane_id_for_key(key)),
            PaneNode::Leaf { .. } => None,
        }
    }

    pub fn pane_key_for_id(&self, id: PaneId) -> Option<PaneKey> {
        match self {
            PaneNode::Leaf { id: leaf, pane_key, .. } if *leaf == id => Some(*pane_key),
            PaneNode::Split { first, second, .. } => first
                .pane_key_for_id(id)
                .or_else(|| second.pane_key_for_id(id)),
            PaneNode::Leaf { .. } => None,
        }
    }

    pub fn all_pane_keys(&self) -> Vec<PaneKey> {
        match self {
            PaneNode::Leaf { pane_key, .. } => vec![*pane_key],
            PaneNode::Split { first, second, .. } => {
                let mut keys = first.all_pane_keys();
                keys.extend(second.all_pane_keys());
                keys
            }
        }
    }

    /// Collect `(PaneKey, TermView)` for every leaf, in tree order.
    pub fn leaves_with_keys(&self, out: &mut Vec<(PaneKey, Entity<TermView>)>) {
        match self {
            PaneNode::Leaf { pane_key, view, .. } => out.push((*pane_key, view.clone())),
            PaneNode::Split { first, second, .. } => {
                first.leaves_with_keys(out);
                second.leaves_with_keys(out);
            }
        }
    }

    /// The first leaf's id (used to pick a fallback active pane).
    pub fn first_leaf_id(&self) -> PaneId {
        match self {
            PaneNode::Leaf { id, .. } => *id,
            PaneNode::Split { first, .. } => first.first_leaf_id(),
        }
    }

    /// Whether this subtree contains a leaf with `id`.
    pub fn contains_leaf(&self, id: PaneId) -> bool {
        match self {
            PaneNode::Leaf { id: leaf, .. } => *leaf == id,
            PaneNode::Split { first, second, .. } => {
                first.contains_leaf(id) || second.contains_leaf(id)
            }
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        self.contains_leaf(id)
    }

    /// Pull `target` out of this tree, collapsing its parent split.
    /// The leaf's `TermView` handle is cloned (same entity, no new PTY).
    pub fn take_leaf(&mut self, target: PaneId) -> Result<PaneNode, TakeLeafError> {
        if let PaneNode::Leaf { id, .. } = self {
            return if *id == target {
                Err(TakeLeafError::LastPane)
            } else {
                Err(TakeLeafError::NotFound)
            };
        }

        let (first_is_target, second_is_target) = match self {
            PaneNode::Split { first, second, .. } => {
                (first.is_leaf(target), second.is_leaf(target))
            }
            PaneNode::Leaf { .. } => (false, false),
        };

        if first_is_target || second_is_target {
            let taken = match self {
                PaneNode::Split { first, second, .. } => {
                    if first_is_target {
                        first.as_ref().clone()
                    } else {
                        second.as_ref().clone()
                    }
                }
                PaneNode::Leaf { .. } => return Err(TakeLeafError::NotFound),
            };
            take_and_collapse(self, first_is_target);
            return Ok(taken);
        }

        if let PaneNode::Split { first, second, .. } = self {
            match first.take_leaf(target) {
                Err(TakeLeafError::NotFound) => second.take_leaf(target),
                other => other,
            }
        } else {
            Err(TakeLeafError::NotFound)
        }
    }

    /// Split the leaf `target` into two along `axis`, giving the new pane
    /// (built by `make`) the second half. Returns the new pane's id, or `None`
    /// if `target` was not found.
    pub fn split(
        &mut self,
        target: PaneId,
        axis: SplitAxis,
        new_id: PaneId,
        new_view: Entity<TermView>,
    ) -> bool {
        match self {
            PaneNode::Leaf { id, pane_key, view } if *id == target => {
                // Rebuild this leaf as a split: first = old leaf, second = new.
                // Keep the original pane_key so Run Ledger ownership stays put.
                let old = PaneNode::leaf_with_key(*id, *pane_key, view.clone());
                *self = PaneNode::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(old),
                    second: Box::new(PaneNode::leaf(new_id, new_view)),
                };
                true
            }
            PaneNode::Leaf { .. } => false,
            PaneNode::Split { first, second, .. } => {
                if first.contains(target) {
                    first.split(target, axis, new_id, new_view)
                } else {
                    second.split(target, axis, new_id, new_view)
                }
            }
        }
    }

    /// Close the leaf `target`. Collapses its parent split into the sibling.
    /// Returns `CloseOutcome`.
    pub fn close(&mut self, target: PaneId) -> CloseOutcome {
        // If the whole tree is just this leaf, the tab should close.
        if let PaneNode::Leaf { id, .. } = self {
            return if *id == target {
                CloseOutcome::TreeEmpty
            } else {
                CloseOutcome::NotFound
            };
        }

        // We're a split: if a direct child is the target leaf, collapse into
        // the surviving sibling by moving the split apart by value.
        let (first_is_target, second_is_target) = match self {
            PaneNode::Split { first, second, .. } => {
                (first.is_leaf(target), second.is_leaf(target))
            }
            PaneNode::Leaf { .. } => (false, false),
        };

        if first_is_target || second_is_target {
            // Take the split apart by value; keep the surviving child.
            take_and_collapse(self, second_is_target);
            return CloseOutcome::Closed;
        }

        // Neither direct child is the target: recurse.
        if let PaneNode::Split { first, second, .. } = self {
            match first.close(target) {
                CloseOutcome::NotFound => second.close(target),
                other => other,
            }
        } else {
            CloseOutcome::NotFound
        }
    }

    fn is_leaf(&self, target: PaneId) -> bool {
        matches!(self, PaneNode::Leaf { id, .. } if *id == target)
    }

    /// Set the ratio of the split addressed by `path` (root-to-split).
    pub fn set_ratio(&mut self, path: &SplitPath, ratio: f32) {
        let clamped = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        let mut node: &mut PaneNode = self;
        for step in &path.0 {
            match node {
                PaneNode::Split { first, second, .. } => {
                    node = match step {
                        Branch::First => first.as_mut(),
                        Branch::Second => second.as_mut(),
                    };
                }
                PaneNode::Leaf { .. } => return,
            }
        }
        if let PaneNode::Split { ratio: r, .. } = node {
            *r = clamped;
        }
    }
}

/// Collapse a `Split` node in place, keeping one child. `keep_second` selects
/// which child survives. The survivor is cloned (an `Entity` clone is a cheap
/// handle copy) and installed in place of the split.
fn take_and_collapse(node: &mut PaneNode, keep_second: bool) {
    if let PaneNode::Split { first, second, .. } = node {
        let survivor = if keep_second {
            second.as_ref().clone()
        } else {
            first.as_ref().clone()
        };
        *node = survivor;
    }
}

/// Result of trying to lift a leaf out of a tree without closing the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeLeafError {
    /// The pane is the only leaf; extracting it would empty the tab.
    LastPane,
    /// No pane with that id exists in this tree.
    NotFound,
}

/// Result of closing a pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The pane was closed and its split collapsed; tab stays open.
    Closed,
    /// The pane was the only one in the tab; the tab should close.
    TreeEmpty,
    /// No pane with that id exists in this tree.
    NotFound,
}

/// Which child of a split to descend into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    First,
    Second,
}

/// Address of a specific split within a tree (root-to-split path).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitPath(pub Vec<Branch>);

impl SplitPath {
    pub fn new() -> Self {
        SplitPath(Vec::new())
    }

    /// A child path extended by one step.
    pub fn child(&self, step: Branch) -> Self {
        let mut v = self.0.clone();
        v.push(step);
        SplitPath(v)
    }
}

/// A rendered pane's screen rectangle, recorded during layout so keyboard
/// neighbor-navigation can pick the nearest pane in a direction.
#[derive(Clone, Copy, Debug)]
pub struct PaneRect {
    pub id: PaneId,
    pub bounds: Bounds<Pixels>,
}

/// Pick the pane nearest `from` in `direction`, from a set of laid-out rects.
/// Returns the chosen `PaneId`, or `None` if there is no pane that way.
pub fn neighbor(rects: &[PaneRect], from: PaneId, direction: Direction) -> Option<PaneId> {
    let origin = rects.iter().find(|r| r.id == from)?;
    let oc = center(&origin.bounds);

    rects
        .iter()
        .filter(|r| r.id != from)
        .filter(|r| in_direction(oc, center(&r.bounds), direction))
        .min_by(|a, b| {
            let da = directional_distance(oc, center(&a.bounds), direction);
            let db = directional_distance(oc, center(&b.bounds), direction);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|r| r.id)
}

fn center(b: &Bounds<Pixels>) -> (f32, f32) {
    (
        f32::from(b.origin.x) + f32::from(b.size.width) / 2.0,
        f32::from(b.origin.y) + f32::from(b.size.height) / 2.0,
    )
}

fn in_direction(from: (f32, f32), to: (f32, f32), dir: Direction) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    match dir {
        Direction::Left => dx < -1.0,
        Direction::Right => dx > 1.0,
        Direction::Up => dy < -1.0,
        Direction::Down => dy > 1.0,
    }
}

/// Distance metric that prefers panes aligned along the travel axis: the
/// primary-axis gap dominates, with the cross-axis offset as a tiebreaker.
fn directional_distance(from: (f32, f32), to: (f32, f32), dir: Direction) -> f32 {
    let (dx, dy) = ((to.0 - from.0).abs(), (to.1 - from.1).abs());
    match dir {
        Direction::Left | Direction::Right => dx + dy * 2.0,
        Direction::Up | Direction::Down => dy + dx * 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(id: PaneId, x: f32, y: f32, w: f32, h: f32) -> PaneRect {
        PaneRect {
            id,
            bounds: Bounds::new(
                gpui::point(gpui::px(x), gpui::px(y)),
                gpui::size(gpui::px(w), gpui::px(h)),
            ),
        }
    }

    #[test]
    fn neighbor_right_and_left() {
        // A | B  (A left, B right)
        let rects = [rect(1, 0.0, 0.0, 100.0, 100.0), rect(2, 100.0, 0.0, 100.0, 100.0)];
        assert_eq!(neighbor(&rects, 1, Direction::Right), Some(2));
        assert_eq!(neighbor(&rects, 2, Direction::Left), Some(1));
        assert_eq!(neighbor(&rects, 1, Direction::Left), None);
        assert_eq!(neighbor(&rects, 2, Direction::Right), None);
    }

    #[test]
    fn neighbor_prefers_aligned_pane() {
        // A on the left; B1 top-right, B2 bottom-right.
        let rects = [
            rect(1, 0.0, 0.0, 100.0, 200.0),
            rect(2, 100.0, 0.0, 100.0, 100.0),
            rect(3, 100.0, 100.0, 100.0, 100.0),
        ];
        // From A moving right, the vertically-nearer of B1/B2 wins; A's center
        // is at y=100 which is the boundary, both equidistant -> deterministic
        // pick of the first minimum.
        assert!(matches!(neighbor(&rects, 1, Direction::Right), Some(2) | Some(3)));
        // From B2 moving up -> B1.
        assert_eq!(neighbor(&rects, 3, Direction::Up), Some(2));
    }
}
