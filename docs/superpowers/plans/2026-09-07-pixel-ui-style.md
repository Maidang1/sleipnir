# Pixel UI Style Implementation Plan

> **Superseded (2026-09-07)**: pixel 已成为唯一 UI 风格，ui_style 设置已移除。本文档仅作历史参考。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in `"ui_style": "pixel"` setting that reskins all Sleipnir chrome with pixel-art geometry (square corners, 2px borders, hard offset shadows, staircase panel corners, blocky controls) while leaving text rendering, fonts, colors, and the default look untouched.

**Architecture:** Geometry decisions converge in one token module (`chrome/pixel.rs`) keyed by a new `UiStyle` enum in `sleipnir_settings`. Render functions already read `TerminalSettings::get_global(cx)` and `ChromeTokens`; they now also read `ui_style` and ask the token module for values. Floating panels get true staircase corners via a `gpui::canvas` background element that paints nested staircase polygons with `Window::paint_path`. Spec: `docs/superpowers/specs/2026-09-07-pixel-ui-style-design.md`.

**Tech Stack:** Rust, GPUI (Zed fork, pinned rev `371a7d4`), serde/schemars settings, `cargo test`.

**Working agreements:**
- Default mode must be pixel-identical to today. Every change is gated on `UiStyle::Pixel`.
- Run `cargo test -p <crate>` after every task; commit after every green task.
- The working tree may contain unrelated user changes — never `git add -A`; stage only files the task touched.

---

### Task 1: `UiStyle` setting in `sleipnir_settings`

**Files:**
- Modify: `crates/sleipnir_settings/src/sleipnir_settings.rs`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `sleipnir_settings.rs`:

```rust
#[test]
fn ui_style_defaults_to_default_and_parses_pixel() {
    // Absent key -> Default.
    let file: SettingsFile = serde_json::from_str("{}").unwrap();
    let mut settings = TerminalSettings::default();
    merge_file(&mut settings, file);
    assert_eq!(settings.ui_style, UiStyle::Default);

    // "pixel" parses.
    let file: SettingsFile = serde_json::from_str(r#"{"ui_style":"pixel"}"#).unwrap();
    let mut settings = TerminalSettings::default();
    merge_file(&mut settings, file);
    assert_eq!(settings.ui_style, UiStyle::Pixel);

    // Round-trip serialization uses snake_case.
    assert_eq!(serde_json::to_string(&UiStyle::Pixel).unwrap(), "\"pixel\"");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p sleipnir_settings ui_style`
Expected: FAIL — `UiStyle` / `ui_style` do not exist (compile error).

- [ ] **Step 3: Implement**

In `sleipnir_settings.rs`, next to the other enums (~line 130):

```rust
/// Chrome geometry skin. `pixel` = square corners, 2px borders, hard
/// shadows, staircase panel corners. Colors still come from the theme.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiStyle {
    #[default]
    Default,
    Pixel,
}
```

Add to `TerminalSettings` struct (near `theme`):

```rust
/// Chrome geometry skin: default | pixel. Default: default.
pub ui_style: UiStyle,
```

Add to `impl Default for TerminalSettings`:

```rust
ui_style: UiStyle::Default,
```

Add to `SettingsFile` (top-level, next to `theme`):

```rust
/// Chrome geometry skin: default | pixel. Default: default.
#[serde(default)]
ui_style: Option<UiStyle>,
```

Add to `merge_file`:

```rust
if let Some(v) = file.ui_style {
    settings.ui_style = v;
}
```

Export nothing new (`UiStyle` is already `pub` in the crate root module).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p sleipnir_settings`
Expected: PASS, including the new `ui_style` test.

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_settings/src/sleipnir_settings.rs
git commit -m "feat(settings): add ui_style setting (default|pixel)"
```

---

### Task 2: Pixel token module `chrome/pixel.rs`

**Files:**
- Create: `crates/sleipnir_ui/src/chrome/pixel.rs`
- Modify: `crates/sleipnir_ui/src/chrome/mod.rs` (add `pub(crate) mod pixel;`)

- [ ] **Step 1: Write the failing tests**

Create `crates/sleipnir_ui/src/chrome/pixel.rs` with the tests first (implementation stubs may `todo!()`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_mode_squares_corners_and_thickens_borders() {
        assert_eq!(radius(UiStyle::Pixel, px(6.0)), px(0.0));
        assert_eq!(border_width(UiStyle::Pixel, px(1.0)), px(2.0));
    }

    #[test]
    fn default_mode_passes_values_through() {
        // Golden values: default mode must not drift from today's look.
        assert_eq!(radius(UiStyle::Default, px(6.0)), px(6.0));
        assert_eq!(border_width(UiStyle::Default, px(1.0)), px(1.0));
        assert!(hard_shadow(UiStyle::Default).is_empty());
    }

    #[test]
    fn pixel_shadow_is_hard_and_offset() {
        let shadows = hard_shadow(UiStyle::Pixel);
        assert_eq!(shadows.len(), 1);
        let s = &shadows[0];
        assert_eq!(s.blur_radius, px(0.0));
        assert_eq!(s.spread_radius, px(0.0));
        assert!(s.offset.x > px(0.0) && s.offset.y > px(0.0));
        assert!(!s.inset);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sleipnir_ui pixel`
Expected: FAIL — module/functions missing.

- [ ] **Step 3: Implement the module**

```rust
//! Pixel-art geometry tokens for the `ui_style = "pixel"` chrome skin.
//!
//! Colors stay with `ChromeTokens`; this module owns *geometry*: radii,
//! border widths, and hard offset shadows. Every function takes the active
//! `UiStyle` and returns today's exact values for `UiStyle::Default`, so
//! default mode is visually unchanged by construction.

use gpui::{BoxShadow, Hsla, Pixels, point, px};
use sleipnir_settings::UiStyle;

/// Staircase corner size for floating panels in pixel mode.
pub const PANEL_CORNER: Pixels = px(6.0);
/// Border width used by the staircase painter and 2px chrome borders.
pub const PIXEL_BORDER: Pixels = px(2.0);

/// Corner radius: pixel mode is always square.
pub fn radius(style: UiStyle, default: Pixels) -> Pixels {
    match style {
        UiStyle::Pixel => px(0.0),
        UiStyle::Default => default,
    }
}

/// Border width: pixel mode doubles to a chunky 2px line.
pub fn border_width(style: UiStyle, default: Pixels) -> Pixels {
    match style {
        UiStyle::Pixel => PIXEL_BORDER,
        UiStyle::Default => default,
    }
}

/// Hard offset drop shadow (no blur, no spread). Empty in default mode so
/// callers keep their existing `.shadow_lg()` etc. via `.when(...)`.
pub fn hard_shadow(style: UiStyle) -> Vec<BoxShadow> {
    match style {
        UiStyle::Default => Vec::new(),
        UiStyle::Pixel => vec![BoxShadow {
            color: Hsla::black().opacity(0.55),
            offset: point(px(6.0), px(6.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
            inset: false,
        }],
    }
}

/// Read the active style from global settings. Shorthand for call sites.
pub fn active_style(cx: &gpui::App) -> UiStyle {
    sleipnir_settings::TerminalSettings::get_global(cx).ui_style
}
```

In `chrome/mod.rs` add `pub(crate) mod pixel;` next to the other `mod` declarations.

Note: the settings overlay's existing inline hard shadow (`app_shell/settings.rs` ~line 206) is NOT removed in this task; Task 5 consolidates it onto `hard_shadow`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sleipnir_ui pixel`
Expected: PASS (3 new tests).

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_ui/src/chrome/pixel.rs crates/sleipnir_ui/src/chrome/mod.rs
git commit -m "feat(ui): pixel geometry token module"
```

---

### Task 3: Staircase panel background (`pixel_panel_bg`)

**Files:**
- Modify: `crates/sleipnir_ui/src/chrome/pixel.rs`

GPUI facts (verified against the pinned rev): `gpui::canvas(prepaint, paint)` gives a custom-paint element; `Window::paint_path(Path<Pixels>, color)` fills a polygon; `Path::new(start: Point<Pixels>)`, `.line_to(point)`, plus `gpui::point(x, y)`. A "stroke" is achieved by painting the outer staircase polygon in the border color, then the inner (inset by `PIXEL_BORDER`) staircase polygon in the fill color on top.

- [ ] **Step 1: Write the failing tests**

The polygon generator is pure and unit-testable:

```rust
#[test]
fn staircase_polygon_cuts_two_steps_per_corner() {
    let pts = staircase_points(
        point(px(0.0), px(0.0)),
        px(100.0),
        px(60.0),
        px(6.0),
    );
    // 4 corners x 4 staircase vertices = 16 points.
    assert_eq!(pts.len(), 16);
    // Starts just right of the top-left corner cut.
    assert_eq!(pts[0], point(px(6.0), px(0.0)));
    // Second point steps down-left toward the corner.
    assert_eq!(pts[1], point(px(3.0), px(0.0)));
    assert_eq!(pts[2], point(px(3.0), px(3.0)));
    assert_eq!(pts[3], point(px(0.0), px(3.0)));
}

#[test]
fn staircase_corner_clamps_on_tiny_panels() {
    let pts = staircase_points(
        point(px(0.0), px(0.0)),
        px(8.0),
        px(8.0),
        px(6.0),
    );
    // Corner clamped to half the smallest side: no degenerate crossing.
    assert!(pts.iter().all(|p| p.x >= px(0.0) && p.y >= px(0.0)));
    assert!(pts.iter().all(|p| p.x <= px(8.0) && p.y <= px(8.0)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p sleipnir_ui staircase`
Expected: FAIL — `staircase_points` missing.

- [ ] **Step 3: Implement**

Add to `pixel.rs`:

```rust
use gpui::{Bounds, Canvas, Path, Point, canvas};

/// Clockwise staircase-corner polygon points for a rect of `w` x `h`
/// positioned at `origin`. `corner` is clamped to half the shortest side so
/// small panels degrade to smaller steps instead of self-intersecting.
pub fn staircase_points(
    origin: Point<Pixels>,
    w: Pixels,
    h: Pixels,
    corner: Pixels,
) -> Vec<Point<Pixels>> {
    let c = corner.min(w / 2.0).min(h / 2.0);
    let s = c / 2.0;
    let (x0, y0) = (origin.x, origin.y);
    let (x1, y1) = (x0 + w, y0 + h);
    vec![
        point(x0 + c, y0),      // top edge, after TL cut
        point(x0 + s, y0),
        point(x0 + s, y0 + s),
        point(x0, y0 + s),      // TL cut done, down the left edge
        point(x0, y1 - s),
        point(x0 + s, y1 - s),
        point(x0 + s, y1),
        point(x0 + c, y1),      // BL cut done, along the bottom
        point(x1 - c, y1),
        point(x1 - s, y1),
        point(x1 - s, y1 - s),
        point(x1, y1 - s),      // BR cut done, up the right edge
        point(x1, y0 + s),
        point(x1 - s, y0 + s),
        point(x1 - s, y0),
        point(x0 + c, y0),      // TR cut done, back along the top
    ]
}

fn staircase_path(bounds: Bounds<Pixels>, inset: Pixels, corner: Pixels) -> Path<Pixels> {
    let origin = bounds.origin + point(inset, inset);
    let w = bounds.size.width - inset * 2.0;
    let h = bounds.size.height - inset * 2.0;
    let pts = staircase_points(origin, w, h, corner);
    let mut path = Path::new(pts[0]);
    for p in &pts[1..] {
        path.line_to(*p);
    }
    path
}

/// Absolute inset-0 canvas that paints a staircase-cornered panel
/// background: border pass first, then the inset fill pass. The caller's
/// panel div keeps its own (hard) box shadow and must NOT set `.bg()` /
/// `.border_*()` in pixel mode.
pub fn pixel_panel_bg(fill: Hsla, border: Hsla) -> Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let corner = PANEL_CORNER
                .min(bounds.size.width / 4.0)
                .min(bounds.size.height / 4.0);
            window.paint_path(staircase_path(bounds, px(0.0), corner), border);
            window.paint_path(staircase_path(bounds, PIXEL_BORDER, corner - PIXEL_BORDER), fill);
        },
    )
    .absolute()
    .inset_0()
}
```

Check `Path` API details against the pinned checkout (`~/.cargo/git/checkouts/zed-a70e2ad075855582/371a7d4/crates/gpui/src/scene.rs` around line 801): constructor name, whether `close()` exists (the fill wraps automatically — verify; if the path does not auto-close, add a final `line_to(pts[0])`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p sleipnir_ui staircase`
Expected: PASS. `cargo build -p sleipnir_ui` must also pass (canvas compiles).

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_ui/src/chrome/pixel.rs
git commit -m "feat(ui): staircase-corner panel background painter"
```

---

### Task 4: Chrome geometry + tab strip

**Files:**
- Modify: `crates/sleipnir_ui/src/chrome/geometry.rs`
- Modify: `crates/sleipnir_ui/src/chrome/tab_strip.rs`
- Modify: `crates/sleipnir_ui/src/app_shell/mod.rs` (content clip radius ~line 2116, toasts ~lines 913/1060/2421/2443)

- [ ] **Step 1: Geometry pixel variant + failing test**

Add to `geometry.rs`:

```rust
/// Chrome insets for the platform, fullscreen state, and UI style.
pub fn for_window_styled(desktop_controls: bool, fullscreen: bool, pixel: bool) -> Self {
    let mut geo = Self::for_window(desktop_controls, fullscreen);
    if pixel {
        geo.tab_radius = px(0.0);
        geo.window_radius = px(0.0);
    }
    geo
}
```

Test (in the existing `mod tests`):

```rust
#[test]
fn pixel_geometry_squares_tabs_and_window() {
    let g = ChromeGeometry::for_window_styled(false, false, true);
    assert_eq!(g.tab_radius, px(0.0));
    assert_eq!(g.window_radius, px(0.0));
    let d = ChromeGeometry::for_window_styled(false, false, false);
    assert_eq!(d, ChromeGeometry::for_window(false, false));
}
```

Run: `cargo test -p sleipnir_ui geometry` — expect FAIL, then implement, expect PASS.
(`ChromeGeometry` may need `#[derive(PartialEq)]` for the assert — add `Debug, PartialEq` if absent.)

- [ ] **Step 2: Wire style into geometry construction**

Find where `ChromeGeometry::for_window` / `standard` is called (grep `for_window(` in `sleipnir_ui`); pass `matches!(TerminalSettings::get_global(cx).ui_style, UiStyle::Pixel)`.

- [ ] **Step 3: Tab strip pixel treatment**

In `tab_strip.rs` `render_tab_chip` (it receives `cx`): read `let style = pixel::active_style(cx);` then:

- `.rounded(geo.tab_radius)` already resolves to 0 via Task 4 Step 1 — no change needed there.
- Renaming/bell accent borders: `.border_1()` → `.border(pixel::border_width(style, px(1.0)))` (lines ~191-192).
- `TabPathPreview` (lines ~105-120): `.rounded(px(6.0))` → `.rounded(pixel::radius(style, px(6.0)))`, `.border_1()` → pixel width, `.when(style == default, |el| el.shadow_lg())` + `.when(pixel, |el| el.shadow(pixel::hard_shadow(style)))`.
- Line ~285/~297 (`rounded(px(3.0))` — badge/dot chips) and ~463 (context menu): same radius/border/shadow substitution.
- Active tab already fuses with content via `active_tab_bg()`; verify visually, no structural change.

- [ ] **Step 4: app_shell/mod.rs content clip + toasts**

- Line ~2116: `el.rounded(geo.window_radius)` — already resolved via geometry; confirm.
- Toasts (lines ~913, ~1060, ~2421, ~2443) and ~159: apply the radius/border/hard-shadow substitution. Toast dot `rounded_full()` (line ~1065) → `rounded(px(0.0))` in pixel mode (square dot).

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p sleipnir_ui` — Expected: PASS.
Run: `cargo build -p sleipnir` — Expected: PASS.

- [ ] **Step 6: Manual smoke check**

`cargo run -p sleipnir`; with default settings the window must look identical to before. Then set `"ui_style": "pixel"` in `~/.config/sleipnir/settings.json`; hot reload should square the tabs and window corners. Screenshot both for the record.

- [ ] **Step 7: Commit**

```bash
git add crates/sleipnir_ui/src/chrome/geometry.rs crates/sleipnir_ui/src/chrome/tab_strip.rs crates/sleipnir_ui/src/app_shell/mod.rs
git commit -m "feat(ui): pixel geometry for tab strip, window corners, toasts"
```

---

### Task 5: Settings overlay — pixel panel + blocky controls

**Files:**
- Modify: `crates/sleipnir_ui/src/app_shell/settings.rs`

- [ ] **Step 1: Panel switches to `pixel_panel_bg` in pixel mode**

In the `panel` div construction (~line 190): read `let style = pixel::active_style(cx);` and branch:

```rust
let panel = div()
    .id("settings-panel")
    // ... size/flex/text props unchanged ...
    .relative()
    .when(style == UiStyle::Default, |el| {
        el.bg(tokens.surface).border_2().border_color(tokens.border)
    })
    .when(style == UiStyle::Pixel, |el| {
        el.shadow(pixel::hard_shadow(style))
            .child(pixel::pixel_panel_bg(tokens.surface, tokens.border))
    })
    .overflow_hidden()
    // ...
```

and remove the now-duplicated inline `BoxShadow` block in favor of `hard_shadow` (keep `.shadow_lg()`-equivalent current behavior for default — check what the panel uses today; today it uses the inline hard shadow for BOTH modes, so default keeps exactly that: move the existing struct into the `Default` arm unchanged).

- [ ] **Step 2: Blocky controls**

Restyle in pixel mode only, via `pixel::radius` / `pixel::border_width` substitutions at each control construction site in this file: toggle track/knob (square knob, `rounded(px(0.0))`, 2px track border, accent knob when on), theme select (2px border, square `▼` affix block), font-size stepper buttons (2px border, no radius), footer buttons (2px border; primary filled with `tokens.accent` and `content_bg` text; 3px hard shadow via `BoxShadow { offset: point(px(3.0), px(3.0)), blur_radius: px(0.0), .. }` — add a `pixel::button_shadow(style)` helper mirroring `hard_shadow` with 3px offset, with tests updated accordingly).

- [ ] **Step 3: Inner dividers/headers**

Header bottom border `border_b_1()` → pixel width; any `rounded` inside rows → token radius.

- [ ] **Step 4: Verify**

Run: `cargo test -p sleipnir_ui` then `cargo run -p sleipnir`, open settings (default mode: unchanged; pixel mode: staircase corners, 2px borders, blocky toggle/select/buttons, hard shadow). Screenshot both.

- [ ] **Step 5: Commit**

```bash
git add crates/sleipnir_ui/src/app_shell/settings.rs crates/sleipnir_ui/src/chrome/pixel.rs
git commit -m "feat(ui): pixel settings panel with blocky controls"
```

---

### Task 6: Remaining floating panels

**Files:**
- Modify: `crates/sleipnir_ui/src/app_shell/palette.rs` (command palette, ~line 226)
- Modify: `crates/sleipnir_ui/src/app_shell/update.rs` (lines ~206, ~396)
- Modify: `crates/sleipnir_ui/src/app_shell/mod.rs` (close-confirm dialog — search `close_confirm` render)
- Modify: `crates/sleipnir_ui/src/plugin_chrome.rs` (plugin consent dialog)

Each floating panel gets the same treatment as Task 5 Step 1: default arm unchanged (existing `rounded(px(10.0))`/`rounded_md()`/`border_1()`/`shadow_lg()`), pixel arm = `pixel_panel_bg` + `hard_shadow` + 2px inner dividers. No new helpers; apply the established pattern. Command-palette row hover highlight stays a flat block (`rounded(px(0.0))` in pixel mode).

- [ ] **Step 1:** Apply to command palette; **Step 2:** update dialog; **Step 3:** close-confirm; **Step 4:** plugin consent.
- [ ] **Step 5:** `cargo test -p sleipnir_ui && cargo build -p sleipnir` — PASS. Manually open each panel in pixel mode; screenshot.
- [ ] **Step 6: Commit**

```bash
git add crates/sleipnir_ui/src/app_shell/palette.rs crates/sleipnir_ui/src/app_shell/update.rs crates/sleipnir_ui/src/app_shell/mod.rs crates/sleipnir_ui/src/plugin_chrome.rs
git commit -m "feat(ui): pixel treatment for remaining floating panels"
```

---

### Task 7: Inline surfaces sweep

**Files:**
- Modify: `crates/sleipnir_ui/src/app_shell/find.rs` (5 × `rounded(px(4.0))` ~lines 315-409)
- Modify: `crates/sleipnir_ui/src/app_shell/panels.rs` (11 sites)
- Modify: `crates/sleipnir_ui/src/app_shell/terminal_menu.rs` (~line 194)
- Modify: `crates/sleipnir_ui/src/app_shell/plugin_paint.rs` (~line 115)
- Modify: `crates/sleipnir_ui/src/app_shell/layout.rs` (~line 245)
- Modify: `crates/sleipnir_ui/src/diff/render.rs` (lines ~168, ~353)
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs` (any remaining sites)
- Modify: `crates/sleipnir_ui/src/chrome/desktop_window_controls.rs` (square caption buttons on Windows/Linux)
- Modify: `crates/sleipnir_ui/src/control_surface.rs` (if it paints rounded/bordered chrome)

Mechanical sweep: every `rounded(px(N))` → `rounded(pixel::radius(style, px(N)))`; every `border_1()` on chrome → `border(pixel::border_width(style, px(1.0)))`; `shadow_lg()` → gated `.when(default).shadow_lg()` / `.when(pixel).shadow(hard_shadow)`. Obtain `style` from `pixel::active_style(cx)` where `cx` is available; otherwise thread it down from the caller (add a parameter, mirroring how `tokens: &ChromeTokens` is already threaded).

After the sweep, `grep -rn "rounded(" crates/sleipnir_ui/src` must show every remaining site either going through `pixel::radius` or documented as intentionally style-independent (e.g., `rounded_full()` avatars — convert to square in pixel mode too).

- [ ] **Step 1:** Sweep find.rs, terminal_menu.rs, layout.rs; **Step 2:** panels.rs; **Step 3:** diff/render.rs, plugin_paint.rs, control_surface.rs; **Step 4:** desktop_window_controls.rs (Windows/Linux caption buttons: remove radius, add 2px hover border in pixel mode; cannot be visually verified on macOS — code-review carefully and rely on CI/build for those targets: `cargo build --target x86_64-pc-windows-msvc` only if cross tooling exists locally, otherwise note as untested-per-platform and keep the change minimal).
- [ ] **Step 5:** `cargo test -p sleipnir_ui && cargo build -p sleipnir` — PASS; grep audit per above.
- [ ] **Step 6: Commit**

```bash
git add crates/sleipnir_ui/src/app_shell/find.rs crates/sleipnir_ui/src/app_shell/panels.rs crates/sleipnir_ui/src/app_shell/terminal_menu.rs crates/sleipnir_ui/src/app_shell/plugin_paint.rs crates/sleipnir_ui/src/app_shell/layout.rs crates/sleipnir_ui/src/diff/render.rs crates/sleipnir_ui/src/sleipnir_ui.rs crates/sleipnir_ui/src/chrome/desktop_window_controls.rs
git commit -m "feat(ui): pixel treatment sweep across inline surfaces"
```

---

### Task 8: Docs, example config, final verification

**Files:**
- Modify: `docs/settings.example.json` (add `"ui_style": "pixel"` with the other top-level keys)
- Modify: `README.md` and `README.zh.md` (one line in the config section)
- Modify: `CHANGELOG.md` (unreleased entry, following the existing format)

- [ ] **Step 1:** Edit the three docs files.
- [ ] **Step 2:** Full gate: `cargo test` (workspace) and `cargo clippy --workspace --all-targets` — resolve warnings introduced by this feature only; pre-existing warnings are out of scope.
- [ ] **Step 3:** Final manual A/B: default mode side-by-side with the pre-feature build (must be indistinguishable), pixel mode against the approved mockup (tab strip, settings, command palette, a dialog, a toast). Toggle the setting live to confirm hot reload.
- [ ] **Step 4: Commit**

```bash
git add docs/settings.example.json README.md README.zh.md CHANGELOG.md
git commit -m "docs: ui_style pixel setting"
```

---

## Done criteria

- `"ui_style": "pixel"` in settings.json restyles all chrome live; `"default"` (or absent) is visually identical to before the feature.
- All token unit tests + existing suite green; no new clippy warnings.
- Mockup fidelity: 2px borders, staircase panel corners, hard offset shadows, blocky toggle/select/stepper/buttons, square tabs and window corners.
