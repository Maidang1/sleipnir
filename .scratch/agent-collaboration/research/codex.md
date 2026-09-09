# Codex: control and observation of a visible live session

Checked: **2026-09-09**. Scope: ticket 01; research, not implementation or an
adapter decision. "Same live session" means the runtime conversation currently
displayed in the human's terminal pane, not another process continuing its saved
history.

Evidence: official OpenAI documentation only. Initial `web.run` requests were
empty; later searches returned references, but full-page opens remained empty.
The substantive evidence below was fetched directly from official Markdown URLs
with `curl -fsSL`, using approved escalation after sandbox DNS failures. These
are documentation snapshots, not binary/version verification. No agent CLI,
authentication, configuration changes, or runtime experiments were performed.

## Finding and capability matrix

The docs explicitly describe connecting the **native terminal UI to app-server**
with `codex --remote <endpoint>`. App-server exposes thread/turn control, events,
and approvals. This establishes an official building block for a visible,
protocol-backed session, but not a verified guarantee that an arbitrary existing
TUI can be attached to, or that a sidecar and TUI safely share approval ownership.
App-server and its WebSocket transport carry experimental/non-production
warnings. [S1, S2]

| Interface | Prompt/control and observation | Same visible live session; human control |
| --- | --- | --- |
| Native `codex` TUI | Initial prompt argument; interactive input, steering, queued follow-ups; rendered output | Human approvals remain in the TUI. External keystroke delivery is terminal automation, not a documented Codex RPC contract. [S2] |
| App-server with remote TUI | `turn/start`, `turn/steer`, `turn/interrupt`; structured events and history reads | Documented TUI connection to an endpoint. Sharing one loaded thread across TUI and sidecar needs verification; not arbitrary-PTY attachment. [S1, S2] |
| `codex exec --json` | Argument/stdin prompt; JSONL progress; final message/file; later `exec resume` | Non-interactive run, not the displayed TUI. Showing its output in a pane does not create interactive takeover. [S5] |
| Codex SDK | TypeScript thread run/resume and result; Python app-server JSON-RPC wrapper | SDK thread continuation does not establish attachment to a live TUI. Transport/runtime distinctions matter. [S6] |
| Hooks / `notify` | Lifecycle callbacks, limited context/continuation decisions, completion payloads | Can observe the executing session when configured and trusted; not a general external prompt/interrupt endpoint. [S3, S4] |
| Codex as MCP client | Agent calls configured tools; tools return results | Tools operate within the agent loop; no documented general TUI-control interface. MCP tool hooks add event-driven calls. [S7, S3] |
| `codex mcp-server` | `codex` starts a conversation; `codex-reply` continues by `threadId`; structured result | Separate server interface, currently documented as deprecated; no live-TUI attachment contract. [S8] |

## App-server: established capabilities and the attachment boundary

The documented topology starts `codex app-server --listen <endpoint>` and
connects the native TUI using `--remote`. Transport options include JSONL stdio,
WebSocket, and WebSocket-over-Unix-socket. CLI `resume` also accepts remote mode.
This differs from independently launching `exec resume` against stored history.
`codex remote-control` manages a daemon/pairing workflow; the CLI reference says
it is not a substitute for `app-server --listen` for local protocol clients.
[S1: "Connect the CLI terminal UI", "Protocol"; S2]

Each protocol connection performs `initialize` then `initialized`.
`thread/start` creates a thread; start/resume establishes event observation.
`thread/read` reads without subscribing, while `thread/loaded/list` identifies
threads loaded in that server. Connection-scoped subscriptions and
`thread/unsubscribe` are documented. They are evidence of a multi-client design,
not, by themselves, a complete simultaneous-controller contract. [S1]

Prompt acceptance is distinct from completion. `turn/start` returns the turn;
`turn/steer` adds input to an active turn and requires matching
`expectedTurnId`, without creating a new turn. `turn/interrupt` acknowledges the
request; the terminal turn status is `interrupted`. Do not interpret an RPC
acknowledgment as completed work. The reviewed docs do not establish ordering
for concurrent human and sidecar submissions or retry idempotency. [S1]

Approvals arrive as server requests, including command, file-change, permission,
and user-input requests. Correlation uses request IDs plus thread/turn/item IDs.
`serverRequest/resolved` means answered **or cleared**, not necessarily approved.
Preserving human control requires retaining a human responder, not automatically
accepting these requests. `approvals_reviewer` distinguishes `user` from
`auto_review`; managed requirements may constrain it. Which connected client
owns a pending approval, duplicate-answer behavior, and ownership after
disconnect remain unverified. [S1: "Approvals"; S9]

App-server's `command/exec` PTY controls target command sessions; they are not
documented as attaching to the Codex TUI. Some other process APIs explicitly
operate outside the Codex sandbox. They must not be confused with turn control.
[S1: "Process execution", "Command execution"]

## Identity, lifecycle, and result evidence

Keep a planning task ID separate from runtime identifiers. A task may require
several turns. Suggested correlation is pane, server instance/endpoint, thread,
session root, turn, item, and outstanding request; this is a planning implication,
not an existing Codex task schema. [Inference from S1, S3, S5]

Important documented distinctions:

- `thread.id` identifies the conversation. `thread.sessionId` identifies its
  live session-tree root; forked threads can retain the root's session ID.
  Therefore session ID alone cannot select one conversation. [S1]
- Hooks carry `session_id`; turn-scoped hooks carry `turn_id`. Subagent hook
  session IDs refer to the parent, with separate agent identifiers. Validate
  cross-interface mappings rather than assuming all similarly named fields are
  interchangeable. [S3]
- `exec --json` documents `thread_id`, but its sample turn events contain no
  turn ID. Do not assume app-server's schema applies to exec JSONL. [S5]

For app-server, use `turn/started`, terminal `turn/completed` status
(`completed`, `interrupted`, `failed`), and authoritative `item/completed`
objects. Thread runtime status includes `idle`, `active`, `notLoaded`, and
`systemError`; `waitingOnApproval` appears in active flags. These are stronger
state evidence than a spinner or silence. Completed agent-message items carry
text and optionally a phase; command items carry output/status. Accumulate
results by IDs and distinguish commentary from final answers. [S1]

Health probes only establish listener health/readiness. No output, a restored
prompt, terminal title, BEL/OSC notification, transcript modification time, or
process exit alone proves task success. Disconnect should become an explicit
unknown state until reconciled. History reads exist, but the reviewed docs do
not establish lossless replay, reconnect cursors, or pending-request recovery.
[S1, S4; interpretation]

## Hooks and notifications: useful but bounded

Current docs list `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, tool
hooks, `Stop`, `Interrupt`, and `SessionEnd`, among others. Command hooks receive
JSON on stdin. Turn hooks can identify the turn; `Stop` includes nullable
`last_assistant_message`. Critically, a blocking `Stop` decision creates another
continuation prompt: observing `Stop` is not final completion. `Interrupt`
reports an active main-thread interruption and cannot prevent/restart it.
`SessionEnd` covers normal closure and other documented end conditions, not a
crash-proof termination guarantee. [S3]

Non-managed hooks require trust of their exact definition. Multiple matching
hooks can run concurrently; background hooks do not produce app-server's
synchronous hook-start/completion notifications. A `PermissionRequest` hook can
bypass the visible approval by returning allow; an observation-only integration
must decline to decide. Hook transcript paths are convenient, but their format
is explicitly unstable. [S3, S1]

`notify` is narrower: currently `agent-turn-complete`, with `thread-id`,
`turn-id`, input messages, and last assistant message in a JSON argument.
`tui.notifications` separately supports attention notifications such as
`approval-requested`, with focus/terminal-dependent delivery. Neither is a
bidirectional session bus. Project config cannot set `notify`; it belongs in an
allowed machine-local configuration layer. [S4, S9]

## Non-interactive and MCP boundaries

Exec accepts an argument or stdin; normal output separates progress on stderr
from the final stdout message. JSONL provides lifecycle events;
`--output-last-message` and `--output-schema` support result collection.
Resumption continues history in a non-interactive execution. It does not turn
the existing visible TUI into the recipient. [S5]

Avoid collapsing all SDKs into exec: official Python documentation explicitly
describes app-server JSON-RPC and a pinned runtime dependency; TypeScript
documents `startThread`, `run`, `resumeThread`, and `finalResponse`. Neither
usage example proves a second client can attach to a user's active TUI. [S6]

MCP configuration sharing between Codex clients is not live-session sharing.
The deprecated MCP-server guide returns `structuredContent.threadId` and result
content; its `codex-reply` thread continuation and approval payloads do not
establish TUI takeover. An MCP tool's result belongs to that invocation, not an
independent global completion signal. [S7, S8; interpretation]

## Unresolved checks and planning implications

No final architecture is selected. A subsequent bounded verification should:

1. Establish installed-version support for remote TUI, chosen transport, schema,
   and hooks. Docs warn that generated schemas are version-specific and hook
   schemas on repository `main` can precede releases. No minimum version or
   OS-wide support claim is established here. [S1, S3]
2. Demonstrate TUI and sidecar observing the same loaded thread/turn, including
   human-entered and automated prompts. Separately test an already-running
   ordinary TUI: no arbitrary PID/PTY attachment contract was found. [S1, S2]
3. Exercise approval routing, human interruption/takeover, simultaneous inputs,
   disconnect/reconnect, and crash recovery without duplicate execution.
4. Preserve sandbox, approval, and hook trust policies; do not copy permissive
   sample settings. Keep transport local or authenticated/TLS-protected:
   app-server docs warn that network listeners can otherwise be unauthenticated.
   Experimental transport/API warnings remain material. [S1, S9]

Represent **documented protocol state**, **hook observations**, and **terminal
heuristics** separately on the planning map. Also distinguish resumed history,
same live runtime, and human-visible approval capability. These are different
acceptance criteria, not interchangeable evidence.

## Sources checked on 2026-09-09

All URLs below were fetched directly; headings identify relevant sections.

- **S1:** <https://learn.chatgpt.com/docs/app-server.md> - "Connect the CLI terminal UI", "Protocol", "Threads", "Turns", "Events", "Approvals", "Message schema".
- **S2:** <https://learn.chatgpt.com/docs/cli/reference.md> - "Global flags", "Command details", "Interactive shortcuts".
- **S3:** <https://learn.chatgpt.com/docs/hooks.md> - "Common input fields", "Review and trust hooks", "Stop", "Interrupt", "SessionEnd", "Schemas".
- **S4:** <https://learn.chatgpt.com/docs/config-file/config-advanced.md> - "Notifications", "Project configuration files".
- **S5:** <https://learn.chatgpt.com/docs/non-interactive-mode.md> - "Make output machine-readable", "Resume a non-interactive session", "Advanced stdin piping".
- **S6:** <https://learn.chatgpt.com/docs/codex-sdk.md> - "TypeScript library", "Python library".
- **S7:** <https://learn.chatgpt.com/docs/extend/mcp.md> - "Supported MCP features", "Connect Codex to an MCP server".
- **S8:** <https://learn.chatgpt.com/docs/mcp-server.md> - "Running Codex as an MCP server".
- **S9:** <https://learn.chatgpt.com/docs/config-file/config-reference.md> - `approvals_reviewer`, `allowed_approvals_reviewers`, project-config restrictions.
