# Which capabilities belong in the plugin, and what must the host expose?

Id: 10
Parent: ../map.md
Labels: wayfinder:grilling
Type: grilling
Mode: HITL
Status: open
Assignee: unassigned
Blocked by: 06, 09

## Question

Given the chosen coordination and lifecycle contracts, what is the minimal
generic host capability surface, and what stays in agent-specific adapters or
the collaboration plugin? Compare the actual plugin protocol and default-off
control surface, including targeted input, pane/session identity, focus,
notifications, process ownership, and cross-window routing. Define grant and
failure boundaries without implying that local plugins are OS-sandboxed.

## Context pointers

Current source inspection on 2026-09-09, not a resolution:

- [Plugin protocol](../../../crates/plugin_protocol/src/v2.rs): `HostEvent`
  exposes shell Run and Pane facts, not an agent-turn lifecycle. `HostCall`
  includes notifications, reading screens, listing/opening panes, rendering,
  and navigating to a Run, but no dedicated targeted agent-prompt operation.
  `Output::Insert` is an invocation result route, not that missing contract.
- [Plugin host integration](../../../crates/sleipnir_ui/src/app_shell/plugins.rs)
  and [runtime](../../../crates/sleipnir_ui/src/plugin_runtime.rs) are the source
  of truth for ownership, routing, and process lifetime; inspect actual behavior
  rather than assuming one resident process per Window.
- [Control surface boundary](../../../docs/adr/0011-control-surface.md): the
  separate default-off control surface can send terminal input, but its `wait`
  observes Run Ledger state rather than an agent task or turn. Reusing it is a
  design choice to evaluate, not implied permission to bypass plugin grants.
