# Disk 3D

A resident v2 plugin that answers "where did my disk space go?" with a real 3D
chart, drawn in a terminal split.

Each cuboid is one direct child of the working directory; its height is that
child's share of the bytes. The camera rotates — drag to spin, wheel to zoom —
so bars hidden behind others can be brought into view, and the legend always
states the exact size and percentage of the highlighted bar.

## How it renders

The plugin projects the chart into a widget tree and `render`s it into a panel.
There is no separate host-side scene call. The rasteriser's framebuffer is
character cells (orthographic projection, per-cell z-buffer, Lambert shading
quantised onto `░▒▓`). It looks like this:

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

`CELL_ASPECT = 0.5` compensates for cells being about twice as tall as wide.

## Design constraints worth knowing

- **The scene is bounded.** The host caps a scene at 256 bars on a 64×64 grid and
  rejects bars outside the grid, so a malformed or hostile plugin cannot force an
  absurd layout. The scanner folds to `MAX_BARS` (12) long before that.
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
- **The camera is host-driven.** Dragging rotates and the wheel zooms, handled
  entirely host-side for smoothness; the plugin's buttons remain for keyboard-free
  and discoverable control, and the two paths reconcile via the no-loopback rule
  above. In the text fallback the buttons are the only input.
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
cargo build --manifest-path examples/sleipnir_plugin_disk3d/Cargo.toml

mkdir -p ~/.config/sleipnir/plugins/disk3d
cp examples/sleipnir_plugin_disk3d/target/debug/sleipnir-plugin-disk3d ~/.config/sleipnir/plugins/disk3d/
cp examples/sleipnir_plugin_disk3d/plugin.json ~/.config/sleipnir/plugins/disk3d/
```

Plugins are off by default; enable them in `~/.config/sleipnir/settings.json`:

```json
{ "plugins": { "enabled": true } }
```

Start Sleipnir, approve the consent prompt (`resident`, `render_panel`,
`subscribe_events`, `read_cwd`), then run
**"Disk 3D: Chart This Directory"** from the command palette. `cd` elsewhere and
the chart follows.

## Preview without launching the terminal

The widget tree is host-rendered, so this example approximates the panel on
stdout — useful when changing the raster:

```sh
cargo run --manifest-path examples/sleipnir_plugin_disk3d/Cargo.toml --example preview -- ~/some/dir
```

## Layout

| File | Responsibility |
| --- | --- |
| `src/raster.rs` | 3D maths, z-buffer, shading, cell framebuffer (text fallback). Knows nothing about disks. |
| `src/scan.rs` | The bounded filesystem walk that produces the numbers. |
| `src/view.rs` | Scan + camera → scene (`build_scene_data`) / widget tree. Pure, so it is unit testable. |
| `src/main.rs` | The resident session: events, actions, camera sync, panel identity. |
