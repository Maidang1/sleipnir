# Default-off control surface

**Status:** accepted

## Context

[ADR-0008](0008-no-builtin-ai.md) allows Sleipnir to be driven from outside —
enumerate panes, read a pane's visible screen, inject keys — and forbids that
surface by default until this ADR exists. It is an attack surface, not just a
feature. External tooling still needs a way to drive a pane and wait for a Run
to settle without Sleipnir becoming an agent.

## Decision

Sleipnir may be driven from outside through a local control surface. The
surface is **default off**. A socket is never created, bound, or accepted
unless the user explicitly enables it via settings `control_surface: true` or
env `SLEIPNIR_CONTROL=1`. A missing key is off.

No model calls. This crate and its listener do not interpret screen contents,
do not call a model, and do not manage API keys
([ADR-0008](0008-no-builtin-ai.md)).

Socket path: `$SLEIPNIR_CONTROL_SOCKET` if set and non-empty, otherwise
`~/.config/sleipnir/control.sock`.

Protocol: one JSON object per line (request, then response).

Verbs:

- `ls` — enumerate panes
- `capture` — read a pane's visible screen
- `send` — inject keys/text
- `wait` — block until a Run Ledger state

`wait` uses Run Ledger states `free` | `failed` | `attention` — not agent
hooks or process-name guesses. `free` is the pane not busy; `failed` and
`attention` are ledger Attention.

## Consequences

- Disabled means no listener, no socket file, no accept loop. The bind is the
  gate.
- Protocol types and the `wait` predicate live in `sleipnir_ctl`. The App binds
  the socket only when the surface is enabled and serves `ls` / `send` /
  `wait` / `capture` against live panes.
