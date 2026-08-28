# External command plugin host

**Status:** accepted

## Context

Sleipnir already has a command palette, structured shell/output semantics and a default-off control surface. Users want to extend those surfaces without forcing every integration into the terminal binary or violating the no-built-in-AI boundary in ADR-0008.

Loading arbitrary native libraries into the UI process would couple plugin crashes and dependency conflicts to the terminal. An unrestricted hook API would also make terminal input and output available without a clear consent boundary.

## Decision

Sleipnir supports manifest-based, out-of-process command plugins.

- Plugins are disabled by default.
- Discovery reads one `plugin.json` from each child of configured plugin roots.
- A versioned manifest contributes commands to the existing command palette.
- Commands receive only declared context fields through stdin or arguments.
- Every declared capability must also appear in the user's global permission allowlist.
- Child processes run away from the UI thread, have a bounded timeout, and return stdout through a small set of routes: ignore, insert into the active pane, or copy to clipboard.
- Arguments are passed directly to the process and are never evaluated by a shell.
- Sleipnir makes no model calls, stores no provider keys and ships only a disabled bridge template for user-supplied external assistants.

The v1 contract deliberately excludes native dynamic libraries, arbitrary UI injection, lifecycle/event hooks, background daemons and automatic downloads.

## Consequences

- Existing command-palette UX becomes the first stable extension point with limited changes to GPUI state.
- A plugin crash cannot unwind the terminal process, but plugins still execute with the user's OS identity. Permission declarations are policy and disclosure, not an OS sandbox.
- The API can later grow event subscriptions and richer contribution types while retaining manifest versioning and out-of-process isolation.
