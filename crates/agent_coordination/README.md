# agent_coordination

Local JSON-lines protocol, in-memory registry, and Unix socket server for
an existing **Coordinator agent** to discover, launch, prompt, wait on,
interrupt, focus, and close **Worker** agents that each occupy a visible
Sleipnir pane.

`sleipnir-agentctl` speaks the dialect below to this server. Requests
acknowledge **acceptance** only. This crate does not call a model, spawn a
shell, drive a PTY, or consume adapter effects — execution awaits an
adapter wired into the Agents plugin/host.

Windows builds compile; `Server::bind` returns a clear unsupported error.

Sleipnir remains the ground an agent runs on ([ADR-0008](../../docs/adr/0008-no-builtin-ai.md)):
no built-in model calls, no approval automation.

## Two queues

Requests acknowledge **acceptance**, never execution (`launch_accepted`,
`prompt_accepted`, `interrupt_accepted`, `focus_accepted`, `close_accepted`).

| Queue | Consumer | Contents | Lifetime |
| --- | --- | --- | --- |
| **Effects** | adapter (single consumer) | `LaunchRequested` (kind/cwd/name/args), `PromptRequested` (text), `InterruptRequested`, `FocusRequested`, `CloseRequested` | durable until adapter ack / `DeliveryFailed` |
| **Facts** | coordinator clients / UI (per-cursor) | session/task/ownership observations | capped ring; overflow increments `facts_dropped` |

`Registry::peek_effects` does not consume. `Registry::facts_since(cursor)`
does not touch the effect log. `Registry::apply` never drains either queue.

Each effect has a monotonic `seq`. Peek is oldest-first. The adapter
acknowledges by **exact `seq`** (`BindPane { seq, session, pane }`,
`PromptDelivered { seq }`, `InterruptDelivered { seq }`,
`FocusDelivered { seq }`, `CloseDelivered { seq }`, or
`DeliveryFailed { seq }`). A stale delivery cannot ack a later queued
request. Kind mismatch and unknown seq are errors and leave the log
untouched.

If the effect log is full, a new request is **rejected** (`effects_rejected`);
payloads already queued are not dropped.

## Protocol

One JSON object per line. Tagged unions, `snake_case`, additive fields via
`#[serde(default)]`. `PROTOCOL_VERSION` is `1`.

Requests carry a correlation `id`. The matching response echoes it.

### Coordinator requests

| `op` | Fields | Response | Meaning |
| --- | --- | --- | --- |
| `list` | | `agents` | snapshots of known sessions |
| `launch` | `kind`, `cwd`, `name?`, `args?` | `launch_accepted` | queue `LaunchRequested`; `kind` is `codex` / `claude` / `gemini` / `opencode`; `cwd` must be absolute and NUL-free; `args` is argv |
| `prompt` | `session`, `text` | `prompt_accepted` | queue `PromptRequested` with the text |
| `wait` | `task` | `wait` | **immediate snapshot**: `status`, `terminal`, optional `result` / `detail`. Stop polling when `terminal` is true. The client owns any timeout. |
| `interrupt` | `session` | `interrupt_accepted` | queue `InterruptRequested`; in-flight tasks become `interrupting` — **not** settled until `InterruptDelivered` |
| `focus` | `session` | `focus_accepted` | queue `FocusRequested`; requires a bound pane |
| `inspect` | `session` | `inspect` | full snapshot including tasks (`result` / `detail` on each task) |
| `human_takeover` | `session` | `taken_over` | writer → human |
| `close` | `session` | `close_accepted` | queue `CloseRequested`; session stays open until the adapter confirms |
| `report_running` | `task` | `reported_running` | worker self-report: task is in flight. Same-user clients can spoof this. |
| `report_awaiting_human` | `task`, `detail?` | `reported_awaiting_human` | worker self-report: native agent waiting on a person; coordinator prompts stay refused. Not an approval answer. |
| `report_result` | `task`, `text` | `reported_result` | worker self-report: assignment no longer in flight. `text` is the payload, not a success flag. |
| `report_session_closed` | `session` | `reported_session_closed` | worker self-report: pane/process gone |

There is **no** `approve` / `deny` operation. There is **no** coordinator
`release_to_coordinator` — release is host/human-originated
(`AdapterUpdate::ReleaseToCoordinator`). Worker `report_*` ops are **not**
approval answers. Any process that can open the socket can spoof them.

Example:

```json
{"id":1,"op":"launch","kind":"codex","cwd":"/work/repo","name":"worker-1"}
{"id":1,"op":"launch_accepted","session":"…","task":"…"}
{"id":2,"op":"prompt","session":"…","text":"implement the tests"}
{"id":2,"op":"prompt_accepted","task":"…"}
{"id":3,"op":"wait","task":"…"}
{"id":3,"op":"wait","task":"…","status":"dispatching","terminal":false}
```

### Task status

`accepted` → `dispatching` → `running` ⇄ `awaiting_human` → `settled` | `unknown` | `failed_delivery`

`interrupting` is in-flight: an interrupt was accepted and is not yet
confirmed. The interrupt **request** never settles a task. When the adapter
acks the exact effect with `InterruptDelivered { seq }`, in-flight /
`interrupting` tasks on that session become `settled` (no longer tracked in
flight; not success). A stale or wrong seq is an error and must not settle a
later task. Interrupt `DeliveryFailed { seq }` remains `unknown`.

- `settled` — assignment no longer in flight (adapter recorded a result, or
  an interrupt was delivered). Not success.
- `unknown` — cannot prove more (pane/session closed mid-flight, or interrupt
  delivery failed).
- `failed_delivery` — adapter could not carry out a launch/prompt effect. Not success.
- `awaiting_human` — native agent waiting on a person. Coordinator prompts are refused.

`wait.terminal` is true for `settled` / `unknown` / `failed_delivery`.

Late adapter results after `unknown`/`failed_delivery` emit `late_result`
and do not resurrect the task. A second result after `settled` emits
`duplicate_result`.

### Ownership

Exactly one writer per open session: `coordinator` or `human`.

- Prompts, interrupts, and close require the coordinator.
- `human_takeover` is an explicit coordinator request.
- `release_to_coordinator` is **only** an adapter/host update (the human
  or host gives the lock back). The excluded writer cannot reclaim it
  over the coordinator wire.
- `focus` and `inspect` are visibility, not write ownership.

## Validation

- `cwd` — absolute, NUL-free, length-capped; oversize/relative is an error.
- `name` — optional; must start with a letter, then `[A-Za-z0-9_-]`, NUL-free.
- `args` — argv, count/length capped, NUL-free. Not a shell line.
- `prompt` text — non-blank after trim, NUL-free, no C0 controls except
  tab/newline/CR, length-capped.
- `BindPane { seq, session, pane }` — `seq` must name that session's
  `LaunchRequested` while unbound; first bind wins; same pane is
  idempotent (including an already-acked seq); a different pane is an
  error.

## Caps, pruning, overflow

- Open sessions: `max_sessions`. Closed sessions retained up to
  `max_closed_sessions`, then oldest closed (and their tasks) are pruned
  (`pruned_sessions` / `pruned_tasks`).
- Tasks: terminal tasks are evicted oldest-first so they cannot wedge
  `max_tasks`.
- Fact ring: drop-oldest with `facts_dropped`; readers use
  `facts_since(cursor)` and see `missed` when a gap opened.
- Effect log: reject new work when full (`effects_rejected`); never
  drop a queued payload. Closing a session drains **all** of its queued
  effects (`effects_drained`) so prompt/interrupt/focus/close cannot wedge
  the cap.

`Registry::stats()` exposes the counters.

## Server

`Server::bind(registry, path)` listens on a Unix socket (JSON-lines, one
request/response per line, `MAX_LINE_BYTES` = 64 KiB). Concurrent clients
share the `Registry`. Stop/drop unlinks the socket **only if the path still
names this listener's inode** — a live server is never hijacked, and dropping
an old `Server` will not unlink a replacement.

- Default path: `~/.config/sleipnir/agent-control.sock`
- Override: `SLEIPNIR_AGENT_CONTROL_SOCKET` or `sleipnir-agentctl --socket`
- Socket mode: `0600`. Parent `…/sleipnir/` is set to `0700` (platforms that
  ignore socket mode bits still need a private directory).
- Live path: connecting succeeds → `AlreadyRunning` (no unlink). Stale path
  (connect fails) is removed and reused.
- Accept errors other than `WouldBlock` are logged and retried with backoff.
- At most `MAX_CLIENTS` (16) handler threads; extras are dropped. Each
  connection has a 5s read/write timeout.
- No adapter consumer is started. Queued effects stay queued until the
  host applies `AdapterUpdate`s.

Diagnostic coordinator ops: `effects` (peek) and `facts` (cursor).

## Security

This surface is an attack surface, in the same family as the default-off
control socket ([ADR-0011](../../docs/adr/0011-control-surface.md)).

- **Local only.** User-private Unix socket, never a network listener.
  Bind refuses to steal a live path. Same-user access is `0600` on the
  socket and `0700` on `~/.config/sleipnir`.
- **Diagnostics expose payloads.** `effects` returns queued
  `PromptRequested` text. Any local process that can open the socket can
  read pending prompts (they may contain secrets). Restrict who can run
  as this user.
- **Not an OS sandbox.** A coordinator that can prompt a worker can type
  into that worker's pane once the adapter delivers the effect.
- **Worker self-reports are same-user trust.** `report_running`,
  `report_awaiting_human`, `report_result`, and `report_session_closed`
  are accepted from any client of this socket. They can be spoofed. They
  never carry a success boolean and never click a native approval UI.
- **No approval answers.** The protocol cannot click a worker's native
  "approve" UI. Coordinator prompts stay refused while a task is
  `awaiting_human`.
- **Caps and validation** as above. Oversize is an error, not truncation.
- **Argv, not a shell.**
- **Opaque ids.** Callers cannot mint a session by guessing a pane key.

## What this crate does not do

- Agent-specific adapters / consuming the effect log
- Host `FocusPane` / `SendText` / `SendKey` calls
- Model/provider APIs, credentials, network
- TCP or a Windows named-pipe listener

## Next wiring contract

The remaining blocker is an adapter consumer inside the Agents
plugin/host that shares this `Registry`:

- `LaunchRequested` → open a visible pane → `BindPane { seq, session, pane }`
- `PromptRequested` → host `SendText` if coordinator owns that session → `PromptDelivered { seq }`
- `InterruptRequested` → allowlisted interrupt key → `InterruptDelivered { seq }`
  (settles in-flight tasks on that session; failure → `Unknown`)
- `FocusRequested` → host `focus_pane` (owning window only) → `FocusDelivered { seq }`
- `CloseRequested` → close/quit the pane → `CloseDelivered { seq }` / `SessionClosed`
- delivery failure → `DeliveryFailed { seq }`

Observe pane/run facts → `TaskRunning` / `TaskAwaitingHuman` /
`TaskResult` / `SessionClosed`. Human returning the pane →
`ReleaseToCoordinator`.

**Never** map a coordinator prompt onto a native approval dialog.

`Registry` is `Clone` (`Arc<Mutex<_>>`) so the server, adapter, and UI
can share one instance.
