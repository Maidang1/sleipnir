# Terminal Emulator UX Capabilities — Research Report

**Research date:** 2026-08-13
**Terminals covered:** Ghostty, kitty, WezTerm, Alacritty, iTerm2, Warp, Windows Terminal
**Evidence priority:** official docs and changelogs first; secondary sources used only as leads or for in-flux items (flagged as such).

---

## Scope & Method

- Ten UX dimensions (search, hyperlink/path detection, shell integration, session restore, theming & fonts, copy/paste, tab/split ergonomics, quick terminal, accessibility, notifications) were researched per terminal.
- Primary sources: ghostty.org/docs, sw.kovidgoyal.net/kitty, wezterm.org, alacritty.org (+ GitHub changelogs), iterm2.com documentation & changelog, docs.warp.dev (+ warpdotdev/themes repo), learn.microsoft.com/windows/terminal (+ microsoft/terminal GitHub releases).
- Every URL cited below was either fetched directly (HTTP 200) or returned by web_search on the research date; no URL was invented. "No date shown" means the page itself carries no date (typical for doc pages); dated items come from changelogs/releases/`Last updated` footers.
- Version state on the research date: Ghostty v1.3.1 (tag), kitty 0.48.2 [2026-07-30], Alacritty v0.17.0, iTerm2 3.6.11 (built 2026-06-02), Windows Terminal 1.25/1.24 (2026-07-16), WezTerm continuous/nightly builds, Warp docs updated Aug 2026.

---

## D1 — Search (regex vs literal, case/whole-word, live highlight, scrollback-in-pager)

**Ghostty** — Built-in search UI added in 1.3.0 (2026-03-09): "You can now search your terminal scrollback with cmd+f on macOS or ctrl+shift+f on GTK. Search highlights all matches in the viewport and allows you to navigate between them" (next/prev: cmd+g / shift+cmd+g macOS, enter/shift+enter GTK). Search is live/incremental (a concurrent search thread re-runs on every input change). Matching is **literal substring, case-insensitive for ASCII only** (`std.ascii.indexOfIgnoreCase`) — **no regex or whole-word option**. Actions: `search`, `start_search`, `search_selection`, `navigate_search`, `end_search`; match colors via `search-foreground/background` and `search-selected-*`. Scrollback-to-file: `write_scrollback_file` ("Write the entire scrollback into a temporary file"), `write_screen_file` ("Write the contents of the screen into a temporary file"), `write_selection_file` — with actions copy (filepath to clipboard), paste, or open ("Open the file in the default OS editor" — `open` on macOS, `xdg-open` on Linux).
- Source: *1.3.0 Release Notes — Ghostty* | https://ghostty.org/docs/install/release-notes/1-3-0 | 2026-03-09
- Source: *Ghostty Keybind Reference* | https://ghostty.org/docs/config/keybind/reference | fetched 2026-08-13
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13 (search keys marked "Available since: 1.3.0")

**kitty** — No in-terminal live search overlay. Default mapping `search_scrollback` (added 0.45.0 [2025-12-24]) "open[s] the scrollback in a pager in search mode"; matching (regex/literal/case) is done by the pager (`scrollback_pager`, less by default). `show_scrollback` is now an action (default `ctrl+shift+h`) that shows scrollback in a pager; `scrollback_pager` supports `INPUT_LINE_NUMBER`/`CURSOR_LINE`/`CURSOR_COLUMN` placeholders. Live incremental search was explicitly refused by the maintainer for performance reasons ("Searching for text live in program output is a huge performance killer… kitty prioritizes performance").
- Source: *Configuration — kitty* | https://sw.kovidgoyal.net/kitty/conf/ | no date shown
- Source: *kitty Changelog* | https://sw.kovidgoyal.net/kitty/changelog/ | 0.45.0 [2025-12-24]
- Source: *Scrollback search · Issue #893* | https://github.com/kovidgoyal/kitty/issues/893 | closed 2018-09-07

**WezTerm** — Incremental search overlay over scrollback (default CTRL-SHIFT-F / CMD-F): "Text from the scrollback that matches the search pattern will be highlighted and the number of matches shown." CTRL-R cycles match type: case-sensitive → case-insensitive → "smart" case → **regular expression**; no whole-word option documented. Scrollback capture via CLI: `wezterm cli get-text` (negative `--start-line` indexes into scrollback); no built-in "open in pager".
- Source: *Scrollback — WezTerm* | https://wezterm.org/scrollback.html | no date shown
- Source: *get-text — WezTerm CLI* | https://wezterm.org/cli/cli/get-text.html | no date shown

**Alacritty** — Vi-mode regex search over the scrollback (footer bar is "used by search regex input"); `SearchForward`/`SearchBackward` (Ctrl+Shift+F/B, or `/`/`?` in vi mode), `SearchNext/Previous/Start`; live highlight with `[colors.search]` matches/focused_match. No case/whole-word options. No open-in-pager (feature request #1615 open since 2018).
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown (v0.17.0-era)
- Source: *Piping visible text to external programs · Issue #1615* | https://github.com/alacritty/alacritty/issues/1615 | 2018-10-02

**iTerm2** — Find panel (Cmd+F) with **regex toggle** ("The ICU syntax is used"); default case mode "Smart Case Sensitivity". `Edit > Find` includes Find Next/Previous, Find Globally (all tabs), Select Matches, Find URLs, Pick Result to Open, and **Filter** ("Non-matching lines are temporarily hidden", added 3.5.0). Live incremental highlighting added in 3.5.0 ("Automatically highlight matches when the search UI is open and the query changes" + animated current-match indicator). No documented open-scrollback-in-pager; export is "Shell > Save Contents".
- Source: *Documentation (one-page) — iTerm2* | https://iterm2.com/documentation-one-page.html | docs version 3.6, no date shown
- Source: *iTerm2 3.5.0 changelog* | https://iterm2.com/downloads/stable/iTerm2-3_5_0.changelog | 3.5.0 (2024-05-17)

**Warp** — Find (Cmd+F / Ctrl+Shift+F) searches "all your Blocks from the bottom up" and can be scoped to one Block; the find modal has a **regex toggle** and a **case-sensitive toggle**; whole-word option not documented. No scrollback-in-pager; closest is right-click "Copy input/output of block".
- Source: *Terminal Block Find — Warp* | https://docs.warp.dev/terminal/blocks/find/ | page footer "Last updated Aug 11, 2026"

**Windows Terminal** — Find dialog (Ctrl+Shift+F): directional search (default bottom-to-top) and a **case-match toggle**; **no regex** in the current docs. Regex search is arriving with the 2026 console refresh (Windows 11 Canary build 29558 adds regex search to conhost and Terminal) — in flux, secondary source.
- Source: *Find in Windows Terminal — Microsoft Learn* | https://learn.microsoft.com/en-us/windows/terminal/search | "Last updated 2025-11-12"
- Source (in-flux, secondary): *Windows 11 gets a rebuilt console engine with regex search* | https://www.helpnetsecurity.com/2026/03/31/windows-11-console-upgrade-speed-boost/ | 2026-03-31

**Best in class (D1):** **iTerm2** for the richest search toolset (regex toggle, smart-case default, live highlight, global/tab-wide search, Filter, find-URLs). **Ghostty** for the strongest first-party "scrollback → file/editor" story (`write_scrollback_file`/`write_screen_file`/`write_selection_file` with configurable actions); kitty's pager-based approach is the classic alternative (0.45.0+).

**Contradictions / uncertainty:** kitty's maintainer argues live incremental search is a performance mistake (issue #893), while six other terminals ship it — a genuine design-philosophy split. Ghostty's search feature dates from 1.3.0 (config keys), but no release-note date for 1.3.0 was retrievable on research day (GitHub API rate-limited). WT regex search is not in stable docs as of the research date.

---

## D2 — Hyperlink / path detection

**Ghostty** — URL matching via `link-url`: "URLs are matched on hover with control (Linux) or command (macOS) pressed and open using the default system application"; custom regex `link` rules documented but the key carries a "TODO: This can't currently be set!" note; `link-previews` (true/false/"osc8", since 1.2.0) shows hover previews, optionally only for OSC 8 hyperlinks. OSC 8 supported (VT reference lists "OSC 8 (Hyperlinks)"). **Clickable file paths since 1.1.0** (PR #4743) and IPv6 URL auto-linking since 1.1.0 (#5285); `open` action opens files in the default OS editor ("open on macOS and xdg-open on Linux"). `file:line:col` precision: **not supported** (opening uses the system opener; no line/column parsing documented).
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13
- Source: *1.1.0 Release Notes — Ghostty* | https://ghostty.org/docs/install/release-notes/1-1-0 | 2025-01-30

**kitty** — `detect_urls yes`: "Detected URLs are highlighted with an underline and the mouse cursor becomes a hand"; `url_color`/`url_style` (curly underline) on mouse-over; `url_prefixes` covers `file ftp ftps gemini git gopher http https irc ircs kitty mailto news sftp ssh`. Click-to-open via `mouse_handle_click link` (default ctrl+shift+click, configurable modifier). OSC 8: `underline_hyperlinks` (hover/always/never) and `show_hyperlink_targets` (hover tooltip with the real URL). The `hints` kitten adds keyboard selection of URLs (`ctrl+shift+e`), paths/filenames (`ctrl+shift+p>f`), and path+line (`ctrl+shift+p>n`; `--type=linenum` matches `path:line` and opens at the line, e.g. `hints --type=linenum --linenum-action=tab nvim +{line} {path}`). `open_actions` customizes per-protocol/mime click actions with URL/FILE_PATH/FRAGMENT variables.
- Source: *Configuration — kitty* | https://sw.kovidgoyal.net/kitty/conf/ | no date shown
- Source: *Hints — kitty* | https://sw.kovidgoyal.net/kitty/kittens/hints/ | no date shown
- Source: *Scripting the mouse click — kitty* | https://sw.kovidgoyal.net/kitty/open_actions/ | no date shown

**WezTerm** — "Implicit hyperlinks are produced by running a series of rules over the output" with a default URL rule (plus `file://` matching added in changelog); hover feedback via the `highlight` field of `hyperlink_rules` ("the range of the matched text that should be highlighted/underlined when the mouse hovers over the link"). OSC 8 ("Hyperlinks in Terminal Emulators" spec) supported. By default "hyperlinks can be opened with a simple mouse click, without any modifier keys"; `OpenLinkAtMouseCursor` can be bound to a modifier. `file:line:col`: OSC 8 `file://…` links are parsed (`wezterm.url.parse` exposes `file_path` and `fragment`), and the recipe opens them in nvim at that line; plain-text `file:line:col` is not in default QuickSelect patterns. QuickSelect (CTRL-SHIFT-SPACE) matches URLs, path fragments, git hashes, IPs, numbers, with `QuickSelectArgs` to run arbitrary Lua on the match.
- Source: *Hyperlinks — WezTerm* | https://wezterm.org/hyperlinks.html | no date shown
- Source: *Quick Select Mode — WezTerm* | https://wezterm.org/quickselect.html | no date shown
- Source: *Use hyperlinks directly in the terminal — WezTerm recipes* | https://wezterm.org/recipes/hyperlinks.html | no date shown

**Alacritty** — `[hints]`: "Terminal hints can be used to find text or hyperlinks in the visible part of the terminal and pipe it to other applications"; per-hint regex or `hyperlinks = true` (OSC 8 links included since 0.11.0), `post_processing` heuristics, actions Copy/Paste/Select/MoveViModeCursor or an arbitrary `command` ("The hint's text is always attached as the last argument" — **no regex-group placeholders**, so `file:line:col` cannot be forwarded to a launcher). Hover underline gated by `mouse = { mods, enabled }`; footer bar doubles as "hyperlink URI preview". Mouse-captured apps: Shift bypasses mouse reporting.
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown
- Source: *Alacritty CHANGELOG* | https://raw.githubusercontent.com/alacritty/alacritty/master/CHANGELOG.md | version 0.11.0

**iTerm2** — **Smart Selection**: quad-click activates rule-based selection; default rules recognize filesystem paths, URIs (mailto/http/https/ssh/telnet), email addresses; rules are editable regexes with a Precision attribute. **Semantic History**: Cmd-click opens the selection — "Open with default app", "Open URL…", "Open with editor…" (with `\1` = filename, `\2` = line number). OSC 8 supported; since 3.4, `file:///tmp/file.txt#123` and `#123:45` apply semantic-history rules (line and line:column precision); hover preview box (3.5.0 changelog); OSC 8 `target=` param added 3.6.7 (built 2026-02-19).
- Source: *Smart Selection — iTerm2* | https://iterm2.com/documentation-smart-selection.html | no date shown
- Source: *Downloads — iTerm2* (versioned changelogs) | https://iterm2.com/downloads.html | 3.6.7 built 2026-02-19; 3.5.0 built 2024-05-17

**Warp** — "Warp supports opening files, folders, and URL links that are within Blocks… Warp parses relative and absolute file paths. Warp also tries to capture line and column numbers attached to the file path" (`file_name:line_num` and `file_name:line_num:column_num`); hover shows a tooltip; Cmd/Ctrl+click opens; right-click copies the absolute path/URL. OSC 8 recognized ("command-line tools can print a short label that links to a longer URL"). Whether the configured editor actually jumps to the captured line/col is not explicitly documented.
- Source: *Files, Links, & Scripts — Warp* | https://docs.warp.dev/terminal/more-features/files-and-links/ | no date shown

**Windows Terminal** — App-generated hyperlinks (OSC 8) since v1.4 (2020-09-22); **automatic hyperlink detection** since v1.5 (2020-11-11, "This is NOT the same thing as automatic hyperlink detection!" was the v1.4 note); Ctrl+click opens. Underline-on-hover behavior refined over versions (1.5–1.23). Tab/Shift+Tab navigates between hyperlinks in Mark Mode, Ctrl+Enter opens (since v1.16, 2022-09-13). File-path opening and `file:line:col` precision are not documented as built-in.
- Source: *Windows Terminal GitHub releases* | https://github.com/microsoft/terminal/releases | v1.4 2020-09-22; v1.5 2020-11-11; v1.16 2022-09-13

**Best in class (D2):** **kitty** (URL/path detection + OSC 8 hover tooltips + hints kitten with `path:line` opening at a line) and **iTerm2** (Smart Selection rules with precision + Semantic History with `\1`/`\2` filename/line, `file:line:col` since 3.4) are the two strongest; Warp's parsed `file:line:col` in Blocks is notable.

**Contradictions / uncertainty:** Alacritty hints cannot forward regex groups, so `file:line:col` precision is impossible there (explicit doc limitation). WezTerm plain-text `file:line:col` is not detected (OSC 8 `file://…#fragment` only). Warp's editor-jump behavior is undocumented. Ghostty has no documented `file:line:col` opening.

---

## D3 — Shell integration (OSC 133, prompt jumping, command-finish, status line)

**Ghostty** — Prompt marking via **OSC 133** with `shell-integration` auto-injection for bash/elvish/fish/nushell/zsh; documented features include working-directory reporting (new tabs/splits inherit cwd), "Prompt marking that enables the 'jump_to_prompt' keybinding", cmd/ctrl+triple-click selects a command's output, alt/option+click moves the cursor within a prompt, bar cursor at prompt; 1.3.0 added "a much more complete and accurate implementation of OSC 133" plus the `click-events`/`cl=line` Semantic-Prompts extensions (native in Fish 4.1+ and Nushell 0.111+). `jump_to_prompt`: "Jump the viewport forward or back by the given number of prompts. Requires shell integration". Command-finish notification: `notify-on-command-finish` (never default / unfocused / always, since 1.3.0) with `notify-on-command-finish-action` (bell default and/or notify) and `notify-on-command-finish-after` (default 5 s). Status line / command duration: not documented.
- Source: *Shell Integration — Ghostty Features* | https://ghostty.org/docs/features/shell-integration | no date shown
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13
- Source: *1.3.0 Release Notes — Ghostty* | https://ghostty.org/docs/install/release-notes/1-3-0 | 2026-03-09

**kitty** — `shell_integration` option (0.24.0+) injects OSC 133 marks into zsh/fish/bash (bundled scripts emit `133;A/C/D`); documented features: "Jump to the previous/next prompt in the scrollback (ctrl+shift+z / ctrl+shift+x)", "Open the output of the last command in a pager such as less (ctrl+shift+g)", click-to-move cursor at prompt, cwd/command in tab title. Command-finish: `notify_on_cmd_finish` (0.32.0 [2024-01-19]) — desktop notification with never/unfocused/invisible/always policies, duration threshold (default 5 s), and actions notify/bell/custom command. Status line / duration: not built-in.
- Source: *Shell integration — kitty* | https://sw.kovidgoyal.net/kitty/shell-integration/ | no date shown
- Source: *kitty Changelog* | https://sw.kovidgoyal.net/kitty/changelog/ | 0.32.0 [2024-01-19]

**WezTerm** — "OSC 133 Escape sequence to define Input, Output and Prompt zones", plus OSC 7 (working directory) and OSC 1337 (user vars: WEZTERM_PROG/USER/HOST); Fedora/Debian/Arch packages auto-enable for bash/zsh. `ScrollToPrompt` action jumps to prompt zones ("not bound by default"). Command-finish notification: **not built-in** (no OSC 99; toast notifications exist via OSC 9/OSC 777 only). Status line: `update-status` event + `window:set_left_status/right_status` (e.g. for battery/cwd); command duration not built-in.
- Source: *Shell Integration — WezTerm* | https://wezterm.org/shell-integration.html | no date shown
- Source: *ScrollToPrompt — WezTerm* | https://wezterm.org/config/lua/keyassignment/ScrollToPrompt.html | no date shown
- Source: *update-status — WezTerm* | https://wezterm.org/config/lua/window-events/update-status.html | no date shown

**Alacritty** — **No OSC 133 support**: the escapes manpage lists supported OSCs (0, 2, 4, 8, 10–12, 50, 52, 104, 110–112) without 133; tracking issue #5850 open, PR #5860 closed unmerged. Consequently no prompt jumping, no command-finish notify, no status line.
- Source: *alacritty-escapes(7)* | https://raw.githubusercontent.com/alacritty/alacritty/master/extra/man/alacritty-escapes.7.scd | no date shown
- Source: *Consider adding OSC 133 · Issue #5850* | https://github.com/alacritty/alacritty/issues/5850 | 2022-02-03 (open)

**iTerm2** — Shell integration scripts emit OSC 133 (`133;A` prompt start, `B` prompt end, `C` command end, `D;status` finished); marks auto-added at each prompt, navigated with Cmd-Shift-Up/Down; Cmd-Up/Down jumps between commands; Command Info shows "how long it ran and its exit status"; "Select Output of Last Command"; command history per user+host; "Alert on next mark" (Cmd-Opt-A) fires a **modal alert when the prompt returns** (3.5.0 added alerts for offscreen sessions). No status-bar command-duration component (duration shown in Command Info panel).
- Source: *Shell Integration — iTerm2* | https://iterm2.com/documentation-shell-integration.html | no date shown
- Source: *iTerm2 shell integration script (zsh)* | https://iterm2.com/shell_integration/zsh | script version 14, no date shown

**Warp** — No OSC 133 documented; Warp uses its own shell integration ("Warpify") that groups each command+output into a **Block** and replaces the prompt by default. Block navigation = jump between commands. Command-finish notification: desktop notifications "when a command completes after a configurable number of seconds" (default threshold 30 s). Command History stores "rich information like exit code, directory, thread, time to finish running" (duration on hover, changelog 2026-05-14). No persistent status line.
- Source: *Terminal Blocks — Warp* | https://docs.warp.dev/terminal/blocks/ | no date shown
- Source: *Desktop Notifications — Warp* | https://docs.warp.dev/terminal/more-features/notifications/ | no date shown
- Source: *Warp Changelog* | https://docs.warp.dev/changelog/ | 2026-05-14

**Windows Terminal** — OSC 133 support documented: "The relevant supported shell integration sequences as of Terminal v1.18 are: OSC 133 A (FTCS_PROMPT), B (FTCS_COMMAND_START), C (FTCS_COMMAND_EXECUTED), D with ExitCode (FTCS_COMMAND_FINISHED)"; marks color-coded by exit status; also smart tab titles (cwd, git branch via OSC 7 / user sequences). Prompt jumping exists via shell-integration marks (keybindings configurable). Command-finish notification: **not built-in**. Status line: not built-in.
- Source: *Windows Terminal shell integration* | https://learn.microsoft.com/en-us/windows/terminal/tips-and-tricks | page updated 2025-08-21
- Source: *Tips & tricks (shell integration section)* | https://learn.microsoft.com/en-us/windows/terminal/tips-and-tricks | "as of Terminal v1.18"

**Best in class (D3):** **iTerm2** (deepest: OSC 133 marks + jumping + command info with duration + finish alert + command history) and **Ghostty** (OSC 133 + `jump_to_prompt` + tunable `notify-on-command-finish` family). **Alacritty** is the notable laggard (no OSC 133 at all).

**Contradictions / uncertainty:** Warp has no documented OSC 133 (proprietary Warpify instead) — a deliberate divergence. WezTerm's `ScrollToPrompt` exists but is unbound by default. WT's "as of v1.18" phrasing implies the list may have grown (newer releases mention shell-integration fixes).

---

## D4 — Session restore (layout + cwd, reattach to processes)

**Ghostty** — `window-save-state` (default/never/always): "saving and restoring window state. Window state includes their position, size, tabs, splits, etc. Some window state requires shell integration, such as preserving working directories" — "currently only supported on macOS. This has no effect on Linux." macOS also has undo/redo of closed windows/tabs/splits (1.2.0). No reattach to running processes and no built-in multiplexer integration (not a multiplexer; tmux runs as a normal app).
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13

**kitty** — Session files (`kitty --session`, `startup_session`): "define kitty windows, tabs and what programs to run in them as well as how to layout the windows"; `save_as_session` saves OS windows, tabs, running programs, working directories (including ssh sessions, "preserving the remote working directory and even the currently running program"). Remote control via `kitty @`/`--listen-on` socket. **No process reattach**: FAQ — kitty does "all of what tmux does… with the exception of remote persistence"; sessions *re-run* programs.
- Source: *Sessions — kitty* | https://sw.kovidgoyal.net/kitty/sessions/ | no date shown
- Source: *FAQ — kitty* | https://sw.kovidgoyal.net/kitty/faq/ | no date shown

**WezTerm** — Multiplexer reattach: "Multiplexing in wezterm is based around the concept of multiplexing domains" — unix/SSH/TLS domains; `wezterm connect <domain>` attaches; `wezterm start --attach`; `wezterm-mux-server` keeps running, and "the window will instead be restored when you next connect to that multiplexer server" (changelog). Workspaces ("Every MuxWindow is associated with a workspace") with `gui-startup`/`mux-startup` layout scripts; automatic save/restore of the local layout on quit is **not** documented (requires a running mux server).
- Source: *Multiplexing — WezTerm* | https://wezterm.org/multiplexing.html | no date shown
- Source: *Workspaces / Sessions — WezTerm recipes* | https://wezterm.org/recipes/workspaces.html | no date shown

**Alacritty** — No session restore; `--working-directory` CLI + `general.working_directory` + IPC `create-window --working-directory` cover per-start cwd only; no detach/attach (processes die with the window); reattachment is delegated to external multiplexers (issue #8454).
- Source: *alacritty(1)* | https://raw.githubusercontent.com/alacritty/alacritty/master/extra/man/alacritty.1.scd | no date shown
- Source: *Alacritty + Tmux + resurrect · Issue #8454* | https://github.com/alacritty/alacritty/issues/8454 | 2025-01-27 (closed)

**iTerm2** — **Server-based reattach**: "Session restoration works by running your jobs within long-lived servers rather than as child processes of iTerm2. If iTerm2 crashes or upgrades, the servers keep going. When iTerm2 restarts, it searches for running servers and connects to them" (macOS window restoration preserves content + scrollback). Window Arrangements: "take a snapshot of your open windows, tabs, and panes… restore… or have it automatically restored when you start iTerm2" (also Load Arrangement From File with contents). tmux integration (`tmux -CC attach`) reopens windows in the same state.
- Source: *Session Restoration — iTerm2* | https://iterm2.com/documentation-restoration.html | no date shown
- Source: *Documentation (one-page) — iTerm2* (arrangements) | https://iterm2.com/documentation-one-page.html | no date shown
- Source: *tmux Integration — iTerm2* | https://iterm2.com/documentation-tmux-integration.html | no date shown

**Warp** — Session restoration: "quickly pick up where you left off in your previous terminal session" — restores windows, tabs, panes, recent Blocks; enabled by default (`restore_session` default true, SQLite-backed); working-directory restore configurable ("previous session's directory" default). **No reattach**: quitting closes sessions (a quit warning lists running processes).
- Source: *Session Restoration — Warp* | https://docs.warp.dev/terminal/sessions/session-restoration/ | no date shown
- Source: *Quit warning — Warp* | https://docs.warp.dev/terminal/more-features/quit-warning/ | no date shown

**Windows Terminal** — "Window/pane layouts can now be saved upon closing, and will be restored upon relaunch" (v1.12, 2021-10-20, #10972); v1.13 (2022) remembers tab titles + maximize/focus state and "save and restore your last opened window, position and all" (Settings > Startup); v1.21 (2024-05-07) "remember and restore the contents of the screen" with the "Open windows from a previous session" startup option. Reattach to running processes: **not supported** (profiles are re-launched).
- Source: *Windows Terminal GitHub releases* | https://github.com/microsoft/terminal/releases | v1.12 2021-10-20; v1.13 2022-05-24; v1.21 2024-05-07

**Best in class (D4):** **iTerm2** — the only one of the seven that reattaches to *running processes* across crash/quit via long-lived servers. **WezTerm** (multiplexer reattach) is second; WT/Warp/Ghostty restore layout/state but not processes.

**Contradictions / uncertainty:** "Session restore" means different things: state restore (WT, Warp, Ghostty), re-run definitions (kitty session files), process reattach (iTerm2, WezTerm mux). Alacritty has none. WT's restore does not reattach; WezTerm's local-layout auto-restore is not documented (requires a mux server).

---

## D5 — Theming & fonts (per-pane fonts, theme library, hot reload, min contrast)

**Ghostty** — Global font config (`font-family`/`font-size`/`font-feature`…); no per-pane font option (a theme file "can set any valid configuration option" including fonts, but fonts stay global). Themes: "Ghostty ships with hundreds of built-in themes" (sourced from iterm2-color-schemes, updated weekly; list via `ghostty +list-themes`); `theme` accepts built-in/custom name or absolute path; `light:name,dark:name` auto-switching. Config reload is **manual**: `reload_config` (default ctrl+shift+, / cmd+shift+,); "Some configuration options cannot be reloaded at runtime." Minimum contrast: **`minimum-contrast`** — "The minimum contrast ratio between the foreground and background colors… as defined by the WCAG 2.0 specification" (ratio 1–21; docs suggest 1.1 for invisible text, 3+ for readability; since 1.3.0).
- Source: *Color Theme — Ghostty Features* | https://ghostty.org/docs/features/theme | no date shown
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13

**kitty** — Per-pane fonts: **no** ("all sub-windows in the same OS window must have the same font size"). Themes: themes kitten with "over three hundred pre-built themes" (kitty-themes, since 0.23.0), live preview, auto light/dark since 0.38.0. Hot reload: config auto-reloads when modified (`auto_reload_config`, added 0.47.0; manual `ctrl+shift+f5`). Minimum contrast: `text_fg_override_threshold` — "A value with the suffix ratio represents the minimum accepted contrast ratio… to meet WCAG level AA a value of 4.5 ratio can be provided."
- Source: *Changing kitty colors — kitty* | https://sw.kovidgoyal.net/kitty/kittens/themes/ | no date shown
- Source: *Configuration — kitty* | https://sw.kovidgoyal.net/kitty/conf/ | no date shown
- Source: *Remote control — kitty* | https://sw.kovidgoyal.net/kitty/remote-control/ | no date shown

**WezTerm** — Per-pane fonts: **no** (global `config.font`; `font_rules` vary by style attributes only). Themes: "WezTerm ships with over 700 color schemes available from iTerm2-Color-Schemes, base16, Gogh and terminal.sexy"; official gallery "1001 Color schemes". Hot reload: config file watched by default (`automatically_reload_config`; CTRL+SHIFT+R). Minimum contrast: `text_min_contrast_ratio` and `reverse_video_cursor_min_contrast` — marked "Since: Nightly Builds Only" (docs cite WCAG 2.0 AA 4.5:1).
- Source: *Colors & Appearance — WezTerm* | https://wezterm.org/config/appearance.html | no date shown
- Source: *Color Schemes — WezTerm gallery* | https://wezterm.org/colorschemes/index.html | no date shown
- Source: *text_min_contrast_ratio — WezTerm* | https://wezterm.org/config/lua/config/text_min_contrast_ratio.html | no date shown

**Alacritty** — Per-pane fonts: **no** (no panes at all; `[font]` is global; per-window IPC `-o` overrides exist). Themes: no built-in library; `general.import` (docs example imports `alacritty-theme/themes/gruvbox_dark.toml`); de-facto collection is the third-party alacritty/alacritty-theme. Hot reload: `general.live_config_reload` default `true` (+ IPC `alacritty msg config`). Minimum contrast: **not supported**.
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown
- Source: *alacritty/alacritty-theme* | https://github.com/alacritty/alacritty-theme | created 2023-01-19

**iTerm2** — Fonts are per-profile (Regular + Non-ASCII font), and since splits can use "Split… with Current Profile", mixed-profile (hence mixed-font) panes are possible — the closest thing to per-pane fonts in this set. Themes: itermcolors import/export + online color gallery. Hot reload: none live; "Load settings from a custom folder or URL" loads at startup (saves on quit). Minimum contrast: documented slider — "If you enable minimum contrast… iTerm2 will guarantee a minimum level of brightness difference between the foreground and background color of every character."
- Source: *Fonts — iTerm2* | https://iterm2.com/documentation-fonts.html | no date shown
- Source: *General Usage — iTerm2* | https://iterm2.com/documentation-general-usage.html | no date shown

**Warp** — Per-pane fonts: **no** (global `[appearance.text]`). Themes: ~21 built-in defaults; custom themes are .yaml in `~/.warp/themes`; official repo warpdotdev/themes holds 136 standard + 179 base16 = 315 files (GitHub API count, not a docs figure). Hot reload: "Warp watches `settings.toml` for changes and applies them instantly." Minimum contrast: `enforce_minimum_contrast` (never / only_named_colors / always; default only_named_colors).
- Source: *Terminal themes — Warp* | https://docs.warp.dev/terminal/appearance/themes/ | no date shown
- Source: *Settings file — Warp* | https://docs.warp.dev/terminal/settings/ | no date shown
- Source: *warpdotdev/themes* | https://github.com/warpdotdev/themes | no date shown

**Windows Terminal** — Per-pane fonts: per-profile (`font`/`fontFace` in profile settings), and panes run per-profile, so different panes can show different fonts. Themes: 16 built-in color schemes (defaults.json, incl. Campbell, One Half, Solarized, Tango, Vintage, CGA, IBM 5153…); a separate "themes" feature (1.21+) with reserved built-ins `dark`/`light`/`system` + custom themes. Hot reload: settings.json is live-reloaded. Minimum contrast: **not supported**.
- Source: *Windows Terminal color schemes* | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/color-schemes | no date shown
- Source: *Windows Terminal themes* | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/themes | no date shown (Preview-era page)
- Source: *Windows Terminal defaults.json* | https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalApp/defaults.json | 16 built-in schemes (fetched 2026-08-13)

**Best in class (D5):** **WezTerm** for theme library size (700+/1001 schemes) + auto-reload + (nightly) WCAG min-contrast; **iTerm2** for minimum contrast (stable, documented) and per-profile fonts. No terminal offers true per-pane independent fonts; kitty explicitly forbids it within one OS window.

**Contradictions / uncertainty:** WezTerm min-contrast is nightly-only vs iTerm2's stable slider. WT "themes" (window chrome) vs "color schemes" (palette) are two different systems. Warp theme count comes from the GitHub repo, not a docs page.

---

## D6 — Copy/paste (copy-on-select default, block selection, alt-screen)

**Ghostty** — `copy-on-select` config: "Whether to automatically copy selected text to the clipboard. true will prefer to copy to the selection clipboard… The default value is **true** on Linux and macOS" (`clipboard` value copies to both; middle-click paste always uses the selection clipboard). Related: `clipboard-read`/`clipboard-write`, `clipboard-trim-trailing-spaces`, `selection-clear-on-typing`/`-on-copy`, `selection-word-chars`. Rectangular/block selection: exists at the engine level (libghostty `OPT_RECTANGLE` gesture) and via option+drag on macOS (issue #2537); 1.2.0 fixed rectangular-selection rendering — but **no user-facing doc explains how to trigger it**; alt-screen behavior not documented in user docs.
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13
- Source: *libghostty: Selection API* | https://libghostty.tip.ghostty.org/group__selection.html | no date shown
- Source: *Rectangular selection — Issue #2537* | https://github.com/ghostty-org/ghostty/issues/2537 | no date shown
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13

**kitty** — `copy_on_select` default **no** ("With this set to clipboard, selecting text with the mouse will cause the text to be copied to clipboard"; can target a private buffer `a1` + `paste_from_buffer`). Rectangular selection via mouse mapping `mouse_map ctrl+alt+left press ungrabbed mouse_selection rectangle`; double/triple-click word/line; `strip_trailing_spaces smart`. Alt-screen: not documented in primary sources.
- Source: *Configuration — kitty* | https://sw.kovidgoyal.net/kitty/conf/ | no date shown

**WezTerm** — Selection copies on mouse release by default binding (`CompleteSelectionOrOpenLinkAtMouseCursor` → "ClipboardAndPrimarySelection"); changelog: "Mouse based selection once again copies to both the clipboard and the primary selection." Block selection: ALT+click/ALT+drag, plus CopyMode block selection (Ctrl+V). Alt-screen: most default mouse bindings still apply (`alt_screen='Any'`); no copy-on-select toggle per se (it's the default binding).
- Source: *Mouse Binding — WezTerm* | https://wezterm.org/config/mouse.html | no date shown
- Source: *Copy Mode — WezTerm* | https://wezterm.org/copymode.html | no date shown

**Alacritty** — `[selection] save_to_clipboard` default **false** (X11 primary-selection convention still applies on Linux). Block selection: Ctrl+drag with mouse, `ToggleBlockSelection` in vi mode (0.4.0 changelog). Alt-screen: not documented (only the Shift-bypass for mouse-capturing apps).
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown
- Source: *Alacritty CHANGELOG* | https://raw.githubusercontent.com/alacritty/alacritty/master/CHANGELOG.md | version 0.4.0

**iTerm2** — **Copy-on-select ON by default**: "text is copied to the clipboard immediately upon being selected" (General Usage, same wording in 3.5/3.6 docs). Rectangular: "If you hold cmd and option while selecting, a rectangular selection will be made"; Copy Mode has Toggle rectangular selection (`<C-v>`); non-contiguous selection via Cmd+drag. Alt-screen: mouse-reporting apps can block selection, "but pressing option will temporarily disable it so you can make a selection".
- Source: *General Usage — iTerm2* | https://iterm2.com/documentation-general-usage.html | no date shown

**Warp** — `copy_on_select` default **true** ("Whether text is automatically copied to the clipboard when selected. Default: true"); changelog notes "'Copy on Select' now works within alt-screens." Rectangular/column selection via CMD-OPT (macOS) / CTRL-ALT drag; double-click smart selection (URLs, paths, emails, IPs, floats). OSC 52 clipboard access configurable (default deny).
- Source: *All settings reference — Warp* | https://docs.warp.dev/terminal/settings/all-settings/ | no date shown
- Source: *Text selection — Warp* | https://docs.warp.dev/terminal/more-features/text-selection/ | no date shown
- Source: *Warp Changelog* | https://docs.warp.dev/changelog/ | (copy-on-select in alt-screens entry)

**Windows Terminal** — `copyOnSelect` global setting, **default false** ("Use the copyOnSelect global setting to automatically copy newly selected text"; "Default value: false"); note: keyboard-driven selection changes don't auto-copy even when enabled. Block (rectangular) selection supported; Shift expands a selection to a point. Alt-screen: not documented.
- Source: *Windows Terminal interaction settings* | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/interaction | no date shown
- Source: *Text selection — Windows Terminal* | https://learn.microsoft.com/en-us/windows/terminal/tips-and-tricks (text selection section) | no date shown

**Best in class (D6):** **iTerm2** (default-on copy + rectangular + non-contiguous + alt-screen workaround) and **Warp** (default-on copy that works in alt-screens + rectangular). Defaults split the field 3-on (iTerm2, Warp, Ghostty) / 4-off (kitty, Alacritty, WT) with WezTerm effectively on via default binding.

**Contradictions / uncertainty:** Warp's docs say copy-on-select "works within alt-screens" while iTerm2 needs Option held in alt-screen — opposite documented behaviors. Ghostty's copy-on-select default is "true on Linux and macOS" per the config reference (worth noting the Linux/macOS nuance).

---

## D7 — Tab/split ergonomics (drag reorder, drag to new window, ratio persistence, pane zoom)

**Ghostty** — Tabs and splits built in; actions `goto_tab`/`move_tab`/`toggle_tab_overview`, `toggle_split_zoom` (a zoomed split "will take up the entire space in the current tab, hiding other splits"), `split-preserve-zoom` (1.3.0), `resize_split`, `equalize_splits`. **Split drag & drop on macOS since 1.3.0**: "You can now reorder splits on macOS by dragging them. When you hover near the top of a split, a grab handle appears… You can also drag splits out of windows or into new tabs" (PR #10090; GTK planned). Tab drag-reorder/drag-to-new-window relies on macOS native tabs; split-ratio persistence across restarts only via macOS `window-save-state` — not otherwise documented.
- Source: *1.3.0 Release Notes — Ghostty* | https://ghostty.org/docs/install/release-notes/1-3-0 | 2026-03-09
- Source: *Ghostty Keybind Reference* | https://ghostty.org/docs/config/keybind/reference | fetched 2026-08-13
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13

**kitty** — Tab dragging: "Allow dragging tabs (opt:drag_threshold) in the tab bar to re-order, move to another OS Window or detach" (0.46.0 [2026-03-11]); window (pane) dragging: "drag and drop of windows to re-arrange them, move them to another tab/OS Window or detach them into a new OS Window" (0.47.0 [2026-05-19]); mouse border-resizing of splits (0.46.0, `window_drag_tolerance`). Split ratios: `launch --bias` ("what fraction of available space the window takes", 0.36.0); layout state persisted in session files. Pane zoom: `toggle_layout stack` ("Useful to 'zoom' a window temporarily"), default `ctrl+alt+z`.
- Source: *kitty Changelog* | https://sw.kovidgoyal.net/kitty/changelog/ | 0.46.0 [2026-03-11]; 0.47.0 [2026-05-19]
- Source: *The launch command — kitty* | https://sw.kovidgoyal.net/kitty/launch/ | no date shown

**WezTerm** — Pane zoom: `TogglePaneZoomState` (default CTRL+SHIFT+Z): "A Zoomed pane takes up all available space in the tab… Switching its zoom state off will restore the prior split arrangement." Tab reorder: keyboard only (`MoveTabRelative`); **mouse drag-reorder and drag-tab-to-new-window are not documented** (open issue #549 since 2021). Split-ratio persistence: not documented (`AdjustPaneSize` interactive only).
- Source: *TogglePaneZoomState — WezTerm* | https://wezterm.org/config/lua/keyassignment/TogglePaneZoomState.html | no date shown
- Source: *Ability to drag re-order tabs, panes · Issue #549* | https://github.com/wezterm/wezterm/issues/549 | 2021-03-16 (open)

**Alacritty** — No tabs/splits at all on Linux/Windows (multi-window only); macOS gets native OS window tabs (Cmd+T, CreateNewTab). Everything else n/a.
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown
- Source: *Split terminal view · Issue #7302* | https://github.com/alacritty/alacritty/issues/7302 | 2023-10-19 (closed, not implemented)

**iTerm2** — "drag and drop tabs to reorder them within a window… drag a tab from a window into a new window by dropping it outside any iTerm2 window's tab bar." Splits (cmd-d/cmd-shift-d); panes resized by dragging the divider. Pane maximize: "You can 'maximize' the current pane--hiding all others in that tab--with cmd-shift-enter." Split-ratio persistence: not explicitly documented (Window Arrangements save tabs+splits; 3.5.0 improved "Arrange split panes preserve horizontality").
- Source: *Documentation (one-page) — iTerm2* | https://iterm2.com/documentation-one-page.html | no date shown

**Warp** — Tab reorder by dragging; "Detach a tab into its own window — drag a tab out of the tab bar and drop it in empty space," and drag onto another window's tab bar (keeps running session/panes); cross-window tab dragging on stable for Windows+macOS (changelog 2026-06-24). Split panes with drag-and-drop between tabs; "Toggle Maximize Pane" (CMD-SHIFT-ENTER); double-click a divider redistributes panes evenly (2026-05-20); pane layout saved/restored on quit (2026-06-17). Custom split-ratio persistence: not explicitly documented.
- Source: *Tabs — Warp* | https://docs.warp.dev/terminal/windows/tabs/ | no date shown
- Source: *Split panes — Warp* | https://docs.warp.dev/terminal/windows/split-panes/ | no date shown
- Source: *Warp Changelog* | https://docs.warp.dev/changelog/ | 2026-06-24, 2026-06-17, 2026-05-20

**Windows Terminal** — Tab drag reorder supported (long-standing); drag a tab into another window ("Terminal should no longer crash when you drag a small tab into a larger window", v1.24/1.25 2026-07-16); right-click context menu with split/move/zoom/close for panes (v1.23 2025-08-26). Pane zoom: `togglePaneZoom` action — "expands the focused pane to fill the entire contents of the window" (not bound by default; command palette). Split ratio persistence: not documented.
- Source: *Windows Terminal panes* | https://learn.microsoft.com/en-us/windows/terminal/panes | no date shown
- Source: *Windows Terminal actions* | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions | no date shown
- Source: *Windows Terminal GitHub releases* | https://github.com/microsoft/terminal/releases | v1.24/1.25 2026-07-16; v1.23 2025-08-26

**Best in class (D7):** **kitty** (drag tabs *and* drag panes between windows, both recent) and **Warp** (drag tabs out to new windows, drag panes between tabs, maximize, layout save). **WezTerm** is the notable gap — drag-reorder is an open request since 2021.

**Contradictions / uncertainty:** WezTerm's lack of mouse tab-dragging vs kitty/Warp/WT/iTerm2 supporting it is the clearest split. Ghostty's tab-drag state is undocumented in primary docs on research day. "Split ratio persistence" is only loosely documented anywhere (Warp saves pane layout on quit; kitty persists layout state in session files; others: not documented).

---

## D8 — Quick Terminal / dropdown

**Ghostty** — `toggle_quick_terminal`: "The quick terminal, also known as the 'Quake-style' or drop-down terminal, is a terminal window that appears on demand from a keybinding, often sliding in from a screen edge such as the top." No default keybind — bind e.g. `global:cmd+backquote=toggle_quick_terminal` (the `global:` prefix is macOS-only and needs accessibility permission). State "is preserved between appearances"; one instance only; not restored on app restart. Configs: `quick-terminal-position/-size/-screen/-animation-duration/-autohide/-space-behavior/-keyboard-interactivity`. Linux: Wayland + wlr-layer-shell-v1 only (GNOME unsupported).
- Source: *Ghostty Keybind Reference* | https://ghostty.org/docs/config/keybind/reference | fetched 2026-08-13
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13

**kitty** — `quick-access-terminal` kitten, page literally titled "Make a Quake like quick access terminal" (0.42.0 [2025-05-11]): "a quick access kitty window will show up at the top of your screen. Run it again, and the window will be hidden"; configurable edge/position, `background_opacity`, `hide_on_focus_loss`, `grab_keyboard`, `start_as_hidden`, instance groups; uses the panel kitten.
- Source: *Make a Quake like quick access terminal — kitty* | https://sw.kovidgoyal.net/kitty/kittens/quick-access-terminal/ | no date shown
- Source: *kitty Changelog* | https://sw.kovidgoyal.net/kitty/changelog/ | 0.42.0 [2025-05-11]

**WezTerm** — **No built-in** quake/dropdown (open feature request #1751); scripting primitives exist: `wezterm.mux.spawn_window { position = … }`, `wezterm start --position`, `window:set_position` (with a Wayland placement caveat).
- Source: *Dropdown/Guake/Visor/hotkey terminal · Issue #1751* | https://github.com/wezterm/wezterm/issues/1751 | 2022-03-23 (open)
- Source: *spawn_window — WezTerm* | https://wezterm.org/config/lua/wezterm.mux/spawn_window.html | no date shown

**Alacritty** — **None** (request #1119 closed 2018; workarounds like tdrop/hammerspoon shared in-thread; only `window.level="AlwaysOnTop"` since 0.15.0 as a related knob).
- Source: *Support drop down windows · Issue #1119* | https://github.com/alacritty/alacritty/issues/1119 | 2018-02-15 (closed)

**iTerm2** — **Dedicated Hotkey Window**: "A dedicated hotkey window is a window that is associated with a profile and has a hotkey attached to it. By pressing the hotkey, the window opens or closes. This is similar to the old Visor app"; options: Pin hotkey window, auto-reopen on app activation, animate show/hide, Floating window (over full-screen apps), multiple hotkeys, double-tap-modifier hotkeys; plus Toggle All Windows and Session Hotkeys.
- Source: *Hotkeys — iTerm2* | https://iterm2.com/documentation-hotkey.html | no date shown

**Warp** — **Global Hotkey**: "a configurable shortcut that can show/hide a dedicated window or all windows on your chosen desktop regardless of whether the app is focused," including "a dedicated Quake-style drop-down window"; dedicated window supports pinned position, width/height ratio relative to screen, autohide on loss of keyboard focus (macOS only).
- Source: *Global Hotkey — Warp* | https://docs.warp.dev/terminal/windows/global-hotkey/ | no date shown

**Windows Terminal** — **Quake Mode**: v1.9 (2021-05-25) "Terminal now supports pinning an instance to the top of the screen that you can summon at any time (colloquially referred to as 'Quake Mode')"; v1.10 added tray icon for quake; v1.11 rebinding; docs: `wt -w _quake` opens a "quake window". (v1.23 2025-08-26 also touched quake behavior.)
- Source: *Windows Terminal GitHub releases* | https://github.com/microsoft/terminal/releases | v1.9 2021-05-25
- Source: *Command-line arguments — Windows Terminal* | https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments | no date shown

**Best in class (D8):** **iTerm2** (longest-matured hotkey window with pin/animate/floating) and **kitty** (0.42+ dedicated quake kitten with rich placement/opacity config). **WezTerm** and **Alacritty** have none.

**Contradictions / uncertainty:** WezTerm's position-scripting primitives could replicate a dropdown but there's no first-class feature; kitty's quake is a kitten (needs a keybinding wired up), Ghostty's is a built-in action with a documented global binding.

---

## D9 — Accessibility (screen readers, min contrast, focus indicators)

**Ghostty** — macOS **read-only** accessibility API integration since 1.2.0 (PR #7601): "Read-only accessibility API integration allows screen readers to read Ghostty's structure and contents… requires accessibility permissions, so it is opt-in"; no GTK/AT-SPI or Linux screen-reader support appears in docs or changelogs 1.0.1–1.3.1. Minimum contrast: `minimum-contrast` (WCAG 2.0, since 1.3.0). Focus-visibility aids: `focus-follows-mouse`, `unfocused-split-opacity`, border colors.
- Source: *1.2.0 Release Notes — Ghostty* | https://ghostty.org/docs/install/release-notes/1-2-0 | 2025-09-15
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13

**kitty** — No screen-reader support documented; macOS accessibility protocol implemented to provide selected text ("Allow 'Speak selection' (Option+Esc) to work properly", PR #5359 merged 2022-08-09). Focus indicators: `active_border_color`/`inactive_border_color`/`bell_border_color`; every action mappable → full keyboard navigation. Min contrast: `text_fg_override_threshold` (WCAG AA 4.5).
- Source: *PR #5359 — kitty* | https://github.com/kovidgoyal/kitty/pull/5359 | merged 2022-08-09
- Source: *Configuration — kitty* | https://sw.kovidgoyal.net/kitty/conf/ | no date shown

**WezTerm** — Screen-reader support **not documented** (tracking issue #913 for visually-impaired users open). Focus indicators: inactive panes "dimmed and de-saturated" (`inactive_pane_hsb`); `pane_focus_follows_mouse`. Min contrast: `text_min_contrast_ratio`/`reverse_video_cursor_min_contrast` (nightly-only; WCAG 4.5:1 cited).
- Source: *Accessibility for the visually impaired · Issue #913* | https://github.com/wezterm/wezterm/issues/913 | 2021-06-30 (open)
- Source: *Colors & Appearance — WezTerm* | https://wezterm.org/config/appearance.html | no date shown

**Alacritty** — Screen-reader support **not supported** (issue #5933 closed 2022-03-07 without implementation). Focus indicator: `unfocused_hollow` cursor (default true). Keyboard navigation: vi mode (Ctrl+Shift+Space) with full cursor motion, inline/semantic search, selection modes. Min contrast: none.
- Source: *accessibility: support screen readers · Issue #5933* | https://github.com/alacritty/alacritty/issues/5933 | closed 2022-03-07
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown

**iTerm2** — Screen-reader (VoiceOver) support **not documented** in official docs/changelogs (only incidental bug fixes: "Fix accessibility bugs in macOS 15" 3.6.x; "Fix a bug where accessibility coordinates were wrong" 3.5.0). Documented: Minimum contrast slider + Light/Dark High Contrast color themes.
- Source: *Documentation (one-page) — iTerm2* | https://iterm2.com/documentation-one-page.html | no date shown
- Source: *Downloads — iTerm2* (changelogs) | https://iterm2.com/downloads.html | 3.5.0 built 2024-05-17

**Warp** — Partial, self-described WIP: "Warp's accessibility features include VoiceOver support, voice input, and configurable verbosity" (macOS); caveats: "There's currently no way to navigate between different UI elements using VO key combinations" and no keyboard-accessible UI navigation yet; terminal features keyboard-accessible (Command Palette). Min contrast: `enforce_minimum_contrast`. Windows/Linux screen readers: not documented.
- Source: *Accessibility — Warp* | https://docs.warp.dev/terminal/more-features/accessibility/ | no date shown

**Windows Terminal** — Strongest documented screen-reader story of the seven: "a shared UI Automation provider was introduced in 2019, enabling accessibility tools to navigate and read contents from the terminal area"; UIA events dispatched since 2020 (cursor position, text output, selection); UIA notifications with text payloads since 2022; dedicated "Screen Readers" section in the accessibility doc.
- Source: *Windows Terminal Accessibility* | https://learn.microsoft.com/en-us/windows/terminal/accessibility (a.k.a. devblogs/microsoft/commandline — see a11y doc) | no date shown; UIA timeline: 2019 provider, 2020 events, 2022 notifications
- Source: *Terminal Accessibility (UIA) overview* | https://learn.microsoft.com/en-us/windows/terminal/terminal-accessibility | no date shown (both fetched 2026-08-13)

**Best in class (D9):** **Windows Terminal** — the only one with documented, continuously-improving UIA screen-reader support (2019→2022 timeline). All others are undocumented or partial (kitty/Warp: macOS speak-selection / VoiceOver WIP; Ghostty: macOS read-only AX).

**Contradictions / uncertainty:** Warp claims VoiceOver support but self-documents navigation limits; kitty ships only "speak selection"; iTerm2's VoiceOver status is undocumented (secondary reviews exist). Min-contrast settings exist only in iTerm2 (stable), WezTerm (nightly), kitty (ratio threshold), Warp (enforce).

---

## D10 — Notifications (command-finish notify, bell options)

**Ghostty** — `notify-on-command-finish` (1.3.0): never (default) / unfocused / always; `notify-on-command-finish-action` (bell and/or notify desktop notification, combinable/negatable e.g. `no-bell,notify`); `notify-on-command-finish-after` (default 5 s); requires shell integration or a shell sending OSC 133 natively. Bell: `bell-features` — system (system alert sound), audio (`bell-audio-path`/`bell-audio-volume`, since 1.2.0 GTK / 1.3.0 macOS), attention (macOS dock bounce), title (🔔 in title), border; every feature negatable with `no-` (fully silent bell possible). `desktop-notifications` (default true): "applications running in the terminal can show desktop notifications using certain escape sequences such as OSC 9 or OSC 777"; `progress-style` renders ConEmu OSC 9;4 progress bars.
- Source: *Ghostty Configuration Reference* | https://ghostty.org/docs/config/reference | fetched 2026-08-13
- Source: *1.3.0 Release Notes — Ghostty* | https://ghostty.org/docs/install/release-notes/1-3-0 | 2026-03-09

**kitty** — `notify_on_cmd_finish` (0.32.0 [2024-01-19]): desktop notification when a long command finishes (needs shell integration), policies never/unfocused/invisible/always, min-duration arg, actions notify/bell/notify-bell/custom command (%c/%s). Bell: `enable_audio_bell`, `visual_bell_duration` (animated flash, 0 disables), `visual_bell_color`, `bell_on_tab` (symbol on tab), `command_on_bell`, `window_alert_on_bell` (taskbar flash/dock bounce), `macos_dock_badge_on_bell`, `bell_path` (custom sound). Programs can also raise notifications via kitty's OSC 99 escape code and the `notify` kitten (0.36.0).
- Source: *Configuration — kitty* | https://sw.kovidgoyal.net/kitty/conf/ | no date shown
- Source: *Desktop notifications — kitty* | https://sw.kovidgoyal.net/kitty/desktop-notifications/ | no date shown
- Source: *kitty Changelog* | https://sw.kovidgoyal.net/kitty/changelog/ | 0.32.0 [2024-01-19]

**WezTerm** — Command-finish notify: **not built-in** (OSC 9 / OSC 777 toasts exist; `notification_handling` controls when toasts show: AlwaysShow default, SuppressFromFocused*; `window:toast_notification()` API). Bell: `audible_bell` (SystemBeep default / Disabled), `visual_bell` (background/cursor flash with fade durations and easing incl. CubicBezier), `colors.visual_bell`, Lua `bell` event.
- Source: *audible_bell / visual_bell — WezTerm* | https://wezterm.org/config/lua/config/audible_bell.html and https://wezterm.org/config/lua/config/visual_bell.html | no date shown
- Source: *notification_handling — WezTerm* | https://wezterm.org/config/lua/config/notification_handling.html | no date shown

**Alacritty** — Command-finish notify: **none** (depends on OSC 133, unsupported). Bell: `[bell]` visual bell (animation easing, duration — 0 disables visual bell by default, color), and `bell.command` ("This program is executed whenever the bell is rung", default None).
- Source: *Alacritty Configuration* | https://alacritty.org/config-alacritty.html | no date shown
- Source: *Alacritty CHANGELOG* | https://raw.githubusercontent.com/alacritty/alacritty/master/CHANGELOG.md | version 0.5.0

**iTerm2** — Bell: "Silence bell" (no sound), "Flash visual bell" ("a bell graphic will be flashed"), "Show bell icon in tabs" (Profiles > Terminal). Notifications: "Send Notification Center alerts… when a terminal beeps, has output after a period of silence, or terminates" with a Filter Alerts panel; Triggers can Post Notification, Ring Bell, Show Alert, Bounce Dock Icon. Command-finish: shell-integration "Alert on next mark" modal + 3.5.0 "Add Alert on Marks in Offscreen Sessions"; 3.5.0 added rich notifications via `OSC 1337;Notification=` (message/title/subtitle/image).
- Source: *Triggers — iTerm2* | https://iterm2.com/documentation-triggers.html | no date shown
- Source: *Documentation (one-page) — iTerm2* | https://iterm2.com/documentation-one-page.html | no date shown

**Warp** — Desktop notifications "when a command completes after a configurable number of seconds or when a running command needs you to enter a password" (while another app is focused): `is_long_running_enabled` (default true), `long_running_threshold` (default 30 s), `is_password_prompt_enabled`, `play_notification_sound`, `is_needs_attention_enabled`; custom hooks via OSC 9 / OSC 777. Bell: audible bell exists but disabled by default (`use_audible_bell` default false); no visual/silent bell modes documented.
- Source: *Desktop Notifications — Warp* | https://docs.warp.dev/terminal/more-features/notifications/ | no date shown
- Source: *Audible terminal bell — Warp* | https://docs.warp.dev/terminal/more-features/audible-bell/ | no date shown

**Windows Terminal** — Command-finish notify: **not built-in**. Bell: `bellStyle` — accepts "all", "audible", "window" (flash title bar), "taskbar" (flash taskbar icon), "none"; **default "audible"**.
- Source: *Windows Terminal advanced settings* | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/advanced | no date shown

**Best in class (D10):** **kitty** — deepest command-finish notification policy (threshold + action matrix incl. custom command) plus the widest bell option set and an OSC 99 notification escape for programs. **iTerm2** (Notification Center + rich OSC 1337 notifications) is close second.

**Contradictions / uncertainty:** Default command-finish behavior differs: Ghostty `never` by default, Warp `is_long_running_enabled` true (30 s threshold), kitty needs explicit config. WT/Alacritty/WezTerm have no built-in command-finish notify. Warp's bell is audible-only and off by default — unlike every other terminal with a default-on bell.

---

## Cross-cutting observations

- **OSC 133 support:** kitty (0.24+), WezTerm, Ghostty (1.3+), iTerm2, Windows Terminal (1.18+) — supported; **Alacritty — not supported**; **Warp — not documented** (proprietary Warpify).
- **Live incremental search:** all except **kitty** (deliberate performance choice).
- **Copy-on-select defaults:** ON — iTerm2, Warp, Ghostty (Linux/macOS); OFF — kitty, Alacritty, Windows Terminal; WezTerm copies on mouse release by default binding.
- **Minimum contrast:** iTerm2 (stable slider), kitty (`text_fg_override_threshold`), WezTerm (`text_min_contrast_ratio`, nightly), Warp (`enforce_minimum_contrast`), Ghostty (`minimum-contrast`, 1.3.0); **not present** in Alacritty and Windows Terminal.
- **Quake/dropdown:** Ghostty, kitty (0.42+), iTerm2, Warp, Windows Terminal (1.9+) have first-class features; WezTerm and Alacritty do not. (Ghostty's global quake keybinding is macOS-only; Wayland-only on Linux.)
- **Session/process restore:** only **iTerm2** (server-based reattach) and **WezTerm** (multiplexer reattach) can reattach to running processes; others restore state/layout only (WT, Warp, Ghostty-macOS) or re-run definitions (kitty); Alacritty none.
- **Per-pane fonts:** no terminal supports true per-pane font independence; iTerm2 and WT approximate via per-profile panes.
- **Tab drag-to-reorder:** kitty, iTerm2, Warp, WT yes; WezTerm no (open request); Ghostty via native macOS tabs; Alacritty n/a.
- **Screen readers:** only Windows Terminal documents real UIA support; Warp (macOS VoiceOver, WIP), kitty (speak-selection), Ghostty (macOS read-only AX); others undocumented/none.

## Limitations & uncertainty

- Docs pages rarely carry dates; changelog/release dates are authoritative where given. Ghostty 1.3.0 released 2026-03-09, 1.3.1 on 2026-03-13 (official release-notes pages).
- "Not documented" (used repeatedly) means *no primary source found*, not proof of absence — the docs may lag the software (e.g., WezTerm nightly features, WT Preview features, kitty 0.48-era additions).
- Windows Terminal regex search is in flux (Canary 29558, 2026) and absent from stable docs — treat as future feature, secondary sources only.
- Warp theme-library size (315) and WezTerm scheme count (1001) come from repo/gallery listings rather than prose docs.
- One research subagent (Windows Terminal) failed before completing; its claims were re-derived directly from fetched Microsoft Learn pages and GitHub release JSONs.

## Sources (aggregated, per terminal)

**Ghostty**
- Configuration Reference | https://ghostty.org/docs/config/reference | fetched 2026-08-13
- Keybind Reference | https://ghostty.org/docs/config/keybind/reference | fetched 2026-08-13
- Features (incl. Shell Integration, Color Theme) | https://ghostty.org/docs/features | no date shown
- 1.1.0 Release Notes | https://ghostty.org/docs/install/release-notes/1-1-0 | 2025-01-30
- 1.2.0 Release Notes | https://ghostty.org/docs/install/release-notes/1-2-0 | 2025-09-15
- 1.3.0 Release Notes | https://ghostty.org/docs/install/release-notes/1-3-0 | 2026-03-09
- 1.3.1 Release Notes | https://ghostty.org/docs/install/release-notes/1-3-1 | 2026-03-13
- Terminal API (VT) reference (OSC 8/OSC 133) | https://ghostty.org/docs/vt/reference | no date shown
- libghostty: Selection API | https://libghostty.tip.ghostty.org/group__selection.html | no date shown
- Rectangular selection · Issue #2537 | https://github.com/ghostty-org/ghostty/issues/2537 | no date shown
- Security advisory GHSA-hfg5-8q2c-crhc (write_*_file permissions) | https://github.com/ghostty-org/ghostty/security/advisories/GHSA-hfg5-8q2c-crhc | 2026 (secondary lead)

**kitty**
- Overview | https://sw.kovidgoyal.net/kitty/overview/ | no date
- Configuration | https://sw.kovidgoyal.net/kitty/conf/ | no date
- Changelog | https://sw.kovidgoyal.net/kitty/changelog/ | 0.32.0 [2024-01-19] … 0.48.2 [2026-07-30]
- Shell integration | https://sw.kovidgoyal.net/kitty/shell-integration/ | no date
- Sessions | https://sw.kovidgoyal.net/kitty/sessions/ | no date
- Hints kitten | https://sw.kovidgoyal.net/kitty/kittens/hints/ | no date
- Themes kitten | https://sw.kovidgoyal.net/kitty/kittens/themes/ | no date
- Quick-access-terminal kitten | https://sw.kovidgoyal.net/kitty/kittens/quick-access-terminal/ | no date
- Desktop notifications | https://sw.kovidgoyal.net/kitty/desktop-notifications/ | no date
- FAQ | https://sw.kovidgoyal.net/kitty/faq/ | no date
- Mappable actions | https://sw.kovidgoyal.net/kitty/actions/ | no date
- open_actions | https://sw.kovidgoyal.net/kitty/open_actions/ | no date
- Issue #893 / PR #5359 / kitty-themes repo (URLs inline above)

**WezTerm**
- Shell Integration | https://wezterm.org/shell-integration.html | no date
- Scrollback | https://wezterm.org/scrollback.html | no date
- Copy Mode | https://wezterm.org/copymode.html | no date
- Quick Select | https://wezterm.org/quickselect.html | no date
- Hyperlinks | https://wezterm.org/hyperlinks.html | no date
- Multiplexing | https://wezterm.org/multiplexing.html | no date
- Colors & Appearance | https://wezterm.org/config/appearance.html | no date
- Color scheme gallery ("1001 Color schemes") | https://wezterm.org/colorschemes/index.html | no date
- Escape Sequences | https://wezterm.org/escape-sequences.html | no date
- CLI get-text | https://wezterm.org/cli/cli/get-text.html | no date
- TogglePaneZoomState / ScrollToPrompt / update-status / text_min_contrast_ratio / audible_bell / visual_bell / notification_handling / spawn_window (URLs inline above) | no date
- Issues #549, #1751, #913 (URLs inline above)

**Alacritty**
- Home | https://alacritty.org/ | v0.17.0 downloads
- Configuration | https://alacritty.org/config-alacritty.html | no date (0.17.0-era)
- bindings (0.17.0) | https://alacritty.org/releases/0.17.0/config-alacritty-bindings.html | 0.17.0
- CHANGELOG (master) | https://raw.githubusercontent.com/alacritty/alacritty/master/CHANGELOG.md | versions 0.4.0/0.5.0/0.11.0/0.15.0/0.16.0
- features.md | https://raw.githubusercontent.com/alacritty/alacritty/master/docs/features.md | no date
- alacritty(1) / alacritty-escapes(7) | https://raw.githubusercontent.com/alacritty/alacritty/master/extra/man/alacritty.1.scd and …/alacritty-escapes.7.scd | no date
- Issues #1615, #5850, #5933, #1119, #7302, #8454; PR #5860 (URLs inline above)

**iTerm2**
- Documentation index | https://iterm2.com/documentation.html | no date
- One-page docs | https://iterm2.com/documentation-one-page.html | docs v3.6, no date
- Smart Selection | https://iterm2.com/documentation-smart-selection.html | no date
- Shell Integration | https://iterm2.com/documentation-shell-integration.html | no date
- Session Restoration | https://iterm2.com/documentation-restoration.html | no date
- Hotkeys | https://iterm2.com/documentation-hotkey.html | no date
- Fonts | https://iterm2.com/documentation-fonts.html | no date
- General Usage | https://iterm2.com/documentation-general-usage.html | no date
- Triggers | https://iterm2.com/documentation-triggers.html | no date
- tmux Integration | https://iterm2.com/documentation-tmux-integration.html | no date
- Downloads/changelogs | https://iterm2.com/downloads.html | 3.5.0 (2024-05-17) … 3.6.11 (2026-06-02)
- 3.5.0 changelog | https://iterm2.com/downloads/stable/iTerm2-3_5_0.changelog | 2024-05-17
- zsh shell integration script | https://iterm2.com/shell_integration/zsh | script v14, no date

**Warp**
- Block Find | https://docs.warp.dev/terminal/blocks/find/ | updated 2026-08-11
- Blocks | https://docs.warp.dev/terminal/blocks/ | no date
- Files, Links, & Scripts | https://docs.warp.dev/terminal/more-features/files-and-links/ | no date
- Session Restoration | https://docs.warp.dev/terminal/sessions/session-restoration/ | no date
- Themes / Custom Themes | https://docs.warp.dev/terminal/appearance/themes/ and …/custom-themes/ | no date
- Settings / All settings | https://docs.warp.dev/terminal/settings/ and …/all-settings/ | no date
- Tabs / Split panes | https://docs.warp.dev/terminal/windows/tabs/ and …/split-panes/ | no date
- Global Hotkey | https://docs.warp.dev/terminal/windows/global-hotkey/ | no date
- Desktop Notifications / Audible bell | https://docs.warp.dev/terminal/more-features/notifications/ and …/audible-bell/ | no date
- Text selection | https://docs.warp.dev/terminal/more-features/text-selection/ | no date
- Accessibility | https://docs.warp.dev/terminal/more-features/accessibility/ | no date
- Changelog | https://docs.warp.dev/changelog/ | 2026-05-14 … 2026-08-07
- warpdotdev/themes repo | https://github.com/warpdotdev/themes | no date

**Windows Terminal**
- Docs home | https://learn.microsoft.com/en-us/windows/terminal/ | no date
- Find | https://learn.microsoft.com/en-us/windows/terminal/search | updated 2025-11-12
- Shell integration / Tips & tricks | https://learn.microsoft.com/en-us/windows/terminal/tips-and-tricks | updated 2025-08-21
- Panes | https://learn.microsoft.com/en-us/windows/terminal/panes | no date
- Actions | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions | no date
- Interaction settings (copyOnSelect) | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/interaction | no date
- Advanced settings (bellStyle) | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/advanced | no date
- Color schemes | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/color-schemes | no date
- Themes | https://learn.microsoft.com/en-us/windows/terminal/customize-settings/themes | no date (Preview-era)
- Command-line arguments (quake) | https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments | no date
- Accessibility | https://learn.microsoft.com/en-us/windows/terminal/accessibility (and terminal-accessibility) | no date
- defaults.json (16 built-in schemes) | https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalApp/defaults.json | fetched 2026-08-13
- GitHub releases (v1.4 … v1.25) | https://github.com/microsoft/terminal/releases | 2020-09-22 … 2026-07-16
