# Current settings and compatibility

Settings live in `~/.config/sleipnir/settings.json` on macOS/Linux and
`%APPDATA%\sleipnir\settings.json` on Windows. See
[settings.example.json](settings.example.json) for an example. Use Reload
Settings to re-read the file; user key-binding changes currently require an
application restart.

## Interface language

`language` selects Sleipnir's own interface language: `"en"` (default) or
`"zh_cn"` (Simplified Chinese). The settings panel and command palette update
immediately; native application menus update after restarting Sleipnir. Terminal
programs keep their own locale and are not affected.

Additional interface languages can be added without changing UI call sites; see
[the locale catalog guide](../crates/sleipnir_settings/locales/README.md).

## Windows and appearance

- Each new window starts with a fresh terminal tab. Window layouts, running
  processes, and scrollback are not restored on restart.
- The top tab strip is the only tab placement. Pixel geometry is the only UI
  style; colors still follow the chosen theme.
- `restore_session`, `tab_placement`, and `ui_style` are removed and ignored.
- `show_tombstone` remains accepted for compatibility but has no effect:
  there is no restored-session banner.

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

## Remote SSH

The `ssh` section configures remote connections. Open the host picker with
**Shell → Connect to Host** (⌘⇧H / Ctrl+Shift+H), the command palette, or
`"action": "connect_host"` in a user key binding. Selecting a host opens a new
tab running `ssh` as an ordinary child process, so every ssh feature — agent
forwarding, jump hosts, port forwarding, `ControlMaster` — keeps working exactly
as it does on the command line.

| Key | Default | Behavior |
| --- | --- | --- |
| `ssh.program` | `"ssh"` | Program used to open a connection. Blank falls back to `ssh` from `PATH` |
| `ssh.read_ssh_config` | `true` | Merge hosts from `~/.ssh/config` into the picker |
| `ssh.askpass` | `true` | Collect passwords/passphrases in a Sleipnir prompt (Unix only) |
| `ssh.hosts` | `[]` | Hosts configured inside Sleipnir |

Each `ssh.hosts` entry takes `alias` (connection name, also the tab title),
`host`, and optionally `user`, `port`, `identity_file`, `proxy_jump`, and
`extra_args`. Application hosts win when their `alias` collides with a
`~/.ssh/config` entry, so an explicit entry always shadows the ssh config.

```json
"ssh": {
  "hosts": [
    {
      "alias": "prod-01",
      "host": "10.0.0.5",
      "user": "deploy",
      "port": 2222,
      "identity_file": "~/.ssh/id_ed25519",
      "proxy_jump": "bastion",
      "extra_args": ["-o", "ServerAliveInterval=30"]
    }
  ]
}
```

Notes:

- Hosts that come from `~/.ssh/config` are launched as `ssh <alias>`, so ssh
  re-reads your configuration itself. Application hosts get explicit `-p`, `-l`,
  `-i`, and `-J` options, and the destination is always the **last** argument so
  `extra_args` can never be run as a remote command.
- With `ssh.askpass` enabled, Sleipnir writes a small helper script next to its
  config and sets `SSH_ASKPASS` / `SSH_ASKPASS_REQUIRE=force` for the child, so
  passwords are typed into a Sleipnir modal instead of the raw terminal. This
  needs OpenSSH 8.4 or newer; on older builds, or on Windows, type the password
  in the terminal as usual. Secrets are never written to disk or logged.
- Remote shells do not receive the OSC 133 shell-integration script, so the Run
  Ledger does not track individual commands inside an SSH session.
- Editing `~/.ssh/config` is picked up the next time the picker opens.

## Optional local extensions

- `plugins.enabled` defaults to `false`. Enabled plugins run as local,
  **unsandboxed** child processes. Host capabilities restrict the RPC, not the
  process's own filesystem or network access. Install only code you trust.
- `control_surface` defaults to `false` and is supported on Unix only. Either
  that setting or `SLEIPNIR_CONTROL=1` enables the local socket.

See [plugins.md](plugins.md) for plugin and control-surface behavior.
