# Disk 3D

A resident v2 plugin that answers "where did my disk space go?" with a real 3D
chart, drawn in a terminal split.

Each cuboid is one direct child of the working directory; its height is that
child's share of the bytes. The camera rotates, so bars hidden behind others can
be brought into view, and the legend always states the exact size and percentage
of the highlighted bar.

```
[disk 3d]  crates  1.74M

                           ███
                          █████
                          █████
                          █████
                          ████▒▒▒░░░
                          ████▒▒▒░░▒▒▒░▒▒▒░░░
                          ████▒▒░░░░▒·▒▒░░░░ ·░▒▒░░░
                          ██·██▒▒░▒░░░░░▒ ▒▒░░░░ ·
                              ░░·░░▒░░░░░░░
────────────────────────────────────────────────────────────
[dir]  sleipnir_ui  840K  47.2%
[#########-----------]
bar 1 of 12  ·  yaw 40°  pitch 24°  zoom 1.0x
<◀> <▶> <▲> <▼> <+> <-> <Next bar> <Spin ½ turn> <Rescan>
```

## Why it renders this way

ADR-0004 declines the kitty graphics protocol and ADR-0017 bans images, custom
fonts and raw colour. So there is no pixel surface to draw on — but there is a
character grid with exact geometry, because the widget renderer counts one
Unicode scalar as one cell and `wrap_text` preserves spaces and honours `\n`.
A `col` of `text` nodes is therefore a stable framebuffer.

The 3D is genuine, not faked with box-drawing characters:

- **Orthographic projection.** Deliberately not perspective: a chart must stay
  measurable, and equal shares must read as equal from any angle.
- **Per-cell z-buffer.** Occlusion is resolved by depth test, not by sorting, so
  bars correctly hide each other at every rotation.
- **Lambert shading** with the light in world space, quantised onto `░▒▓`. This
  is what sells the depth — faces brighten and darken as the model turns.
- **Auto-fit framing.** The projected bounding box is measured every frame and
  mapped into the available cells, so the scene cannot drift off-surface at any
  rotation, zoom or bar count.

`CELL_ASPECT = 0.5` compensates for cells being about twice as tall as wide;
without it a cube renders as a column.

## Design constraints worth knowing

- **Node budget is the binding constraint.** One `text` per raster row means
  rows and nodes are the same currency (ADR-0017 caps a tree at 500 nodes /
  depth 16). `canvas_rows` clamps the image up front, and `clamp_to_budget` is a
  last-resort guard that trims raster rows rather than letting the host truncate
  the tree and take the legend and controls with it. A typical split uses ~42
  nodes.
- **Bars sit on a near-square grid**, not in a line. A single row of bars would
  make the depth axis carry no information, which would make the 3D decorative.
- **Small entries keep a visible plinth** (`MIN_BAR_SHARE`). Real directories are
  dominated by one entry — `target/` is ~99% of this repo — and a strictly linear
  scale renders everything else invisible and unclickable. Heights stay strictly
  proportional above the floor, and exact bytes are always in the legend.
- **Panel, not Block.** A Block is pinned to one finished run in scrollback; a
  Panel gets the room a chart needs and survives focus changes. The `PaneKey` is
  minted once and reused, so every later render replaces the same panel in place
  instead of opening new splits.
- **Buttons are the only input.** The schema has no text fields, no drag and no
  key handling for plugin surfaces (ADR-0017 constraint 3), so every camera
  control is a `btn`.
- **The spin is a bounded sweep.** The SDK holds a locked stdout for the whole
  session, so a background thread cannot render, and an endless spin would stop
  the plugin answering events. "Spin ½ turn" draws a fixed number of frames.

## Safety

- The walk is bounded by depth (6), entry count (40k) and wall clock (1.5s), and
  reports `partial` when a cap is hit — the numbers are then a lower bound, and
  the badge says so rather than quietly under-reporting.
- **Symlinks are never followed.** `fs::symlink_metadata` sizes the link itself;
  following links would both double-count and make the walk unbounded on a
  cycle. (`DirEntry::metadata()` follows links — hence the explicit call.)
- Unreadable entries are counted and surfaced, not hidden.
- `subscribe_events` is narrowed to `CwdChanged` only. Run contents, ports and
  focus are not needed and are not requested.
- A `CwdChanged` event only redraws a panel that already exists; an event never
  conjures a split the user did not ask for.

## Install

```sh
cargo build -p sleipnir-plugin-disk3d

mkdir -p ~/.config/sleipnir/plugins/disk3d
cp target/debug/sleipnir-plugin-disk3d ~/.config/sleipnir/plugins/disk3d/
cp crates/sleipnir_plugin_disk3d/plugin.json ~/.config/sleipnir/plugins/disk3d/
```

Plugins are off by default; enable them in `~/.config/sleipnir/settings.json`:

```json
{ "plugins": { "enabled": true } }
```

Start Sleipnir, approve the consent prompt (`resident`, `render_panel`,
`subscribe_events`, `read_cwd`), then run **"Disk 3D: Chart This Directory"**
from the command palette. `cd` elsewhere and the chart follows.

## Preview without launching the terminal

The widget tree is host-rendered, so this example approximates the panel on
stdout — useful when changing the raster:

```sh
cargo run -p sleipnir-plugin-disk3d --example preview -- ~/some/dir
```

## Layout

| File | Responsibility |
| --- | --- |
| `src/raster.rs` | 3D maths, z-buffer, shading, cell framebuffer. Knows nothing about disks. |
| `src/scan.rs` | The bounded filesystem walk that produces the numbers. |
| `src/view.rs` | Scan + camera → widget tree. Pure, so it is unit testable. |
| `src/main.rs` | The resident session: events, actions, panel identity. |
