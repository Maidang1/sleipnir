# Sleipnir plugins

Sleipnir plugins are **independent binaries** the terminal launches as child
processes and talks to over a versioned RPC (ADR-0015). A plugin runs in its own
process, so a crash or panic never unwinds the terminal. Plugins are written in
any language; Rust is first-class through the `sleipnir-plugin` SDK.

This preserves ADR-0008: Sleipnir supplies context and routing; the plugin (and
whatever it shells out to) supplies behavior. Sleipnir makes no model calls and
stores no provider credentials.

## Architecture

```
plugin_protocol   shared wire types (Hello/Ready/Invoke/Invoked/Shutdown)
      ▲
      ├── sleipnir-plugin (SDK)  ── authors implement `Plugin`, call `run()`
      │         ▼ compiled into
      │   an independent plugin binary
      ▼
plugin_host (supervisor)  ── discovers plugin.json, launches the binary,
                             handshakes, invokes, enforces permissions,
                             isolates crashes
```

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

Discovery root (next to `settings.json`):

- macOS/Linux: `~/.config/sleipnir/plugins`
- Windows: platform config dir + `sleipnir/plugins`

Each child directory is one plugin and contains `plugin.json`. Extra roots may be
added via `plugins.directories`. Reload Settings refreshes the catalog.

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
  default, strictest) or `resident` (reserved for a future connection-caching
  supervisor; behaves as `on_demand` in v1).
- `permissions`: capabilities each command needs. A command runs only when every
  permission is present in `plugins.allowed_permissions`.

Plugin and command IDs use lowercase ASCII letters, digits, `-`, and `_`.

## Writing a plugin (Rust SDK)

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

See `crates/sleipnir_plugin_beautify` for a complete example.

## Invocation contract

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

## Permissions

- `read_selection`, `read_visible_screen`, `read_cwd`, `read_title`
- `write_terminal`, `clipboard`
- `network` (a declaration/policy gate for what the plugin may do; Sleipnir does
  not inspect network activity)

## Safety and lifecycle

- Plugins are off by default and run out of process; a crash cannot unwind the
  terminal.
- The RPC is versioned; a version mismatch is refused.
- Permissions are gated before the plugin binary is even launched.
- Per-invocation timeout is capped to 1–300 seconds (default 30); a wedged
  plugin is killed and reaped.
- Reload Settings re-reads manifests and permission policy.

Later phases can add resident-connection caching, plugin-initiated host calls,
event hooks, custom UI panels, and (for a wider audience) signing — all behind
protocol version negotiation, without breaking v1 plugin binaries.
