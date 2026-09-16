# Code quality handoff

Snapshot of remaining structural work after the 2026-09-14 thermo-nuclear
review and the implementation on `feat/code-clean` through `58c0dc1`
(2026-09-16). This is not a product changelog.

Each remaining item is a model change, not a file split. Splitting
`registry.rs` or adding another `impl AppShell` file without deleting a
concept is the failure mode of the previous extraction.

## Already landed (do not redo)

1. **`sleipnir_paths`**. Canonical `~/.config/sleipnir` (not
   `dirs::config_dir()` on macOS). Settings, plugins, grants, `control.sock`,
   and `agent-control.sock` all derive from one directory. Env overrides stay
   in `sleipnir_ctl::socket_path` and `agent_coordination::default_socket_path`.
2. **Terminal pointer model**. `PointerSession` (`Idle` / `AppMouse` /
   `PendingLink` / `Selecting`). Entering `AppMouse` clears host hover.
   Gutter (`Event::GutterClicked`, `GutterMark`) is gone. OSC-8 opens through
   `Event::Open`. `NewNavigationTarget` is gone. Hover updates live on
   `InteractionState`. `insert_sleipnir_terminal_env` writes `SLEIPNIR_TERM`
   / `TERM_PROGRAM=sleipnir`. `AbsLine` is the one absolute-row type
   (`to_grid` / `to_display`). The crate header admits GPUI input events;
   `sleipnir_ui` owns painting.
3. **Keyboard owner is owned**. `AppShell.input: InputMode` holds Confirm /
   Consent / TabMenu / TerminalMenu / Rename / Find / Overlay. `OverlayKind`
   has no `None` and no `PluginConsent`. Capture-key matches `InputOwner`.
   Overlay click-away goes through `set_input` / AppShell `dismiss_*`, not
   `InputMode` fields directly.
4. **Partial protocol cleanup**. Host `Permission` and identity `to_v2` are
   gone. Manifests use `plugin_protocol::v2::Capability`.
   `MAX_SEND_TEXT_CHARS` lives in `plugin_protocol`.
5. **Registry mailbox, framing, claim**. Per-session effect mailboxes;
   interrupt names a task. `try_claim(seq)` is a seq-typed ack lease.
   `ClaimedEffect::commit_launch` / `commit_ok` ack that seq; **drop is the
   only releaser** (a failed ack still ends the lease). Public `apply` of
   delivery acks is `EffectNotClaimed`. Takeover drains coordinator
   Prompt/Interrupt/Close/Launch and leaves `FocusRequested`. JSON packing
   lives in `frame.rs`; `Request::*Page` is gone. `TaskStatus::Accepted` is
   gone. Errors are `CoordError`. `PROTOCOL_VERSION` is `2`. Do **not** split
   `registry.rs` until a concept is deleted (tests are most of the line count).
6. **DrawScene and host-local 3D gone.** `HostCall::DrawScene` / `SceneData`
   are gone from v2. disk3d renders a widget tree. Do not invent a replacement
   3D protocol.
7. **One `Anchor`**. `plugin_protocol::v2::Anchor`; ledger and row geometry
   `pub use` it.
8. **One `Lifecycle`**. Host `PluginLifecycle` is
   `pub use plugin_protocol::v2::Lifecycle`. ctl `Send` uses `insert_text`.
9. **WorkspaceIo + per-window PluginHost.** `plugin_host_calls::WorkspaceIo`
   is the executor for list/read/open/focus/send/close/`scroll_to_run`.
   `CallPlan::execute` is unit-tested against `FakeWorkspace`.
   `handle_host_call` is `authorize_and_plan → execute → reply`.
   `control_surface::dispatch` uses `AppWorkspaceIo`. `Wait` is ctl-only;
   `Notify` is not a `WorkspaceIo` method. `AppShell.plugin: PluginHost`
   (`plugin_window.rs`) owns chrome + event watch. `PluginRuntime` stays a
   process `Global` (supervisor / catalog / 16 ms pump).
10. **BlockOverlay.** `term_element.rs` does not import `plugin_runtime`.
    Blocks are GPUI overlays using `paint_laid_out` / `paint_node` from
    `app_shell/plugin_paint.rs`. Dual `LaidOutKind` quad painter is deleted.
    Alt-screen still skips blocks.
11. **Panel surfaces live on the tree.** `LeafContent::Panel(PanelView)` owns
    `PanelSurface`. `PanelRegistry` is deleted. Close/detach is drop of the
    leaf. `apply_panel_render` writes the tree (`update_panel_surface` /
    `insert_panel_leaf`).
12. **Launch is one adapter transaction.** Spawn + `commit_launch(Bound)` is
    the success path; a failed bind closes the pane in the same function.
    `Adapter.panes` is gone. `Registry::session_for_pane` is the pane lookup.
    Adapter keeps only launch-detection state (`kind`, `launch_task`,
    `bound_at_ms`, `detected`).
13. **Supervisor one session map.** `instances: HashMap<Uuid, Arc<Session>>`
    plus `resident_index: HashMap<String, Uuid>`. Not two maps each holding
    an `Arc`.
14. **`atomic_write` is the lock + save path** for settings persist, run
    ledger `save_runs`, updater transaction / health marker / active pointer,
    and plugin grants. API: `save_atomic`, `save_atomic_with(SaveOptions {
    mode, parent_mode })`, `with_file_lock`, `quarantine`. Sibling `.tmp` /
    `.lock`, not `*.json.tmp`.
15. **Render is paint-only.** Plugin inbound pump is the 16 ms
    `PluginRuntime` task (`plugin_dispatch`: route and `poll_plugin_events`
    are independent; empty inbound still walks the watch). Ledger focus and
    pane-facts refresh run on AppShell's 200 ms `_housekeeping` timer, not
    `Render`.

## Remaining work

Do these **in this order**. Do not reopen items 9–15.

### 1. Hook install still bypasses `atomic_write`

`crates/sleipnir_settings/src/agent_hooks.rs` writes the hook script and
Claude/Codex JSON with raw `fs::write` into `~/.claude` / `~/.codex`.

**Do:** same lock + `save_atomic` as settings/ledger. Socket path is already
`sleipnir_paths::agent_control_socket_path`. Do **not** move the crate into
the agents plugin in the same pass unless that move is small and tests stay
green.

Keep `plugin_grants` on `atomic_write::save_atomic` (already).

### 2. Settings document model (optional; not merge-blocking)

`sleipnir_settings` is still a GPUI kitchen sink. `merge_file` is a long
`if let Some` chain. `inject_osc133` / `run_ledger` / `theme` exist at top
level and under `terminal { }`. `persist_terminal_bool` is stringly JSON.
Setters clone-patch-apply.

**Do (when picked up):** one in-memory document; `update(|s| …)` then persist
through `with_file_lock` + `save_atomic`. Delete `persist_terminal_bool`.

Themes: 14 copy-paste `fn mocha() -> TerminalPalette` constructors in
`themes.rs` should become one `ThemeSpec` table. Data, not a new abstraction
layer.

## What not to do

- Do not extract more `impl AppShell` / `impl Terminal` files. Terminal
  still takes GPUI input (`Window` / `Mouse*Event` / `actions!`); further
  file splits without moving that boundary are the previous failure mode.
- Do not rewrite `command_dispatch.rs`. It is a `CommandId` trampoline;
  keep `dispatch_command` as the only command entry.
- Do not extend v2 for another product-shaped verb like `DrawScene`.
- Do not add another `Option<FooState>` overlay. Put new owners on
  `InputMode`, and mutate through `set_input`.
- Do not resurrect gutter triangles or host 3D scene types.
- Do not clone `~/.config/sleipnir` again. Import `sleipnir_paths`.
- Do not split `registry.rs` without deleting a concept (including collapsing
  `commit_launch` / `commit_ok` into a new `ClaimOutcome` type — that is a
  rename unless it deletes a dialect).
- Do not re-do PluginHost compositor, BlockOverlay, PanelRegistry,
  adapter pane map, or supervisor `live`+`active`.

## Suggested next session

Start at **hook install uses `atomic_write` (remaining §1)**. PluginHost
ownership, launch binding, supervisor maps, file-lock discipline for app
config, Terminal `AbsLine`/hover, and Render pumps have landed.

```bash
cargo test -p agent_coordination
cargo test -p sleipnir_plugin_agents
cargo test -p plugin_host -p plugin_protocol -p sleipnir_ui --lib
cargo test -p atomic_write -p sleipnir_settings -p run_ledger
cargo test -p terminal
```
