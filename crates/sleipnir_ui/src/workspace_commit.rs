//! Unified Workspace Mutation Pipeline.
//!
//! Every structural change to tabs/panes commits through this boundary,
//! guaranteeing that focus restoration, window title updates, tab scroll,
//! ledger focus sync, and UI notifications never get forgotten or
//! desynchronized across different operations.

use crate::app_shell::AppShell;
use gpui::{Context, Window};

#[cfg(test)]
mod workspace_regression_tests {
    use crate::pane_tree::SplitAxis;
    use crate::tab_convert::{ConvertTree, LayoutTree, Tab};

    fn split_tab() -> Tab<LayoutTree> {
        Tab {
            id: 1,
            tree: LayoutTree::split(
                SplitAxis::Horizontal,
                LayoutTree::leaf(10),
                LayoutTree::leaf(20),
            ),
            active_pane: 10,
            custom_title: None,
            zoomed_pane: Some(10),
        }
    }

    #[test]
    fn workspace_regression_zoom_follows_focus() {
        let mut tab = split_tab();
        tab.active_pane = 20;
        tab.reconcile_pane_focus();
        assert_eq!(tab.zoomed_pane, Some(20));
    }

    #[test]
    fn workspace_regression_zoom_follows_new_split() {
        let mut tab = split_tab();
        tab.tree = tab.tree.graft(LayoutTree::leaf(30));
        tab.active_pane = 30;
        tab.reconcile_pane_focus();
        assert_eq!(tab.zoomed_pane, Some(30));
    }

    #[test]
    fn workspace_regression_deleted_zoom_is_cleared_and_focus_repaired() {
        let mut tab = split_tab();
        tab.tree.extract_leaf(10).unwrap();
        tab.reconcile_pane_focus();
        assert_eq!(tab.active_pane, 20);
        assert_eq!(tab.zoomed_pane, None);
    }

    #[test]
    fn workspace_regression_unzoomed_focus_stays_unzoomed() {
        let mut tab = split_tab();
        tab.zoomed_pane = None;
        tab.active_pane = 99;
        tab.reconcile_pane_focus();
        assert_eq!(tab.active_pane, 10);
        assert_eq!(tab.zoomed_pane, None);
    }
}

impl AppShell {
    /// Canonical commit point for workspace mutations.
    ///
    /// Every effect here is idempotent and cheap, so there is no per-callsite
    /// opt-out: the window title is derived from the active *pane's* title, so
    /// even pane splits and intra-tab focus moves can change it.
    pub(crate) fn commit_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            tab.reconcile_pane_focus();
            let mut panes = Vec::new();
            tab.tree.leaves_with_keys(&mut panes);
            for (pane, view) in panes {
                view.update(cx, |view, _| view.bind_pane_key(pane));
            }
        }
        self.focus_active(window, cx);
        self.sync_ledger_focus(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        // The find bar searched the previously active pane; re-run it so the
        // count and highlights describe the pane that is on screen now.
        self.refresh_find_for_active_pane(cx);
        cx.notify();
    }
}
