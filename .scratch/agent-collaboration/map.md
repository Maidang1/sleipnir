# Agent Collaboration Plugin Decision Map

Labels: wayfinder:map
Status: open
Owner: Madinah
Created: 2026-09-09

## Destination

Resolve the product and integration decisions needed to implement an optional
Sleipnir plugin for collaboration between existing coding agents, with agent
status display and progress notifications. Finish with a clear implementation
handoff, not a built feature.

## Notes

- Confirmed in the planning conversation: integrate Codex, Claude Code,
  Gemini CLI, and OpenCode in the first supported set. Feature parity between
  them is not assumed.
- An existing Coordinator agent starts Worker agents, assigns work, waits for
  results, and synthesizes the outcome. The plugin is not a new AI engine.
- Every participating agent has its own visible terminal Pane. The human can
  inspect its full interactive interface, answer approval requests, and take
  over manually. A headless-only API is not automatically a suitable adapter.
- Plugin responsibilities include coordination access, state display, and
  notifications. The exact division between adapters, plugin, and host remains
  a decision, not an already-selected architecture.
- Preserve the existing no-built-in-AI product boundary. Do not silently add
  model calls, provider accounts, or credential management to the terminal.
- The map remains the authority for unresolved product decisions. On
  2026-09-09 the user explicitly authorized implementation to begin before the
  map was complete; the first slice is intentionally read-only and is recorded
  in
  [Agent collaboration, slice 1](../../docs/superpowers/plans/2026-09-09-agent-collaboration-slice-1.md).
  It must not pre-decide the remaining control, workspace, lifecycle, or
  notification questions.
- Consult `wayfinder`, `domain-modeling`, and `research`; use `prototype` for
  interaction questions. The `grilling` skill is unavailable locally: follow
  wayfinder's one-question-at-a-time live conversation instead.
- Vocabulary: [Agent Collaboration](../../CONTEXT.md) and the existing
  [Sleipnir glossary](../../docs/glossary.md).
- Local source context: [Plugins](../../docs/plugins.md),
  [No built-in AI](../../docs/adr/0008-no-builtin-ai.md), and
  [Default-off control surface](../../docs/adr/0011-control-surface.md).
- Tracker: local Markdown; no remote tracker was configured. Child issues
  live in `issues/`. A ticket's `Parent` identifies this map. `Blocked by`
  refers to child IDs; all listed blockers must be `resolved`. The frontier
  is open, unassigned, unblocked children, ordered by ID. Claim by setting
  `Status: claimed` and `Assignee: Madinah` before work. Record research
  executor separately. Resolve with `## Answer`, `Status: resolved`, and a
  one-line linked gist below. Answers live only in their ticket; research
  evidence lives in linked assets archived on `research/<name>` branches.

## Decisions so far

- [Codex: visible-session control and lifecycle evidence](issues/01-codex-integration.md)
  — Remote TUI plus app-server is the official structured same-session candidate;
  ordinary TUI attachment and safe multi-client control remain unproven, while
  exec/SDK/MCP continuations are not substitutes for a visible live session.
- [Claude Code: visible-session control and lifecycle evidence](issues/02-claude-code-integration.md)
  — Hooks and SDK/headless surfaces are useful, but no official interface was
  found for controlling an arbitrary already-running foreground TUI; persisted,
  background, SDK-managed, and visible sessions must remain distinct.
- [OpenCode: visible-session control and lifecycle evidence](issues/04-opencode-integration.md)
  — Its client/server TUI and attachable sessions provide a structured
  same-session route, subject to explicit writer serialization, runtime schema
  detection, human-owned approvals, and experimental TUI API caveats.
- [Gemini CLI: visible-session control and lifecycle evidence](issues/03-gemini-cli-integration.md)
  — Hooks can observe and influence lifecycle boundaries in the visible TUI,
  but general steering is not documented; ACP and headless JSON are separate
  execution modes rather than attachment to that foreground process.

## Not yet specified

- Adapter installation and onboarding shape, once required agent-specific
  hooks, configuration changes, and control transports are known.
- Compatibility upkeep and degraded experiences as the real differences
  between the four integrations become clearer.
- Operational and failure-recovery scenarios that emerge from the chosen
  session, control, and workspace boundaries.
- Implementation handoff and verification scope once the user-facing
  interaction and host/plugin responsibilities have been decided.

## Out of scope

- Building a new agent runtime or putting model calls and provider credential
  management into the Sleipnir core.
- Shipping automatic coordination, prompt injection, or approval handling
  before the remaining decision tickets are resolved. The explicitly approved
  read-only observer slice is the sole execution exception so far.
- Expanding the initial supported-agent set beyond the four confirmed tools;
  additional integrations require a separate scope change.
