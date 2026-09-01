# Kitty graphics protocol: track, do not implement yet

**Status:** accepted (reopens the "no graphics protocol" boundary from M0–M15)

**Last verified:** 2026-09-01. This decision is still current. The workspace pins
`alacritty_terminal` at `561594c` and `vte` at `94ce0d5`; those revisions only
carry Sleipnir's OSC 133/9/777 hook. The terminal model still has no image store
or placement model, and `TermElement` still has no image-rendering layer. A
separate `kitty-protocol` branch in the `vte` repository is not referenced by
this workspace and is not product support.

## Context

The M0–M15 non-goals listed "Kitty graphics protocol" as explicitly out of
scope. That boundary is the weakest one we hold: kitty graphics is now the
de-facto standard for inline images in terminals (Ghostty, WezTerm, kitty
support it; the tool ecosystem — `yazi`, `nvim` image preview, `chafa`, `mpv`
`--vo=kitty`, presentation tools — is built on it). Alacritty still ships no
graphics and remains widely used, so "no graphics" is survivable, but it means
Sleipnir is incompatible with a growing ecosystem.

This ADR records the integration-cost assessment so the decision to track is
explicit rather than an unexamined default.

## Assessment of integration cost

Sleipnir's render path is **not** Alacritty's and **not** Ghostty's. It is the
GPUI text-batch path shared with Zed:

- `TermElement` is a GPUI `canvas` (`crates/sleipnir_ui/src/term_element.rs`):
  `prepaint` resolves a grid of cells into glyph text batches; `paint` draws
  background quads → selection → search/hover underlines → text batches →
  cursor, all against the character-grid `LayoutState`.
- The terminal model (`crates/terminal`) is `alacritty_terminal` (zed fork),
  which has **no** image/sixel/graphics support at all — so kitty graphics is
  purely additive at the parser level.

Bringing kitty graphics in therefore has three separable pieces:

1. **Protocol parser** (terminal crate): the kitty graphics protocol is a
   chunked, placement-based protocol over OSC (`ESC ] 1337 ; File=...` and the
   `APC ... ST` graphics payloads, or the iTerm2 `OSC 1337;File=` path). This
   needs a parser + an image store keyed by image id, with placements (a
   z-indexed list of `image_id` per screen cell/row). **Estimated: medium
   (2–4 weeks).** It is independent of the renderer and could be tested
   headlessly against the kitty spec's test images.

2. **Renderer integration** (sleipnir_ui): images must be drawn interleaved
   with cells at the correct z-order. GPUI can paint images as textured quads in
   `paint`, but the current `TermElement` only paints text batches and solid
   quads; there is no image-quad layer. This is the risky part: keeping image
   z-order correct relative to background/selection/text, handling scrolling
   (images are placement-anchored, not cell-anchored), and reflow on resize are
   all non-trivial. **Estimated: medium-high (2–4 weeks), highest-risk.**

3. **Settings + toggle**: a `graphics` opt-in (default off), mirroring how
   Ghostty/WezTerm gate it. Trivial once 1–2 land.

**Verdict:** real but additive; no fork of `alacritty_terminal` is required
(unlike OSC 133/9/777 shell-integration, which is a separate blocker — see the
roadmap). The renderer piece is the reason this is not a quick win: images do
not fit the existing "text batch only" paint pass, so a parallel image layer and
z-order/scroll/reflow semantics must be designed deliberately.

## Decision

Track, do not implement. Revisit when one of these holds:

- a user-visible `yazi` / `nvim` image-preview / `mpv --vo=kitty` use case
  becomes a frequent request, or
- the shell-integration work lands and the terminal model already gains a
  richer content layer that images can reuse.

Keep the boundary documented as "tracking" rather than "not doing".

## Consequences

- Sleipnir remains incompatible with kitty-graphics clients until this is
  implemented; document that clearly when users ask about inline images.
- The `no graphics` non-goal is rescinded; the roadmap moves this from
  "明确不做" to "跟踪".
