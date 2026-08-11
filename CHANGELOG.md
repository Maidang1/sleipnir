# Changelog

## 0.1.1

### Features
- Settings panel with theme picker (`⌘,`), WezTerm-style section tabs
- Theme selection persists to `~/.config/sleipnir/settings.json`
- Theme cycle (`⌘⇧P`) also persists

### Fixes
- Honor terminal cursor hide (`CSI ?25l` / `CursorShape::Hidden`) so full-screen TUIs no longer show a spurious blinking cursor on status text
- Paint underline, bar, and hollow cursor shapes correctly
- Forward right/middle mouse down+up into the PTY so mouse-mode apps (e.g. Herdr tab context menus) receive clicks

## 0.1.0

- Initial macOS release
