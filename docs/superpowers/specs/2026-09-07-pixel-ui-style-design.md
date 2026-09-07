# Pixel UI Style Design

> **Superseded (2026-09-07)**: pixel 已成为唯一 UI 风格，ui_style 设置已移除。本文档仅作历史参考。

**Status:** Approved in conversation

**Goal:** Add a switchable pixel-art geometry skin ("中度像素 / NES.css 感") to the whole Sleipnir UI, layered on top of the existing color-theme system, without changing text rendering, fonts, or the default look.

## Context

Sleipnir's chrome colors are derived from the active terminal palette via `ChromeTokens` (`crates/sleipnir_ui/src/chrome/chrome_tokens.rs`), so color theming is already centralized. Geometry is not: roughly 39 `rounded()` call sites and assorted border/shadow values are hardcoded across `sleipnir_ui` (tab strip, find bar, command palette, settings overlay, panels, diff view, toasts, menus). The settings panel already ships one pixel-style artifact — a hard, blur-free offset drop shadow — but there is no systematic treatment.

The user wants the terminal's *design details* to read as pixel-art while keeping text perfectly normal: no pixel fonts, no scanlines, no palette replacement. The reference direction chosen from visual mockups is "B · 中度像素": 2px solid borders, stepped (staircase) panel corners, hard offset shadows, blocky controls (toggles, buttons, selects), square traffic-light substitutes where the app draws its own window controls. The treatment must apply **uniformly** to every surface, and it must be opt-in via a setting so the default appearance is untouched.

The pinned GPUI (Zed fork) provides `Window::paint_path` and a `Path` builder, which makes true staircase-corner polygons feasible without shader work.

## Decisions

- Pixel styling is a **runtime setting**, `"ui_style": "default" | "pixel"`, default `default`, hot-reloaded with the rest of the settings pipeline.
- Pixel mode is a **geometry skin only**. Colors continue to come from `TerminalPalette` → `ChromeTokens`; pixel mode must work over every built-in and custom theme unchanged.
- All visual parameters converge in a **single token module** (`chrome/pixel.rs`). Call sites ask the module for values; they never hardcode pixel constants.
- **Stepped corners are real geometry**, painted with GPUI `Path` by a dedicated `PixelFrame` custom element — not simulated with overlay squares (overlays break over non-solid backdrops like terminal text).
- Scope is **unified**: every chrome surface adopts the skin. No per-surface opt-outs.
- macOS native traffic lights stay native; only the app's own window controls (Windows/Linux `desktop_window_controls.rs`) are restyled.

## Scope

### In scope

- New `ui_style` setting in `sleipnir_settings` (parse, default, hot reload, example config, docs).
- New `crates/sleipnir_ui/src/chrome/pixel.rs` token module:
  - `radius(style, default) -> Pixels` — pixel mode always returns 0.
  - `border_width(style) -> Pixels` — 2px in pixel mode.
  - `hard_shadow(style) -> Vec<BoxShadow>` — offset ~(6,6), zero blur, black ~55%; the existing settings-panel shadow moves here.
  - Control metrics for blocky toggle / button / select / stepper / scrollbar.
  - `STEP` constants for the staircase corner (6px corner, 3px steps).
- New `PixelFrame` custom GPUI element: paints a staircase-corner polygon (shadow pass + fill pass + 2px stroke pass) behind its children in pixel mode; renders as a plain rectangle (or is simply unused) in default mode. Used by floating surfaces: settings overlay, command palette, update dialog, close-confirm dialog, plugin consent.
- Uniform restyle of all chrome surfaces under pixel mode:
  - `chrome/tab_strip.rs` — square tabs, 2px borders, active tab visually fused with the content area, square close buttons.
  - `chrome/geometry.rs` — content clip radius resolves to 0 in pixel mode.
  - `chrome/desktop_window_controls.rs` — square min/max/close buttons.
  - `app_shell/`: settings overlay (all controls), command palette, find bar, update dialog, panels (run ledger, plugin monitor, pane facts), terminal/context menus, toasts in `sleipnir_ui.rs`.
  - `diff/render.rs`, `plugin_paint.rs`, `plugin_chrome.rs`, `control_surface.rs` — borders/shadows/radii via the token module.
- Unit tests for token functions (both modes) and `ui_style` deserialization.

### Out of scope (YAGNI)

- Pixel/bitmap fonts, scanlines, CRT or dither effects.
- Any change to terminal cell rendering, cursor, or scrollback (block cursor already exists).
- Color/palette changes; the skin rides on the existing theme system.
- Restyling macOS native window traffic lights.
- "重度像素" extras from the mockup (dither dividers, color-block title bars, chunky status bar).

## Architecture

### Settings flow

`settings.json` → `sleipnir_settings::Settings.ui_style: UiStyle` (new enum `UiStyle { Default, Pixel }`) → existing hot-reload watcher → `AppShell` state → render functions read the current style and pass it to the token module. `UiStyle` lives in `sleipnir_settings` so both `sleipnir_ui` and any future consumer share one definition.

### Token module

`chrome/pixel.rs` contains only pure functions and constants over `UiStyle` and existing token types (`Hsla`, `Pixels`, `BoxShadow`). Default mode returns today's exact values, guaranteeing zero visual regression when the setting is off. This module is the single seam the skin hangs on; the ~39 call sites become mechanical substitutions.

### PixelFrame

A custom GPUI `Element` that wraps children, lays them out normally, and in `paint` first draws the staircase polygon: an offset solid black pass (the hard shadow), then the fill pass (`tokens.surface`), then the 2px stroke pass (`tokens.border`). Corner staircase: 6px cut in two 3px steps, matching the approved mockup. Children paint above. Only floating panels adopt it; inline surfaces (tabs, buttons, find bar rows) use flat rectangles with token-driven borders and shadows.

## Data flow

Render path: `AppShell` reads `Settings.ui_style` → passes `UiStyle` into surface render helpers → helpers call `pixel::*` token functions for radius/border/shadow and choose `PixelFrame` vs plain div for floating panels. Hot reload flips `UiStyle` in state and triggers a normal re-render; no special invalidation needed.

## Error handling

- Unknown `ui_style` values fail settings parsing with the existing settings error surface (same as other enum settings); the app keeps running with the previous value.
- `PixelFrame` must degrade gracefully: if a panel is too small for the 6px staircase, clamp the step size rather than drawing degenerate polygons.

## Testing

- `sleipnir_settings`: deserialize `"ui_style": "pixel"`, default when absent, reject unknown values.
- `pixel.rs`: pixel mode → radius 0 / 2px border / hard shadow present; default mode → today's values (golden values asserted to prevent accidental default regression).
- Existing `chrome_tokens` contrast tests must stay green (colors untouched).
- Manual verification: run the app in both modes, screenshot tab strip + settings panel + command palette + a toast, compare against the approved mockup; `cargo test` and `cargo clippy` green.

## Rollout

Single feature branch; setting documented in `docs/settings.example.json` and README config section. Default stays `default`, so shipping is zero-risk to existing users.
