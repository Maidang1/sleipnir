# Fork-pin alacritty_terminal to patch OSC dispatch

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

Pin `alacritty_terminal` and `vte` to personal GitHub forks and keep the OSC
patch there — do **not** copy either tree into this repo.

| Crate | Fork | Baseline | Branch |
|-------|------|----------|--------|
| `alacritty_terminal` | [Maidang1/alacritty](https://github.com/Maidang1/alacritty) | zed-industries/alacritty `4c129667` | `sleipnir-osc-custom` |
| `vte` | [Maidang1/vte](https://github.com/Maidang1/vte) | alacritty/vte `v0.15.0` | `sleipnir-osc-custom` |

The vte fork adds `Handler::osc_custom` and forwards unhandled OSC to it. The
alacritty fork implements that hook on `Term` and emits `Event::Osc133` /
`Event::DesktopNotification` through the existing `EventProxy`
(→ `ZedListener` → `PtyEvent` → our scanners/handlers).

Root `Cargo.toml` pins both by `rev`. `[patch.crates-io] vte` forces
`alacritty_terminal`'s crates.io `vte 0.15.0` onto the same fork.

Alternatives considered and rejected:

- **Copy the trees into `vendor/`** — the patch is ~100 lines; committing two
  full crate snapshots makes every upstream bump a recopy + re-apply, and
  fights this repo's "pin GPUI, don't vendor" rule.
- **`EventedPty` tee wrapper** — `EventedReadWrite::reader()` returns
  `&mut Self::Reader` with no lifetime, so a tee reader over the inner PTY
  becomes self-referential. More complex than a two-repo pin.
- **Stay unpatched** — leaves jump-prompt broken for real shells and blocks
  click-to-move-cursor, triple-click-select-output, auto-inject, and OSC 9/777.

## Consequences

- Bumping means: merge the new upstream rev into `sleipnir-osc-custom` on each
  fork, re-apply the OSC patch if it conflicts, then update the `rev` values in
  root `Cargo.toml`. Record both pins in `UPSTREAM.md`.
- The forks stay on their upstream licenses (alacritty_terminal Apache-2.0; vte
  Apache-2.0 OR MIT). The patch is localized to `Handler for Term`, the event
  enum, and vte's unhandled-OSC arm.
- Bump `alacritty_terminal` and `vte` together: the Term impl requires
  `Handler::osc_custom`.
- `scripts/upstream-diff.sh` can compare Zed's alacritty pin against ours; it
  does not apply the OSC patch for you.
