# Gemini CLI: same-live-Pane integration evidence

Checked **2026-09-09** for Wayfinder planning; static primary-source research, not
a runtime verification. GitHub's latest stable release was **v0.59.0**, published
2026-09-08 21:13:43 UTC (September 9 in Shanghai), commit
`fb0d535af931b27c51e87e5e6ade72905b1e8390`. Sources below are pinned to that release
unless identified as ACP protocol documentation. `web.run` search/open returned
empty responses; official sources were fetched using `curl` after approved
network escalation. No agent CLI, account, credentials, or configuration was
accessed or changed. [R]

## Finding and capability matrix

**No documented general-purpose control attachment to an already-running Gemini
TUI was established.** Hooks can observe and influence that live process at
lifecycle boundaries; terminal interaction preserves its human UI. Headless JSON
and ACP are distinct execution paths, not attachments. The entrypoint returns
into ACP before rendering the interactive UI; headless execution follows a
different branch. This is a bounded finding, not proof against every possible
internal mechanism or an adapter architecture decision. [E]

| Interface | Same live Gemini TUI? | Prompt / interrupt | Observation / results | Approval and limits |
|---|---|---|---|---|
| Interactive terminal | Yes | Launch with `gemini` or `-i`; subsequent keyboard input; context-sensitive cancel keys | Rendered conversation; no typed completion envelope | Native human approval/takeover; terminal automation is not a Gemini RPC. [C,K,U] |
| Command hooks | Yes, when configured in that process | Context/block/retry at hook boundaries; no unsolicited idle prompt endpoint | Session, agent, model, tool, permission events; `AfterAgent.prompt_response` | Notification cannot grant permission; hooks are not durable messaging. [H,B] |
| `-p ... --output-format stream-json` | No TUI | One input/run; no documented inbound prompt/approval stream | JSONL messages, tool events, final status/stats | Separate headless process even when launched visibly. [J,E] |
| `--acp` | No built-in TUI alongside it | JSON-RPC session prompt/cancel/mode methods | Session updates, tool IDs, prompt response/stop reason | Client supplies permission UI; not attachment to the existing Pane TUI. [A,E,P] |
| MCP servers | Tools/context within the live agent | Model invokes tools; user invokes discovered prompt slash commands | Tool-scoped requests/results | Gemini is the MCP client; not a session-control server. [M] |
| IDE companion | Yes, limited integration | Context updates and diff acceptance/rejection | File/diff-specific state | Human can edit/accept/reject an offered diff; no general prompt/cancel contract. [I] |
| Transcript / telemetry / DevTools | Yes, observational | No documented task-submission interface | Persisted messages; correlated telemetry; diagnostic streams | Storage and diagnostic evidence, not authoritative input ownership. [T,O,D] |

## Interactive operation and human authority

With a TTY, `gemini` starts the REPL; `gemini -i "request"` submits an initial
prompt and remains interactive. `-i` rejects piped stdin. `-p` explicitly selects
headless operation. Resume flags load saved conversation history, not a live
process connection. Source also exposes `--session-id` for a new caller-named
session, mutually exclusive with `--resume` and `--session-file`; this is an
identity aid, not an attachment token. [C,E]

Enter submits; Tab can queue a prompt. Ctrl+C cancels a request or, depending on
input state, clears input/quits. The streaming UI handles Escape while responding
or awaiting confirmation when the shell is not focused. Focused shells and
dialogs change key interpretation. Thus PTY input could target the actual Pane
process, but reliable focus detection, input arbitration, cancellation
acknowledgement, and takeover races remain unverified integration work. [K,U]

Keep approval distinctions explicit: `default`, `auto_edit`, `yolo`, and `plan`
have different authority. MCP `trust: true` bypasses server-tool confirmations;
hook `allow` is not human approval. The companion specification provides a
genuine narrower same-session bridge: context notifications and asynchronous
diff decisions, with an authenticated local MCP server and workspace/PID-based
discovery. It does not supply general task steering. [C,M,I]

## Hooks: useful lifecycle evidence, incomplete control contract

Command hooks receive JSON on stdin and return JSON on stdout. Configured hooks
normally run synchronously, with a default 60-second timeout. Settings include
`hooks.<event>`, canonical `hooksConfig.enabled` (default true; restart required),
and disabled hook names. Project hook fingerprints/trust and settings layers can
alter availability; merely installing a hook file proves nothing about the live
process. [H,G]

Base input contains `session_id`, `transcript_path`, `cwd`, `hook_event_name`, and
`timestamp`. `BeforeAgent` supplies the prompt; `AfterAgent` supplies prompt,
response, and `stop_hook_active`. Tool hooks supply names/arguments/results;
model hooks expose requests/responses, including streamed chunks. `SessionStart`
distinguishes startup/resume/clear. [H]

`BeforeAgent` can deny the turn or append context; `BeforeTool` can deny/rewrite
arguments; supported `continue: false` responses stop agent execution at that
boundary. `AfterAgent` denial requests another attempt. These callbacks neither
provide immediate out-of-band cancellation nor wake an idle session with a new
task. A final-response callback can precede hook-induced retry, so it alone is
not proof of settled success. Exceptions/cancellation paths need separate
evidence. [H,B]

`Notification` with `ToolPermission` reports an impending confirmation but cannot
approve it. Source emits it **before** generating the internal confirmation
correlation UUID and entering `AwaitingApproval`; consumers must not treat it as
a resolvable permission handle or exact UI-ready acknowledgement. No public
approval-resolution hook was found. [H,Q]

The hook payload has no documented turn ID, message ID, or tool-call ID;
identical prompts and concurrent same-name tools cannot be uniquely joined from
text alone. Source may return an empty `transcript_path`. Telemetry separately
documents `session.id` and `prompt_id` on prompt/tool/API events; this improves
correlation but does not create a hooks-to-turn mapping guarantee. [B,O]

Failure caveats matter: documentation describes non-0/2 exits as warnings, while
the runner's plain-text fallback treats nonzero exits other than 1 as denial.
Prefer valid JSON with exit 0 for explicit decisions; do not assume hooks are a
fail-closed approval service. `SessionEnd` is documented best-effort although
graceful cleanup awaits its call in source; crash/kill delivery remains
unguaranteed. [H,B,E]

## Headless and ACP: structured, but different sessions/surfaces

Headless JSONL has `init` (`session_id`, model), `message`, `tool_use`,
`tool_result`, `error`, and `result`. Every event has a timestamp; tools join by
`tool_id`, but message/result events do not carry a turn ID. Accumulate assistant
deltas; `result` holds status/stats, not the answer text. Tool output is optional.
The documented exit codes include 0, 1, 42, and 53. A hook-requested stop can emit
`result.status: success`, so transport success is not task acceptance. [J]

No bidirectional approval channel appears in that schema. Headless configuration
describes ASK_USER policy decisions becoming DENY and excludes conversational
`ask_user`; pre-authorized actions differ from native human approval. A visible
terminal displaying JSON does not turn this run into the interactive session.
[J,E]

ACP is **Agent Client Protocol**, launched with `gemini --acp` over NDJSON
JSON-RPC stdio. Although the cheatsheet still labels `--experimental-acp`
experimental, the release's ACP guide and parser use `--acp`; the old flag is a
deprecated alias. The pinned package uses ACP SDK 0.16.1. Do not infer stability
from inconsistent documentation labels. [A,C,R]

After initialization, `session/new` returns `sessionId`; `session/prompt` drives
work, `session/update` streams text/thoughts/tool states, and
`session/request_permission` asks the client for a decision. The original prompt
RPC response provides `stopReason`; text arrives separately. Cancellation uses
`session/cancel`; the protocol requires cancelling pending permission requests
and awaiting the prompt's cancelled response. `toolCallId` correlates tools;
updates are session-scoped, not stamped with the prompt RPC ID. Gemini source
aborts its prior pending prompt when another starts: overlapping prompts are not
an independent turn queue. [A,P]

Gemini advertises `loadSession`; its implementation reads saved data, reconstructs
chat, creates a Session, and replays history. It does not locate the live TUI
process. The current generic ACP spec also describes capability-gated
`session/resume`/`close`; this release's dispatcher neither advertises those
capabilities nor implements those handlers. Generic protocol capability is not
Gemini implementation evidence. [A,P]

## State evidence, caveats, and planning implications

Treat `BeforeAgent` as submitted-work evidence, model/tool events as activity,
permission notification as pending-human evidence, and `AfterAgent` as a
candidate result. Missing events, silence, or process existence cannot distinguish
idle, blocked, disconnected, cancelled, or successful. Retain an unknown state;
no authoritative same-TUI public state-query endpoint was established. This is
an inference from the contracts above, not a proposed final state machine.
[H,B,Q,E]

Automatic transcripts preserve messages/tool records with IDs, but v0.59.0
writes **JSONL**, despite the hook reference calling the transcript JSON. Records
include metadata updates and rewinds; they are not simply append-only chat text.
Legacy JSON is supported; disk-full can disable recording. Prefer the reported
path and version-aware parsing over filename guesses. Telemetry may contain
prompts and authenticated-user identifiers. DevTools is opt-in, emits
session-tagged diagnostics, and has a debugger-trigger endpoint, not a documented
prompt/cancel/approval API. [T,O,D]

Node >=20 is specified; recommended platforms include macOS 15+, Windows 11
24H2+, and Ubuntu 20.04+. Hook shells, environment sanitization, sandbox paths,
terminal focus, and keyboard support require platform checks. macOS Terminal
does not support Shift+Enter; Windows Terminal/Node combinations constrain
Shift+Enter/Shift+Tab. No platform or authenticated runtime was tested here.
[R,K,H]

**Planning implications, not decisions:** same-TUI ownership and structured ACP
ownership must remain separate capability claims. Hooks/telemetry warrant
evaluation for observation, not presumed universal steering. Any future spike
should verify correlation across retry/clear/resume, human/machine input races,
permission timing, cancel-during-tool behavior, transcript completeness, and
shutdown loss. Separate headless or restored sessions cannot satisfy a
same-live-session requirement merely by sharing a directory or session ID.
[E,A,B,T]

## Primary sources and sections

- **[R]** [Release](https://github.com/google-gemini/gemini-cli/releases/tag/v0.59.0), [revision](https://github.com/google-gemini/gemini-cli/commit/fb0d535af931b27c51e87e5e6ade72905b1e8390), [package](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/package.json), [installation: Recommended system specifications](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/get-started/installation.mdx).
- **[E]** [Entrypoint: main, ACP/UI dispatch](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/gemini.tsx#L758-L805); [configuration: arguments, loadCliConfig](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/config/config.ts).
- **[C]** [CLI commands/options](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/cli/cli-reference.md).
- **[K]** [Keyboard: Basic Controls, Text Input, Limitations](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/reference/keyboard-shortcuts.md).
- **[U]** [useGeminiStream: cancelOngoingRequest, useKeypress, submitQuery](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/ui/hooks/useGeminiStream.ts).
- **[H]** [Hooks reference: schemas/event contracts](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/hooks/reference.md); [overview: Configuration, Security](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/hooks/index.md).
- **[G]** [Configuration: hooksConfig](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/reference/configuration.md#hooksconfig).
- **[B]** [Hook payload construction](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/core/src/hooks/hookEventHandler.ts); [runner: executeCommandHook, convertPlainTextToHookOutput](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/core/src/hooks/hookRunner.ts); [client: hook state, sendMessageStream](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/core/src/core/client.ts).
- **[Q]** [Confirmation: resolveConfirmation, notifyHooks](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/core/src/scheduler/confirmation.ts#L153-L175).
- **[J]** [Headless reference](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/cli/headless.md); [event types](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/core/src/output/types.ts); [runNonInteractive](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/nonInteractiveCli.ts).
- **[A]** [ACP guide](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/cli/acp-mode.md); [dispatcher](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/acp/acpRpcDispatcher.ts); [session: prompt/cancel/runTool](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/acp/acpSession.ts); [manager: newSession/loadSession](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/acp/acpSessionManager.ts).
- **[P]** ACP specification, live checked: [Prompt Turn: lifecycle/cancellation](https://agentclientprotocol.com/protocol/prompt-turn.md); [Session Setup: capability negotiation/loading](https://agentclientprotocol.com/protocol/session-setup.md).
- **[M]** [MCP: architecture, trust, prompts as slash commands](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/tools/mcp-server.md).
- **[I]** [IDE companion: communication, context, diffing](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/ide-integration/ide-companion-spec.md).
- **[T]** [Session management: saving/resuming](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/cli/session-management.md); [recording service: loadConversationRecord, initialize, appendRecord, rewindTo](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/core/src/services/chatRecordingService.ts).
- **[O]** [Telemetry: Configuration, Observability reference/Logs](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/docs/cli/telemetry.md).
- **[D]** [DevTools service: setupInitialActivityLogger](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/cli/src/utils/devtoolsService.ts); [server: routes/WebSocket handling](https://github.com/google-gemini/gemini-cli/blob/v0.59.0/packages/devtools/src/index.ts).
