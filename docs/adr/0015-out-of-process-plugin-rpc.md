# Out-of-process plugins: independent binaries over a versioned RPC

**Status:** proposed (supersedes the plugin model in [ADR-0013](0013-external-command-plugins.md))

## Context

[ADR-0013](0013-external-command-plugins.md) shipped a *fork-once command*
plugin model: a `plugin.json` names a `program`, the terminal spawns it once,
pipes context on stdin, reads stdout, and the process exits. That is really a
"configured shell command in the command palette", not a plugin *program*. It
has no lifecycle, no state, no back-channel: the plugin cannot call the host,
cannot subscribe to events, cannot keep state between invocations.

The intended model is different: **the host ships as a binary, and plugins are
independent binaries loaded and used alongside it.** Three constraints shape it:

1. **Rust ecosystem.** Plugin authors should write plain Rust — `cargo new`, add
   a dependency, implement a trait, `cargo build`.
2. **Strict first, widen later.** Isolation and a minimal capability surface are
   the baseline; capabilities grow behind version negotiation.
3. **A small trusted group of authors** (not an open marketplace yet), so
   signing / sandboxing / a registry are explicitly out of scope for v1.

Constraints 1 and 2 are in tension. The most "Rust-native" loading mechanism —
`dlopen` a `cdylib` into the host process — is also the *least* strict: a plugin
panic unwinds the terminal, and Rust has no stable ABI, so a host rebuild breaks
every existing plugin binary (UB) unless the boundary is frozen behind a C ABI /
`abi_stable`, which erodes the Rust-native feel it was chosen for.

We resolve the tension by ranking **strictness above loading convenience** (per
constraint 2) and recovering the Rust-native experience a different way.

## Decision

Plugins are **independent executables** the host launches as child processes and
speaks to over a **versioned, line-delimited JSON RPC**. The "Rust ecosystem"
requirement is met by an **official SDK crate**, not by dynamic linking.

### Loading model

- A plugin is a normal binary (any language; Rust is first-class via the SDK).
- The host **launches** the plugin as a child process and completes a handshake.
  "Loaded" means "running and connected", not `dlopen`.
- Plugins run in their **own process**: a plugin crash or panic cannot unwind the
  terminal. This is the strictness baseline and is non-negotiable for v1.
- Transport is line-delimited JSON over the child's stdin/stdout, reusing the
  house IPC style of the control surface ([ADR-0011](0011-control-surface.md),
  `sleipnir_ctl`): one JSON object per line, tagged unions, a pure types crate
  with no I/O.

### Rust SDK (`sleipnir-plugin` crate)

- The host publishes an SDK crate. An author writes:
  ```rust
  struct MyPlugin;
  impl Plugin for MyPlugin {
      fn manifest(&self) -> Manifest { /* id, commands, capabilities */ }
      fn invoke(&mut self, req: Invoke, host: &mut HostHandle) -> Invoked { /* ... */ }
  }
  fn main() { sleipnir_plugin::run(MyPlugin); }
  ```
- The SDK owns the handshake, the read/decode/dispatch/encode loop, and
  serialization. The author sees Rust types and a trait — no wire format, no
  process plumbing. This is how the "Rust ecosystem" goal is satisfied while the
  product still gets process isolation.
- Because the contract is the *wire protocol*, not a Rust ABI, the host can be
  rebuilt and upgraded without breaking existing plugin binaries, as long as the
  protocol version is honored.

### Capability model (strict, minimal, versioned)

- The handshake carries a **protocol version** and a **capability set** the
  plugin requests; the host replies with what it grants, intersected with the
  user's `plugins.allowed_permissions` allowlist (carried over from ADR-0013).
- **v1 capability surface is deliberately at parity with ADR-0013 and no more**:
  the plugin receives declared context (`selection`, `visible_screen`, `cwd`,
  `title`) with an invocation and returns a result routed as
  `ignore` / `insert` / `copy`.
- Explicitly **not in v1** (reserved in the protocol behind capability flags for
  later widening): plugin-initiated host calls (read screen on demand, inject
  keys), event subscriptions (run-finished hooks), custom UI panels, background
  daemons beyond the request/response session.

### Lifecycle & supervision (host side)

- The host owns launch, handshake, per-invocation dispatch, timeout, and
  teardown. A crashed or non-responsive plugin is killed and logged; the terminal
  is unaffected.
- Discovery still reads plugin directories; a manifest now points at (or is
  emitted by) a binary rather than naming an arbitrary `program` + args to fork.

## Consequences

- The plugin model changes from "fork a command once" to "run an independent
  binary and talk to it", which is the model originally intended. ADR-0013's
  command/context/permission *concepts* survive; its execution model is replaced.
- Strictness is preserved by process isolation and a minimal, negotiated
  capability surface; the Rust-native experience is preserved by the SDK. The two
  original constraints are both met without `dlopen`'s ABI/crash hazards.
- The protocol is versioned, so capabilities (host-callbacks, event hooks, UI
  contribution) can be added later without breaking v1 plugin binaries — matching
  "strict first, widen later".
- No signing, registry, or OS sandbox in v1 (small trusted author group). These
  can be layered on when the audience widens, without changing the RPC contract.
- Migration: the existing `plugin_host` crate is refactored from a fork-once
  runner into a process supervisor + RPC host. The `beautify` example is rewritten
  as an independent SDK-based binary.
