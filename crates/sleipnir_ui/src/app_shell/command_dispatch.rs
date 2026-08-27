//! Command Dispatcher: Unified action mapping for AppShell.
//!
//! Maps CommandId from the Command Palette, menus, and shortcuts to canonical
//! workspace actions. Every arm delegates to a named `AppShell` method, so a
//! command has exactly one implementation no matter which entry point fires it.
//!
//! This is a child module of `app_shell` specifically so it can call those
//! methods while they stay private to the shell.

use super::{AppShell, open_sleipnir_window};
use crate::FONT_SIZE_STEP;
use crate::command_palette::CommandId;
use crate::pane_tree::SplitAxis;
use gpui::{Context, Window};

impl AppShell {
    /// Canonical dispatcher for all palette/menu/shortcut commands.
    pub(crate) fn dispatch_command(
        &mut self,
        id: CommandId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            CommandId::NewTab => self.add_tab(window, cx),
            CommandId::ClosePane => self.request_close_active_pane(window, cx),
            CommandId::NextTab => self.next_tab(window, cx),
            CommandId::PrevTab => self.prev_tab(window, cx),
            CommandId::SplitRight => self.split_active(SplitAxis::Horizontal, window, cx),
            CommandId::SplitDown => self.split_active(SplitAxis::Vertical, window, cx),
            CommandId::OpenSettings => self.open_settings(cx),
            CommandId::ReloadSettings => self.reload_settings(cx),
            CommandId::CycleTheme => self.cycle_theme(cx),
            CommandId::CheckForUpdates => self.begin_update_check(window, cx),
            CommandId::Find => self.open_find(cx),
            CommandId::ToggleCommandPalette => self.open_palette(cx),
            CommandId::IncreaseFontSize => self.step_font_size(FONT_SIZE_STEP, cx),
            CommandId::DecreaseFontSize => self.step_font_size(-FONT_SIZE_STEP, cx),
            CommandId::ResetFontSize => self.reset_font_size(cx),
            CommandId::NewWindow => open_sleipnir_window(cx),
            CommandId::TogglePaneZoom => self.toggle_pane_zoom(window, cx),
            CommandId::ToggleBroadcast => self.toggle_broadcast(cx),
            CommandId::JumpPrevPrompt => self.jump_prompt(-1, cx),
            CommandId::JumpNextPrompt => self.jump_prompt(1, cx),
            CommandId::ToggleQuickSelect => self.toggle_quick_select(cx),
            // Same lightweight window as ⌘N (M15).
            CommandId::OpenQuickTerminal => open_sleipnir_window(cx),
            CommandId::ExportScrollback => self.export_scrollback(cx),
            CommandId::ClearRunLedger => self.request_clear_run_ledger(cx),
            CommandId::ToggleRunLedger => self.toggle_run_ledger(cx),
            CommandId::MarkTabSeen => self.mark_active_tab_seen(cx),
            CommandId::SendSelection => self.send_selection_to_pty(cx),
            CommandId::PipeSelection => self.pipe_selection(cx),
            CommandId::SendGitDiff => self.send_git_diff_to_pty(cx),
            CommandId::ToggleHistorySearch => self.toggle_history_search(cx),
            CommandId::TogglePaneFacts => self.toggle_pane_facts(cx),
            CommandId::ToggleDiff => self.toggle_diff(window, cx),
        }
    }
}
