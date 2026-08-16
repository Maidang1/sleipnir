# No built-in AI: Sleipnir is the ground an agent runs on

**Status:** accepted

## Context

Sleipnir's primary user is **a person who runs coding agents in a terminal** —
the human is the user, the agent is their workload. That makes AI-adjacent
feature requests inevitable, and "we don't do AI" is too vague to decide them:
does piping selected text to a user-configured CLI count? Does an external
program driving a pane count?

Without a line drawn here, every request re-litigates the product boundary.

## Decision

**Sleipnir ships no model calls, no chat UI, and no API-key management.** It is
the ground an agent runs on, not the agent.

Allowed (these serve the human running the agent):

- **Piping to user-configured external commands** — e.g. a binding that sends the
  selection to whatever CLI the user names. Sleipnir supplies the plumbing; the
  user supplies the program. No model, no key, no network in our code.
- **Structured shell/output semantics** — OSC 133 prompt/command boundaries,
  exit-status awareness, notifications. This is what makes long agent runs legible.
- **Being driven from outside** — a local control surface (enumerate panes, read a
  pane's visible screen, inject keys) so external tooling can drive the terminal.
  Must be **default-off, explicitly opted in, and covered by its own ADR** before
  any of it lands: it is an attack surface, not just a feature.

Forbidden:

- Model/inference calls from Sleipnir's own process.
- A chat panel, inline completion UI, or "explain this error" surface that
  Sleipnir itself powers.
- API key storage, provider settings, or account/billing surface.

Stated for users, in these words:

> Sleipnir has no built-in model calls, chat UI, or API-key management. It is the
> terminal your agent runs *in*, not an agent. If you want to talk to an AI inside
> your terminal, use Warp or Wave.

## Consequences

- **We give up a growing user segment** — people who want AI *in* the terminal.
  That is the price of the boundary, and it is accepted deliberately, not by
  omission.
- Performance work stays aimed at the agent-as-workload profile: redraw-heavy
  small-write streams, off-screen panes costing nothing, long-run legibility
  (see `scripts/bench/README.md` §5).
- Feature requests are triaged against the allow/forbid lists above rather than
  against taste. A request that needs a model call in our process is closed by
  this ADR; a request that needs better structured output is in scope.
- The complementary boundary — how much of the terminal an *external* agent may
  drive — is deliberately deferred, and stays forbidden by default until the
  control-surface ADR exists.
