# Terminal UX Research — My Primary-Source Verification Notes
Research date: 2026-08-13. Parent task: compare 7 terminals across 10 UX dimensions.

## Method
- 7 per-terminal background subagents (Ghostty, kitty, WezTerm, Alacritty, iTerm2, Warp, Windows Terminal) researched official docs + changelogs, returning structured evidence.
- I independently fetched and grepped primary pages via curl to cross-check key claims.
- All URLs below were fetched (HTTP 200) or seen in web_search results on 2026-08-13; pages marked "(fetched)" verified directly.

## Ghostty (fetched https://ghostty.org/docs/config/reference and /docs/config/keybind/reference)
- Search EXISTS: actions `search`, `start_search`, `end_search`, `navigate_search`, `search_selection` (keybind reference); config keys search-foreground/background, search-selected-*, "Available since: 1.3.0". Ghostty tags show v1.3.1 latest.
- `write_scrollback_file` ("Write the entire scrollback into a temporary file"), `write_screen_file` ("Write the contents of the screen into a temporary file"), `write_selection_file` — official keybind reference. Actions: copy / open in editor etc.
- `jump_to_prompt` — "Jump the viewport forward or back by the given number of prompts. Requires shell integration."
- Shell integration: prompt marking via OSC 133; `shell-integration-features` (bar/block/underline); shells like Fish v4 / Nu 0.111+ native.
- `notify-on-command-finish` (never/unfocused..., since 1.3.0), `notify-on-command-finish-action` (bell default), `notify-on-command-finish-after`.
- Quick terminal: `toggle_quick_terminal` — "Quake-style or drop-down terminal... keybind = global:cmd+backquote"; only one instance; not restored on restart.
- `window-save-state` (default/never/always): saves/restores position, size, tabs, splits; cwd via shell integration; macOS.
- `link-url`: URLs matched on hover with control (Linux)/command (macOS); `link-previews` (true/false/"osc8", since 1.2.0).
- `split-preserve-zoom` (since 1.1.0/1.3.0) — zoomed splits.
- Theme config: built-in themes + custom theme dirs; light/dark themes.
- copy-on-select referenced in `selection-clear-on-copy` docs (Ghostty has a copy-on-select feature; default off per selection-clear-on-copy text).

## kitty (fetched conf + changelog)
- `search_scrollback` default mapping (0.45.0 [2025-12-24]) — opens scrollback in pager (less) in search mode; live in-terminal incremental search refused (issue #893, 2018).
- `show_scrollback` now an action (ctrl+shift+h).
- `notify_on_cmd_finish` (0.40.0 [2025-03-08]) + bell option; `bell_on_tab`, `visual_bell_*`, `bell_path`, `enable_audio_bell`, `command_on_bell`.
- `detect_urls`, `url_color/url_style`, `url_prefixes` (file ftp ftps gemini git gopher http https irc ircs kitty mailto news sftp ssh); ctrl+shift+click open.
- `copy_on_select` default no (0.14.0, 2019).
- OSC 133 shell integration since 0.24.0; jump prev/next prompt ctrl+shift+z/x; open last command output in pager ctrl+shift+g.
- Quake: quick-access-terminal kitten, 0.42.0 [2025-05-11].
- Sessions: --session files, save_as_session; no process reattach.
- Themes kitten: 300+ themes; `text_fg_override_threshold` min contrast (WCAG AA 4.5); per-pane fonts NOT supported.
- Latest 0.48.2 [2026-07-30].

## WezTerm (fetched shell-integration.html, quickselect.html, copymode.html, appearance.html)
- Search overlay: incremental live highlight, match cycling incl. regex; no whole-word option.
- `wezterm cli get-text` — scrollback capture (not pager).
- Implicit hyperlink rules (default URL rule) + OSC 8; open-uri event; QuickSelect (ctrl+shift+space).
- OSC 133/7/1337; ScrollToPrompt action (not bound by default); no built-in command-finish notify; update-status event for status line.
- Multiplexer reattach: `wezterm connect`/`start --attach`; workspaces partial; no auto local layout restore.
- Fonts global only (no per-pane fonts); 700+ schemes / gallery "1001 Color schemes"; auto config reload; text_min_contrast_ratio (nightly).
- Copy-on-select default (mouse release copies); ALT+drag block; alt-screen mostly supported.
- TogglePaneZoomState (ctrl+shift+z); tab drag-reorder NOT documented (issue #549 open).
- No built-in quake (issue #1751 open); scripting primitives only.
- audible_bell/visual_bell; OSC 9/777 toasts; no command-finish notify built-in.

## Alacritty (fetched config-alacritty.html)
- Search: regex-based (footer "search regex input"), vi-mode / and ?; live highlight via colors.search; no case/whole-word options; no open-in-pager (issue #1615 open).
- Hints: regex + hyperlinks (OSC 8 since 0.11.0), hover underline with mods, launch command with matched text only (no group placeholders).
- OSC 133 NOT supported (escapes manpage; issue #5850 open; PR #5860 closed unmerged).
- No session restore; --working-directory CLI only.
- No per-pane fonts (no panes at all); no tabs on Linux/Windows (macOS native tabs only); live_config_reload default true; no minimum contrast.
- save_to_clipboard default false; block selection via Ctrl+drag or vi mode.
- No quick terminal (issue #1119 closed).
- No screen reader (issue #5933 closed 2022); unfocused_hollow cursor; vi mode.
- Bell: visual bell animation (duration 0 = off by default), bell.command.
- Latest v0.17.0.

## iTerm2 (subagent-verified; my probes confirmed URLs 200)
- Find: regex toggle (ICU), smart case default, live highlight + filter (3.5.0); no open-in-pager (Save Contents only).
- Smart Selection rules + Semantic History (cmd-click; \1 filename \2 line); OSC 8; file://...#123:45 line:col (3.4).
- Shell integration: OSC 133 marks; Cmd-Shift-Up/Down jump; Alert on next mark (command-finish modal); command Info shows duration.
- Session restoration: long-lived servers, reattach after crash/upgrade (documentation-restoration.html).
- Fonts per profile (mixed-font panes possible via split with current profile); itermcolors import/export; "Load settings from a custom folder"; Minimum contrast slider.
- Copy-on-select BY DEFAULT ON ("text is copied to the clipboard immediately upon being selected"); rectangular = cmd+option drag; non-contiguous cmd+drag.
- Tab drag reorder, drag to new window; pane maximize cmd+shift+enter; split-ratio persistence not documented.
- Hotkey window (dedicated, quake-like, pin/animate/floating).
- VoiceOver: not documented; a11y via macOS bugs only.
- Bell: Silence bell, Flash visual bell, bell icon in tabs; Notification Center alerts; OSC 1337 Notification= (3.5.0).
- Current stable 3.6.11 (built 2026-06-02).

## Warp (fetched docs pages + subagent)
- Find: regex toggle + case toggle; block-scoped; no whole-word documented; no scrollback-in-pager.
- Link detection: files/folders/URLs in Blocks; parses file:line:col; hover tooltip; Cmd/Ctrl+click; OSC 8.
- Shell integration: own "Warpify"; NO OSC 133 documented; blocks; desktop notifications on command finish (default threshold 30 s); Command History stores exit code/duration.
- Session restoration: windows/tabs/panes/Blocks restored; enabled by default; no process reattach.
- Themes: ~21 built-in, 315 in GitHub repo (136 standard + 179 base16); settings.toml hot reload; enforce_minimum_contrast.
- copy_on_select default true; rectangular via CMD-OPT/CTRL-ALT drag.
- Tabs drag reorder, drag out to new window; split panes drag-drop; Toggle Maximize Pane (CMD-SHIFT-ENTER).
- Global Hotkey: dedicated Quake-style drop-down window.
- Accessibility: VoiceOver partial (WIP); Windows/Linux screen-reader not documented.
- Bell: audible bell default off; notifications configurable.

## Windows Terminal (my fetches of learn.microsoft.com + GitHub releases JSON)
- Find: Ctrl+Shift+F, directional, case match only — NO regex in docs (page last updated 2025-11-12); regex search arriving via console refresh in Windows 11 Canary build 29558 (2026) (secondary: helpnetsecurity/ittrip).
- OSC 8 since v1.4 (2020-09-22); automatic hyperlink detection since v1.5 (2020-11-11); hyperlinks navigable via Tab in Mark Mode (1.16+).
- Shell integration: OSC 133 A/B/C/D (FTCS_*) since v1.18 (docs "as of Terminal v1.18", page updated 2025-08-21).
- Session restore: window/pane layouts saved on close & restored on relaunch since v1.12 (2021-10-20, #10972); screen contents restored since v1.21 (2024-05-07, "Open windows from a previous session"); NOT process reattach.
- Themes: built-in color schemes 16 (defaults.json); themes feature (dark/light/system + custom) since 1.21.
- copyOnSelect default false; block selection supported.
- togglePaneZoom action (not bound by default); drag tab reorder; drag tab into another window (crash fix v1.24/1.25 2026-07-16); right-click context menu split/zoom (1.23 2025-08-26).
- Quake mode: v1.9 (2021-05-25, "pinning an instance to the top of the screen... 'Quake Mode'"); `wt -w _quake`.
- Accessibility: UIA provider (2019+), UIA events 2020, UIA notifications 2022 (wt_a11y.md).
- bellStyle: all/audible/window/taskbar/none, default "audible"; no command-finish notify; no minimum contrast setting.

## Cross-cutting notes / contradictions
- copy-on-select defaults: iTerm2 ON, Warp ON; kitty OFF, Alacritty OFF (save_to_clipboard false), WT OFF (copyOnSelect false), WezTerm copies on mouse release by default binding; Ghostty has copy-on-select (default off per docs text).
- OSC 133: kitty (0.24+), WezTerm, Ghostty (1.3), iTerm2, WT (1.18+) support; Alacritty does NOT; Warp uses proprietary Warpify, OSC 133 not documented.
- Live incremental search: iTerm2 (3.5.0+), WezTerm, Ghostty, WT, Alacritty, Warp; kitty explicitly refuses (performance, issue #893) and does pager-based search instead.
- Quake/dropdown: kitty 0.42 quick-access-terminal, Ghostty toggle_quick_terminal, iTerm2 Hotkey Window, WT Quake Mode (1.9+), Warp Global Hotkey; WezTerm no (issue #1751); Alacritty no (issue #1119).
- Session restore: iTerm2 (server reattach, strongest), WT (layout+contents since 1.12/1.21), Warp (layout+blocks), Ghostty (window state, macOS), kitty (session files, no reattach), WezTerm (multiplexer reattach, partial), Alacritty (none).
- Per-pane fonts: none of the 7 support true per-pane font independence natively (iTerm2 approximates via per-profile splits; WT via per-profile panes; kitty explicitly same font size per OS window; WezTerm global; Ghostty per-window font? — subagent pending; Warp global; Alacritty global).
- Tab drag-reorder: iTerm2, WT, Warp, kitty (0.46+), Ghostty (pending); WezTerm NOT (issue #549).
