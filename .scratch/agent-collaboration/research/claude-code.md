# Claude Code integration research: visible-session control and lifecycle evidence

**Scope and method.** This note answers ticket 02 only. It reviews Anthropic's public Claude Code documentation and Anthropic-maintained source repositories, checked **2026-09-09**. It does not run Claude Code, inspect a local installation, mutate configuration, or choose an adapter architecture. “Same live TUI” below means the already-running interactive `claude` process occupying a visible terminal Pane, rather than a second process which reloads the same persisted transcript.

## Bottom line

**Verified:** Claude Code exposes several officially documented automation and observation surfaces: lifecycle hooks; non-interactive `claude -p`; the Python/TypeScript Agent SDK; MCP tools; background-agent/agent-view commands; and in-product subagents/agent teams. They can start work, report events/results, enforce or answer selected permission decisions, and resume persisted conversations in specified modes.

**Verified negative / bounded conclusion:** I found **no official documentation of a public control/attachment API that can inject a prompt into, stream/subscribe to, interrupt, or answer approval/questions for an arbitrary already-running interactive terminal TUI process**. The documented session mechanisms persist and restore a conversation; the SDK gives programmatic control over its own SDK-managed session; and `claude attach` attaches a terminal to an explicitly dispatched *background* session. None is documented as attaching an automation client to the foreground TUI that already owns a Pane. Therefore, an architecture claiming same-live-pane control would be an **unverified inference**, not a supported Claude Code integration contract.

This is not proof that it is technically impossible (terminal/PTY automation is outside the product API), only that no supported interface was located in the stated official sources.

## Interface-by-interface evidence

### 1. Interactive CLI and stored sessions

**Verified.** The CLI reference's **“CLI commands”** documents `claude` for an interactive session and `claude "query"` for an interactive session with an initial prompt. It separately documents `claude -p "query"` as an SDK query which exits, `-c`/`--continue`, and `-r`/`--resume <session>`. The same reference describes `claude agents` as an interactive agent view for monitoring and dispatching parallel *background* sessions, optionally JSON-listable, and `claude attach <id>` as attaching a terminal to a background session. [CLI reference — “CLI commands”](https://code.claude.com/docs/en/cli-reference)

**Verified.** The session guide's **“Resume a session”** says sessions are continuously saved to local transcript files; `--continue`, `--resume`, and `/resume` restore a chosen conversation. Its **“What a resumed session restores”** begins by listing persisted conversation history, including tool calls and results. Crucially, it says CLI-created `-p`/SDK sessions are not in the interactive session picker/normal `--continue`, although an ID can resume one. This documents persistence and later restoration, not control of the original live process or its terminal. [Manage sessions — “Resume a session”; “What a resumed session restores”](https://code.claude.com/docs/en/sessions)

**Interpretation.** A session ID is suitable to correlate transcript history and a resumed/new run, but it does **not** establish a single-owner/live-turn protocol. The docs do not state that a second `--resume <id>` process shares the original process's prompt queue, stream, approval modal, or interrupt channel. Treat “same transcript” and “same live Pane” as distinct until Anthropic documents otherwise.

### 2. Headless CLI and Agent SDK

**Verified.** The headless guide's **“Basic usage”** says `claude -p` is non-interactive, runs the prompt, and exits; it supports structured output and streaming options, returns process exit codes, and can use `--continue`. It explicitly says `-p` cannot combine with `--bg`; `--cloud` has separate conflict/queueing rules. Its **“Start faster with bare mode”** says normal headless runs load much of the same startup configuration as interactive sessions, while `--bare` skips hooks, skills, custom commands, subagents, plugins, MCP, memory, and `CLAUDE.md`. [Run Claude Code programmatically — “Basic usage”; “Start faster with bare mode”](https://code.claude.com/docs/en/headless)

**Verified.** That page's navigation documents explicit supported automation topics: **“Stop a run with SIGTERM,” “Stream responses,” “Follow subagent messages,” “Read session metadata,” “Auto-approve tools,” “Turn off permission prompts in unattended runs,”** and **“Continue conversations.”** These are evidence for lifecycle/result collection in a headless process, but do not say they control an interactive TUI process. [Run Claude Code programmatically](https://code.claude.com/docs/en/headless)

**Verified.** The Python SDK reference's **“Choosing between query() and ClaudeSDKClient”** contrasts `query()` (one exchange/new session by default; no interrupts) with `ClaudeSDKClient` (multiple exchanges in the same context; manual connection control; interrupts supported). **“query()”** says calls begin fresh unless `continue_conversation=True` or `resume` is supplied in options, and yields arriving messages through an async iterator. [Agent SDK reference — Python, “Choosing between query() and ClaudeSDKClient”; “query()”](https://code.claude.com/docs/en/agent-sdk/python)

**Interpretation.** The SDK is the strongest documented programmatic surface for launch, bidirectional turn control, streaming messages, SDK-owned approvals/hooks, interrupt, and session correlation. Yet “same session” in this reference means the SDK client's conversation/connection. It is not documented as a client for an independently launched foreground TUI. An SDK `resume` should be described conservatively as continuing persisted conversation state, not attaching to the existing Pane.

**Correlation/lifecycle evidence.** For headless/SDK automation, retain the documented session ID/metadata plus the subprocess/run ID generated by the integrating system, and record message/event order from streaming. For an interactive session, hook payloads can add lifecycle evidence (next section). No cited source defines a universal turn ID or task ID that joins foreground TUI, hooks, SDK runs, and agent teams; such a correlation model remains an integration-layer design issue.

### 3. Hooks: observation, policy, and limited intervention

**Verified.** The hooks reference's **“Hook lifecycle”** says hooks can be shell commands, HTTP endpoints, MCP tool calls, prompts, or subagents, receive JSON context, and run at defined session/turn/tool cadences. It says the same hook events fire in terminal, IDE, Desktop, and web sessions. Events include SessionStart/SessionEnd; UserPromptSubmit; PreToolUse/PostToolUse; PermissionRequest and PermissionDenied; Notification; MessageDisplay; SubagentStart/SubagentStop; TaskCreated/TaskCompleted; and Stop/StopFailure. [Hooks reference — “Hook lifecycle”; “Hook events”](https://code.claude.com/docs/en/hooks)

**Verified.** Hooks may inspect input and “optionally return a decision”; PreToolUse can block a call, and PermissionRequest is fired when a tool needs a permission decision. This supports audit/lifecycle observation and deterministic guardrails inside the live session. [Hooks reference — “Hook lifecycle”](https://code.claude.com/docs/en/hooks)

**Boundary.** A hook is fired by Claude Code in response to Claude Code lifecycle events. The documentation does not describe a hook endpoint as an external command channel that can submit a new user prompt, take over the TUI, send Ctrl-C, or operate its widgets. It can influence particular hook decisions, not provide general remote control of the active terminal UI.

### 4. Permissions, questions, and human takeover

**Verified.** The permissions guide's **“Permission system”** specifies that manual mode requests approval for Bash, edits, WebFetch, WebSearch, and other tool categories as applicable; “Yes, and don't ask again” may persist some rules, while edit approval lasts for the session. Auto mode replaces some routine human prompts with a classifier, while explicit ask/deny rules remain effective. [Configure permissions — “Permission system”; “Permission modes”](https://code.claude.com/docs/en/permissions)

**Interpretation.** A visible interactive Pane preserves native human approval/takeover because its UI owns those prompts. Headless/SDK can instead use pre-approved tools, a permission mode, or SDK approval callbacks (the headless guide directs readers to SDK docs for callbacks). Neither source demonstrates arbitration where an external automation client answers a currently visible TUI approval while a human may also answer it. That race/ownership behavior is unknown and should not be assumed.

### 5. MCP, subagents, and agent teams

**Verified.** MCP gives Claude Code access to external tools/data sources; the MCP reference covers local/remote server transports, server status, approvals, elicitation requests, resources, and “Use Claude Code as an MCP server.” It is a tool-integration protocol, not documentation of controlling a running TUI. [Connect Claude Code to tools via MCP — “What you can do with MCP”; “Use Claude Code as an MCP server”](https://code.claude.com/docs/en/mcp)

**Verified.** Custom subagents run in their own context windows and return results within a single session. The parallel-work guide distinguishes them from separately dispatched sessions and calls all workers Claude sessions. [Create custom subagents — “Create custom subagents”; “Built-in subagents”](https://code.claude.com/docs/en/sub-agents) [Run agents in parallel — “Choose an approach”](https://code.claude.com/docs/en/agents)

**Verified/caveat.** Agent teams coordinate separate sessions with a lead, shared task list, and inter-agent messaging, but are experimental, disabled by default through `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`, and have known resumption/task-coordination/shutdown limitations. The page is specifically “as of v2.1.178.” [Orchestrate teams of Claude Code sessions — “Enable agent teams”; “How agent teams work”](https://code.claude.com/docs/en/agent-teams)

**Interpretation.** These are Claude-internal delegation/coordination options, not an external orchestration API for a pre-existing foreground TUI. Agent teams may visibly use terminals depending on display mode, but that is not the requested same-Pane remote-control contract.

## Configuration, version, and platform caveats

**Verified.** Configuration derives from hierarchical JSON settings (user, project, local/managed scopes); terminal, IDE, and Desktop share settings files, whereas web/cloud sessions are different machines with different coverage. Verify effective loading with the product's configuration/status facilities before relying on hooks or permissions. [Claude Code settings — “Settings files and precedence”](https://code.claude.com/docs/en/settings)

**Verified.** Supported local platforms are macOS 13+, Windows 10 1809+/Windows Server 2019+, Ubuntu 20.04+, Debian 10+, and Alpine 3.19+, on x64/ARM64 with an internet connection; shells include Bash, Zsh, PowerShell, and CMD. Native releases auto-update; Homebrew stable and latest channels differ. [Advanced setup — “System requirements”; “Install Claude Code”; “Auto-updates”](https://code.claude.com/docs/en/setup)

**Risk.** The surfaces are version-sensitive: agent teams explicitly references v2.1.178, CLI gateway has a stated v2.1.195 minimum, and session cross-project behavior changed before v2.1.223. Pin/test exact installed CLI and SDK versions before accepting any operational guarantee. [CLI reference — “CLI commands”](https://code.claude.com/docs/en/cli-reference) [Manage sessions — “Resume a session”](https://code.claude.com/docs/en/sessions)

## Open questions requiring product-level verification

1. Whether any undocumented but supported `claude` IPC/control socket, remote-control feature, or `claude attach` variant exists for an already-foreground TUI. None was found in the official pages reviewed.
2. Whether two concurrent processes resuming the same transcript are prevented, serialized, or allowed; and how they interact with live approvals, compaction, transcript writes, and cancellation.
3. Exact SDK event schemas carrying session/turn/task/subagent IDs and their stability across CLI/SDK versions; confirm in the pinned SDK reference/source before implementation.
4. Whether foreground interactive `claude` emits sufficiently complete hooks (including UI prompt/question transitions) for a reliable observer, and whether asynchronous HTTP/MCP hook failure/retry semantics meet lifecycle-audit requirements.
5. The scope/semantics of “Use Claude Code as an MCP server”: verify whether it can expose a named session and, if so, whether that session is necessarily a new server-owned session rather than a live TUI attachment.

