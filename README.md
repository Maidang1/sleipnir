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
- Native PTY/ConPTY support and session restore
- Smart paste, path links, and system-aware themes
- Search in scrollback, diff inspection, and run ledger tracking
- Zed-compatible `terminal.*` settings and hot reload

## Install

### macOS

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

### Windows

Download the latest `Sleipnir-<ver>-windows-x64.exe` from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases), then run it.

### Linux

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

Linux releases include `.deb` packages and portable tarballs for x86_64 and ARM64.

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
- `restore_session`
- `confirm_close`
- `key_bindings`
- `terminal.bell`
- `notify_on_command_finish_secs`
- `run_ledger`

See [`docs/settings.example.json`](docs/settings.example.json) for the full example config.

## Quick shortcuts

- New window: `⌘N` / `Ctrl+Shift+N`
- New tab: `⌘T` / `Ctrl+Shift+T`
- Command palette: `⌘⇧K` / `Ctrl+Shift+P`
- Find in scrollback: `⌘F` / `Ctrl+Shift+F`
- Theme reload: `⌘⇧R` / `Ctrl+Shift+R`

## License

Sleipnir is licensed under the Apache 2.0 and GPL v2 licenses. See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-GPL](LICENSE-GPL).
