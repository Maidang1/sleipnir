# Divider drag resizes PTYs in real time

**Status:** accepted

Dragging the Divider between two Panes reflows both sides' PTYs continuously
during the drag (each side gets a live `SIGWINCH` / resize as the ratio changes),
rather than showing a preview line and resizing only on mouse-up. We chose this
because live reflow is what makes splits feel native and responsive; the
preview-line alternative is cheaper but feels laggy.

## Consequences

- Divider drag emits frequent resize events; the resize path (grid reflow +
  PTY winsize) must be cheap enough to run per-frame without visible flicker.
  If profiling shows this is too costly, revisit and fall back to
  resize-on-release.
