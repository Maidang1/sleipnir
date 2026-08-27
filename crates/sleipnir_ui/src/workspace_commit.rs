//! Unified Workspace Mutation Pipeline.
//!
//! Every structural change to tabs/panes commits through this boundary,
//! guaranteeing that focus restoration, window title updates, tab scroll,
//! session saving debounce, ledger focus sync, and UI notifications
//! never get forgotten or desynchronized across different operations.

use crate::app_shell::AppShell;
use gpui::{Context, Window};

impl AppShell {
    /// Canonical commit point for workspace mutations.
    ///
    /// Every effect here is idempotent and cheap, so there is no per-callsite
    /// opt-out: the window title is derived from the active *pane's* title, so
    /// even pane splits and intra-tab focus moves can change it.
    pub(crate) fn commit_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active(window, cx);
        self.sync_ledger_focus(window, cx);
        self.sync_window_title(window, cx);
        self.tab_scroll_handle.scroll_to_item(self.active);
        self.schedule_session_save(cx);
        cx.notify();
    }
}
