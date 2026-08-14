# Changelog

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
