# Vendored alacritty_terminal to patch OSC dispatch

**Status:** accepted

## Context

Sleipnir's terminal model is `alacritty_terminal` (zed's fork). The competitive
research and roadmap identified shell-integration OSC support (OSC 133 markers,
OSC 9 / OSC 777 notifications) as a top gap, but the upstream fork **does not
handle those OSC codes at all**: its `vte::ansi::Handler` impl for `Term` never
overrides `osc_dispatch`, so 133/9/777 are silently dropped. The existing
`osc133.rs` / `osc_notify.rs` scanners in `crates/terminal` are only fed by the
display-only `write_output` path; the real PTY path (alacritty's `EventLoop`)
never exposes the raw bytes.

## Decision

Vendor `alacritty_terminal` into `vendor/alacritty_terminal` (a snapshot of the
zed fork at `4c129667ce56611becdc82de6e28218c80e2e88f`, Apache-2.0) and switch
root `Cargo.toml` to `path = "vendor/alacritty_terminal"`, so we can patch the
term's `osc_dispatch` to recognize 133/9/777 and emit events through the
existing `EventProxy` (→ `ZedListener` → `PtyEvent` → our scanners/handlers).

Alternatives considered and rejected:

- **Fork on GitHub + alternate `git` URL** — no fork access/URL available to point
  Cargo at.
- **`EventedPty` tee wrapper** — reading the byte stream before vte requires
  re-implementing `EventedReadWrite` with self-referential reader state; more
  complex and riskier than vendoring.
- **Stay unpatched** — leaves jump-prompt broken for real shells and blocks
  click-to-move-cursor, triple-click-select-output, auto-inject, and OSC 9/777.

## Consequences

- Bumping alacritty means re-copying upstream source over
  `vendor/alacritty_terminal/src` and re-applying the patch (keep the vendored
  `Cargo.toml`). See `UPSTREAM.md`.
- The vendored crate stays Apache-2.0; our patch is minimal and localized to the
  `Handler for Term` impl + event enum.
- `vte` is also vendored (`vendor/vte`, 0.15.0) and forced via
  `[patch.crates-io] vte = { path = "vendor/vte" }` so `Handler::osc_custom`
  is visible to `alacritty_terminal`. Bump both together.
- `scripts/upstream-diff.sh` no longer covers `alacritty_terminal` (it is not a
  git dep anymore); track our diff manually.
