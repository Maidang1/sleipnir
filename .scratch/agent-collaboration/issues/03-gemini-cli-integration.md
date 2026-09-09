# Gemini CLI: visible-session control and lifecycle evidence

Id: 03
Parent: ../map.md
Labels: wayfinder:research
Type: research
Mode: AFK
Status: resolved
Assignee: Madinah
Executor: Descartes (01a08501-fb39-71b1-ab9a-cd110539df15)
Blocked by: none

## Question

Which officially supported Gemini CLI interfaces can start, prompt, observe,
interrupt, and collect results from the same interactive session shown in a
terminal Pane, while preserving human approval and takeover? Separate interactive
hooks, headless streaming, MCP, and any supported agent-control protocol. Identify
turn/session correlation, state evidence, configuration, version/platform caveats,
and unverified gaps. Findings must use primary sources checked on 2026-09-09;
do not select the final adapter architecture.

## Research context

- Evidence asset: [Integration findings](../research/gemini-cli.md).
- Archive branch: `research/agent-collaboration-gemini-cli`.
- Worktree: `/private/tmp/sleipnir-wayfinder-gemini-cli-20260909`.
- Archived at commit `849becaef9d4d3fea12028f80f3343a9b2bd1799` on
  `research/agent-collaboration-gemini-cli`.

## Answer

Gemini CLI hooks are the documented structured observation surface for the
same foreground interactive process. They expose session, prompt, model, tool,
permission, task, and agent lifecycle boundaries and permit limited blocking or
context modification. They do not provide a general external endpoint that can
wake an idle TUI with a new prompt, immediately cancel it, or resolve its native
approval UI. Terminal keystrokes could operate that Pane, but remain terminal
automation with unresolved focus, race, and acknowledgement semantics.

Headless streaming JSON and `--acp` provide stronger machine-control contracts,
but both are separate execution modes rather than attachment to the existing
interactive TUI. ACP has explicit prompt, update, cancellation, tool-call, and
permission messages, while loading a saved session reconstructs history rather
than locating a live TUI process. Gemini's MCP client and IDE companion likewise
offer tools or narrow context/diff integration, not general session control.

For state, hooks may support evidence such as prompt accepted, tool/model
activity, permission requested, and candidate response completion, but hook
payloads lack a documented universal turn/task identifier and some shutdown
events are best effort. Maintain explicit unknown state and avoid equating
`AfterAgent`, a successful headless transport result, session persistence, and
task completion. See [Integration findings](../research/gemini-cli.md) for the
pinned v0.59.0 evidence and runtime checks still required.
