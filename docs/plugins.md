# Sleipnir plugins

Sleipnir plugins are **independent binaries** the terminal launches as child
processes and talks to over a versioned RPC (ADR-0015, ADR-0016). A plugin runs
in its own process, so a crash or panic never unwinds the terminal. Plugins are
written in any language; Rust is first-class through the `sleipnir-plugin` SDK.

This preserves ADR-0008: Sleipnir supplies context and routing; the plugin (and
whatever it shells out to) supplies behavior. Sleipnir makes no model calls and
stores no provider credentials.

## Architecture

```
plugin_protocol   shared wire types
      ▲
      ├── sleipnir-plugin (SDK)
      │     ├── v1  Plugin + run()          on-demand text transformers
      │     └── v2  v2::Plugin + v2::run()  resident, events, Render, HostCall
      │         ▼ compiled into
      │   an independent plugin binary
      ▼
plugin_host (supervisor)  ── discovers plugin.json, launches the binary,
                             handshakes, enforces grants, isolates crashes
```

The host accepts protocol **N and N-1** (ADR-0016 §8). Today that is v2 and v1.

## Enable plugins

Plugins are off by default. In `settings.json`:

```json
{
  "plugins": {
    "enabled": true,
    "directories": [],
    "allowed_permissions": [
      "read_selection",
      "read_visible_screen",
      "read_cwd",
      "read_title",
      "write_terminal",
      "clipboard"
    ]
  }
}
```

`plugins.enabled` is the master switch: when it is `false`, manifests are not
read and nothing is launched. Reload Settings refreshes the catalog.

Discovery root (next to `settings.json`):

- macOS/Linux: `~/.config/sleipnir/plugins`
- Windows: platform config dir + `sleipnir/plugins`

Each child directory is one plugin and contains `plugin.json`. Extra roots may be
added via `plugins.directories`.

**v1** commands are gated by `plugins.allowed_permissions` (one allowlist shared
by every plugin). **v2** plugins use per-plugin grants in
`~/.config/sleipnir/plugin-grants.json` instead (see [Grants and consent](#grants-and-consent)
below). `allowed_permissions` is not a substitute for that prompt.

## `plugin.json` (v1)

`plugin.json` is retained so you can audit a plugin's binary, lifecycle, and
requested permissions **without running it**.

```json
{
  "id": "my-plugin",
  "name": "My Plugin",
  "version": "0.1.0",
  "api_version": 1,
  "enabled": true,
  "lifecycle": "on_demand",
  "binary": "./my-plugin",
  "args": [],
  "commands": [
    {
      "id": "inspect",
      "title": "Plugin: Inspect Pane",
      "description": "A short description",
      "keywords": ["inspect"],
      "permissions": ["read_selection", "clipboard"],
      "timeout_secs": 15
    }
  ]
}
```

- `binary`: the plugin executable. Relative paths resolve from the plugin
  directory; bare names resolve via `PATH`.
- `lifecycle`: `on_demand` (launched per invocation, then shut down — the
  default, strictest) or `resident`. In v1, `resident` is recorded but every
  invocation is still a cold start.
- `permissions` (on each command): capabilities that command needs. A v1 command
  runs only when every permission is present in `plugins.allowed_permissions`.
- `api_version`: omit or `1` for v1. The host still loads these after it learned
  v2 (N / N-1 window).

Plugin and command IDs use lowercase ASCII letters, digits, `-`, and `_`.

## `plugin.json` (v2)

v2 adds plugin-level permissions and allows a resident with no palette commands
(it lives on events and `Render`). `api_version` must be `2`.

```json
{
  "id": "failed-run",
  "name": "Failed Run",
  "version": "0.1.0",
  "api_version": 2,
  "enabled": true,
  "lifecycle": "resident",
  "binary": "./sleipnir-plugin-failed-run",
  "permissions": ["subscribe_events", "render_block", "read_cwd"]
}
```

- `permissions` at the **plugin** level is the pre-launch audit for capabilities
  that are not tied to a palette command. `Ready.requests` must be a subset of
  the union of plugin-level and per-command permissions (plus `resident` when
  `lifecycle` is `resident`). An over-request is refused, not granted.
- A v2 resident may omit `commands`. A v1 plugin, and a v2 `on_demand` plugin,
  still need at least one command.
- `lifecycle: "resident"` means the host keeps the process and fans events at
  it. It is also a capability: the host treats it as declared.

## Writing a plugin (Rust SDK, v1)

```rust
use sleipnir_plugin::{Plugin, Manifest, CommandSpec, Capability, Invoke, Output, Lifecycle, run};

struct MyPlugin;
impl Plugin for MyPlugin {
    fn manifest(&self) -> Manifest { /* id, commands, capabilities, lifecycle */ }
    fn invoke(&mut self, req: Invoke) -> Output {
        Output::copy("done")
    }
}
fn main() { run(MyPlugin); }
```

`cargo build` produces an independent binary. Point `plugin.json`'s `binary` at
it. The SDK owns the handshake and the RPC loop; you only see Rust types.

## Writing a plugin (Rust SDK, v2)

A resident, event-driven, rendering plugin implements `v2::Plugin` and calls
`v2::run`. The SDK multiplexes events, actions, host-call replies and
invocations, correlating by `id`, and never deadlocks a `HostCall` against an
intervening event.

```rust
use sleipnir_plugin::v2::{
    run, Plugin, Context, Manifest, Lifecycle, Capability, EventFilter, EventKind,
    HostEvent, RenderTarget, col, text, badge, btn, Tone,
};

struct FailedRun;
impl Plugin for FailedRun {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "failed-run".into(),
            name: "Failed Run".into(),
            version: "0.1.0".into(),
            description: String::new(),
            lifecycle: Lifecycle::Resident,
            commands: vec![],
        }
    }
    fn requests(&self) -> Vec<Capability> {
        vec![
            Capability::Resident,
            Capability::SubscribeEvents,
            Capability::RenderBlock,
            Capability::ReadCwd,
        ]
    }
    fn event_filter(&self) -> EventFilter {
        // RunFinished does not carry the redacted command; that arrives on
        // RunStarted. Subscribe to both, nothing else.
        EventFilter { panes: vec![], kinds: vec![EventKind::RunStarted, EventKind::RunFinished] }
    }
    fn on_event(&mut self, event: HostEvent, ctx: &mut Context<'_>) {
        let HostEvent::RunFinished { run_id, exit_code: Some(code), .. } = event else { return };
        if code == 0 { return; }
        let _ = ctx.render(
            RenderTarget::Block { anchor: run_id },
            col().gap(1)
                .child(badge("failed", Tone::Err))
                .child(text("command failed").bold())
                .child(btn("Retry", "retry").arg(run_id.to_string())),
        );
    }
    fn on_action(&mut self, _id: sleipnir_plugin::v2::BlockId, action: &str, arg: Option<&str>, ctx: &mut Context<'_>) {
        if action == "retry" { /* send a new Render for the same Block */ let _ = (arg, ctx); }
    }
}
fn main() { run(FailedRun); }
```

Widget builders (`col`, `row`, `text`, `badge`, `btn`, `code`, `bar`, `spark`,
`sep`) produce the closed ADR-0017 set. Colour is semantic [`Tone`] only — no
hex, no RGB.

`Context::render` is a push: it can be sent at any time, not only as a reply.
`Context::call` issues a `HostCall` and waits for the matching `Reply`.

See `crates/sleipnir_plugin_failed_run` for a complete, runnable Block example,
and `crates/sleipnir_plugin_disk3d` for a Panel example that renders a 3D
disk-usage chart (a software rasteriser whose framebuffer is a `col` of `text`
rows — the closed widget set is enough for that, since one Unicode scalar is one
cell and `wrap_text` honours `\n`).

## Render targets

A `Render` names one of three mounts (ADR-0017). One widget schema, one
renderer; the mount is where the tree is placed.

| Target | Capability | Where it appears |
| --- | --- | --- |
| `RenderTarget::Block { anchor: RunId }` | `render_block` | Inside scrollback, anchored to that run (ADR-0018) |
| `RenderTarget::Panel { pane }` | `render_panel` | Occupies a split |
| `RenderTarget::Status` | `render_status` | Chrome: badges, a status slot, dynamic palette rows |

Every surface carries a non-suppressible attribution marker naming the plugin
(ADR-0016 §7). The renderer draws it; a crafted tree cannot hide it.

Update model is Elm-style whole-tree replacement: `Render` → user clicks `Btn`
→ host sends `Action { block_id, action, arg }` → plugin `Render`s again. There
is no patch protocol.

## Invocation contract (v1)

On each command the host sends declared context and the plugin returns a routed
result.

**Context** (populated only when the command holds the matching permission):
`cwd`, `title`, `selection`, `visible_screen`.

**Output routes:**

- `Output::ignore()`: discard.
- `Output::insert(text)`: type into the pane's PTY; requires `write_terminal`.
  Note the bytes are *executed* as shell input, so this is for sending commands,
  not for displaying styled output.
- `Output::copy(text)`: copy to the clipboard; requires `clipboard`.

## Capabilities

The v1 seven are snapshots taken when the user asked. The v2 additions are
categorically stronger and are **never implied** by the v1 set (ADR-0016 §4).

| Capability | Kind | Meaning |
| --- | --- | --- |
| `read_selection` | v1 | current selection |
| `read_visible_screen` | v1 | what's on screen |
| `read_cwd` | v1 | working directory |
| `read_title` | v1 | pane title |
| `write_terminal` | v1 | type into the PTY |
| `clipboard` | v1 | clipboard |
| `network` | v1 | declaration/policy gate; Sleipnir does not inspect traffic |
| `resident` | v2 | process stays up between invocations |
| `subscribe_events` | v2 | **continuous observation** of runs, ports, cwd, focus; narrow with `EventFilter` |
| `render_block` | v2 | draw into scrollback |
| `render_panel` | v2 | draw a split |
| `render_status` | v2 | draw into chrome |
| `host_call_notify` | v2 | plugin-initiated notification |
| `host_call_read_screen` | v2 | read any pane's screen |
| `host_call_list_panes` | v2 | list open panes |
| `host_call_open_pane` | v2 | open a new pane |

`subscribe_events` is the significant semantic escalation: a plugin moves from
"runs when you pick it" to "watches every command you run". It must be requested
explicitly.

## Grants and consent

v2 grants live in `~/.config/sleipnir/plugin-grants.json`, **not** in
`settings.json`. A grant is bound to the **SHA-256 of the binary**. If the
bytes change, the grant is void and consent is asked again — otherwise a benign
plugin could self-update into a malicious one while keeping permissions the
user approved for different code (ADR-0016 §5).

On first run, binary change, or added capabilities, Sleipnir shows a consent
prompt listing the capabilities in plain language. Approve writes a grant
record (`tier: local` today — labelled "unsandboxed, local" in the UI). Deny
writes nothing. Closing the overlay is a deny.

Until an external installation channel ships, plugins are Tier 0/1 only
(built-in / locally authored). Tier 2 sandboxing is required before an open
marketplace (ADR-0016 §6).

The Plugin Monitor (command palette: "Plugin Monitor") lists running plugins
and offers a kill switch. A non-zero running count is always shown in chrome.

## Safety and lifecycle

- Plugins are off by default and run out of process; a crash cannot unwind the
  terminal.
- The RPC is versioned; a version mismatch is refused. The host accepts N and
  N-1.
- v1 permissions are gated before the plugin binary is launched. v2 grants are
  checked the same way: no grant, no launch.
- Per-invocation timeout is capped to 1–300 seconds (default 30); a wedged
  on-demand plugin is killed and reaped. A wedged resident is isolated from the
  UI thread (bounded queues, stderr drain) and can be killed from the Monitor.
- Reload Settings re-reads manifests, permission policy, and restarts granted
  residents that are not already live.

## Install and verify the example plugin

`crates/sleipnir_plugin_failed_run` is a workspace member. It is **not** a
dependency of the terminal binary. It subscribes to run events and, when a
command exits non-zero, draws a Block in scrollback with the redacted command,
the exit code, the duration, and a **Retry** button. Pressing Retry re-renders
the same Block with a visibly different tree (the full Render → click → Action
→ re-Render loop).

Plugins are off by default. Exact steps:

```sh
# 1. Build the example (and the app)
cargo build -p sleipnir-plugin-failed-run
cargo build -p sleipnir

# 2. Install into the default plugin directory
mkdir -p ~/.config/sleipnir/plugins/failed-run
cp target/debug/sleipnir-plugin-failed-run ~/.config/sleipnir/plugins/failed-run/
cp crates/sleipnir_plugin_failed_run/plugin.json ~/.config/sleipnir/plugins/failed-run/

# 3. Enable plugins in settings.json (create the file if needed)
#    ~/.config/sleipnir/settings.json
#    { "plugins": { "enabled": true } }
```

Then:

1. Start Sleipnir. A consent prompt lists `subscribe_events`, `render_block`,
   `read_cwd`, and `resident`. Approve it. Deny writes nothing and the plugin
   will not start.
2. In a pane, run a command that fails, e.g. `false` or `exit 1`.
3. A Block appears under that run: a red `failed` badge, the redacted command,
   the exit code, the duration, and **Retry**.
4. Click **Retry**. The Block updates in place (`queued`, the button is gone).
   The plugin does not re-execute the command — the point is to see the round
   trip.

If the prompt does not appear, confirm `plugins.enabled` is `true` and Reload
Settings. If the Block does not appear, confirm the binary path in that
`plugin.json` matches the copied executable and that you approved consent.
