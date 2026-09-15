# Code quality handoff

Snapshot of remaining structural work after the 2026-09-14 thermo-nuclear
review and the following implementation passes. This is not a product changelog.

Do the remaining items **in this order**. Each item is a model change, not a
file split. Splitting `registry.rs` or adding another `impl AppShell` file
without deleting a concept is the failure mode of the previous extraction.

## Already landed (do not redo)

1. **`sleipnir_paths`**. Canonical `~/.config/sleipnir` (not
   `dirs::config_dir()` on macOS). Settings, plugins, grants, `control.sock`,
   and `agent-control.sock` all derive from one directory. Env overrides stay
   in `sleipnir_ctl::socket_path` and `agent_coordination::default_socket_path`.
2. **Terminal pointer model**. `PointerSession` (`Idle` / `AppMouse` /
   `PendingLink` / `Selecting`) plus hover on `InteractionState`.
   `Terminal::hovered_word()` is `None` in mouse-mode by construction. Dead
   gutter (paint, click, `Event::GutterClicked`, `GutterMark`) is gone. OSC-8
   opens through `Event::Open`, not `cx.open_url`. `NewNavigationTarget` is
   gone.
3. **Keyboard owner**. `InputMode` + `AppShell::handle_capture_key` replaced
   the overlay if-ladder in `Render`. TermView no longer handles NewTab /
   settings / theme actions and no longer emits `Request*`; those bubble to
   AppShell, same as CloseTab.
4. **Partial protocol cleanup**. Host `Permission` and identity `to_v2` are
   gone. Manifests use `plugin_protocol::v2::Capability`.
   `MAX_SEND_TEXT_CHARS` lives in `plugin_protocol` and is re-exported by the
   SDK / `plugin_host_calls`.
5. **Registry mailbox, framing, claim**. Per-session effect mailboxes;
   interrupt names a task (idle interrupt is rejected). `try_claim(seq) ->
   ClaimedEffect` with `commit` / drop; sidecar `Mutex<()>` is gone. A
   `PromptDelivered` ack is `Dispatching -> Running`. JSON packing lives in
   `frame.rs`; `Request::*Page` is gone; the client reads frames on one
   connection. `TaskStatus::Accepted` is gone. Errors are `CoordError`.
   Coordination `PROTOCOL_VERSION` is `2`. `registry.rs` is still one file
   (ready to split session / task / effects / facts / validate).
6. **DrawScene out of v2**. `Capability::HostCallDrawScene`,
   `HostCall::DrawScene`, `HostCallResult::SceneOk`, and `SceneData` /
   `SceneBar` / `SceneCamera` are gone from `plugin_protocol::v2`. SDK
   `draw_scene` is gone. disk3d renders a widget tree. Host-local 3D types
   remain in `panel_scene_paint.rs` for existing panel paint/camera; they
   are not protocol types.

Verified after that pass: `agent_coordination` (85), `sleipnir_plugin_agents`
(106), `plugin_host` (78), `plugin_protocol` (23), `sleipnir_ui --lib` (357),
disk3d example (41). Re-run those packages after each item below.

## Remaining work

Do these **in this order**.

### 1. PluginHost owns surfaces; AppShell paints

Previous extraction sliced `impl AppShell` across ~20 files. Ownership did
not move. `PluginRuntime` is a process `Global`; execution is per-window
(`PanelRegistry`, `ChromeRegistry`, `PluginEventWatch`, consent, camera
drag, `handle_host_call`). `PluginDispatcher` walks every `AppShell` every
16 ms.

Also still true:

- `AppShell::handle_host_call` and `control_surface::dispatch` are two hosts
  for the same verbs (list / read / send / open / focus / close). Send paths
  differ (`insert_text` vs `input_bytes`).
- `term_element.rs` (~2011 lines) paints plugin blocks and calls
  `plugin_runtime::push_action`. Dual `LaidOutKind` painters (panel GPUI
  divs vs terminal quads) with "must not drift" comments.
- Panel leaf is `LeafContent::Panel { plugin_id: String }` plus a parallel
  `PanelRegistry` keyed by `PaneKey`.
- Agents `adapter.rs` keeps a second session/pane map on top of the
  registry. Launch is not atomic: PTY exists before `BindPane`.
- `plugin_host` supervisor has two maps (`live` by plugin id, `active` by
  instance id) and a two-phase OnDemand connect.
- Host `PluginLifecycle` is still a clone of wire `Lifecycle`.
- Three `Anchor` structs (`plugin_protocol::v2`, `run_ledger`,
  `row_geometry`) with the same `{ line, column }` and the same
  "never persist" comment. `RunId` / `PaneKey` were already single-sourced.

**Do:** `Workspace` (tabs / focus / close) + `PluginHost` (session / surfaces
/ consent / host calls) + `OverlayStack`. AppShell becomes a compositor.
One `WorkspaceIo` for ctl and plugins. `handle_host_call` becomes
`reply(plan.execute(io))`. One `LaidOutPainter`, or a `BlockOverlay`
element so the grid painter never imports `plugin_runtime`.
`pub use plugin_protocol::v2::Anchor` in ledger and row geometry.

Template for Agents: `sleipnir_plugin_runledger` (`rows` / `state` / `view`).
Leave `plugin_grants` alone.

Optional leftover of the registry pass: split `registry.rs` now that mailbox
+ framing landed (session / task / effects / facts / validate). Tests move
with the domain they pin. Host-local `panel_scene_paint` can die with the
dual painters in this item.

### 2. Grow `atomic_write` into the real write discipline

The crate documents one discipline and only exports `save_atomic` +
quarantine.

Still duplicated:

- Settings and run ledger each open a sibling `.lock`, `lock()`, body,
  `unlock()`.
- Updater reimplements stage/sync/rename with a different tmp name
  (`path.with_extension("json.tmp")` vs sibling `.tmp`).
- Health marker and active pointer are more copies.
- First settings create and Claude hook install use raw `fs::write`.
- `sleipnir_settings` is a GPUI kitchen sink, so `config_dir` used to be
  cloned (that clone is gone; the kitchen sink is not). `merge_file` is
  ~130 lines of `if let Some`. `inject_osc133` / `run_ledger` / `theme`
  exist at top level and under `terminal { }`. `persist_terminal_bool`
  is stringly JSON. `agent_hooks.rs` still installs into `~/.claude` /
  `~/.codex` with unlocked writes.

**Do:** `atomic_write::with_file_lock` plus `save_atomic` options (`mode`,
`parent_mode`, `exclusive_create`). Settings, ledger, grants, transaction,
health marker all call that. One in-memory settings document;
`update(|s| …)` instead of five clone-patch-apply setters. Move hooks to
the agents plugin; generate the socket with
`sleipnir_paths::agent_control_socket_path` (already used) and patch
Claude JSON with the same lock.

Themes: 14 copy-paste `fn mocha() -> TerminalPalette` constructors should
become one `ThemeSpec` table. That is data wearing a Rust costume.

### 3. Finish Terminal leftovers (still a GPUI god object)

`PointerSession` landed. The rest of the Terminal judo did not.

- `impl Terminal` in `crates/terminal/src/lib.rs` is still ~1600 production
  lines. `ViewportState` / `InteractionState` / `SemanticsState` are data
  bags; methods still live on `Terminal`.
- The crate header still claims "No GPUI rendering lives here." `Terminal`
  takes `Window` / `Mouse*Event` / `Context` and defines `actions!`.
- `insert_zed_terminal_env` still writes `ZED_TERM=true` and
  `TERM_PROGRAM=zed` (`crates/terminal/src/lib.rs`).
- Coordinate conversion is still not one `AbsLine` type (`row_map` vs
  `absolute_to_grid_line` vs `jump_prompt` vs vi selection synthesizing
  pixels).
- `Osc133Scanner` / `scan_osc_notify` remain public for unit tests; production
  OSC comes from the alacritty backend.

**Do:** move methods onto the three state structs (or sibling modules
`viewport.rs` / `interaction.rs` / `semantics.rs` as `impl Terminal` that only
orchestrate). Change env to `SLEIPNIR_TERM` / `TERM_PROGRAM=sleipnir`. Do not
extract twenty one-function files while leaving hover logic correct but
geometry still forked.

**Do not:** resurrect gutter paint.

### 4. Finish InputMode leftovers (derived, not owned)

`InputMode` is a read-only projection. Confirm, tab menu, terminal menu, and
rename are still `Option` fields on `AppShell`. Illegal combinations remain
representable. `ui_mode.rs` says this out loud.

- `crates/sleipnir_ui/src/ui_mode.rs` (`OverlayKind`, `InputMode`)
- `crates/sleipnir_ui/src/app_shell/mod.rs` (`close_confirm`, `tab_menu`,
  `terminal_menu`, `rename`, `input_mode()`, `handle_capture_key`)
- `command_dispatch.rs` is still a 40-arm trampoline onto `self`.
- `AppShell::render` still polls plugins and pane facts as a side-effect pump
  (`poll_plugin_events`, `sync_plugin_surfaces`).

**Do:** make one owned input-mode enum so Confirm / Menu / Rename cannot
coexist with a modal overlay. Keep `dispatch_command(CommandId)` as the only
command entry. Move the 16 ms plugin pump and the per-frame event watch off
`Render` (that overlaps item 1 above).

## What not to do

- Do not extract more `impl AppShell` / `impl Terminal` files while the
  owner is still the god object.
- Do not extend v2 for another product-shaped verb like `DrawScene`.
- Do not add another `Option<FooState>` overlay. Finish `InputMode` first.
- Do not resurrect gutter triangles.
- Do not clone `~/.config/sleipnir` again. Import `sleipnir_paths`.

## Suggested next session

Start at **PluginHost owns surfaces (section 1)**. Registry mailbox/framing
and DrawScene-out-of-v2 have landed. Host-local `panel_scene_paint` can die
with the dual painters in that item.

```bash
cargo test -p agent_coordination
cargo test -p sleipnir_plugin_agents
cargo test -p plugin_host -p plugin_protocol -p sleipnir_ui --lib
```
