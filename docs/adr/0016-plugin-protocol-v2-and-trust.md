# Plugin protocol v2: bidirectional RPC, per-plugin grants, trust tiers

**Status:** proposed (extends [ADR-0015](0015-out-of-process-plugin-rpc.md); retires its
"small trusted author group" premise)

## Context

[ADR-0015](0015-out-of-process-plugin-rpc.md) established the right *loading*
model — plugins are independent binaries spoken to over a versioned,
line-delimited JSON RPC — and then deliberately kept the *capability* surface at
parity with [ADR-0013](0013-external-command-plugins.md). The result is a
manually-triggered text transformer:

> the user picks a command from the palette → the host spawns a process → hands
> it four string snapshots (`cwd` / `title` / `selection` / `visible_screen`) →
> reads back one string → routes it to `ignore` / `insert` / `copy` → the process
> dies.

Three structural gaps follow from that shape.

**Only one trigger exists.** The app already computes a rich structured-facts
layer — `run_ledger` holds a full per-command state machine (command, cwd,
duration, exit code, `Running`/`Succeeded`/`Failed`/`Unknown`/`Abandoned`),
`chrome/pane_facts.rs` holds cwd, foreground process, the descendant process
tree and listening TCP ports, and `chrome/agent.rs` identifies which coding agent
runs in which pane. **A plugin can see none of it.** The data exists; there is no
outlet.

**Output is text only, and `insert` is destructive.** `docs/plugins.md` states it
plainly: the inserted bytes are *executed* as shell input, "so this is for
sending commands, not for displaying styled output". A plugin therefore has no
way to *show the user anything*. The app renders a full diff inspector with
minimap and syntax highlighting; a plugin cannot render one styled line.

**`resident` is declared but not implemented.** It is parsed, recorded on the
loaded command, and its `to_wire()` carries `#[allow(dead_code)]`. Every
invocation is a cold start, so no plugin can hold state, watch a file, or keep a
connection.

There is a further consequence that is not a missing feature but a hole in the
product boundary. [ADR-0008](0008-no-builtin-ai.md) forbids built-in AI on the
grounds that plugins and user-configured external commands are where AI belongs.
Under v1, the *only* thing an AI plugin can do is type generated text into the
user's shell to be executed. It cannot explain an error, cannot show a suggested
diff, cannot present anything for review before it runs. **The escape hatch
ADR-0008 promises does not currently exist.**

Finally, ADR-0015 scoped itself to "a small trusted group of authors (not an open
marketplace yet), so signing / sandboxing / a registry are explicitly out of
scope". **We now intend to attract external plugin authors.** That premise is
therefore retired, and its consequences must be paid explicitly rather than
inherited silently.

## Decision

Protocol v2 widens the RPC from request/response to **bidirectional multiplexed
messaging**, and replaces the single global permission allowlist with
**per-plugin grants bound to binary identity**, under **explicit trust tiers**.

### 1. Correlation ids and bidirectional messages

Every message carries an `id`. A resident plugin may have several events, actions
and host calls in flight at once; without correlation there is no way to pair a
reply with its cause. This is the least visible and most easily omitted change in
the v1 → v2 move.

```rust
enum HostMessage {
    Hello { protocol_version: u32, granted: Vec<Capability>, plugin_instance_id: Uuid },
    Invoke { id: u64, command_id: String, context: InvokeContext },
    Event  { id: u64, event: HostEvent },                       // new
    Action { id: u64, block_id: BlockId, action: String, arg: Option<String> }, // new
    Reply  { id: u64, result: HostCallResult },                 // new
    Shutdown,
}

enum PluginMessage {
    Ready   { protocol_version: u32, manifest: Manifest, requests: Vec<Capability> },
    Invoked { id: u64, output: Output },
    Failed  { id: u64, message: String },
    Render  { id: u64, target: RenderTarget, tree: Widget },     // new
    Call    { id: u64, call: HostCall },                         // new
}
```

`Render` and its `Widget` payload are specified in
[ADR-0017](0017-plugin-widget-schema.md); `RenderTarget::Block` placement is
specified in [ADR-0018](0018-block-rendering-and-coordinates.md).

### 2. Events: expose the facts layer that already exists

```rust
enum HostEvent {
    RunStarted        { run_id: RunId, command: String, cwd: Option<String> },
    RunFinished       { run_id: RunId, exit_code: Option<i32>, duration_ms: u64 },
    PortOpened        { pane: PaneKey, pid: u32, addr: String },
    ForegroundChanged { pane: PaneKey, agent: Option<String> },
    CwdChanged        { pane: PaneKey, cwd: String },
    PaneFocused       { pane: PaneKey },
}
```

Every source is already computed: `run_ledger` for runs, `pane_facts` for ports,
`agent.rs` for agent identity. v2 opens an outlet; it does not add
instrumentation. `command` in `RunStarted` is the **redacted** form — the ledger
redacts at capture time (`run_ledger::redact`) and plugins never see the raw
line.

### 3. Host calls: a plugin-initiated back-channel

```rust
enum HostCall {
    Notify     { title: String, body: String },
    ReadScreen { pane: PaneKey },
    ListPanes,
    OpenPane   { cwd: Option<String>, command: Option<String> },
}
```

Each maps to an existing capability already reachable through the control surface
([ADR-0011](0011-control-surface.md)), so v2 adds no power that the machine did
not already expose locally — it changes *who* may ask.

### 4. Capabilities: new grants sit a tier above the v1 seven

The v1 permissions (`read_selection`, `read_visible_screen`, `read_cwd`,
`read_title`, `write_terminal`, `clipboard`, `network`) are all "read one
snapshot, when the user asked". The new ones are categorically stronger and are
never implied by the old set:

| Capability | Why it is stronger |
| --- | --- |
| `resident` | the process keeps running between invocations |
| `subscribe_events` | **continuous observation** rather than one snapshot; narrowable by pane and by event kind |
| `render_block` / `render_panel` / `render_status` | the plugin draws into the app's own surfaces |
| `host_call:notify` / `host_call:read_screen` / `host_call:list_panes` / `host_call:open_pane` | plugin-initiated action, not user-initiated |

`subscribe_events` is the significant semantic escalation: a plugin moves from
"runs when you pick it" to "watches every command you run". It must be requested
explicitly and is never granted as a side effect of anything else.

### 5. Per-plugin grants, keyed by binary hash

`plugins.allowed_permissions` is a single allowlist shared by all plugins. That
is incompatible with per-plugin consent and is superseded for v2 plugins.

Grants live in their own file, **not** in `settings.json`, because `settings.json`
is hand-edited by the user while a grant record must be bound to the identity of
a binary:

```jsonc
// ~/.config/sleipnir/plugin-grants.json
{
  "version": 1,
  "grants": {
    "port-watcher": {
      "granted": ["read_cwd", "subscribe_events", "render_block"],
      "binary_hash": "sha256:ab12…",
      "granted_at": "2026-01-01T00:00:00Z",
      "tier": "sandboxed"
    }
  }
}
```

**`binary_hash` is load-bearing: if the binary changes, the grant is void and
consent is asked again.** Without it, a benign plugin can self-update into a
malicious one while keeping permissions the user approved for different code.

### 6. Trust tiers

ADR-0015 assumed trusted authors. With external authors, an installed plugin is
an arbitrary binary running with the user's full OS identity — and v2 additionally
offers it residency and continuous observation. Rather than block the whole
programme on a cross-platform sandbox, trust is explicit and staged:

| Tier | Source | Capability | Sandbox |
| --- | --- | --- | --- |
| 0 | built-in / first-party | full | n/a |
| 1 | locally authored by the user | full, **labelled "unsandboxed, local" in the UI** | none |
| 2 | externally installed | grants only; no `exec`; no network unless granted; filesystem limited to its own directory | **required** |

**No external installation channel ships before Tier 2 sandboxing exists.** Until
then plugins are Tier 0/1 only. Sandbox mechanisms are platform-specific and
deliberately deferred (macOS seatbelt, Linux bubblewrap/landlock/seccomp, Windows
AppContainer/Job Object).

### 7. Provenance is mandatory in the UI

Once a plugin can draw into scrollback, the user must always be able to tell
"program output" from "plugin-drawn". Therefore:

- every plugin-rendered surface carries a visible attribution marker naming its
  plugin, and this is not suppressible;
- a status indicator shows how many plugins are running. "Unsuppressible" binds
  the *plugin*, not the host: no plugin can hide, restyle or spoof it. The host
  hides it at zero, because a permanent "0 plugins" spends trusted chrome on the
  one state that carries no information, and the Monitor stays reachable from the
  command palette. Any non-zero count is always shown;
- a Plugin Monitor panel (reusing the existing overlay framework) lists each
  plugin's process, resource use, last invocation, event counts, recent log tail,
  and offers a kill switch.

### 8. Compatibility window

`versions_compatible` is currently `host == plugin`. Under an external ecosystem
that means every host release breaks every plugin. v2 accepts the current and
previous protocol versions (`N` and `N-1`), and the protocol is versioned with
semver intent: additive fields stay compatible via `#[serde(default)]`; removals
and semantic changes require a version bump.

## Consequences

- The facts layer the app already computes becomes an extension surface, and
  ADR-0008's promised escape hatch becomes real: an AI plugin can *show* the user
  an explanation or a proposed change instead of only typing into their shell.
- ADR-0015's author-trust premise is retired. Signing, sandboxing and
  distribution move from "out of scope" to "required before external
  installation" — the cost is now explicit rather than silently inherited.
- `plugins.allowed_permissions` is superseded for v2 plugins by
  `plugin-grants.json`. v1 plugins continue to work under the old allowlist
  during the compatibility window.
- Binary-hash-bound grants mean plugin updates re-prompt for consent. This is
  friction by design; the alternative silently transfers trust to unreviewed code.
- Ordering constraint: **grants and the Monitor land before `subscribe_events`.**
  Shipping continuous observation first would create a broad, already-released
  permission surface that cannot be withdrawn.
- The host gains real supervisory duties it does not have today: connection
  caching for residency, liveness and resource accounting, event fan-out with
  backpressure, and per-plugin teardown. A slow or wedged resident plugin must
  never stall the UI thread or the event bus.
