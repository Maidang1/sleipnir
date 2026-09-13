<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir app icon" width="160" height="160" />

# Sleipnir

A fast, native terminal emulator for macOS, Windows, and Linux.

GPU-rendered, tab- and split-aware, with multi-window sessions and smooth scrollback.

[Features](#features) · [Install](#install) · [Build](#build-from-source) · [Config](#configuration)

</div>

---

Sleipnir is a standalone terminal built on [GPUI](https://gpui.rs), with a forked terminal backend for native PTY/ConPTY behavior. It focuses on responsiveness, layout flexibility, and a terminal workflow that feels like a first-class app instead of a shell wrapper.

## Features

- GPU-rendered terminal with smooth redraw and scrollback
- Tabs, splits, and multi-window sessions
- Native PTY/ConPTY support; every new window starts with a fresh tab
- Smart paste, path links, and system-aware themes
- Search in scrollback, diff inspection, and in-memory command status tracking
- Zed-compatible `terminal.*` settings and hot reload
- Built-in Agents panel and local worker coordination, enabled out of the box
- Optional external process-based plugins for panels, inline blocks, and command-palette actions (off by default)

Window layouts and terminal scrollback are not restored after restarting.
Persistent command history and the Run Ledger panel are provided by the
[optional Run Ledger plugin](crates/sleipnir_plugin_runledger/README.md), which
you must install and enable separately.

## Install

### macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

### Windows

Download the latest `Sleipnir-<ver>-windows-x64.exe` (portable binary) or `Sleipnir-<ver>-windows-x64.zip` (portable archive) from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases), then run it.

Windows builds are currently not code-signed, so SmartScreen may warn on first launch — click **More info → Run anyway** to proceed.

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

Linux releases include `.deb` packages and portable tarballs for x86_64 and ARM64.

Updates work differently per platform: on macOS, **Check for Updates** upgrades the app in place; on Windows and Linux it opens the Releases page so you can download the latest build manually.

## Build from source

```bash
cargo run -p sleipnir
```

To build a release binary:

```bash
cargo build --release -p sleipnir
```

## Configuration

Settings live in:

- macOS / Linux: `~/.config/sleipnir/settings.json`
- Windows: `%APPDATA%\sleipnir\settings.json`

Common options include:

- `theme` / `custom_theme`
- `confirm_close`
- `key_bindings`
- `terminal.bell`
- `notify_on_command_finish_secs`
- `run_ledger`

See [`docs/settings.example.json`](docs/settings.example.json) for the full example config
and [`docs/settings.md`](docs/settings.md) for current behavior and removed settings.
`run_ledger: "memory"` collects core command facts; the legacy `"persist"` value
has the same in-memory behavior. Disk history is owned by the optional plugin.
See [`docs/plugins.md`](docs/plugins.md) for plugin development and the local,
unsandboxed trust model.

## Built-in agent coordination

Agents starts automatically in its own process. Open **Agents: Open panel**
from the command palette, or use `sleipnir agentctl list`. In a Sleipnir pane,
`"$SLEIPNIR_BIN" agentctl list` also works without adding the application to PATH.
No separate plugin or client installation is needed; agent CLIs still need to
be installed independently. Coordination is Unix-only; Windows is observer-only.

This is a same-user local control socket, not a sandbox or an approval proxy.
Native agent approvals stay with the human. To opt out, set
`"plugins": { "builtin_agents": false }`. The separate `plugins.enabled` setting
continues to control external plugins only, which remain off by default.
See [Agents](crates/sleipnir_plugin_agents/README.md) for details.

## Quick shortcuts

- New window: `⌘N` / `Ctrl+Shift+N`
- New tab: `⌘T` / `Ctrl+Shift+T`
- Command palette: `⌘⇧K` / `Ctrl+Shift+P`
- Find in scrollback: `⌘F` / `Ctrl+Shift+F`
- Theme reload: `⌘⇧R` / `Ctrl+Shift+R`

## License

The application and terminal crates declare `GPL-3.0-or-later`; the local
`gpui_platform` crate and upstream GPUI stack use Apache 2.0. These are
component-specific licenses, not a choice of two licenses for the whole app.
See [LICENSE-GPL](LICENSE-GPL), [LICENSE-APACHE](LICENSE-APACHE), the crate
manifests, and [UPSTREAM.md](UPSTREAM.md).
