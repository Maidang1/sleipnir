# Current settings and compatibility

Settings live in `~/.config/sleipnir/settings.json` on macOS/Linux and
`%APPDATA%\sleipnir\settings.json` on Windows. See
[settings.example.json](settings.example.json) for an example. Use Reload
Settings to re-read the file; user key-binding changes currently require an
application restart.

## Windows and appearance

- Each new window starts with a fresh terminal tab. Window layouts, running
  processes, and scrollback are not restored on restart.
- The top tab strip is the only tab placement. Pixel geometry is the only UI
  style; colors still follow the chosen theme.
- `restore_session`, `tab_placement`, and `ui_style` are removed and ignored.
- `show_tombstone` remains accepted for compatibility but has no effect:
  there is no restored-session banner.
- `terminal.starfield` (default `false`) draws sparse, subtle stars behind
  terminal text, including over application-supplied background colors.
  Enable **Settings > General > Starfield**, or set `"starfield": true` inside
  the `terminal` object and reload settings. The UI toggle saves immediately
  and refreshes all windows. Stars use the theme's foreground color and keep
  stable anchors within each pane while scrolling or resizing. Each star gently
  brightens, dims, and moves 0.8-2.4 pixels around its anchor with independent
  5-9 second brightness cycles and 7-13 second position cycles. Animation
  requests at most 20 refreshes per second per visible pane, stops when the
  window is inactive or the pane is hidden, and stays static when GPUI's
  reduced-motion preference is enabled. Selection, search highlights, and the
  cursor stay above them.

## Command facts and persistent history

`run_ledger` controls the core's in-memory command facts, used by failed-tab
attention, the Dock badge, plugin run events/anchors, and `sleipnir-ctl wait`:

| Value | Core behavior |
| --- | --- |
| `off` | Do not collect command facts |
| `memory` | Collect command facts in memory only |
| `persist` | Legacy alias with the same behavior as `memory` |

When the key is absent the default remains `persist` for compatibility; it
does **not** write history to disk. Newly generated configurations explicitly
use `memory` and omit the obsolete retention and tombstone keys.

The [optional Run Ledger plugin](../crates/sleipnir_plugin_runledger/README.md)
owns the panel and `runs.json`. It must be built, installed, enabled, and
approved separately; it is not bundled into the app release. Keep core command
collection enabled if you want the plugin to receive run events.

`run_ledger_max_runs` is still parsed for compatibility but no longer controls
core or plugin retention. `run_ledger_retention_days` was removed and is
ignored. Do not rely on either key to limit persistent history.

`run_ledger_redact` controls capture-time command redaction. Redaction is a
heuristic, not a guarantee that every secret will be removed.

## Optional local extensions

- `plugins.enabled` defaults to `false`. Enabled plugins run as local,
  **unsandboxed** child processes. Host capabilities restrict the RPC, not the
  process's own filesystem or network access. Install only code you trust.
- `control_surface` defaults to `false` and is supported on Unix only. Either
  that setting or `SLEIPNIR_CONTROL=1` enables the local socket.

See [plugins.md](plugins.md) and [ADR-0011](adr/0011-control-surface.md).
