# M11: Visual Polish — Cursor Fade Blink, Scroll Momentum, URL Hover Feedback

| Field | Value |
|-------|-------|
| **Status** | Draft |
| **Date** | 2025-01-20 |
| **Scope** | `term_element.rs` paint layer + `TermView` animation state |
| **Goal** | Native macOS visual refinement — make Sleipnir feel as polished as a first-party Apple app |

---

## Overview

Three focused visual improvements that elevate Sleipnir's "native feel" without touching tab/session/settings architecture. All changes are confined to the terminal paint layer and view state.

---

## 1. Cursor Blink Ease-In-Out

### Problem

Terminal cursor blink is a hard on/off toggle. macOS native text cursors (TextEdit, Xcode, Notes) use a smooth ease-in-out fade. The hard blink reads as dated and un-native.

### Current state

- Backend emits `Event::BlinkChanged(bool)` when the app changes blink mode.
- `term_element.rs` `paint_terminal_cursor` paints at full alpha or not at all (via `CursorShape::Hidden`).
- Settings expose `blinking: "terminal_controlled"` which defers to the shell/app.

### Design

Introduce a blink animation state on `TermView`:

```rust
struct BlinkState {
    /// Current opacity: 0.0 (invisible) to 1.0 (fully visible).
    phase: f32,
    /// Whether currently fading in (true) or out (false).
    rising: bool,
    /// Whether blink is active (terminal_controlled or forced on).
    active: bool,
}
```

**Behavior:**

- **Cycle:** 530ms half-period (macOS standard). Total cycle = ~1060ms.
- **Curve:** Ease-in-out (cubic or cosine — `phase = 0.5 - 0.5 * cos(π * t / half_period)`).
- **Keystroke reset:** Any input resets `phase` to 1.0 and restarts the timer. Cursor stays solid while typing.
- **Steady mode:** When `blinking == false` (app hides blink), lock `phase` at 1.0. No timer runs.
- **Focus loss:** When pane loses focus, paint a `HollowBlock` at full alpha (existing behavior). No blink.

**Paint integration:**

`paint_terminal_cursor` receives `blink_alpha: f32` and multiplies the cursor color's alpha channel:

```rust
let cursor_color = palette.cursor.opacity(blink_alpha);
```

**Timer mechanism:**

Use GPUI's frame-request or a repeating `cx.spawn` timer (~16ms tick for 60fps smoothness). On each tick, advance `phase` along the cosine curve and `cx.notify()` to trigger repaint.

**Settings interaction:**

| `blinking` value | Behavior |
|------------------|----------|
| `"terminal_controlled"` | Respect app's DECSCUSR; animate when blinking=true |
| `"on"` | Always animate |
| `"off"` | phase locked at 1.0 |

### Files changed

| File | Change |
|------|--------|
| `crates/sleipnir_ui/src/sleipnir_ui.rs` | Add `BlinkState` field to `TermView`; timer setup/teardown; keystroke reset |
| `crates/sleipnir_ui/src/term_element.rs` | Pass `blink_alpha` into `paint_terminal_cursor`; apply to cursor color |

---

## 2. Scroll Momentum

### Problem

macOS trackpad users expect inertia scrolling (flick and release → content coasts to a stop).

### Current state

`terminal.rs` `determine_scroll_lines` handles `TouchPhase`:

```rust
TouchPhase::Started => { self.scroll_px = px(0.); None }
TouchPhase::Moved => { /* accumulate delta, return scroll lines */ }
TouchPhase::Ended | TouchPhase::Cancelled => None,
```

### Analysis

macOS delivers momentum-phase scroll events as additional `NSEvent` with `momentumPhase != .none` after the user lifts their fingers. GPUI bridges these as `ScrollWheelEvent` with `TouchPhase::Moved` (the delta comes from the system inertia simulation).

The key insight: `TouchPhase::Ended` fires once when fingers lift, then momentum events arrive as new `Started` → `Moved` sequences (or continued `Moved` depending on GPUI's mapping).

### Action plan

1. **Verify empirically:** Run Sleipnir, flick-scroll on trackpad, observe whether content coasts.
2. **If momentum already works:** Mark as "already implemented" — no code change.
3. **If momentum is missing:** Investigate whether GPUI filters momentum-phase NSEvents or whether `TouchPhase::Ended` resets state prematurely. Fix will be minimal (likely removing the `scroll_px = 0` reset on `Started` if it fires between direct and momentum phases, or not resetting on `Ended`).

### Expected outcome

High probability: already working. Worst case: a 2-3 line fix in `determine_scroll_lines`.

### Files changed (if needed)

| File | Change |
|------|--------|
| `crates/terminal/src/terminal.rs` | Adjust `TouchPhase` handling in `determine_scroll_lines` |

---

## 3. URL Hover Visual Feedback

### Problem

⌘+hover on a URL detects and stores the link (backend already does this), but the UI shows no visual affordance — no underline, no cursor change. Users don't know something is clickable until they try clicking.

### Current state

- Backend: `schedule_find_hyperlink` runs on ⌘+mousemove, stores result in `last_content.last_hovered_word: Option<HoveredWord>`.
- `HoveredWord` contains `word_match: Range` (terminal grid coordinates of the link text).
- Click with ⌘ held triggers `Event::Open(target)` → `open_navigation_target` → `cx.open_url()`.
- UI: `term_element.rs` sets `CursorStyle::IBeam` unconditionally. Never reads `last_hovered_word`. No underline.

### Design

**In prepaint:**

Read `content.last_hovered_word` and convert `word_match` range to underline rects:

```rust
// In LayoutState:
hovered_underlines: Vec<UnderlineRect>,

struct UnderlineRect {
    line: i32,
    start_col: i32,
    end_col: i32,
}
```

Convert using the same `range_rects`-style logic already used for selection/search highlights.

**In paint:**

1. Draw underlines: For each `UnderlineRect`, paint a 1px rect at the bottom of the line-height span. Color: `palette.ansi[4]` (blue from theme) at ~0.8 alpha — consistent with how web browsers style links.

2. Cursor style: When `hovered_underlines` is non-empty, use `CursorStyle::PointingHand` instead of `IBeam`:

```rust
let cursor_style = if layout.hovered_underlines.is_empty() {
    gpui::CursorStyle::IBeam
} else {
    gpui::CursorStyle::PointingHand
};
window.set_cursor_style(cursor_style, &layout.hitbox);
```

**Lifecycle:** No extra state management needed. When ⌘ is released, backend clears `last_hovered_word` → next frame has no underlines → cursor reverts to IBeam. All reactive via existing `cx.notify()` on mouse move.

### Files changed

| File | Change |
|------|--------|
| `crates/sleipnir_ui/src/term_element.rs` | Add `hovered_underlines` to `LayoutState`; read from content in prepaint; paint underlines; conditional cursor style |

---

## Summary

| Feature | Complexity | Risk | Dependencies |
|---------|-----------|------|--------------|
| Cursor blink ease-in-out | Medium | Low (isolated to view/paint) | None |
| Scroll momentum | Small (likely 0 change) | None | Verify GPUI behavior |
| URL hover feedback | Small | Low (data already available) | None |

All three features are independent and can be implemented/shipped in any order.

## Non-goals

- Cursor position lerp/interpolation (wrong for terminals)
- Overscroll bounce (wrong for terminals)
- URL text recoloring (too invasive for cell paint pipeline)
- Hover tooltip showing full URL (future enhancement)
- Theme switch cross-fade animation (M12+)
- Tab open/close animation (M12+)

## Testing

- Cursor blink: Visual inspection across all cursor shapes (block, bar, underline) × blink on/off × focused/unfocused.
- Scroll: Trackpad flick on macOS — confirm coast behavior.
- URL hover: ⌘+hover over `https://...` in terminal output — confirm underline appears, cursor changes, both disappear on ⌘ release.
