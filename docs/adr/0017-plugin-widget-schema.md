# Plugin widget schema: one closed set, one renderer, three mount points

**Status:** proposed (renders the `Render` message of
[ADR-0016](0016-plugin-protocol-v2-and-trust.md))

## Context

[ADR-0016](0016-plugin-protocol-v2-and-trust.md) lets a plugin send `Render`, but
not what it may render. Four options were considered for the payload:

- **ANSI text.** Cheapest, most terminal-native, no new vocabulary — but no
  interaction, and no way to lay anything out.
- **Markdown subset.** A middle ground; the existing `syntax` crate already
  highlights fenced code.
- **Declarative widget tree.** Enough expressive power for dashboards and
  reviewable diffs, and it can carry interaction.
- **Images / kitty graphics.** Most freedom, but
  [ADR-0004](0004-kitty-graphics-track-not-implement.md) explicitly declines the
  graphics protocol, and [ADR-0007](0007-frozen-vt-fork.md) warns that a new
  feature inside the frozen parser needs its own rebase story. Out of scope.

A widget tree is chosen. The cost must be stated up front: **a schema is a
permanent public contract.** With external authors ([ADR-0016](0016-plugin-protocol-v2-and-trust.md))
anything accepted in v1 can never be removed, only deprecated. Additions are
cheap; removals are impossible. The only workable strategy is to open
deliberately narrow.

A second concern is duplication. Widgets are wanted in three places — inside
scrollback (Block), occupying a split (Panel), and in the chrome (badges, status
items). Three independent implementations would mean three schemas and three sets
of bugs.

## Decision

### One crate, three mount points

A new `sleipnir_widget` crate owns the schema, layout, rendering, hit-testing and
event plumbing exactly once. Mount points consume it:

```
        sleipnir_widget  (schema + layout + render + hit-test + events)
                 │
     ┌───────────┼───────────┐
     ▼           ▼           ▼
   Block       Panel       Chrome
```

**Panel is the first mount point; Block is deliberately later.** Panel reuses the
existing `pane_tree`, so splits, focus, zoom, tabs and session restore come free,
and it touches no coordinate math. Block requires a new row-geometry model, mouse
hit-testing changes, and scroll work ([ADR-0018](0018-block-rendering-and-coordinates.md)).
Validating the schema on the cheap mount point first means Block reduces to a
placement problem, with schema questions already settled.

### The v1 widget set is closed

```jsonc
// layout
{"t":"col","gap":1,"children":[…]}
{"t":"row","gap":2,"children":[…]}
// leaves
{"t":"text","s":"hello","fg":"accent","bold":true}
{"t":"code","lang":"rust","s":"fn main(){}"}   // via the existing `syntax` crate
{"t":"badge","s":"3000","tone":"ok"}           // ok | warn | err
{"t":"bar","v":0.6}
{"t":"spark","vs":[1,2,3,5,8]}
{"t":"sep"}
// interaction — the only interactive node
{"t":"btn","s":"Retry","action":"retry","arg":"run-42"}
```

Unknown `t` values render as an inert placeholder rather than failing the tree, so
a plugin written against a newer host degrades instead of disappearing.

### Five constraints, each load-bearing

**1. Colour is semantic tokens only** — `accent`, `ok`, `warn`, `err`, `dim`,
`fg`, `dim_bg`. No hex, no RGB. Raw colours would let plugins ignore the user's
theme; with a theme switch ([ADR-0002](0002-auto-theme-appearance.md) tracks
system appearance automatically) every such plugin would render broken. Tokens
resolve through the same `ChromeTokens` path the app's own chrome uses.

**2. Sizes are in cells, not pixels.** The terminal's unit of measure is the
character cell. Cell units make a Block's height an integer row count rather than
an arbitrary pixel value, which is what keeps
[ADR-0018](0018-block-rendering-and-coordinates.md)'s geometry tractable, and they
survive font-size changes (`IncreaseFontSize` and friends) without per-plugin
rework.

**3. Interaction is `btn` + an opaque action string, and nothing else.** No text
inputs, no drag, no focus management, no scroll containers. Text entry is what the
command palette is for. This is the single largest scope cut in this ADR: input
widgets would drag in focus order, IME, accessibility and keyboard routing, all of
which already exist for the terminal grid and would have to be reconciled with a
plugin-defined tree.

**4. No custom fonts, no images, no HTML/CSS subset.** Consistent with
[ADR-0004](0004-kitty-graphics-track-not-implement.md).

**5. Trees are bounded — 500 nodes, depth 16 — and over-budget trees are
truncated with a visible marker.** External authors will produce pathological
trees, whether by accident or otherwise, and layout cost is paid on the UI thread
and re-paid on every reflow ([ADR-0003](0003-live-pty-reflow-on-drag.md) resizes
continuously during a divider drag).

### Update model: Elm-style whole-tree replacement

```
plugin ──▶ Render { target, tree }                     push a tree
user clicks btn ──▶ host ──▶ Action { block_id, action, arg }
plugin ──▶ Render { target, tree' }                    push a new tree
```

The host diffs against the previous tree and repaints. **No patch protocol in
v1** — bounded trees make whole-tree replacement affordable, and a patch format
would be a second permanent contract with its own consistency failure modes.

Plugins hold no handles to host UI objects. The tree is data; the host owns every
element. A plugin that dies leaves a stale tree the host can render, dim, or
discard on its own terms.

### Attribution

Per [ADR-0016](0016-plugin-protocol-v2-and-trust.md) §7, every rendered surface
carries a non-suppressible marker naming the plugin that produced it. The renderer
draws it — not the mount point, and never the plugin — so it cannot be spoofed by
a crafted tree.

## Consequences

- The schema becomes a permanent public contract, versioned with the protocol.
  Starting narrow is the only lever; that will be felt as "the widget set is too
  small" long before it is felt as "the widget set is too large", and that is the
  intended direction of pressure.
- Semantic tokens and cell units mean plugin surfaces track the user's theme and
  font size for free, and keep working when either changes.
- Banning inputs keeps the app's keyboard, focus and IME model intact. Plugins
  needing structured input must route through the palette; that will be a
  recurring request, and the answer is expected to stay no in v1.
- One renderer for three mount points means one place to optimise and one place to
  fix. It also means Block and Panel cannot diverge into two dialects.
- Layout runs on the UI thread. Node/depth caps and cell-quantised sizing are what
  keep a plugin tree inside the per-frame budget that ADR-0003's live reflow
  assumes.
- `code` reuses the `syntax` crate, so plugin-rendered snippets match the diff
  inspector ([ADR-0012](0012-diff-inspector.md)) instead of introducing a second
  highlighting style.
