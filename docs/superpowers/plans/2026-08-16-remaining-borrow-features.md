# Remaining borrow-table features (one goal)

Ship every leftover item from the tty7 borrow table except the explicit non-goals
(daemon PTY, vendor logos, built-in chat, native SSH, full Git client,
persisted scrollback, agent resume/fork).

## Agents (Herdr, exclusive files)

| Agent | Owns |
|---|---|
| ledgerui | `crates/sleipnir_ui/src/run_ledger_panel.rs`, `crates/sleipnir_ui/src/chrome/tombstone.rs` |
| inputpipe | `crates/sleipnir_ui/src/chrome/send_context.rs` |
| ctladr | `docs/adr/0011-control-surface.md`, `crates/sleipnir_ctl/` |

Orchestrator owns shared wiring: settings, actions, `app_shell`, menus, keymap, badges, tray/dock, tmux preset, history overlay.

## Non-goals

Same as the original borrow table "不要进清单" rows.
