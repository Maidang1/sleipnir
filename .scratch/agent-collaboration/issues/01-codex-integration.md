# Codex: visible-session control and lifecycle evidence

Id: 01
Parent: ../map.md
Labels: wayfinder:research
Type: research
Mode: AFK
Status: resolved
Assignee: Madinah
Executor: Anscombe (01a08501-1838-7723-a06e-159689c6dfd7)
Blocked by: none

## Question

Which officially supported Codex interfaces can start, prompt, observe, interrupt,
and collect results from the same interactive session shown in a terminal Pane,
while preserving human approval and takeover? Distinguish TUI, app-server, exec,
hooks, and MCP capabilities rather than assuming that separate clients share a
live session. Identify turn/session correlation, state evidence, configuration,
version/platform caveats, and unverified gaps. Findings must use primary sources
checked on 2026-09-09; do not select the final adapter architecture.

## Research context

- Evidence asset: [Integration findings](../research/codex.md).
- Archive branch: `research/agent-collaboration-codex`.
- Worktree: `/private/tmp/sleipnir-wayfinder-codex-20260909`.
- Archived at commit `5d9c56cc0c3e872d265f2aa232a838d3b48150da` on
  `research/agent-collaboration-codex`.

## Answer

Codex has an official path that can plausibly satisfy the visible-session
requirement: run `codex app-server` and connect the native TUI with
`codex --remote`. App-server exposes structured thread, turn, item, approval,
interrupt, and history operations. That is materially stronger than terminal
screen heuristics and is the only researched Codex interface that explicitly
connects the native TUI to a protocol endpoint.

This does **not** establish that a sidecar may safely attach to any already
running ordinary Codex TUI, nor that a TUI and coordinator can concurrently own
prompt and approval handling without races. Those behaviors require a bounded
runtime verification before choosing the adapter. `codex exec`, the TypeScript
SDK's thread continuation, and the deprecated MCP-server path create or resume
non-interactive/server conversations; none proves control of the same visible
TUI. Hooks and `notify` provide useful lifecycle evidence but are observation
and policy hooks, not a complete bidirectional control channel.

For state, prefer app-server's thread/turn/item events and explicit approval
requests. Keep plugin task identity separate from Codex thread, live session
root, turn, item, and request IDs. A turn completion, a session closure, a hook
notification, and a terminal becoming quiet are distinct facts. Preserve human
approval ownership; an observation adapter must not silently answer permission
hooks. See [Integration findings](../research/codex.md) for the capability
matrix, source links, caveats, and required verification scenarios.
