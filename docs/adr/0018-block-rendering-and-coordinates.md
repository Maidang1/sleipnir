# Block rendering: row geometry, pixel scrolling, and the frozen fork boundary

**Status:** proposed (places the `RenderTarget::Block` of
[ADR-0016](0016-plugin-protocol-v2-and-trust.md); renders the schema of
[ADR-0017](0017-plugin-widget-schema.md))

## Context

A Block is plugin-rendered content ([ADR-0017](0017-plugin-widget-schema.md))
placed *inside scrollback*, anchored to one Run, scrolling with the text around
it. It is the most terminal-native mount point — a terminal is a timeline, and a
Block is an entry on it — and the most expensive, because it is the only one that
breaks an assumption the renderer makes everywhere.

### The assumption

Every screen coordinate today is a linear function of a uniform row height:

```rust
// term_element.rs — y-coordinate sites: 97, 327, 581, 606, 627, 880, 932
origin.y + line as f32 * dimensions.line_height
// osc133.rs:66
absolute_to_display_line(abs, history, offset) = abs - history + offset
```

Hit-testing makes the same assumption in the inverse direction:

```rust
// mappings/mouse.rs:222 — selection, clicks, mouse reporting
let mut line = (pos.y / cur_size.line_height) as i32;
// terminal.rs:2841
let row = (pos.y / terminal_bounds.line_height()).round() as usize;
```

Blocks make row height non-uniform. Both directions must go through a new
mapping.

### What is safe

The audit result that makes this feasible: **none of those sites live in the
vendored parser.** They are in `crates/sleipnir_ui/src/term_element.rs`,
`crates/terminal/src/terminal.rs` and `crates/terminal/src/mappings/mouse.rs` —
all host-side code. `crates/terminal/src/alacritty*` is untouched.

So the grid stays a pure uniform character grid, and Blocks change only the
**grid → pixel** mapping. This is the approach Zed's editor takes for block
decorations. [ADR-0007](0007-frozen-vt-fork.md) is not endangered: no new feature
enters the frozen parser, and its rebase story is unaffected.

### What is not safe

`display_offset` — the scroll position — is owned by the grid and is **an integer
row count**. `scroll_display(Scroll::Delta(lines))` moves it by whole lines
(`terminal.rs:1248`, `alacritty.rs:203`). Today the smooth-scroll accumulator
already reflects this: `scroll_px` accumulates pixels only to convert them into a
line delta and then discards the remainder (`terminal.rs:2543-2552`).

Pixel-accurate scrolling over a document with variable-height Blocks cannot be
expressed as an integer `display_offset`. That is the real cost of this ADR.

## Decision

### 1. `RowGeometry`: one mapping, both directions

All row↔pixel conversion moves behind a single host-side type. The scattered
`* line_height` arithmetic is replaced by calls to it.

```rust
/// Host-side. Knows nothing about the grid's internals; the grid knows nothing
/// about it.
pub struct RowGeometry {
    line_height: Pixels,
    /// Ascending by anchor. Heights are integer cell rows per ADR-0017.
    blocks: Vec<(i32 /* anchor absolute line */, u16 /* height in rows */)>,
}

pub enum HitTarget {
    Cell { line: i32, column: usize },
    Block { id: BlockId, local: GpuiPoint<Pixels> },
}

impl RowGeometry {
    /// Row → pixel. A step function, no longer linear.
    fn y_for(&self, abs_line: i32) -> Pixels;
    /// Pixel → row or block. Replaces `(pos.y / line_height) as i32`.
    fn hit(&self, y: Pixels) -> HitTarget;
    /// Total document height, for scroll extent.
    fn total_height(&self) -> Pixels;
}
```

Block heights are integer row counts, because ADR-0017 specifies sizes in cells.
This keeps `y_for` exact — no floating-point accumulation across a long
scrollback.

### 2. Pixel scrolling: a host-side sub-row offset

The grid keeps its integer `display_offset`. The host adds a **sub-row pixel
offset** on top:

```rust
struct ViewportPosition {
    /// Integer row scroll — still the grid's `display_offset`.
    row: usize,
    /// Host-side remainder in [0, row_height_at(row)). Never sent to the grid.
    sub: Pixels,
}
```

- Wheel input accumulates into `sub`; whole rows spill over into
  `Scroll::Delta(lines)` as they do now. The remainder is **retained** instead of
  discarded (`terminal.rs:2551`), which is what makes scrolling pixel-accurate.
- Painting translates the whole grid by `-sub`, and paints one extra row of
  overscan at each edge.
- `scroll_to_absolute` and prompt jumps (`JumpPrevPrompt` / `JumpNextPrompt`,
  `scroll_to_anchor`) resolve through `RowGeometry` and set `sub` explicitly,
  so a jump lands a Block flush against the viewport edge rather than clipped.

The grid never learns about `sub`. This is what keeps the change out of the frozen
fork.

**Consequence accepted:** the terminal gains a scroll position the grid does not
know, so anything reading `display_offset` alone now reads an approximation.
`RowGeometry` plus `ViewportPosition` become the single source of truth for
"what is on screen where", and every consumer must be routed through them. Sites
currently reading `content.display_offset` directly
(`term_element.rs:307, 317, 323, 362, 667, 681-682`) are part of this migration.

### 3. Interaction with ADR-0003 (live reflow on divider drag)

[ADR-0003](0003-live-pty-reflow-on-drag.md) requires PTYs to reflow continuously
during a divider drag. With Blocks, each frame of a drag would otherwise
re-layout every visible widget tree, re-derive its height, and shift every anchor
below it.

**Decision: Blocks freeze during an active drag.** While a drag is in progress,
Blocks keep their last computed heights and are painted as fixed-height
placeholders; they re-layout once on drag end. Text reflow stays live, so
ADR-0003's intent — splits feel native — is preserved for the terminal content
that the ADR was written about.

This is a deliberate, scoped narrowing of ADR-0003 rather than a reversal: the
alternative (re-laying out bounded-but-nontrivial trees every frame, per
ADR-0017's node caps) risks exactly the per-frame cost ADR-0003 warns about, and
the fallback ADR-0003 itself names is resize-on-release.

### 4. Alt-screen: Blocks are hidden

Full-screen applications (vim, tmux) own the whole grid and have no scrollback
semantics; the existing gutter already disables itself under
`Modes::ALT_SCREEN` (`term_element.rs:346`). Blocks do the same: while the alt
screen is active, `RowGeometry` reports no blocks, so geometry degenerates to the
current linear mapping and every coordinate path behaves exactly as it does today.

Blocks reappear on return to the primary screen. **Accepted cost:** a Block
visibly disappears and comes back around a full-screen program. The alternative —
projecting scrollback decorations onto a grid an application believes it fully
controls — corrupts that application's display.

### 5. Selection across a Block

A Block is **not** selectable text and is skipped by selection. A selection
spanning from above a Block to below it covers the text rows on both sides and
excludes the Block's content; the copied text contains no widget text. Selection
remains a grid-coordinate concept, exactly as the grid defines it.

### 6. Lifecycle

- A Block is anchored to a `RunId`. Its position derives from that Run's
  `Anchor { line, column }` (`run_ledger::run::Anchor`), which is process-local
  and never persisted.
- History shrink (`clear`, `ED 3`) rebases Blocks with the markers, reusing
  `rebase_markers_after_history_shrink` (`osc133.rs:74`); a Block whose anchor
  fell inside the removed region is dropped.
- Scrollback eviction drops the Block with its anchor.
- Blocks are **not** restored across restarts. Anchors are process-local, and a
  restored Block would claim a scrollback position that no longer means anything.
- A Block outlives the plugin process that drew it. Its last tree stays rendered
  and is visibly marked stale, per ADR-0017's host-owns-the-tree rule.

## Consequences

- Blocks land without touching the vendored parser, so
  [ADR-0007](0007-frozen-vt-fork.md)'s freeze and rebase story hold.
- **The renderer acquires a document-layout model.** Uniform row height stops
  being an invariant; `RowGeometry` becomes load-bearing for painting,
  hit-testing, selection, search-match highlighting, gutter marks and prompt
  jumps. Every existing linear coordinate site is a migration target, and any
  future one that bypasses `RowGeometry` is a bug that will present as
  mouse-vs-paint drift.
- Scroll position is split between the grid (`display_offset`, integer rows) and
  the host (`sub`, pixels). This is the price of 1a and the main new source of
  subtle bugs: any code path that reads one without the other is wrong.
- ADR-0003 is narrowed for Blocks only (frozen during drag), with text reflow
  still live.
- Blocks vanish under the alt screen by design, and geometry provably degrades to
  today's behaviour there — which also gives a useful bisection tool: if a
  coordinate bug disappears in alt-screen, it is a `RowGeometry` bug.
- Because ADR-0017 has no scroll containers, pixel scrolling buys continuity of
  motion rather than any new capability. Blocks scroll smoothly with the text;
  they have no interior scrolling of their own. Should scroll containers ever be
  added, this decision is the prerequisite that makes them expressible.
- This is the most expensive item in the plugin programme and is sequenced last,
  after the widget schema has been validated on the Panel mount point.
