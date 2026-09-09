# OpenCode: visible-session control and lifecycle evidence

Id: 04
Parent: ../map.md
Labels: wayfinder:research
Type: research
Mode: AFK
Status: resolved
Assignee: Madinah
Executor: Planck (01a08502-48ec-7950-81ba-e01f573337d4)
Replacement executor: Euler (01a0850c-e5e6-71d1-89ce-9ee5dfed79d1)
Blocked by: none

## Question

Which officially supported OpenCode interfaces can start, prompt, observe,
interrupt, and collect results from the same interactive session shown in a
terminal Pane, while preserving human approval and takeover? Verify the server,
SDK, events, plugins, and terminal attach semantics, including whether UI and
automation truly address the same live session. Identify correlation, state
evidence, authentication/configuration, version/platform caveats, and unverified
gaps. Findings must use primary sources checked on 2026-09-09; do not select the
final adapter architecture.

## Research context

- Evidence asset: [Integration findings](../research/opencode.md).
- Archive branch: `research/agent-collaboration-opencode`.
- Worktree: `/private/tmp/sleipnir-wayfinder-opencode-20260909`.
- Archived at commit `ecbb9c4b568f44023db352392f1e1a430eccb405` on
  `research/agent-collaboration-opencode`.

## Answer

OpenCode has the strongest documented same-visible-session integration of the
four candidates researched so far. Its TUI is a client of an OpenCode server;
`opencode attach <url> --session <id>` attaches a visible TUI to a selected
server session, while the SDK/server API can prompt, observe SSE events, abort,
read messages/results, and route permission or question requests against that
same server-side session.

This supports a structured adapter without terminal-screen parsing, but does
not settle concurrent writer ownership. No reviewed contract guarantees the
ordering of simultaneous human and coordinator prompts, exactly-once event
delivery, or targeting one particular TUI when several clients share a server.
Writer actions should therefore be serialized by a future coordination policy,
with human approval retained through server-mediated pending requests.

The runtime `/doc` OpenAPI schema and detected version should be authoritative:
the public docs and current source differ around legacy/new permission routes,
and current TUI HTTP routes include experimental surfaces. Directory/workspace
scope and local server authentication must be explicit. See
[Integration findings](../research/opencode.md) for the evidence matrix and
remaining verification cases.
