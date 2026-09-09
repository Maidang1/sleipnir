# Claude Code: visible-session control and lifecycle evidence

Id: 02
Parent: ../map.md
Labels: wayfinder:research
Type: research
Mode: AFK
Status: resolved
Assignee: Madinah
Executor: Peirce (01a08501-adff-7e82-9285-cf4700f1a11f)
Replacement executor: Gibbs (01a0850c-c678-70d1-af19-5dea0e8bd8ed)
Blocked by: none

## Question

Which officially supported Claude Code interfaces can start, prompt, observe,
interrupt, and collect results from the same interactive session shown in a
terminal Pane, while preserving human approval and takeover? Separate native
hooks, interactive CLI, headless SDK/CLI, MCP, and built-in teams/subagents.
Identify turn/session correlation, state evidence, configuration, version/platform
caveats, and unverified gaps. Findings must use primary sources checked on
2026-09-09; do not select the final adapter architecture.

## Research context

- Evidence asset: [Integration findings](../research/claude-code.md).
- Archive branch: `research/agent-collaboration-claude-code`.
- Worktree: `/private/tmp/sleipnir-wayfinder-claude-code-20260909`.
- Archived at commit `738a12f12644459cd0f66c2d10276d1d5f51bead` on
  `research/agent-collaboration-claude-code`.

## Answer

Claude Code officially provides several useful but distinct integration
surfaces: lifecycle hooks; non-interactive `claude -p`; the Agent SDK; MCP;
background sessions; subagents; and experimental agent teams. These can start
and observe work, stream results, interrupt SDK-managed sessions, and expose
permission/task events.

No reviewed official interface can attach an automation client to an arbitrary
already-running foreground Claude Code TUI and then prompt, interrupt, or answer
questions for that exact live process. `--continue` and `--resume` restore a
persisted conversation in a process; they do not document concurrent ownership
of the original Pane. `claude attach` targets explicitly dispatched background
sessions, not arbitrary foreground sessions. Therefore same transcript, same
SDK session, same background session, and same visible foreground TUI must remain
separate concepts.

Hooks are appropriate structured observation points for interactive sessions,
but they are not a general remote-control channel. The plugin must preserve
native human approval ownership and must not infer that a shared session ID
permits concurrent prompt/interrupt/approval actions. See
[Integration findings](../research/claude-code.md) for official-source evidence,
version-sensitive features, and unanswered runtime questions.
