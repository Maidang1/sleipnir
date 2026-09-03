# Kitty graphics: still tracking, but the cost estimate was wrong in both directions

**Status:** accepted (supersedes the cost assessment in
[ADR-0004](0004-kitty-graphics-track-not-implement.md); the decision to track
rather than ship is unchanged)

**Supersedes:** the "Assessment of integration cost" and "Last verified"
sections of ADR-0004. The Context, Decision and Consequences of ADR-0004 stand.

## Why this is being reopened

ADR-0004 asked to revisit when "the shell-integration work lands and the
terminal model already gains a richer content layer that images can reuse."
Both have now happened: OSC 133 ships, and [ADR-0018](0018-block-rendering-and-coordinates.md)
gave the host a non-uniform row-height model (`row_geometry`) plus a host-side
painting path (`panel_scene_paint`, `DrawScene`).

That was the trigger this re-evaluation started from. It is not the most
important finding.

## The finding that matters: it was built, then deleted, unexamined

Kitty graphics was **implemented in this repository and removed two days
later**, and neither the ADR nor the changelog records that it happened.

| Commit | Date | Effect |
| --- | --- | --- |
| `51daed0` | 2026-09-01 | Adds `crates/terminal/src/kitty_graphics/` (~1,350 lines, 4 modules) plus a z-ordered image-quad layer in `term_element.rs` (+140 lines) |
| `d7ec27e` | 2026-09-02 | disk3d renders via a `WriteGraphics` host call built on it |
| `32afbbd` | 2026-09-03 | Removes all of it |

Measured at `32afbbd^`, the deleted implementation was:

| Module | Lines | Tests |
| --- | --- | --- |
| `store.rs` | 1,150 | 11 |
| `protocol.rs` | 420 | 11 |
| `transmission.rs` | 260 | 10 |
| `receiver.rs` | 248 | 4 |
| `apc_scanner.rs` | 187 | 7 |
| **Total** | **2,265** | **43** |

It was not a sketch. It had chunked APC transmission, an image store with
eviction, placements, `rebase_after_history_shrink` (the scroll problem
ADR-0004 called the risky part), Unicode-placeholder handling, and
animation/frame support.

Three process failures compound here:

1. **`51daed0` added the implementation and, in the same commit, edited
   ADR-0004 to say "Last verified: 2026-09-01 … the terminal model still has no
   image store or placement model, and `TermElement` still has no
   image-rendering layer."** That statement was false as written by the commit
   that wrote it. An ADR that contradicts its own commit is worse than no ADR:
   it launders an unverified claim as a verified one.

2. **`32afbbd` removed it as one bullet inside an unrelated refactor**
   ("extract atomic_write crate, drop kitty graphics, split app_shell plugin
   UI"), with no recorded reason. A 2,265-line capability with 43 tests should
   not leave the tree as a sub-bullet.

3. **The removal was incomplete.** `base64`, `flate2`, `png` and `memmap2` are
   still declared in the workspace `Cargo.toml` with **zero** consuming crates.

**Before any decision about kitty graphics, the real question is why a
substantial, tested implementation was written and deleted inside 48 hours
without a written rationale.** If it was abandoned because it did not work, that
finding is the single most valuable input to this ADR and it has been lost. If
it was abandoned because it was out of scope, that is an ADR-level decision that
skipped the ADR. This document cannot recover that reason; the author can.

## Corrected cost assessment

ADR-0004 estimated 2–4 weeks for the parser and 2–4 weeks for the renderer,
calling the renderer "the risky part." The deleted attempt is evidence on both
halves, and it revises the estimate **down for the parser and up for the parts
ADR-0004 did not enumerate**.

### Cheaper than estimated

- **Parser.** Done once, in ~1,100 lines with 32 tests, and headless-testable
  as ADR-0004 predicted. Recoverable from git rather than rewritten.
- **Row geometry.** ADR-0004 predates ADR-0018. Non-uniform row heights,
  `y_for` / `hit` as exact inverses, and history-shrink rebasing now exist and
  are tested. An image is, geometrically, a Block-shaped hole — the hardest
  coordinate work is already paid for.
- **Host-side painting.** `panel_scene_paint` established that the host can
  own projection and paint non-text content against real pixel bounds.

### More expensive than stated

ADR-0004's renderer estimate covered z-order, scrolling and reflow. It did not
cost these, and they do not shrink:

- **Pixel-accurate scrolling is still unsolved.** ADR-0018 §"What is not safe"
  is explicit: `display_offset` is an integer row count owned by the grid, and
  pixel-accurate scrolling over variable-height content "cannot be expressed as
  an integer `display_offset`." Blocks live with this because their height is an
  integer row count (`Block.height: u16`). An image whose natural height is not
  a whole number of rows either gets rounded — visible drift against the text it
  is anchored to — or forces the split-scroll problem ADR-0018 deferred.
- **Memory is a denial-of-service surface.** kitty caps each buffer at 320MB.
  The deleted `store.rs` had eviction, but a per-buffer quota interacts with
  session restore, multiple panes and the Run Ledger. This is policy work, not
  parser work.
- **`ADR-0007` (frozen VT fork) exposure.** APC arrives through the vendored
  parser. ADR-0018 was safe precisely because "none of those sites live in the
  vendored parser." Graphics does not obviously inherit that property, and each
  hook into the frozen fork carries a rebase cost forever.
- **Attribution/selection semantics.** ADR-0018 decision 5 keeps widget text out
  of copied selections. Images need the same answer, and "what does copying a
  region containing an image do" is unanswered.

## Decision

**Keep tracking. Do not schedule implementation. Do three things now, none of
which is the feature.**

1. **Record why `32afbbd` removed it.** One paragraph from the author, appended
   to this ADR. Until that exists, any estimate here is guessing against
   evidence that someone already has.
2. **Delete the four orphaned dependencies** (`base64`, `flate2`, `png`,
   `memmap2`), or state which upcoming work keeps them.
3. **Correct the false "Last verified" claim in ADR-0004** so it no longer
   asserts something its own commit disproved.

The revisit triggers from ADR-0004 have technically fired, but the honest
reading is that the enabling work (ADR-0018) lowered the *geometry* cost while
leaving the *scroll* cost exactly where it was — and scroll is the one ADR-0018
itself flagged as unresolved. Shipping graphics on top of an integer
`display_offset` would either ship visible drift or force the pixel-scroll
rewrite as an undeclared prerequisite.

If it is scheduled later, the sequence is: recover `51daed0`'s parser from git →
decide the pixel-scroll question on its own merits (it is owed to Blocks
regardless) → then the renderer. Not the reverse.

## Consequences

- Sleipnir stays incompatible with kitty-graphics clients. Unchanged.
- The claim "we have never implemented this" is retired; the repository history
  says otherwise and the next person to cost this work will find `51daed0`.
- Pixel-accurate scrolling is now named as a **shared prerequisite** of both
  Blocks and graphics, rather than a Block-local deferral. Whichever ships
  first pays for it.
- The four orphaned deps are a live finding, not a hypothetical: they are
  compiled into the dependency graph for no consumer.
