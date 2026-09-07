# Run Ledger

The Run Ledger product surface as a resident v2 plugin: **what ran here** —
every command the terminal reports, its outcome, and a jump back to its
output. The core only emits facts (`run_started` / `run_finished` /
`pane_closed` / `pane_focused`); this plugin owns the ledger, persistence,
and all of the UI that used to live in `sleipnir_ui`.

## What it does

- **Status strip** (titlebar band, ≤ 24 cells): one summary badge — `✗N`
  (Err tone) while failed runs wait unseen, else `●N` (Accent) while runs
  are in flight — plus two buttons. The buttons are also extracted as
  command palette entries: **Run Ledger** opens the panel, **Clear** wipes
  the ledger and deletes `runs.json`. The badge doubles as a tab chip.
- **Panel** (a split, opened via the palette command or the strip button):
  runs grouped 进行中 / 待看 / 今天 / 更早, each row showing state icon,
  command and duration — the same grouping and wording the built-in overlay
  used. Jumpable rows (current launch, not Abandoned) are buttons that send
  the host `scroll_to_run` and mark the run seen. Inferred runs (busy-probe
  guesses, no scrollback anchor) keep their button; the host degrades the
  jump to a pane focus. Rows are capped (200) so the widget tree stays far
  under the 500-node budget.
- **Persistence**: the same `~/.config/sleipnir/runs.json` the core used to
  write (format version 1, unchanged), so existing history carries over.
  History loads as already-seen; a run left `Running` at shutdown comes back
  `Abandoned`. Saves happen after `run_finished` / `pane_closed` / Clear,
  under the store's cross-process lock with merge-by-id, so two Sleipnir
  windows cannot drop each other's runs.
- **Focus handling**: focusing a pane marks all of its runs seen, and a run
  that finishes in the focused pane never raises Attention.

## Build & install

```sh
cargo build --release -p sleipnir_plugin_runledger

mkdir -p ~/.config/sleipnir/plugins/runledger
cp target/release/sleipnir-plugin-runledger ~/.config/sleipnir/plugins/runledger/
cp crates/sleipnir_plugin_runledger/plugin.json ~/.config/sleipnir/plugins/runledger/
```

Plugins are off by default; enable them in `~/.config/sleipnir/settings.json`:

```json
{ "plugins": { "enabled": true } }
```

Start Sleipnir and approve the consent prompt (`resident`,
`subscribe_events`, `render_panel`, `render_status`,
`host_call_scroll_to_run`). Consent is asked once per binary + permission
set; a rebuilt binary or a widened permission list re-prompts.

## Protocol capabilities used

| Capability | Why |
| --- | --- |
| `resident` | holds the ledger and the id map across events |
| `subscribe_events` | narrowed to `run_started`, `run_finished`, `pane_closed`, `pane_focused` |
| `render_status` | summary badge + palette-contributing buttons |
| `render_panel` | the grouped run list in a split |
| `host_call_scroll_to_run` | jump a pane's scrollback back to a run's output |

## Layout

| File | Responsibility |
| --- | --- |
| `src/rows.rs` | Grouping and row formatting, ported from the deleted core overlay. Pure. |
| `src/state.rs` | The ledger, the local↔host run-id map, and `runs.json` persistence. |
| `src/view.rs` | State → Status strip / Panel widget trees. Pure. |
| `src/main.rs` | The resident session: events, actions, panel identity. Thin. |
