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
   `PendingLink` / `Selecting`). Entering `AppMouse` (and any sync while
   mouse-mode is on) clears host hover rather than filtering it at read.
   Dead gutter (paint, click, `Event::GutterClicked`, `GutterMark`) is gone.
   OSC-8 opens through `Event::Open`, not `cx.open_url`. `NewNavigationTarget`
   is gone.
3. **Keyboard owner is owned**. `AppShell.input: InputMode` holds Confirm /
   Consent / TabMenu / TerminalMenu / Rename / Find / Overlay. Those are no
   longer sibling `Option`s. `OverlayKind` has no `None` and no
   `PluginConsent`. Capture-key matches `InputOwner` (a copy tag so the
   match does not hold a borrow) and every modal swallows non-platform keys.
   TermView no longer handles NewTab / settings / theme.
4. **Partial protocol cleanup**. Host `Permission` and identity `to_v2` are
   gone. Manifests use `plugin_protocol::v2::Capability`.
   `MAX_SEND_TEXT_CHARS` lives in `plugin_protocol` and is re-exported by the
   SDK / `plugin_host_calls`.
5. **Registry mailbox, framing, claim**. Per-session effect mailboxes;
   interrupt names a task (idle interrupt is rejected). `try_claim(seq)` is
   a seq-typed ack lease: `ClaimedEffect::commit(ClaimOutcome)` cannot name
   another seq; public `apply` of `*Delivered` / `BindPane` /
   `DeliveryFailed` is `EffectNotClaimed`. Drop releases the flag; the
   effect stays queued. Takeover drains coordinator Prompt/Interrupt/Close
   (and Launch) and leaves `FocusRequested`. `PromptDelivered` is
   `Dispatching -> Running`. JSON packing lives in `frame.rs`;
   `Request::*Page` is gone; the client reads frames on one connection.
   `TaskStatus::Accepted` is gone. Errors are `CoordError`. Coordination
   `PROTOCOL_VERSION` is `2`. Do **not** split `registry.rs` until a concept
   is deleted (tests are most of the line count).
6. **DrawScene out of v2, and host-local 3D gone.**
   `Capability::HostCallDrawScene`, `HostCall::DrawScene`,
   `HostCallResult::SceneOk`, and `SceneData` / `SceneBar` / `SceneCamera`
   are gone from `plugin_protocol::v2`. SDK `draw_scene` is gone. disk3d
   renders a widget tree. Host-local 3D (`panel_scene_paint`, `set_scene`,
   camera drag) is deleted; the host paints the widget tree only. Do not
   invent a replacement 3D protocol. PluginHost ownership has not landed.
7. **One `Anchor`**. `plugin_protocol::v2::Anchor` is the type; ledger and
   row geometry `pub use` it. `plugin_block` no longer converts between two
   copies.
8. **One `Lifecycle`**. Host `PluginLifecycle` is
   `pub use plugin_protocol::v2::Lifecycle`. Manifests and `Ready` share the
   enum. ctl `Send` uses `insert_text`, same as plugin `SendText`.

Verified after that pass: `agent_coordination` (85), `sleipnir_plugin_agents`
(106), `plugin_host` (78), `plugin_protocol` (23), `sleipnir_ui --lib` (357),
disk3d example (41). Re-run those packages after each item below.

## Remaining work

Do these **in this order**.

### 1. PluginHost owns surfaces; AppShell paints

**Slice 1 landed (WorkspaceIo + per-window PluginHost field):**

- **One `WorkspaceIo` for ctl and plugins.** `plugin_host_calls::WorkspaceIo`
  is the single executor for the workspace verbs (`list_terminal_panes`,
  `read_screen`, `open_pane`, `focus_pane`, `send_text`, `send_key`,
  `request_close_pane`, `scroll_to_run`). `CallPlan::execute(&mut impl
  WorkspaceIo) -> HostCallResult` is the only place those side effects run,
  and it is unit-tested against an in-memory fake (`FakeWorkspace`).
  `AppShell::handle_host_call` is now `authorize_and_plan → execute → reply`:
  the ~150-line match of side effects on `AppShell` is gone, replaced by a
  `ShellWorkspaceIo` adapter. `control_surface::dispatch` backs the same trait
  with `AppWorkspaceIo` (over the live windows), so ctl `ls` / `capture` /
  `send` share the pane walk and `insert_text` write with the plugin verbs
  instead of a parallel `list_panes` / `view_for_pane` copy. `Wait` stays
  ctl-only (no plugin twin). `Notify` is a host side effect next to `execute`,
  never a `WorkspaceIo` method. Dead `filter_listed_panes` / `send_text_result`
  helpers are deleted (`live_terminal_panes` already excludes panels).
- **Per-window `PluginHost` field.** `AppShell` holds one `plugin: PluginHost`
  (`crates/sleipnir_ui/src/plugin_window.rs`) instead of three sibling
  `plugin_panels` / `plugin_chrome` / `plugin_watch` fields. The three
  registries are private; every call site goes through a `PluginHost` method
  (`apply_panel_render`, `apply_chrome_status`, `sync_chrome_live`,
  `mark_panels_stale`, `watch_*`, the panel accessors), not `self.plugin_panels`
  from twenty files. AppShell still paints from it and keeps `InputMode` / tabs
  / focus. `PluginRuntime` stays a process `Global` (supervisor / catalog /
  16 ms pump); surfaces are per-window (tab detach transfers panel surfaces
  through `clone_panel_surfaces` / `insert_panel_surfaces`).

**Slice 2a landed (BlockOverlay):**

- `term_element.rs` no longer imports `plugin_runtime`. Block painting moved to
  GPUI div overlays in `TermView::render`, using the shared `paint_laid_out` /
  `paint_node` from `app_shell/plugin_paint.rs`. The quad-based
  `paint_laid_node` / `paint_block` and the dual `LaidOutKind` match in
  `term_element.rs` are deleted. Block clicks route through
  `TermView::try_block_click` (uses `terminal.hit_local` + `push_action`).
  Alt-screen still skips blocks.

**Slice 2b landed (PanelView tree-owned, PanelRegistry gone from PluginHost):**

- `LeafContent::Panel(PanelView)` owns the `PanelSurface` directly in the pane
  tree. `PanelView` is a plain struct (not a gpui Entity) holding the surface
  data. `PanelRegistry` is no longer used by `PluginHost` — panel surfaces
  travel with the tree on clone/detach instead of through
  `clone_panel_surfaces` / `insert_panel_surfaces`.
- `apply_panel_render` in `plugins.rs` does its own grant/terminal/occupied/
  owner-instance checks inline and writes directly to the tree via
  `update_panel_surface` (Replace) or `insert_panel_leaf` (Create).
- `render_plugin_panel` reads the surface from the tree leaf instead of
  `self.plugin.panel()`.
- `sync_plugin_surfaces` walks tree leaves to mark stale instead of
  `plugin.mark_panels_stale()`.
- Close and tab-detach paths no longer call `remove_panel` / `remove_panels`;
  dropping the `PaneNode` drops the surface.

**Still AppShell-owned / not done (slice 2c+ of this item):**

- `AppShell::render` still drives the pane-facts refresh side-effect pump
  (item 4). The 16 ms plugin pump already runs off `Render` in
  `plugin_dispatch` (poll vs route split); the event watch is walked there.
- Agents `adapter.rs` second session/pane map and launch atomicity are
  untouched. `plugin_host` supervisor still has two maps.

**Do (remaining):** Delete dead `PanelRegistry` code from `plugin_panel.rs`
(the struct, its methods, and its tests). Keep `PanelSurface` and `ApplyPanel`.
Keep `InputMode` as the overlay stack and `plugin_grants` alone.

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

### 4. Move the plugin pump off Render

`InputMode` is owned. What remains:

- `command_dispatch.rs` is still a 40-arm trampoline onto `self` (fine as a
  dispatcher; the problem is AppShell still being the god object).
- `AppShell::render` still polls plugins and pane facts as a side-effect pump
  (`poll_plugin_events`, `sync_plugin_surfaces`).

**Do:** keep `dispatch_command(CommandId)` as the only command entry. Move
the 16 ms plugin pump and the per-frame event watch off `Render` (that
overlaps item 1 above).

## What not to do

- Do not extract more `impl AppShell` / `impl Terminal` files while the
  owner is still the god object.
- Do not extend v2 for another product-shaped verb like `DrawScene`.
- Do not add another `Option<FooState>` overlay. Put new owners on
  `InputMode`.
- Do not resurrect gutter triangles or host 3D scene types.
- Do not clone `~/.config/sleipnir` again. Import `sleipnir_paths`.
- Do not split `registry.rs` without deleting a concept.

## Suggested next session

Start at **PluginHost owns surfaces (section 1)**. Registry claim lease,
owned `InputMode`, host 3D deletion, and single `Anchor`/`Lifecycle` have
landed. Dual LaidOutKind painters and the Render plugin pump remain in that
item. PluginHost ownership has not landed.

```bash
cargo test -p agent_coordination
cargo test -p sleipnir_plugin_agents
cargo test -p plugin_host -p plugin_protocol -p sleipnir_ui --lib
```
