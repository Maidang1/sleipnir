//! Sleipnir — standalone terminal (HIG-aligned window chrome).

mod app_menus;

use app_menus::{Hide, HideOthers, Quit, ShowAll, app_menus};
use gpui::{App, KeyBinding};
use gpui_platform::application;
use release_channel::AppVersion;
use sleipnir_settings::{self, KeyBindingSpec, TerminalSettings};
use sleipnir_ui::{
    ActivateTab, BindingContext, BuiltinAction, CheckForUpdates, ClearRunLedger, CloseTab,
    CycleTheme, DecreaseFontSize, Find, FindNext, FindPrev, FocusPaneDown, FocusPaneLeft,
    FocusPaneRight, FocusPaneUp, IncreaseFontSize, JumpNextPrompt, JumpPrevPrompt, MarkTabSeen,
    NewTab, NewWindow, NextTab, OpenQuickTerminal, OpenSettings, PipeSelection, PrevTab,
    ReloadSettings, ResetFontSize, SendGitDiff, SendSelection, SplitDown, SplitRight,
    ToggleBroadcast, ToggleCommandPalette, ToggleDiff, ToggleHistorySearch, TogglePaneFacts,
    TogglePaneZoom, ToggleQuickSelect, ToggleRunLedger, builtin_bindings, install_finder_services,
    last_window_close_quits, open_sleipnir_window, tmux_preset_bindings,
};
use terminal::{
    Clear, Copy, Paste, PasteText, ScrollLineDown, ScrollLineUp, ScrollPageDown, ScrollPageUp,
    ScrollToBottom, ScrollToTop, SelectAll, SendKeystroke, SendText, ShowCharacterPalette,
    ToggleViMode,
};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let app = application().with_assets(sleipnir_ui::AgentAssets);
    // Last-window close does not quit on macOS. Dock / Cmd-Tab reactivation
    // fires `applicationShouldHandleReopen` with no visible windows; without
    // this callback the click is a no-op and the process stays headless.
    #[cfg(target_os = "macos")]
    {
        app.on_reopen(|cx| {
            if cx.windows().is_empty() {
                open_sleipnir_window(cx);
                return;
            }
            // All windows may be minimized; AppKit reports no visible windows and
            // GPUI still holds the handles — bring one back instead of spawning.
            if let Some(handle) = cx.windows().first().copied() {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            }
        });
    }
    app.run(|cx: &mut App| {
        AppVersion::init_with(env!("CARGO_PKG_VERSION"), cx);
        sleipnir_settings::init(cx);

        // App-menu actions (always available so validation enables the items).
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        // AppShell's NewWindow handler dies with the last window; keep
        // New Window working while the process is still up.
        cx.on_action(|_: &NewWindow, cx| open_sleipnir_window(cx));

        if last_window_close_quits() {
            cx.on_window_closed(|cx, _window_id| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        }

        let mut keys = Vec::new();
        for spec in builtin_bindings() {
            keys.extend(key_bindings_for_builtin(&spec));
        }
        if sleipnir_settings::TerminalSettings::get_global(cx).keybinding_preset
            == sleipnir_settings::KeybindingPreset::Tmux
        {
            for spec in tmux_preset_bindings() {
                keys.extend(key_bindings_for_builtin(&spec));
            }
        }
        cx.bind_keys(keys);

        // User key binding overrides from settings.json (M9).
        bind_user_key_bindings(cx);

        // After keybindings so menu items pick up key equivalents from the keymap.
        cx.set_menus(app_menus());

        // Finder Services ("New Sleipnir Tab/Window Here") must be bound before
        // didFinishLaunching returns so a cold-start invocation is not dropped.
        install_finder_services(cx);

        open_sleipnir_window(cx);
        cx.activate(true);
    });
}

fn key_bindings_for_builtin(spec: &sleipnir_ui::BuiltinBinding) -> Vec<KeyBinding> {
    let contexts: Vec<Option<&str>> = match spec.context {
        BindingContext::Global => vec![None],
        BindingContext::Terminal => vec![Some("Terminal")],
        BindingContext::Both => vec![Some("AppShell"), Some("Terminal")],
    };
    contexts
        .into_iter()
        .map(|ctx| bind_action(&spec.key, spec.action, ctx))
        .collect()
}

fn bind_action(key: &str, action: BuiltinAction, context: Option<&str>) -> KeyBinding {
    match action {
        BuiltinAction::Quit => KeyBinding::new(key, Quit, context),
        BuiltinAction::Hide => KeyBinding::new(key, Hide, context),
        BuiltinAction::HideOthers => KeyBinding::new(key, HideOthers, context),
        BuiltinAction::Copy => KeyBinding::new(key, Copy, context),
        BuiltinAction::Paste => KeyBinding::new(key, Paste, context),
        BuiltinAction::PasteText => KeyBinding::new(key, PasteText, context),
        BuiltinAction::SelectAll => KeyBinding::new(key, SelectAll, context),
        BuiltinAction::Clear => KeyBinding::new(key, Clear, context),
        BuiltinAction::ShowCharacterPalette => KeyBinding::new(key, ShowCharacterPalette, context),
        BuiltinAction::ToggleViMode => KeyBinding::new(key, ToggleViMode, context),
        BuiltinAction::ScrollLineUp => KeyBinding::new(key, ScrollLineUp, context),
        BuiltinAction::ScrollLineDown => KeyBinding::new(key, ScrollLineDown, context),
        BuiltinAction::ScrollPageUp => KeyBinding::new(key, ScrollPageUp, context),
        BuiltinAction::ScrollPageDown => KeyBinding::new(key, ScrollPageDown, context),
        BuiltinAction::ScrollToTop => KeyBinding::new(key, ScrollToTop, context),
        BuiltinAction::ScrollToBottom => KeyBinding::new(key, ScrollToBottom, context),
        BuiltinAction::NewTab => KeyBinding::new(key, NewTab, context),
        BuiltinAction::CloseTab => KeyBinding::new(key, CloseTab, context),
        BuiltinAction::NewWindow => KeyBinding::new(key, NewWindow, context),
        BuiltinAction::NextTab => KeyBinding::new(key, NextTab, context),
        BuiltinAction::PrevTab => KeyBinding::new(key, PrevTab, context),
        BuiltinAction::ReloadSettings => KeyBinding::new(key, ReloadSettings, context),
        BuiltinAction::OpenSettings => KeyBinding::new(key, OpenSettings, context),
        BuiltinAction::CycleTheme => KeyBinding::new(key, CycleTheme, context),
        BuiltinAction::SplitRight => KeyBinding::new(key, SplitRight, context),
        BuiltinAction::SplitDown => KeyBinding::new(key, SplitDown, context),
        BuiltinAction::FocusPaneLeft => KeyBinding::new(key, FocusPaneLeft, context),
        BuiltinAction::FocusPaneRight => KeyBinding::new(key, FocusPaneRight, context),
        BuiltinAction::FocusPaneUp => KeyBinding::new(key, FocusPaneUp, context),
        BuiltinAction::FocusPaneDown => KeyBinding::new(key, FocusPaneDown, context),
        BuiltinAction::CheckForUpdates => KeyBinding::new(key, CheckForUpdates, context),
        BuiltinAction::ToggleCommandPalette => KeyBinding::new(key, ToggleCommandPalette, context),
        BuiltinAction::Find => KeyBinding::new(key, Find, context),
        BuiltinAction::FindNext => KeyBinding::new(key, FindNext, context),
        BuiltinAction::FindPrev => KeyBinding::new(key, FindPrev, context),
        BuiltinAction::TogglePaneZoom => KeyBinding::new(key, TogglePaneZoom, context),
        BuiltinAction::ToggleBroadcast => KeyBinding::new(key, ToggleBroadcast, context),
        BuiltinAction::JumpPrevPrompt => KeyBinding::new(key, JumpPrevPrompt, context),
        BuiltinAction::JumpNextPrompt => KeyBinding::new(key, JumpNextPrompt, context),
        BuiltinAction::ToggleQuickSelect => KeyBinding::new(key, ToggleQuickSelect, context),
        BuiltinAction::OpenQuickTerminal => KeyBinding::new(key, OpenQuickTerminal, context),
        BuiltinAction::ToggleRunLedger => KeyBinding::new(key, ToggleRunLedger, context),
        BuiltinAction::ToggleHistorySearch => KeyBinding::new(key, ToggleHistorySearch, context),
        BuiltinAction::ToggleDiff => KeyBinding::new(key, ToggleDiff, context),
        BuiltinAction::IncreaseFontSize => KeyBinding::new(key, IncreaseFontSize, context),
        BuiltinAction::DecreaseFontSize => KeyBinding::new(key, DecreaseFontSize, context),
        BuiltinAction::ResetFontSize => KeyBinding::new(key, ResetFontSize, context),
        BuiltinAction::ActivateTab(n) => KeyBinding::new(key, ActivateTab(n), context),
        BuiltinAction::SendKeystroke(ks) => KeyBinding::new(key, SendKeystroke(ks.into()), context),
        BuiltinAction::SendText(text) => KeyBinding::new(key, SendText(text.into()), context),
    }
}

/// Layer optional user key bindings from `settings.json` on top of builtins.
fn bind_user_key_bindings(cx: &mut App) {
    let bindings = TerminalSettings::get_global(cx).key_bindings.clone();
    if bindings.is_empty() {
        return;
    }
    let mut keys = Vec::new();
    for spec in &bindings {
        let expanded = key_bindings_for_spec(spec);
        if expanded.is_empty() {
            log::warn!(
                "unknown key binding action {:?} for key {:?}",
                spec.action,
                spec.key
            );
        } else {
            keys.extend(expanded);
        }
    }
    if !keys.is_empty() {
        log::info!("applying {} user key binding(s)", keys.len());
        cx.bind_keys(keys);
    }
}

fn key_bindings_for_spec(spec: &KeyBindingSpec) -> Vec<KeyBinding> {
    let contexts: Vec<&str> = match spec.context.as_deref() {
        None | Some("") => vec!["AppShell", "Terminal"],
        Some(c) => vec![c],
    };
    let mut out = Vec::new();
    for ctx in contexts {
        let kb = match spec.action.as_str() {
            "new_tab" => KeyBinding::new(&spec.key, NewTab, Some(ctx)),
            "close_tab" | "close_pane" => KeyBinding::new(&spec.key, CloseTab, Some(ctx)),
            "next_tab" => KeyBinding::new(&spec.key, NextTab, Some(ctx)),
            "prev_tab" => KeyBinding::new(&spec.key, PrevTab, Some(ctx)),
            "split_right" => KeyBinding::new(&spec.key, SplitRight, Some(ctx)),
            "split_down" => KeyBinding::new(&spec.key, SplitDown, Some(ctx)),
            "open_settings" => KeyBinding::new(&spec.key, OpenSettings, Some(ctx)),
            "reload_settings" => KeyBinding::new(&spec.key, ReloadSettings, Some(ctx)),
            "cycle_theme" => KeyBinding::new(&spec.key, CycleTheme, Some(ctx)),
            "check_for_updates" => KeyBinding::new(&spec.key, CheckForUpdates, Some(ctx)),
            "find" | "search" => KeyBinding::new(&spec.key, Find, Some(ctx)),
            "find_next" => KeyBinding::new(&spec.key, FindNext, Some(ctx)),
            "find_prev" => KeyBinding::new(&spec.key, FindPrev, Some(ctx)),
            "toggle_command_palette" | "command_palette" => {
                KeyBinding::new(&spec.key, ToggleCommandPalette, Some(ctx))
            }
            "increase_font_size" | "font_size_up" => {
                KeyBinding::new(&spec.key, IncreaseFontSize, Some(ctx))
            }
            "decrease_font_size" | "font_size_down" => {
                KeyBinding::new(&spec.key, DecreaseFontSize, Some(ctx))
            }
            "reset_font_size" | "font_size_reset" => {
                KeyBinding::new(&spec.key, ResetFontSize, Some(ctx))
            }
            "new_window" => KeyBinding::new(&spec.key, NewWindow, Some(ctx)),
            // Terminal actions (scroll, clipboard, send)
            "copy" => KeyBinding::new(&spec.key, Copy, Some(ctx)),
            "paste" => KeyBinding::new(&spec.key, Paste, Some(ctx)),
            "paste_text" => KeyBinding::new(&spec.key, PasteText, Some(ctx)),
            "select_all" => KeyBinding::new(&spec.key, SelectAll, Some(ctx)),
            "clear" => KeyBinding::new(&spec.key, Clear, Some(ctx)),
            "scroll_line_up" => KeyBinding::new(&spec.key, ScrollLineUp, Some(ctx)),
            "scroll_line_down" => KeyBinding::new(&spec.key, ScrollLineDown, Some(ctx)),
            "scroll_page_up" => KeyBinding::new(&spec.key, ScrollPageUp, Some(ctx)),
            "scroll_page_down" => KeyBinding::new(&spec.key, ScrollPageDown, Some(ctx)),
            "scroll_to_top" => KeyBinding::new(&spec.key, ScrollToTop, Some(ctx)),
            "scroll_to_bottom" => KeyBinding::new(&spec.key, ScrollToBottom, Some(ctx)),
            "toggle_vi_mode" => KeyBinding::new(&spec.key, ToggleViMode, Some(ctx)),
            "show_character_palette" => KeyBinding::new(&spec.key, ShowCharacterPalette, Some(ctx)),
            "clear_run_ledger" => KeyBinding::new(&spec.key, ClearRunLedger, Some(ctx)),
            "toggle_run_ledger" => KeyBinding::new(&spec.key, ToggleRunLedger, Some(ctx)),
            "mark_tab_seen" | "mark_as_seen" => KeyBinding::new(&spec.key, MarkTabSeen, Some(ctx)),
            "toggle_pane_facts" | "pane_facts" => {
                KeyBinding::new(&spec.key, TogglePaneFacts, Some(ctx))
            }
            "send_selection" => KeyBinding::new(&spec.key, SendSelection, Some(ctx)),
            "pipe_selection" => KeyBinding::new(&spec.key, PipeSelection, Some(ctx)),
            "send_git_diff" => KeyBinding::new(&spec.key, SendGitDiff, Some(ctx)),
            "toggle_history_search" | "history_search" => {
                KeyBinding::new(&spec.key, ToggleHistorySearch, Some(ctx))
            }
            "toggle_diff" => KeyBinding::new(&spec.key, ToggleDiff, Some(ctx)),
            _ => continue,
        };
        out.push(kb);
    }
    out
}
