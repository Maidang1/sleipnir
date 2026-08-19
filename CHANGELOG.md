# Changelog

## Unreleased

## 0.2.1

### Features
- **Windows:** GPUI `WindowsPlatform` + ConPTY, Ctrl/Ctrl+Shift keymap (does not steal `Ctrl+C` / `Ctrl+W` / `Ctrl+D`), `%APPDATA%\sleipnir\` settings/session, Cascadia Mono default font, PowerShell path quoting, and last-window-close quits
- Windows custom titlebar: drawn min / max / close buttons, drag from chrome, settings gear
- CI `windows-latest` job builds/tests `-p sleipnir` and attaches `Sleipnir-<ver>-windows-x64.zip` (plus `.sha256`) to the GitHub Release

### Changes
- Default tab placement is the top strip (`tab_placement: top`). Side rail remains available
- README and website document Windows from source / zip; Linux stays unsupported
- In-place auto-update remains macOS-only; Windows Check for Updates opens the releases page
- Control surface (`sleipnir-ctl`) stays Unix-socket and is a no-op on Windows

## 0.2.0

### Features
- Diff inspector overlay (`⌥⌘G` / command palette / View → Diff Inspector / chrome **Diff** button / rail `+N −M`): `git diff HEAD` of the focused pane's work tree, split (default) or unified (`v`), file tree, full-file upgrade with expandable `⋯ N hidden lines`, tree-sitter on Rust/Python/JS/JSON, minimap (`m`), word-level intra-line highlights, file/hunk jump (`n` `p` `]` `[`). Not a pane. `send_git_diff` still pastes the raw patch into the PTY
- Tab placement is Side (default left rail) or Top (horizontal strip). Both group by git workspace (no group header), with agent marks, in-group drag, and New tab. Switch in Settings, View → Toggle Tab Placement, or the command palette
- Side-rail rows stay two lines: title, then branch + dirty `+N −M`
- Top-strip chips show only the last two cwd components (`myself/harbor`); no branch or dirty mark
- Tab chrome no longer draws run dots (running ● / succeeded ✓). A tab with failed Attention washes the whole chip a faint red
- Agent monograms on tabs for known coding-agent processes (`claude`, `codex`, …); no house placeholder when the tab is just a shell; `agent_icons: false` hides them
- New tabs inherit the workspace git root so they stay in the same group
- `clear_run_ledger` / `toggle_run_ledger` / `mark_tab_seen` in command palette and menus. Mark Tab as Seen clears Attention without deleting Run records
- Close-confirm names the busy foreground process when one is known
- Drag a tab onto another tab's panes to merge it as a split (same sessions); drag a pane grip onto the tab list to extract it as its own tab. Dropping the visible tab on the pane area still detaches it to a new window
- Pane Facts overlay (View / command palette): cwd, foreground process, descendant process tree, and listening ports for the focused pane
- Run Ledger overlay (`⌘⇧L`): grouped runs, jump to the pane and scroll to the OSC 133 Anchor
- Pane gutter triangles on command start/end lines (overlay; hidden on alt screen)
- Menu-bar Attention item (`show_tray_icon`) and Dock badge of failed Attention count
- Default-off control surface (ADR-0011): listens when `control_surface: true` or `SLEIPNIR_CONTROL=1`; `sleipnir-ctl ls/send/wait/capture` drive live panes
- Restore tombstone banner from prior-launch Run metadata (no scrollback); dismisses on type; skips in-flight / unrecognized last commands; `show_tombstone: false` hides it
- Send Selection / Send Git Diff to the focused pane; optional `pipe_selection_command`
- Shell history search overlay (`⌘⇧;`) over `HISTFILE` / `~/.zsh_history`
- `keybinding_preset: tmux` adds `ctrl-b` pane/tab chords

### Fixes
- Side-rail git dirty mark no longer walks the work tree on the UI thread (tab switches stalled for ~1s+ in repos with `target/` / `node_modules/`)
- Side-rail `+N −M` is line counts from `git diff --numstat HEAD`, computed off the UI thread (typing no longer flashes the row; the old index parser showed only a bogus delete count)
- CI macOS bundle now includes `Sleipnir.sdef` so the AppleScript dictionary ships in GitHub Release builds

### Breaking Changes
- `terminal.inject_osc133` now defaults to `true`
- New `runs.json` file written to config dir by default (`run_ledger: "off"` disables)
- **macOS only:** Windows and Linux are no longer supported. Prebuilt `.zip` / `.deb` / `.tar.gz` artifacts, `install.ps1`, `install-linux.sh`, `make-linux-package.sh`, and the Windows/Linux CI jobs are gone. The crate fails to compile on those targets.

## 0.1.11

### Features
- **Linux from source:** GPUI `LinuxPlatform` (X11 + Wayland), Vulkan rendering, Ctrl/Ctrl+Shift keymap (does not steal `Ctrl+C` / `Ctrl+W` / `Ctrl+D`), `~/.config/sleipnir/` settings/session, Ubuntu Mono default font, `xdg-open` for paths, and libnotify completion notifications
- `make-linux-package.sh` produces a portable `Sleipnir-<ver>-linux-x86_64.tar.gz` and a native `sleipnir_<ver>_amd64.deb` (`.desktop` entry + hicolor icons)
- CI `ubuntu-latest` job builds/tests `-p sleipnir` and attaches the `.tar.gz` + `.deb` to the GitHub Release
- One-line Linux install (`curl | bash`) downloads the latest `.deb` and installs it with apt (`SLEIPNIR_TARBALL=1` for the portable tarball)

### Fixes
- Windows CI: `path_opener_program()` is `None` on Windows (paths open via `cmd /C start`); the unit test no longer requires `Some`

### Changes
- README and website treat Linux as a shipped platform: one-line install, `.deb` / `.tar.gz` downloads, Vulkan/Wayland notes, and Linux shortcuts
- In-place auto-update stays macOS-only; Linux Check for Updates opens the releases page

### Features
- **Find in scrollback:** regex (`.*`) and match-case (`Aa`) toggles in the find bar (`⌥⌘R` / `⌥⌘C` on macOS). Literal + case-insensitive remains the default; toggling regex passes the query through as a regex, and match-case forces case-sensitive matching via inline flags.
- **Export scrollback:** **Shell → Export Scrollback…** (macOS) / **File → Export Scrollback…** (Windows), also in the command palette, writes the active pane's scrollback to a timestamped temp file and opens it in the default editor.
- **Link/path hover preview:** hovering a URL or path shows a small tooltip with the matched text (on top of the M11 underline + pointing hand).
- **Tab drag reorder + detach:** drag a tab chip onto another to reorder (a ghost chip follows the pointer); drop a tab onto the terminal area to detach it into a new window (its panes keep running).
- **Shell integration:** new tabs and splits now inherit the active pane's working directory instead of opening in `$HOME`.
- **Notification matrix:** `notify_on_command_finish_mode` (`never` / `unfocused` / `always`, default `unfocused`) controls when the command-finish notification fires, on top of the existing `notify_on_command_finish_secs` threshold.
- **Themes:** added Dracula and One Dark built-in palettes; `custom_theme` accepts a full hex palette (background/foreground/ANSI 16/cursor/selection); extra named palettes come from `themes.json` in the config dir; the Settings → theme picker lists built-ins and user themes with swatches and type-to-filter search.
- **Accessibility:** the terminal now exposes the visible screen as a read-only accessible value (`Role::MultilineTextInput` + label "Terminal"), so VoiceOver can read the current output (Ghostty-parity, read-only AX).
- **OSC 9/777 desktop notifications:** programs can trigger a macOS notification via `ESC ] 9 ; msg` or kitty's `ESC ] 777 ; notify ; msg` on both the display-only path and the real PTY (fork-pinned alacritty_terminal + vte emit `DesktopNotification` through `ZedListener`).
- **OSC 133 on the real PTY:** shell-integration markers (`ESC ] 133 ; A/B/C/D`) now reach jump-prompt (`⌘⇧↑`/`⌘⇧↓`) through the same fork-pin hook, not just `write_output`.
- **Shell semantics:** opt-in `terminal.inject_osc133` sources OSC 133 A/B/C/D into new zsh/bash/fish sessions (skipped if another terminal already injects, or for `shell -c`). Option/Alt-click in the current prompt sends left/right to move the shell cursor; Cmd/Ctrl-triple-click selects the marked command’s output (plain triple-click still selects lines).
- **AppleScript:** shipped a minimal scripting dictionary (`Sleipnir.sdef`) — read-only `name`/`version`/`frontmost` plus `quit` (Ghostty 1.3.0-style), wired via `NSAppleScriptEnabled` + `OSAScriptingDefinition`.

## 0.1.10

### Features
- **Windows from source:** GPUI `WindowsPlatform` + ConPTY, Ctrl/Ctrl+Shift keymap (does not steal `Ctrl+C` / `Ctrl+W` / `Ctrl+D`), `%APPDATA%\sleipnir\` settings/session, Cascadia Mono default font, PowerShell path quoting, and last-window-close quits
- CI `windows-latest` job builds/tests `-p sleipnir` and attaches `Sleipnir-<ver>-windows-x64.zip` (plus `.sha256`) to the GitHub Release

### Changes
- One-line macOS install (`curl | bash`) on README and the website: download the latest Release, verify SHA-256, install to `/Applications`, and clear Gatekeeper quarantine with `xattr -cr`
- README and website document Windows from source; prebuilt installers stay macOS-only
- In-place auto-update remains macOS-only; Windows Check for Updates opens the releases page

## 0.1.9

### Features
- **M11 visual polish:** ease-in-out cursor blink (~530ms half-period, solid after typing); URL/path hover underline + pointing hand; trackpad momentum via platform events
- **M12 daily gaps:**
  - **Font zoom:** `⌘+` / `⌘=` / `⌘-` / `⌘0` (window-scoped; not persisted; reset restores settings size)
  - **New window:** `⌘N` opens an independent OS window with its own tabs/shells
  - **Close confirm:** `confirm_close` (`dirty` \| `always` \| `never`, default `dirty`) prompts when a non-shell foreground job is running
  - **Path open:** cmd-click path-like targets (`file.rs:12`, relative to pane cwd) when `path_links` is true (default)
  - **Bell:** `terminal.bell` supports `off` \| `system` \| `visual` (system beep / tab chrome flash)
  - **Copy on select:** toggle in Settings → General (`terminal.copy_on_select`, default still `false`)
- **M13 splits:** pane zoom (`⌘⇧Enter`), unfocused pane dim, broadcast mode banner (`⌘⇧B`)
- **M14 shell collaboration:** OSC 133 detect + jump prev/next prompt (`⌘⇧↑`/`⌘⇧↓`); `notify_on_command_finish_secs` for unfocused long jobs
- **M15 optional:** Quick Terminal (`⌘⇧N`), Quick Select mode (`⌘⇧O`), `background_opacity` (default opaque)

### Fixes
- Restore a window when clicking the Dock icon (or choosing **Shell → New Window** / `⌘N`) after the last window was closed; the process stays running on macOS, but previously had no reopen handler
- Close-confirm **Close** now actually closes the pane: the dialog panel was missing the mouse-down stop that settings/update/palette overlays have, so the click hit the backdrop and cancelled instead
- Scrolling a modal overlay (settings / palette / update / close-confirm) no longer also scrolls the terminal underneath; overlays now `occlude()` so GPUI drops the terminal from the scroll hit-test

### Changes
- **App icon full-bleed redesign:** remove the circular inset so the mark fills the entire rounded square (same solid-fill style as typical macOS icons), with a larger knight silhouette and integrated terminal prompt

## 0.1.8

### Fixes
- Restore plain `Ctrl+C` / `Ctrl+V` delivery to the PTY so foreground programs receive `SIGINT` / control bytes again; clipboard actions stay on `⌘C`/`⌘V`, `⌃⇧C`/`⌃⇧V`, and `⌃⌘V`
- Add unit tests for clipboard-key routing and terminal control-byte mapping (`Ctrl+C` → `0x03`, `Ctrl+V` → `0x16`)

## 0.1.7

### Improvements
- **Decouple TermView from AppShell:** terminal views now communicate via events instead of holding a direct reference to the shell, improving testability and reducing coupling
- **Debounced session persistence:** rapid tab/split changes no longer thrash the disk; writes are coalesced via an async debounce
- **Font fallbacks now work:** user-configured `font_fallbacks` are properly passed to the text renderer, fixing CJK/emoji display
- **Safe auto-update rollback:** the update helper now backs up the old `.app` before replacing it; if the swap fails, the backup is restored instead of leaving the user without an app
- **Expanded user key bindings:** `key_bindings` in settings.json now supports terminal actions (`copy`, `paste`, `scroll_line_up/down`, `scroll_page_up/down`, `scroll_to_top/bottom`, `toggle_vi_mode`, `clear`, `select_all`, `show_character_palette`)

### Fixes
- Fix potential double-execution of clipboard shortcuts (cmd-c/cmd-v) by removing redundant handling in the key-down path
- Fix selection/search highlight clipping at column 500 on ultra-wide terminals
- Fix temporary paste file naming to use a monotonic counter instead of wall-clock time (avoids collisions on clock skew)

## 0.1.6

### Changes
- App icon: enlarge the knight mark and terminal prompt so they fill the icon safe area

## 0.1.5

### Features
- **Session restore (M8):** tabs, splits, custom titles, and per-pane working directories are saved to `~/.config/sleipnir/session.json` and restored on launch (`restore_session`, default `true`)
- **Command palette (M9):** `⌘⇧K` fuzzy action list; optional `key_bindings` array in settings.json
- **Find in scrollback (M10):** `⌘F` / `⌘G` / `⌘⇧G` with match highlights
- **Font ligatures (M10):** `terminal.font_ligatures` setting (default off)

## 0.1.4

### Changes
- Check for Updates now opens a centered modal dialog (Download & Install / Restart & Update / Release Notes) instead of a cramped top bar
- No automatic update check on launch — updates are only checked when you open **Sleipnir → Check for Updates…** (`⌘⇧U`); removed the `auto_update` setting
- `Esc` or clicking the backdrop closes the update dialog

## 0.1.3

### Fixes
- Fix crash on launch (`there is no reactor running`): the updater used a Tokio-based HTTP client, but Sleipnir runs on GPUI's smol executor. Switched to a blocking `ureq` (rustls) client run on the background executor.

## 0.1.2

### Features
- Auto-update: checks GitHub Releases on launch (toggle with `auto_update` in settings.json) and via **Sleipnir → Check for Updates…** / `⌘⇧U`
- Update notification bar with download progress and one-click **Restart & Update**
- Downloads are verified against a published `.zip.sha256` sidecar before install; falls back to the releases page when in-place replacement isn't permitted

## 0.1.1

### Features
- Native macOS menu bar: Sleipnir / Shell / Edit / View / Window (minimal parity with Terminal.app / Kaku)
- Settings panel with theme picker (`⌘,`), WezTerm-style section tabs
- Theme selection persists to `~/.config/sleipnir/settings.json`
- Theme cycle (`⌘⇧P`) also persists
- GitHub Dark / GitHub Light themes (Primer palette)

### Fixes
- Honor terminal cursor hide (`CSI ?25l` / `CursorShape::Hidden`) so full-screen TUIs no longer show a spurious blinking cursor on status text
- Paint underline, bar, and hollow cursor shapes correctly
- Forward right/middle mouse down+up into the PTY so mouse-mode apps (e.g. Herdr tab context menus) receive clicks

## 0.1.0

- Initial macOS release
