# Changelog

## Unreleased

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
