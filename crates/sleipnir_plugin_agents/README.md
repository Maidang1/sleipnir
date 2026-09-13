# Agents

A resident plugin for agent collaboration: conservative observer display
**plus** a generic coordination adapter. It watches the facts the host
already computes and answers one question conservatively: which panes run a
known coding agent (claude, codex, gemini, opencode, … — whatever the host's
foreground catalog reports), and what is the strongest claim the observed
events support about each?

This is not an agent runner. There is no approval answering, no model or
provider code, no network access, and no persistence. The human keeps
ultimate control of every agent in its own visible pane (ADR-0008): the
plugin watches and displays, posts a single OS notification per proven
unseen session exit, and — for sessions a coordinator explicitly launches —
drives that pane through the host's granted calls.

Scope note: the passive display recognizes whatever the host's foreground
catalog reports — generically, including agents outside the initial set.
The coordination adapter launches and writes only the four researched
integrations: Codex, Claude Code, Gemini CLI, and OpenCode.

## Coordination socket

With the **full** delivery grant set approved (`host_call_open_pane`,
`host_call_focus_pane`, `host_call_send_text`, `host_call_send_key`,
`host_call_request_close_pane`), the plugin serves the `agent_coordination`
JSON-lines protocol on a local Unix socket so a Coordinator agent (via
`sleipnir-agentctl`) can manage worker agents in visible panes:

- Path: `$SLEIPNIR_AGENT_CONTROL_SOCKET` if set, else
  `~/.config/sleipnir/agent-control.sock` (mode 0600). Unix only; on other
  platforms, a bind failure, or any missing delivery grant the plugin logs
  a clear stderr line (naming the missing grants) and runs observer-only —
  the socket is never partially up.
- The server starts once, after hello/grants, and lives for the plugin's
  lifetime. Requests are answered by the shared
  `agent_coordination::Registry`; the plugin's ~50ms tick delivers **at most
  one** pending effect per wake, oldest first, and every effect is acked
  with its exact `seq` (or `DeliveryFailed` on any refusal/host error). A
  host "rate limited" error is the one exception: the effect stays queued
  under the same seq and is retried with a 1s backoff, failing delivery only
  after 10 consecutive rate-limit failures. Note the host's limiter window
  (10 calls / 5s) makes the effective coordination throughput ~2
  effects/second, and a single host call may occupy a tick for up to its
  30s deadline while inbound host events queue (the SDK caps that queue at
  64) — keep coordinator bursts modest.
- Coordinator ops: `list`, `launch`, `prompt`, `wait`, `interrupt`,
  `focus`, `inspect`, `close`, `human_takeover`, plus `effects` / `facts`
  diagnostics. Responses acknowledge *acceptance*, never execution.
- `launch` maps the kind to a direct executable — `codex`, `claude`,
  `gemini`, `opencode` — spawned as argv via `host_call_open_pane` (never a
  shell line). The session's launch task settles only when the agent
  process is actually detected in the pane's foreground, with the neutral
  result `agent process detected`. If no matching agent appears within 15s
  (missing binary, instant exit, or a permanently mismatched catalog id),
  the coordination session closes — the launch task becomes `unknown`, a
  terminal non-success state — and the pane is left on screen, unmanaged;
  it is never auto-closed. A launch whose pane spawn fails closes its
  session immediately (`failed_delivery` + closed). A prompt is admitted
  only after detection.
- `prompt` types into the bound pane via `host_call_send_text`
  (bracketed-paste-aware, then Enter), only while the coordinator owns the
  session. The payload is an envelope: self-report instructions plus the
  original prompt, unmodified, between `----- task -----` markers — see
  [Worker reports](#worker-reports). **Generic limitation: a delivered
  prompt stays `running` — this adapter cannot observe turn completion.**
  A future native adapter (`TaskResult`), an interrupt delivery, a worker
  `report-result`, or the session closing is what moves it: interrupt
  *delivery* settles in-flight work (`settled` — no longer in flight,
  **not** success); interrupt failure and session close mark it `unknown`.
  Never success either way.
- `interrupt` sends the allowlisted `ctrl-c` via `host_call_send_key`;
  `focus` focuses the pane in its owning window via `host_call_focus_pane`;
  `close` asks the host to close the pane via `host_call_request_close_pane`.
  Interrupt and close re-check ownership at delivery time, exactly like
  prompt. Close follows the user-policy path: a busy pane may show a
  confirmation the user can cancel — `CloseAccepted`/delivery then only
  means the request reached the pane; the session stays open (check
  `inspect`) until the real `PaneClosed` event arrives.
- Ownership: exactly one writer per session. `human_takeover` blocks new
  prompts, interrupts, and closes at the registry, and any already-queued
  prompt/interrupt/close effect fails delivery rather than touching a
  human-owned pane. The human hands a session back with the panel's
  **Release** button on a human-owned managed row — a host-side action, not
  a coordinator-wire op.
- There is no approve/deny path. Nothing here ever answers a native agent
  approval prompt. A worker can self-report `awaiting_human` (a native
  dialog, a question) so the panel and strip can surface it; that report is
  coordination metadata, never an approval answer. A coordinator prompt
  sent while a native approval dialog is open is still typed as plain text
  — coordinators must not send answers.

## Worker reports

When the adapter delivers a coordinator `prompt`, it pastes an **envelope**
into the worker pane: self-report instructions, then the original prompt
unmodified between `----- task -----` / `----- end task -----` markers.
The envelope is UI text the host pastes; the plugin never executes it as a
shell command. An envelope that would exceed the host's 8192-character
`send_text` cap is rejected before the host call (`failed_delivery`) — the
user text is never truncated.

Workers self-report over the coordination socket with `sleipnir-agentctl`.
When `$SLEIPNIR_AGENT_CONTROL_SOCKET` is set, the envelope includes a
safely quoted `--socket '<path>'` argument; otherwise the bare command is
enough (it already uses the same default path as the plugin's server).

- `report-running` is optional — prompt delivery already marks the task
  `running`.
- `report-awaiting-human <task> …` when blocked on a person.
- `report-result <task> --stdin` when the response or artifact is ready
  (pipe the result on stdin).
- `report-session-closed <session>` only when actually exiting.

These reports are coordination metadata. They never answer a native
approval prompt; approvals stay with the human watching the pane.

## Status model

Every status is **process/session status, never turn or task progress**. Per
agent pane, exactly one of four states — the most conservative one the
events prove:

| State | Meaning | Evidence |
| --- | --- | --- |
| `Running` | the shell Run containing the agent process is still open | `run_started` seen without its `run_finished` |
| `Exited — unseen` | the latest agent session in the pane ended; the pane has not been focused since | `run_finished` matching the containing run |
| `Exited — seen` | the latest agent session ended and the human has looked at the pane since | as above, but the pane was focused at the time or since |
| `Unknown` | an agent is foreground but no containing run is pinned | `foreground_changed` only |

Two consequences of the evidence, stated plainly:

- **`Running` does not mean the agent is computing.** An interactive agent's
  launch Run stays open the whole session, including while the agent waits
  at its prompt for input. It means only "the agent process's containing
  Run is active".
- **"Exited" implies nothing about success.** Interrupts, denied turns, and
  crashes also end sessions, and a shell exit code is not an agent task
  outcome — so exit codes are not surfaced either.

The containing run is **pinned**: whichever run was open when the session
became `Running` (recognition adoption or the first `run_started` while
tracked) stays the session's evidence. The host ledger abandons a pane's
previous run on a new start without delivering a finish for it, so a later
`run_started` — a nested OSC 133 run, a busy-probe guess — never moves the
pin, and its finish never reads as the session exit. An `Unknown` pane has
no pinned run, so no `run_finished` can prove an exit there: an arbitrary
finish leaves it `Unknown`. And on identity replacement (a different agent,
or the same agent restarted), a cached run that belonged to the outgoing
record is never inherited — the fresh session starts `Unknown` until a
`run_started` that postdates the switch.

Deliberate absences:

- **No task percentage, no turn progress, no readiness, no "waiting for
  approval" state.** The current host protocol carries no such fact, so the
  plugin never shows one.
- **`Unknown` instead of guessing.** A turn could have been in flight before
  the plugin launched; quiet does not prove idle.
- **Only a finished Run proves an exit.** `foreground_changed` with no agent
  does **not** mean the session ended: the host identity is only the current
  foreground command, and a transient child/tool process can briefly hold
  the foreground while the agent stays alive. A `None` therefore changes
  nothing — the record keeps its status and its cached run, so the matching
  `run_finished` settles `Running → Exited` whenever it arrives, and a
  re-detected same agent simply continues its record. A record never flips
  from seen back to unseen; it leaves the panel only when a different agent
  takes the foreground (a fresh session replaces it) or the pane closes.

### Observation ordering

The host emits `cwd_changed` before `foreground_changed` on first
observation, and a `run_started` can arrive up to a full foreground-poll
interval before the agent is recognized. The plugin therefore caches the
latest cwd and the currently open run **per pane, independent of agent
recognition**: when `foreground_changed` names an agent, the record adopts
the cached cwd and active run, so an agent whose launch command was already
observed starts as `Running`, not `Unknown`. `pane_closed` clears the cache
with the record. `foreground_changed` with no agent clears nothing — it is
only a loss of foreground detection, possibly transient — so the cached run
survives until its matching `run_finished`; a run started during the
detection gap — typically the next agent's own launch command, after any
shell interlude — replaces the cached run and is adopted normally.

## What it renders

- **Status strip** (titlebar band, ≤ 24 cells): one summary badge — `!N`
  (Warn tone) while exited sessions wait unseen, else `◐N` (Warn) while
  managed sessions await a human, else `●N` (Accent) while agent processes
  have an open Run — plus one button, **Agents**, which is also extracted as
  a command palette entry and opens the panel. The badge is deliberately not
  a checkmark and not Ok tone: an exit is not a success.
- **Panel** (a split, opened via the palette command or the strip button):
  rows grouped Running / Exited — unseen / Exited — seen / Unknown, each
  showing status icon, agent id, and cwd basename, under a caption stating
  this is process/session status, not task progress. "Running"/"Unknown"
  describe the current agent process; the "Exited" groups describe the
  latest session. Below the observer groups, a **Managed sessions** section
  lists every coordination session from the registry — open *and* closed:
  kind, name (or short session id), writer (Coordinator/Human),
  bound/unbound/closed, and the latest task's status (`dispatching` /
  `running` / `awaiting human` / `interrupting` / `settled` / `unknown` /
  `failed delivery` — never a success claim). An awaiting-human row also
  shows the worker's self-reported note (flattened and clipped); a row
  with a recorded result shows a flattened, clipped excerpt marked
  `· result: …` — never the full payload, never a success claim.
  `settled` means no longer in flight, not that the work succeeded.
  Both sections share one 440-node row budget (observer rows cost ≤3,
  managed rows ≤6), keeping the whole tree under the 500-node cap with a
  single truncation note.
- **Managed-session actions** (open sessions only; closed rows are dim
  text): **Focus** on bound rows; **Take over** on coordinator-owned rows
  (submits `human_takeover`, then queues a focus); **Release** on
  human-owned rows; **Interrupt** on coordinator-owned rows whose latest
  task is in flight; **Close** on coordinator-owned rows. Every action —
  focus included — is submitted as a registry *request*, never a direct
  host call, so ownership gates and the effect log stay authoritative and
  delivery happens on the next tick. There is no prompt text input in the
  panel — the widget schema has none — so prompts keep going through
  `sleipnir-agentctl`, and there is no approval-answer action anywhere.
- **Clickable observer rows**: only *exited* rows (seen or unseen) with a
  run observed this session render as buttons, calling the host's existing
  `scroll_to_run` — the same safe navigation the Run Ledger uses. The
  conservative choice is deliberate: `scroll_to_run` jumps to the run's
  *start* anchor, which for an exited session is exactly the completed
  output the user asked to review, but on a still-open run would yank the
  user away from the live output tail. Running and Unknown rows are plain
  text — noninteractive by choice, not because an API is missing. A row is
  marked seen only when the host confirms the jump with
  `HostCallResult::Ok`; a denied, rate-limited, or unknown-run click
  acknowledges nothing.
- **Notifications**: exactly one OS notification when a tracked session's
  containing run finishes while its pane is unfocused
  (`Running → Exited — unseen`). No notification when the pane is focused,
  on `foreground_changed` losing the agent, on Unknown/orphan finishes, on
  mismatched, stale, duplicate, or shell-interlude finishes, or when the
  host denies the call (a denial changes nothing and is not retried). The
  text names the agent and the cwd basename when known and says only that
  the session exited and output is ready to review — never success, never
  task completion, never approval state.

## Built in; enabled by default

Agents is compiled into `sleipnir` and automatically started as an isolated
child process (`sleipnir --builtin-agents`). A normal `cargo build -p sleipnir`
or packaged release includes it: no extra binary, plugin directory, manifest
copy, settings edit, or first-run plugin consent is required. Open
**Agents: Open panel** from the command palette to see the panel; opening it
is not required for the coordination service to run.

The host supplies only the capability set from the compiled-in manifest.
Host-side permission checks, request validation, rate limits, coordinator /
human ownership, and native agent approval boundaries still apply. Built-in
trust comes from host-owned provenance, never a user manifest or a matching
plugin id. The `agents` id is reserved; an old external installation is ignored.
The Plugin Monitor labels the service **built-in** and can stop it.

External plugins remain off by default. `plugins.enabled` controls only
external discovery. To explicitly disable the built-in service, merge this
into `settings.json` and reload settings (or restart):

```json
{ "plugins": { "builtin_agents": false } }
```

The socket is local and user-private, but any process running as the same
OS user can connect and issue coordinator requests. It is not an OS sandbox
or a model service. Windows currently runs observer-only because the
coordination transport is Unix-only. Socket bind failures also degrade to
observer-only and appear in the Plugin Monitor's stderr log.

## Built-in client

No separate `sleipnir-agentctl` installation is needed:

```sh
sleipnir agentctl list
sleipnir agentctl launch-wait codex /absolute/repo --name worker --timeout-ms 20000
```

Sleipnir panes export `SLEIPNIR_BIN`, so GUI launches work even when the app
is not on PATH:

```sh
"$SLEIPNIR_BIN" agentctl list
```

Worker prompt envelopes use the shell-quoted absolute terminal executable
with the `agentctl` subcommand. External agent CLIs (`codex`, `claude`,
`gemini`, `opencode`) must still be installed separately. The standalone
`sleipnir-plugin-agents` and `sleipnir-agentctl` binaries remain buildable for
SDK testing and tooling; normal terminal installation needs neither.

## Protocol capabilities used

| Capability | Why |
| --- | --- |
| `resident` | holds the per-pane state across events |
| `subscribe_events` | narrowed to `foreground_changed`, `run_started`, `run_finished`, `pane_focused`, `pane_closed`, `cwd_changed` |
| `render_status` | summary badge + palette-contributing button |
| `render_panel` | the grouped agent list in a split |
| `host_call_scroll_to_run` | jump to an exited session's output in an agent pane |
| `host_call_notify` | one OS notification when a tracked session exits unseen |
| `host_call_open_pane` | launch a worker agent as argv in a new visible pane |
| `host_call_focus_pane` | focus a managed pane in its owning window |
| `host_call_send_text` | deliver a coordinator prompt into a bound pane |
| `host_call_send_key` | deliver the allowlisted interrupt key (`ctrl-c`) |
| `host_call_request_close_pane` | request pane close via the user-policy path |

## Layout

| File | Responsibility |
| --- | --- |
| `src/state.rs` | The per-pane observer state machine over the six events. Pure. |
| `src/view.rs` | State → Status strip / Panel widget trees. Pure. |
| `src/adapter.rs` | The generic coordination adapter: effect-log delivery, session↔pane map, ownership gates. Host contact only via the `HostCalls` trait. |
| `src/main.rs` | The resident session: events, actions, panel identity, socket lifecycle, tick. Thin. |

Follow-ups (adapter control transports, richer state from agent-native
hooks) are tracked in
`docs/superpowers/plans/2026-09-09-agent-collaboration-slice-1.md`.
