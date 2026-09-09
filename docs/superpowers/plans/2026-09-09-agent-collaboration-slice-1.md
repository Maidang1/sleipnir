# Agent collaboration, slice 1: non-controlling Agents observer plugin

**Status:** implemented (2026-09-09). Scope deliberately bounded; the product
decisions listed below remain open.

## What this slice is

`crates/sleipnir_plugin_agents` — a resident v2 plugin (`plugin.json` id
`agents`) that observes the six facts the host already computes and renders a
status strip plus an on-demand panel: one row per pane running a known coding
agent, with a conservative status. Its one outward action beyond rendering is
a single OS notification per proven unseen session exit.

- Observes: `foreground_changed`, `run_started`, `run_finished`,
  `pane_focused`, `pane_closed`, `cwd_changed` (nothing else requested).
- Tracks the known agent kind per pane verbatim from `foreground_changed.agent`
  (the host owns the catalog; an unrecognized id is displayed as-is).
- Per-pane states are process/session status, never turn or task progress:
  `running` (the shell Run containing the agent process is still open —
  including while the agent waits at its prompt), `exited-unseen` /
  `exited-seen` (the latest agent session ended; implies nothing about
  success), `unknown` (agent foreground, no containing run pinned — also
  the stale/ambiguous case).
- Durable, honestly-sourced exit records: only a `run_finished` matching
  the agent's containing run proves an exit. `foreground_changed` with no
  agent is merely a loss of foreground detection — the host identity is the
  current foreground command, and a transient child/tool process can hold it
  while the agent stays alive — so `None` never creates `exited-*`; the
  record keeps its status (`running` stays `running`, `unknown` stays
  `unknown`) and the cached run is preserved so a later matching finish can
  still settle `running → exited-*`. A seen record never flips back; a
  re-detected same agent continues its record; a different agent (including
  the same agent restarted after a proven exit) replaces it; only
  `pane_closed` removes a row.
- Observation ordering: the host emits `cwd_changed` before
  `foreground_changed` on first sighting, and `run_started` can precede agent
  recognition by a full foreground-poll interval. Latest cwd and the open run
  are cached per pane independent of recognition and adopted when
  `foreground_changed` names the agent — including a new agent launched
  after a shell interlude. The adopted run is then **pinned** as the
  session's containing run: a later `run_started` in the pane never moves
  the pin (a nested OSC 133 run or busy-probe guess must not false-exit the
  session), and on identity replacement a cached run that belonged to the
  outgoing agent is never inherited. `pane_closed` clears record and cache;
  `foreground_changed` with no agent is a no-op for both record and cache.
- Status strip: `!N` exited-unseen (Warn — deliberately no checkmark/Ok
  tone: an exit is not a success) / `●N` running badge + **Agents** button
  (extracted as a palette entry). Panel: grouped rows captioned
  "process/session status only — not task progress"; only *exited* rows
  with a this-session run id are clickable, jumping via the existing safe
  `scroll_to_run` host call. Its anchor is the run's *start* — the completed
  output for an exited session, but a jump away from the live tail for an
  open run — so running/unknown rows are noninteractive text (no
  focus-by-pane host call exists, and adding one is a host protocol change
  this slice does not make). A click marks the row seen only on
  `HostCallResult::Ok`; denied/rate-limited/unknown-run navigation
  acknowledges nothing.
- Notifications (`host_call_notify`, implemented): exactly one OS
  notification when a tracked session's pinned containing run finishes
  while its pane is unfocused (`running → exited-unseen`). None on focused
  exits, `foreground_changed` losses, unknown/orphan finishes, mismatched,
  stale, duplicate, or shell-interlude finishes, or a denied call (never
  retried). Text names the agent and cwd basename and says only that the
  session exited and output is ready to review.

## Hard boundaries (chosen because later decisions are open)

- No automatic prompt injection, no approval answering. The four integration
  tickets (`.scratch/agent-collaboration/issues/01..04`) all found that
  same-visible-session control is either unproven or requires per-agent
  verification; nothing here writes to a PTY or answers an approval.
- No model/provider code, no network calls, no credentials (ADR-0008).
- No persistence: state is per-session, derived purely from live events.
- No host protocol changes: the plugin uses only existing capabilities.
- Never claims task percentage or approval-blocked status: the current
  protocol proves neither, and research notes warn that exit codes, silence,
  and hook events do not prove task success. Exit codes are not surfaced.

## Assumptions made

- `foreground_changed` with `agent: None` means only "the current foreground
  command is not a known agent" — it can be a transient child/tool process
  while the agent stays alive, so it never marks an exit and never drops or
  alters the row; deletion happens only on `pane_closed`.
- An exit (the containing run's `run_finished`) in the focused pane counts
  as seen (mirrors Run Ledger's focus rule); seen never reverts to unseen.
- `scroll_to_run` is used only for exited sessions: its scroll-to-start
  side effect lands on the completed output there, whereas on a still-open
  run it would pull the user away from the live output tail.

## Follow-up decisions (not made here)

- Adapter control transports per agent (Codex app-server + remote TUI,
  OpenCode server/SSE, Claude Code hooks, Gemini CLI hooks) — each needs the
  bounded runtime verification its ticket lists before any write path.
  Scope: the passive display recognizes the host's foreground catalog
  generically (any agent id the host reports is shown verbatim), but
  structured control adapters stay scoped to the four researched
  integrations — Codex, Claude Code, Gemini CLI, OpenCode.
- A focus-by-pane host call (or panel-row pane navigation) if noninteractive
  rows prove limiting.
- Richer state (approval-pending, turn completion) only when an adapter
  supplies protocol-proven facts; until then `unknown` stays the honest
  answer.
- Coordination contract, workspace isolation, and session scope: open tickets
  05–10 under `.scratch/agent-collaboration/issues/`.
