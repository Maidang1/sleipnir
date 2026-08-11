# Changelog

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
