//! Restoring the saved window on launch and writing it back as tabs and panes
//! change. The on-disk format itself lives in `crate::session`.
//!
//! A child module of `app_shell` so it can read and rebuild the shell's private
//! tab/pane state without widening it to the crate.

use gpui::{App, AppContext as _, Context, SharedString, Window};

use super::{AppShell, Tab, snapshot_tree, tree_contains};
use crate::pane_tree::{MIN_RATIO, PaneNode, SplitAxis};
use crate::plugin_panel::drop_session_panels;
use crate::session::{
    SessionAxis, SessionFile, SessionNode, SessionTab, load_session, resolve_cwd, restore_pane_key,
    sanitize_session, save_session, session_path,
};
use sleipnir_settings::TerminalSettings;

impl AppShell {
    pub(super) fn try_restore_session(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let path = session_path();
        let Some(raw) = load_session(&path) else {
            return false;
        };
        let Some(session) = sanitize_session(raw) else {
            return false;
        };
        self.restore_from_session(session, window, cx);
        !self.tabs.is_empty()
    }

    fn restore_from_session(
        &mut self,
        session: SessionFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.clear();
        let mut max_pane = 0u64;
        let mut max_tab = 0u64;
        for (i, stab) in session.tabs.into_iter().enumerate() {
            max_pane = max_pane.max(stab.tree.max_pane_id());
            let tab_id = (i as u64) + 1;
            max_tab = max_tab.max(tab_id);
            let Some(restored_tree) = drop_session_panels(stab.tree) else {
                continue;
            };
            let Some(tree) = self.materialize_tree(&restored_tree, window, cx) else {
                continue;
            };
            let active_pane = if tree_contains(&tree, stab.active_pane) {
                stab.active_pane
            } else {
                tree.first_leaf_id()
            };
            self.tabs.push(Tab {
                id: tab_id,
                tree,
                active_pane,
                custom_title: stab
                    .custom_title
                    .filter(|s| !s.is_empty())
                    .map(SharedString::from),
                zoomed_pane: None,
            });
        }
        self.next_id = max_tab + 1;
        self.next_pane_id = max_pane + 1;
        self.active = session.active_tab.min(self.tabs.len().saturating_sub(1));
        self.focus_active(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        cx.notify();
        log::info!(
            "restored session: {} tab(s), active={}",
            self.tabs.len(),
            self.active
        );
    }

    fn materialize_tree(
        &mut self,
        node: &SessionNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneNode> {
        match node {
            SessionNode::Leaf { id, cwd, pane_key } => {
                let view = self.spawn_term_view_with_cwd(resolve_cwd(cwd.as_deref()), window, cx);
                Some(PaneNode::leaf_with_key(
                    *id,
                    restore_pane_key(*pane_key),
                    view,
                ))
            }
            SessionNode::Panel { .. } => None,
            SessionNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.materialize_tree(first, window, cx);
                let second = self.materialize_tree(second, window, cx);
                match (first, second) {
                    (Some(a), Some(b)) => Some(PaneNode::Split {
                        axis: match axis {
                            SessionAxis::Horizontal => SplitAxis::Horizontal,
                            SessionAxis::Vertical => SplitAxis::Vertical,
                        },
                        ratio: (*ratio).clamp(MIN_RATIO, 1.0 - MIN_RATIO),
                        first: Box::new(a),
                        second: Box::new(b),
                    }),
                    (only, None) | (None, only) => only,
                }
            }
        }
    }

    fn snapshot_session(&self, cx: &App) -> SessionFile {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| SessionTab {
                custom_title: tab.custom_title.as_ref().map(|s| s.to_string()),
                active_pane: tab.active_pane,
                tree: snapshot_tree(&tab.tree, cx),
            })
            .collect();
        SessionFile {
            version: crate::session::SESSION_VERSION,
            active_tab: self.active,
            tabs,
        }
    }

    pub(super) fn persist_session_now(&self, cx: &App) {
        if !TerminalSettings::get_global(cx).restore_session {
            return;
        }
        let session = self.snapshot_session(cx);
        if session.tabs.is_empty() {
            return;
        }
        let path = session_path();
        if let Err(err) = save_session(&path, &session) {
            log::warn!("failed to save session to {}: {err}", path.display());
        } else {
            log::debug!(
                "session saved: {} tab(s) → {}",
                session.tabs.len(),
                path.display()
            );
        }
    }

    /// Mark layout dirty and write session after a short debounce so rapid
    /// tab switches don't thrash the disk.
    pub(crate) fn schedule_session_save(&mut self, cx: &mut Context<Self>) {
        // Cancel any pending save timer and start a fresh debounce.
        // Dropping the previous task cancels the prior write, so only the most
        // recent structural change is persisted.
        self._session_save_task = Some(cx.spawn(async move |this, cx| {
            // Yield a few times via background executor to defer the write
            // past any rapid burst of sequential calls.
            for _ in 0..3 {
                cx.background_spawn(std::future::ready(())).await;
            }
            this.update(cx, |this, cx| {
                this.persist_session_now(cx);
            })
            .ok();
        }));
    }
}
