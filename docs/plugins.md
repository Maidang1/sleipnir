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
plugin_protocol   shared wire types (v2)
      ▲
      ├── sleipnir-plugin (SDK)
      │     └── Plugin + run()  resident, events, Render, HostCall
      │         ▼ compiled into
      │   an independent plugin binary
      ▼
plugin_host (supervisor)  ── discovers plugin.json, launches the binary,
                             handshakes, enforces grants, isolates crashes
```

Protocol **v2** is the only supported dialect. Manifests must declare
`api_version: 2`; v1 manifests are rejected at load time. The wire-level
acceptance window (ADR-0016 §8) covers only dialects the host still
implements, so today it is exactly `{2}`; it widens to N-1 when a v3 lands
and v2 stays implemented, so that bump will not break v2 plugins.

## Enable plugins

Plugins are off by default. In `settings.json`:

```json
{
  "plugins": {
    "enabled": true,
    "directories": []
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

Plugins are gated by per-plugin grants in
`~/.config/sleipnir/plugin-grants.json` (see [Grants and consent](#grants-and-consent)
below). First run, a binary change, or newly requested capabilities all prompt
for consent; nothing launches without a grant.

## `plugin.json`

`plugin.json` is retained so you can audit a plugin's binary, lifecycle, and
requested permissions **without running it**. `api_version` must be `2`. A
resident may omit `commands` (it lives on events and `Render`); an `on_demand`
plugin still needs at least one command.

```json
{
  "id": "my-plugin",
  "name": "My Plugin",
  "version": "0.1.0",
  "api_version": 2,
  "enabled": true,
  "lifecycle": "resident",
  "binary": "./my-plugin",
  "permissions": ["subscribe_events", "render_block", "read_cwd"]
}
```

- `binary`: the plugin executable. Relative paths resolve from the plugin
  directory; bare names resolve via `PATH`.
- `lifecycle`: `on_demand` (launched per invocation, then shut down) or
  `resident` (the host keeps the process and fans events at it). `resident` is
  also a capability: the host treats it as declared.
- `permissions` at the **plugin** level is the pre-launch audit for capabilities
  that are not tied to a palette command. `Ready.requests` must be a subset of
  the union of plugin-level and per-command permissions (plus `resident` when
  `lifecycle` is `resident`). An over-request is refused, not granted.
- `permissions` (on each command): capabilities that command needs. Each must
  be covered by the plugin's stored grant.

Plugin and command IDs use lowercase ASCII letters, digits, `-`, and `_`.

## Writing a plugin (Rust SDK)

A resident, event-driven, rendering plugin implements `Plugin` and calls
`run`. The SDK multiplexes events, actions, host-call replies and
invocations, correlating by `id`, and never deadlocks a `HostCall` against an
intervening event.

```rust
use sleipnir_plugin::{
    run, Plugin, Context, Manifest, Lifecycle, Capability, EventFilter, EventKind,
    HostEvent, RenderTarget, col, text, badge, btn, Tone,
};

struct MyPlugin;
impl Plugin for MyPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "my-plugin".into(),
            name: "My Plugin".into(),
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
    fn on_action(&mut self, _id: sleipnir_plugin::BlockId, action: &str, arg: Option<&str>, ctx: &mut Context<'_>) {
        if action == "retry" { /* send a new Render for the same Block */ let _ = (arg, ctx); }
    }
}
fn main() { run(MyPlugin); }
```

Widget builders (`col`, `row`, `text`, `badge`, `btn`, `code`, `bar`, `spark`,
`sep`) produce the closed ADR-0017 set. Colour is semantic [`Tone`] only — no
hex, no RGB.

`Context::render` is a push: it can be sent at any time, not only as a reply.
`Context::call` issues a `HostCall` and waits for the matching `Reply`.

See `crates/sleipnir_plugin_disk3d` for a complete, runnable Panel example
that renders a 3D disk-usage chart (a software rasteriser whose framebuffer is
a `col` of `text` rows — the closed widget set is enough for that, since one
Unicode scalar is one cell and `wrap_text` honours `\n`).

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

## Invocation contract

On each command the host sends declared context and the plugin returns a routed
result.

**Context** (populated only when the command holds the matching permission):
`cwd`, `title`, `selection`, `visible_screen`.

**Output routes:**

- `Output::Ignore`: discard.
- `Output::insert(text)`: type into the pane's PTY; requires `write_terminal`.
  Note the bytes are *executed* as shell input, so this is for sending commands,
  not for displaying styled output.
- `Output::copy(text)`: copy to the clipboard; requires `clipboard`.

## Capabilities

The snapshot reads are taken when the user asked. The observation, rendering,
and host-call capabilities are categorically stronger and are **never implied**
by the snapshot set (ADR-0016 §4).

| Capability | Tier | Meaning |
| --- | --- | --- |
| `read_selection` | snapshot | current selection |
| `read_visible_screen` | snapshot | what's on screen |
| `read_cwd` | snapshot | working directory |
| `read_title` | snapshot | pane title |
| `write_terminal` | snapshot | type into the PTY |
| `clipboard` | snapshot | clipboard |
| `network` | snapshot | declaration/policy gate; Sleipnir does not inspect traffic |
| `resident` | elevated | process stays up between invocations |
| `subscribe_events` | elevated | **continuous observation** of runs, ports, cwd, focus; narrow with `EventFilter` |
| `render_block` | elevated | draw into scrollback |
| `render_panel` | elevated | draw a split |
| `render_status` | elevated | draw into chrome |
| `host_call_notify` | elevated | plugin-initiated notification |
| `host_call_read_screen` | elevated | read any pane's screen |
| `host_call_list_panes` | elevated | list open panes |
| `host_call_open_pane` | elevated | open a new pane |

`subscribe_events` is the significant semantic escalation: a plugin moves from
"runs when you pick it" to "watches every command you run". It must be requested
explicitly.

## Grants and consent

Grants live in `~/.config/sleipnir/plugin-grants.json`, **not** in
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
- The RPC is versioned; a version mismatch is refused. Only v2 manifests load.
- Grants are checked before the plugin binary is launched: no grant, no launch.
- Per-invocation timeout is capped to 1–300 seconds (default 30); a wedged
  on-demand plugin is killed and reaped. A wedged resident is isolated from the
  UI thread (bounded queues, stderr drain) and can be killed from the Monitor.
- Reload Settings re-reads manifests, permission policy, and restarts granted
  residents that are not already live.

## Install a plugin

Plugins are off by default. A plugin installs as one directory per plugin under
the plugin root, containing `plugin.json` and the binary it names. Example with
a locally authored plugin:

```sh
# 1. Build the plugin (and the app)
cargo build -p <plugin-package>
cargo build -p sleipnir

# 2. Install into the default plugin directory
mkdir -p ~/.config/sleipnir/plugins/<id>
cp target/debug/<plugin-binary> ~/.config/sleipnir/plugins/<id>/
cp path/to/plugin.json ~/.config/sleipnir/plugins/<id>/

# 3. Enable plugins in settings.json (create the file if needed)
#    ~/.config/sleipnir/settings.json
#    { "plugins": { "enabled": true } }
```

Then:

1. Start Sleipnir (or Reload Settings). A consent prompt lists the plugin's
   requested capabilities in plain language. Approve it. Deny writes nothing
   and the plugin will not start.
2. Confirm the plugin is live in the Plugin Monitor (command palette: "Plugin
   Monitor").

If the prompt does not appear, confirm `plugins.enabled` is `true` and Reload
Settings. If the plugin does not start, confirm the binary path in that
`plugin.json` matches the installed executable and that you approved consent.
