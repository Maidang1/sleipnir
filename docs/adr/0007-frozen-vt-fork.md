# The VT fork is frozen: no routine upstream syncs

**Status:** accepted (operational follow-up to [ADR-0005](0005-vendored-alacritty-term.md))

## Context

ADR-0005 pinned `alacritty_terminal` and `vte` to personal forks carrying the
OSC dispatch patch (OSC 133 / 9 / 777 + `Handler::osc_custom`). It recorded *why*
the fork exists but not *how* it is maintained. Two policies were possible:

- **Track upstream continuously** — rebase the patch on every upstream release.
  Buys upstream VT behaviour fixes for free; costs recurring rebase work on the
  parser, the most conformance-sensitive code in the project.
- **Freeze** — stay on the pinned revs, resolve conflicts by hand only when a
  sync is actually needed.

## Decision

**Freeze.** The pinned revs in `UPSTREAM.md` are the baseline; there is no
routine sync cadence. When a sync becomes necessary (a needed upstream fix, or a
Zed bump that forces an `alacritty_terminal` bump), conflicts are resolved by
hand at that moment.

Two obligations come with this, because the frozen code parses **untrusted
bytes**:

1. `UPSTREAM.md` carries a **Divergence** list — exactly what our forks add on
   top of the baseline. Rebasing must not depend on anyone's memory.
2. When bumping the Zed `rev`, scan the upstream `alacritty_terminal` / `vte`
   history for **crash / hang / unbounded-allocation fixes** in the parser and
   pull those specifically. Cosmetic and feature commits may be skipped
   indefinitely; a malicious-escape-sequence panic or OOM may not.

## Consequences

- Predictable maintenance: no recurring rebase tax on the parser.
- **We own VT conformance drift.** Upstream fixes to xterm-compat edge cases do
  not arrive on their own; a user-reported incompatibility is our bug to fix,
  not a "bump the dep" fix.
- **We own parser robustness.** Since we do not follow upstream, the crash/OOM
  scan at Zed-bump time is the only channel through which hardening fixes reach
  us. Skipping it silently converts this ADR into a security regression.
- Adding a *new* feature inside the fork (e.g. kitty graphics, which touches
  parser + renderer) makes any future rebase harder. Such a change needs its own
  ADR that answers "how does this patch get rebased?" before it lands.
