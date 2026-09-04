//! Tab lifecycle: adding, closing, activating and reordering tabs, the inline
//! rename, and moving tabs between windows.
//!
//! A child module of `app_shell` so it can mutate the shell's private tab list
//! and rename state without widening them to the crate.

use gpui::{Context, Entity, Focusable as _, SharedString, Window};
use std::path::PathBuf;

use super::{
    AppShell, CloseConfirmState, ClosedTab, ConfirmKind, RenameState, Tab,
    open_sleipnir_window_with_tab, rebase_detached_tab, reorder_insert_index,
};
use crate::TermView;
use crate::chrome::active_after_close;
use crate::pane_tree::{PaneId, PaneNode};
use crate::tab_convert::{extract_pane, merge_tab};

const CLOSED_TAB_HISTORY_LIMIT: usize = 10;

fn push_closed_tab(history: &mut Vec<ClosedTab>, closed: ClosedTab) {
    if history.len() == CLOSED_TAB_HISTORY_LIMIT {
        history.remove(0);
    }
    history.push(closed);
}

impl AppShell {
    pub(crate) fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self
            .active_working_directory(cx)
            .map(|cwd| crate::chrome::workspace::spawn_cwd(&cwd));
        self.add_tab_at(cwd, window, cx);
    }

    pub(crate) fn add_tab_at(
        &mut self,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let view = self.spawn_term_view_with_cwd(cwd, window, cx);
        self.tabs.push(Tab {
            id,
            tree: PaneNode::leaf(pane_id, view),
            active_pane: pane_id,
            custom_title: None,
            zoomed_pane: None,
        });
        self.active = self.tabs.len() - 1;
        self.commit_workspace(window, cx);
    }

    /// Begin an inline rename for the given tab, seeding the editable buffer
    /// with the text currently shown on the chip.
    pub(crate) fn begin_rename(&mut self, tab_id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let buffer = tab.path_label(cx).to_string();
        self.rename = Some(RenameState { tab_id, buffer });
        cx.notify();
    }

    /// Commit the in-progress rename to the target tab. An empty buffer clears
    /// the custom title so the tab falls back to the pane title (side) or cwd
    /// path (top).
    pub(super) fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(state) = self.rename.take() {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == state.tab_id) {
                let trimmed = state.buffer.trim();
                tab.custom_title = if trimmed.is_empty() {
                    None
                } else {
                    Some(SharedString::from(trimmed.to_string()))
                };
            }
            self.sync_window_title(window, cx);
            self.schedule_session_save(cx);
            cx.notify();
        }
    }

    /// Abandon the in-progress rename without changing the tab title.
    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            cx.notify();
        }
    }

    /// Handle a keystroke while an inline rename is active. Returns true if the
    /// keystroke was consumed by the rename editor.
    pub(super) fn rename_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.rename.is_none() {
            return false;
        }
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                self.commit_rename(window, cx);
                true
            }
            "escape" => {
                self.cancel_rename(cx);
                true
            }
            "backspace" => {
                if let Some(state) = self.rename.as_mut() {
                    state.buffer.pop();
                    cx.notify();
                }
                true
            }
            _ => {
                // Append any typed printable character to the buffer.
                if let Some(ch) = event.keystroke.key_char.as_ref() {
                    if !ch.is_empty() && !ch.chars().any(|c| c.is_control()) {
                        if let Some(state) = self.rename.as_mut() {
                            state.buffer.push_str(ch);
                            cx.notify();
                        }
                    }
                }
                // While renaming, swallow every other key too so shortcuts
                // (e.g. ⌘W, ⌘T) and stray terminal input don't fire mid-edit.
                true
            }
        }
    }

    pub(crate) fn request_close_tab(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_confirm.is_some() {
            return;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let mut leaves = Vec::new();
        self.tabs[index].tree.leaves(&mut leaves);
        let first_busy = leaves
            .into_iter()
            .map(|(_, view)| view)
            .find(|view| view.read(cx).looks_busy(cx));
        let policy = sleipnir_settings::TerminalSettings::get_global(cx).confirm_close;
        let needs_confirm = match policy {
            sleipnir_settings::ConfirmClose::Never => false,
            sleipnir_settings::ConfirmClose::Always => true,
            sleipnir_settings::ConfirmClose::Dirty => first_busy.is_some(),
        };
        if needs_confirm {
            let name =
                first_busy.and_then(|view| view.read(cx).foreground_process_command_name(cx));
            self.close_confirm = Some(CloseConfirmState {
                message: crate::chrome::close_copy::close_confirm_message(name.as_deref()).into(),
                kind: ConfirmKind::CloseTab(tab_id),
            });
            cx.notify();
        } else {
            self.close_tab_at(index, window, cx);
        }
    }

    pub(crate) fn close_tab_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        push_closed_tab(
            &mut self.closed_tabs,
            ClosedTab {
                cwd: self.tabs[index].workspace_cwd(cx),
                title: self.tabs[index].custom_title.clone(),
            },
        );
        // Drop any inline rename targeting the tab being removed.
        if let Some(state) = self.rename.as_ref() {
            if self.tabs[index].id == state.tab_id {
                self.rename = None;
            }
        }
        let closed_keys = self.tabs[index].tree.all_pane_keys();
        self.plugin_panels.remove_all(closed_keys.iter().copied());
        for pane in closed_keys {
            self.apply_pane_closed(pane, cx);
        }
        match active_after_close(self.active, index, self.tabs.len()) {
            None => {
                self.tabs.remove(index);
                // Always keep at least one tab.
                self.add_tab(window, cx);
            }
            Some(new_active) => {
                self.tabs.remove(index);
                self.active = new_active;
                self.commit_workspace(window, cx);
            }
        }
    }

    pub(super) fn close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = self.active.min(self.tabs.len() - 1);
        self.close_tab_at(idx, window, cx);
    }

    pub(crate) fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.closed_tabs.pop() else {
            return;
        };
        self.add_tab_at(closed.cwd, window, cx);
        if let Some(tab) = self.tabs.last_mut() {
            tab.custom_title = closed.title;
        }
        self.sync_window_title(window, cx);
        self.schedule_session_save(cx);
        cx.notify();
    }

    pub(crate) fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.active = index;
            self.commit_workspace(window, cx);
        }
    }

    /// Move the dragged tab so it sits immediately before `target_id` (tab drag
    /// reorder). `active` follows the previously-active tab to its new index.
    pub(crate) fn reorder_tab(
        &mut self,
        dragged_id: u64,
        target_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.tabs.iter().position(|t| t.id == dragged_id) else {
            return;
        };
        let Some(target) = self.tabs.iter().position(|t| t.id == target_id) else {
            return;
        };
        if dragged_id == target_id {
            return;
        }
        let active_id = self.tabs.get(self.active).map(|t| t.id);
        let tab = self.tabs.remove(from);
        // Insertion index: dropping on a target to the right shifts it left by
        // one once the dragged tab is removed.
        let to = reorder_insert_index(from, target);
        self.tabs.insert(to, tab);
        if let Some(active_id) = active_id {
            self.active = self
                .tabs
                .iter()
                .position(|t| t.id == active_id)
                .unwrap_or(0);
        }
        self.commit_workspace(window, cx);
    }

    /// Move a tab out of this window into a fresh window (drag tab to the
    /// content area). Keeps ≥1 tab here; the detached tab's panes keep running.
    fn detach_tab_to_new_window(
        &mut self,
        tab_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.len() <= 1 {
            return;
        }
        let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };
        let Some(new_active) = active_after_close(self.active, idx, self.tabs.len()) else {
            return;
        };
        let tab = self.tabs.remove(idx);
        self.active = new_active;
        self.commit_workspace(window, cx);
        open_sleipnir_window_with_tab(tab, cx);
    }

    /// Tab dropped on the visible pane area: a *different* tab merges in as a
    /// pane; dropping the visible tab itself still detaches it to a new window.
    pub(super) fn on_tab_dropped_on_pane_area(
        &mut self,
        dragged_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(visible_id) = self.tabs.get(self.active).map(|t| t.id) else {
            return;
        };
        if dragged_id == visible_id {
            self.detach_tab_to_new_window(dragged_id, window, cx);
        } else {
            self.merge_tab_into_visible(dragged_id, window, cx);
        }
    }

    fn merge_tab_into_visible(
        &mut self,
        source_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dest_id) = self.tabs.get(self.active).map(|t| t.id) else {
            return;
        };
        let Ok(dest_idx) = merge_tab(&mut self.tabs, source_id, dest_id) else {
            return;
        };
        // A successful merge always leaves the destination tab behind.
        self.active = dest_idx.min(self.tabs.len() - 1);
        self.commit_workspace(window, cx);
    }

    /// Drop a pane onto the tab list at `insert_at` (clamped).
    pub(crate) fn extract_pane_to_tab(
        &mut self,
        pane_id: PaneId,
        insert_at: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_id = self.next_id;
        let extractable = self
            .tabs
            .iter()
            .any(|tab| tab.tree.is_terminal_leaf(pane_id));
        if !extractable {
            // A Panel extracted into its own tab would be a workspace with no
            // shell. Panels stay attached to the tab that hosts them.
            return;
        }
        let Ok(idx) = extract_pane(&mut self.tabs, pane_id, insert_at, new_id) else {
            return;
        };
        self.next_id += 1;
        // A successful extract only ever adds a tab.
        self.active = idx.min(self.tabs.len() - 1);
        self.commit_workspace(window, cx);
    }

    /// Replace this window's placeholder tab with a detached `tab` and re-wire
    /// each pane's observers to route events here.
    pub(super) fn adopt_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        // Drop the placeholder tab `new` created (its shell exits).
        self.tabs.clear();
        let mut tab = tab;
        let mut leaves = Vec::new();
        tab.tree.leaves(&mut leaves);
        let max_pane = leaves.iter().map(|(id, _)| *id).max().unwrap_or(0);
        let adopted = rebase_detached_tab(max_pane);
        tab.id = adopted.tab_id;
        self.next_id = adopted.next_id;
        self.next_pane_id = adopted.next_pane_id;
        let views: Vec<Entity<TermView>> = leaves.into_iter().map(|(_, v)| v.clone()).collect();
        self.tabs.push(tab);
        self.active = 0;
        for view in &views {
            self.wire_term_view(view, window, cx);
        }
        self.commit_workspace(window, cx);
    }

    pub(crate) fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let next = (self.active + 1) % self.tabs.len();
        self.activate(next, window, cx);
    }

    pub(crate) fn prev_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let prev = if self.active == 0 {
            self.tabs.len() - 1
        } else {
            self.active - 1
        };
        self.activate(prev, window, cx);
    }

    pub(crate) fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_terminal(cx) {
            let handle = view.focus_handle(cx);
            window.focus(&handle, cx);
        } else {
            // Panel (or empty): keep keys on the shell, never a leftover PTY.
            window.focus(&self.focus_handle, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_tab(index: usize) -> ClosedTab {
        ClosedTab {
            cwd: Some(PathBuf::from(format!("/tmp/{index}"))),
            title: Some(format!("tab {index}").into()),
        }
    }

    #[test]
    fn closed_tab_history_keeps_ten_most_recent_tabs() {
        let mut history = Vec::new();
        for index in 0..12 {
            push_closed_tab(&mut history, closed_tab(index));
        }
        assert_eq!(history.len(), CLOSED_TAB_HISTORY_LIMIT);
        assert_eq!(
            history.first().and_then(|tab| tab.title.as_deref()),
            Some("tab 2")
        );
        assert_eq!(
            history.last().and_then(|tab| tab.title.as_deref()),
            Some("tab 11")
        );
    }
}
