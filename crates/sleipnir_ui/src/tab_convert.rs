//! Same-window tab ↔ pane layout transforms.
//!
//! Pure over an abstract tree so tests do not need GPUI or a PTY. The live
//! `PaneNode` implements the same trait; `AppShell` calls these functions.

use crate::pane_tree::{PaneId, PaneNode, SplitAxis};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertError {
    /// Source and destination are the same tab.
    SameTab,
    MissingTab,
    MissingPane,
    /// The pane is the last leaf in its tab.
    LastPane,
}

/// A tab as the convert routines see it.
#[derive(Clone, Debug, PartialEq)]
pub struct TabView<T> {
    pub id: u64,
    pub tree: T,
    pub active_pane: PaneId,
    pub custom_title: Option<String>,
    pub zoomed_pane: Option<PaneId>,
}

/// Tree surgery needed to merge or extract without creating new sessions.
pub trait ConvertTree: Sized {
    fn leaf_count(&self) -> usize;
    fn first_leaf_id(&self) -> PaneId;
    fn contains_leaf(&self, id: PaneId) -> bool;
    fn graft(self, incoming: Self) -> Self;
    fn extract_leaf(&mut self, id: PaneId) -> Result<Self, ConvertError>;
}

/// Identity-only tree for tests (and any caller that does not hold a `TermView`).
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutTree {
    Leaf(PaneId),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<LayoutTree>,
        second: Box<LayoutTree>,
    },
}

#[cfg(test)]
impl LayoutTree {
    pub fn leaf(id: PaneId) -> Self {
        LayoutTree::Leaf(id)
    }

    pub fn split(axis: SplitAxis, first: LayoutTree, second: LayoutTree) -> Self {
        LayoutTree::Split {
            axis,
            ratio: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn is_leaf(&self, target: PaneId) -> bool {
        matches!(self, LayoutTree::Leaf(id) if *id == target)
    }

    fn leaf_ids(&self) -> Vec<PaneId> {
        match self {
            LayoutTree::Leaf(id) => vec![*id],
            LayoutTree::Split { first, second, .. } => {
                let mut ids = first.leaf_ids();
                ids.extend(second.leaf_ids());
                ids
            }
        }
    }
}

#[cfg(test)]
impl ConvertTree for LayoutTree {
    fn leaf_count(&self) -> usize {
        match self {
            LayoutTree::Leaf(_) => 1,
            LayoutTree::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    fn first_leaf_id(&self) -> PaneId {
        match self {
            LayoutTree::Leaf(id) => *id,
            LayoutTree::Split { first, .. } => first.first_leaf_id(),
        }
    }

    fn contains_leaf(&self, id: PaneId) -> bool {
        match self {
            LayoutTree::Leaf(leaf) => *leaf == id,
            LayoutTree::Split { first, second, .. } => {
                first.contains_leaf(id) || second.contains_leaf(id)
            }
        }
    }

    fn graft(self, incoming: Self) -> Self {
        LayoutTree::split(SplitAxis::Horizontal, self, incoming)
    }

    fn extract_leaf(&mut self, id: PaneId) -> Result<Self, ConvertError> {
        if let LayoutTree::Leaf(leaf) = self {
            return if *leaf == id {
                Err(ConvertError::LastPane)
            } else {
                Err(ConvertError::MissingPane)
            };
        }
        let (first_is, second_is) = match self {
            LayoutTree::Split { first, second, .. } => (first.is_leaf(id), second.is_leaf(id)),
            LayoutTree::Leaf(_) => (false, false),
        };
        if first_is || second_is {
            return Ok(take_layout_child(self, first_is));
        }
        if let LayoutTree::Split { first, second, .. } = self {
            match first.extract_leaf(id) {
                Err(ConvertError::MissingPane) => second.extract_leaf(id),
                other => other,
            }
        } else {
            Err(ConvertError::MissingPane)
        }
    }
}

#[cfg(test)]
fn take_layout_child(node: &mut LayoutTree, take_first: bool) -> LayoutTree {
    match std::mem::replace(node, LayoutTree::Leaf(0)) {
        LayoutTree::Split { first, second, .. } => {
            let (keep, taken) = if take_first {
                (second, first)
            } else {
                (first, second)
            };
            *node = *keep;
            *taken
        }
        other => {
            *node = other;
            LayoutTree::Leaf(0)
        }
    }
}

impl ConvertTree for PaneNode {
    fn leaf_count(&self) -> usize {
        PaneNode::leaf_count(self)
    }

    fn first_leaf_id(&self) -> PaneId {
        PaneNode::first_leaf_id(self)
    }

    fn contains_leaf(&self, id: PaneId) -> bool {
        pane_contains(self, id)
    }

    fn graft(self, incoming: Self) -> Self {
        PaneNode::Split {
            axis: SplitAxis::Horizontal,
            ratio: 0.5,
            first: Box::new(self),
            second: Box::new(incoming),
        }
    }

    fn extract_leaf(&mut self, id: PaneId) -> Result<Self, ConvertError> {
        match PaneNode::take_leaf(self, id) {
            Ok(node) => Ok(node),
            Err(crate::pane_tree::TakeLeafError::LastPane) => Err(ConvertError::LastPane),
            Err(crate::pane_tree::TakeLeafError::NotFound) => Err(ConvertError::MissingPane),
        }
    }
}

fn pane_contains(node: &PaneNode, id: PaneId) -> bool {
    match node {
        PaneNode::Leaf { id: leaf, .. } => *leaf == id,
        PaneNode::Split { first, second, .. } => {
            pane_contains(first, id) || pane_contains(second, id)
        }
    }
}

/// Fold `source` into `dest` as a sibling subtree. `dest` stays; `source` is removed.
/// Returns the destination's index after the edit.
pub fn merge_tab<T: ConvertTree>(
    tabs: &mut Vec<TabView<T>>,
    source_id: u64,
    dest_id: u64,
) -> Result<usize, ConvertError> {
    if source_id == dest_id {
        return Err(ConvertError::SameTab);
    }
    let source_idx = tabs
        .iter()
        .position(|t| t.id == source_id)
        .ok_or(ConvertError::MissingTab)?;
    let dest_idx = tabs
        .iter()
        .position(|t| t.id == dest_id)
        .ok_or(ConvertError::MissingTab)?;
    let source = tabs.remove(source_idx);
    let dest_idx = if dest_idx > source_idx {
        dest_idx - 1
    } else {
        dest_idx
    };
    let mut dest = tabs.remove(dest_idx);
    dest.tree = dest.tree.graft(source.tree);
    dest.zoomed_pane = None;
    if !dest.tree.contains_leaf(dest.active_pane) {
        dest.active_pane = dest.tree.first_leaf_id();
    }
    tabs.insert(dest_idx, dest);
    Ok(dest_idx)
}

/// Pull `pane_id` out of its tab and insert it as a new tab at `insert_at`.
/// Returns the new tab's index. Refuses the last pane of a tab.
pub fn extract_pane<T: ConvertTree>(
    tabs: &mut Vec<TabView<T>>,
    pane_id: PaneId,
    insert_at: usize,
    new_tab_id: u64,
) -> Result<usize, ConvertError> {
    let source_idx = tabs
        .iter()
        .position(|t| t.tree.contains_leaf(pane_id))
        .ok_or(ConvertError::MissingPane)?;
    if tabs[source_idx].tree.leaf_count() <= 1 {
        return Err(ConvertError::LastPane);
    }
    let taken = tabs[source_idx].tree.extract_leaf(pane_id)?;
    if tabs[source_idx].active_pane == pane_id {
        tabs[source_idx].active_pane = tabs[source_idx].tree.first_leaf_id();
    }
    if tabs[source_idx].zoomed_pane == Some(pane_id) {
        tabs[source_idx].zoomed_pane = None;
    }
    let insert_at = insert_at.min(tabs.len());
    tabs.insert(
        insert_at,
        TabView {
            id: new_tab_id,
            tree: taken,
            active_pane: pane_id,
            custom_title: None,
            zoomed_pane: None,
        },
    );
    Ok(insert_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u64, tree: LayoutTree) -> TabView<LayoutTree> {
        let active = tree.first_leaf_id();
        TabView {
            id,
            tree,
            active_pane: active,
            custom_title: None,
            zoomed_pane: None,
        }
    }

    #[test]
    fn merge_two_single_pane_tabs() {
        let mut tabs = vec![tab(1, LayoutTree::leaf(10)), tab(2, LayoutTree::leaf(20))];
        let dest = merge_tab(&mut tabs, 2, 1).expect("merge");
        assert_eq!(dest, 0);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].id, 1);
        assert_eq!(tabs[0].tree.leaf_ids(), vec![10, 20]);
        assert_eq!(tabs[0].active_pane, 10);
    }

    #[test]
    fn merge_split_tab_lands_as_subtree() {
        let split = LayoutTree::split(
            SplitAxis::Vertical,
            LayoutTree::leaf(21),
            LayoutTree::leaf(22),
        );
        let mut tabs = vec![tab(1, LayoutTree::leaf(10)), tab(2, split)];
        merge_tab(&mut tabs, 2, 1).unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].tree.leaf_ids(), vec![10, 21, 22]);
        match &tabs[0].tree {
            LayoutTree::Split { second, .. } => {
                assert_eq!(second.leaf_ids(), vec![21, 22]);
            }
            LayoutTree::Leaf(_) => panic!("dest should be a split holding the incoming tree"),
        }
    }

    #[test]
    fn extract_leaf_to_chosen_index() {
        let split = LayoutTree::split(
            SplitAxis::Horizontal,
            LayoutTree::leaf(10),
            LayoutTree::leaf(11),
        );
        let mut tabs = vec![tab(1, split), tab(2, LayoutTree::leaf(20))];
        let at = extract_pane(&mut tabs, 11, 1, 3).expect("extract");
        assert_eq!(at, 1);
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[0].id, 1);
        assert_eq!(tabs[0].tree.leaf_ids(), vec![10]);
        assert_eq!(tabs[1].id, 3);
        assert_eq!(tabs[1].tree.leaf_ids(), vec![11]);
        assert_eq!(tabs[1].active_pane, 11);
        assert_eq!(tabs[2].id, 2);
        assert_eq!(tabs[2].tree.leaf_ids(), vec![20]);
    }

    #[test]
    fn extract_rejects_last_pane() {
        let mut tabs = vec![tab(1, LayoutTree::leaf(10))];
        assert_eq!(
            extract_pane(&mut tabs, 10, 0, 2),
            Err(ConvertError::LastPane)
        );
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].tree.leaf_ids(), vec![10]);
    }

    #[test]
    fn merge_rejects_same_tab() {
        let mut tabs = vec![tab(1, LayoutTree::leaf(10))];
        assert_eq!(merge_tab(&mut tabs, 1, 1), Err(ConvertError::SameTab));
    }

    #[test]
    fn extract_missing_pane() {
        let mut tabs = vec![tab(1, LayoutTree::leaf(10))];
        assert_eq!(
            extract_pane(&mut tabs, 99, 0, 2),
            Err(ConvertError::MissingPane)
        );
    }
}
