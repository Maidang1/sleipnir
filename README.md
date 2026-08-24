<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir app icon" width="160" height="160" />

# Sleipnir

**A fast, native terminal emulator for macOS, Windows, and Linux: GPU-rendered, tab- and split-aware.**

Built on [GPUI](https://gpui.rs) (the UI framework behind [Zed](https://github.com/zed-industries/zed))
with a forked terminal backend.

[Features](#features) · [Install](#install) · [Config](#config) · [Shortcuts](#shortcuts)

[English](README.md) · [中文](README.zh.md)

</div>

---

## About

Sleipnir is a standalone terminal that renders on the GPU through GPUI, so
scrolling and redraw stay smooth even under heavy output. It ships with a real
host PTY (ConPTY on Windows), IME support, multi-tab and split panes, multi-window
sessions, adaptive theming that follows the system appearance, and a
file-manager-friendly clipboard that turns pasted images into quoted paths.

Prebuilt downloads on [GitHub Releases](https://github.com/Maidang1/sleipnir/releases)
include a macOS `.dmg`, a Windows `*-windows-x64.exe`, and native Linux packages
for x86_64 and ARM64: `.deb` files plus portable tarballs.

The name comes from Norse myth: Odin's eight-legged steed, the fastest horse in the
nine worlds. The app icon abstracts that into a minimal horse-head mark over a terminal
prompt.

## Features

- **GPU rendering** - smooth scrollback and redraw via GPUI (Metal on macOS, Direct3D 11 on Windows, Vulkan on Linux); Wayland and X11 are supported, with an ease-in-out cursor blink on every platform.
- **Tabs, splits & panes** - a top strip that groups by git workspace with no group header, showing the last two cwd components (`myself/harbor`) per tab. Right-click rename overrides. Split right/down, jump tabs, move focus, drag tabs to reorder inside a group or drop onto the terminal to detach into a new window; drag a background tab onto the visible panes to merge it as a split; drag a pane grip onto the tab list to extract it as a tab; pane zoom and unfocused dim. Known coding agents show a letter mark (`claude` → `C`, `codex` → `X`, …); a plain shell has no placeholder. A tab with failed Attention washes faint red (no running/success dots). Mark Tab as Seen clears Attention without deleting Run records.
- **Multi-window** - `⌘N` on macOS or `Ctrl+Shift+N` on Windows/Linux opens an independent window with its own tabs and shells.
- **Font zoom** - `⌘+` on macOS or `Ctrl+Shift++` on Windows/Linux (plus the matching `-` / `0` shortcuts) resizes the grid for the current window without persisting it.
- **Adaptive themes** — Catppuccin flavors plus Tokyo Night, Nord, Gruvbox, Solarized,
  GitHub Dark/Light, Dracula, One Dark; `auto` follows the system light/dark appearance.
  Extra palettes go in `themes.json` in the config dir (`"theme": "kanagawa"`, see
  `docs/themes.example.json`).
- **Smart paste** — paste an image to get a shell-quoted temp-file path; paste file-manager selections as quoted paths; force text-only paste with `⌃⌘V` on macOS or `Ctrl+Alt+V` on Windows/Linux.
- **Zed-compatible config** - reuse your `terminal.*` settings; hot-reload with `⌘⇧R` on macOS or `Ctrl+Shift+R` on Windows/Linux.
- **vi mode** — keyboard-driven selection and navigation.
- **Accessibility** — the terminal exposes the visible screen as a read-only accessible value (VoiceOver can read the current output), like Ghostty's read-only AX.
- **Session restore** — tabs, splits, and working directories survive relaunch.
- **Command palette** - discover actions with `⌘⇧K` on macOS or `Ctrl+Shift+P` on Windows/Linux; optional key binding overrides live in settings. `keybinding_preset: tmux` adds `ctrl-b` tab/pane chords. **Pane Facts** (View menu) shows the focused pane's directory, process tree, and listen ports.
- **Diff inspector** — open it from the chrome **Diff** button, rail `+N −M`, or `⌥⌘G` on macOS / `Ctrl+Alt+Shift+G` on Windows/Linux. It shows split (default) or unified (`v`) `git diff HEAD` for the focused pane's work tree, a file tree, expandable hidden context, tree-sitter highlighting (Rust/Python/JS/JSON), a minimap (`m`), and word-level intra-line changes. It is not a pane. `n`/`p` jump hunks, `]`/`[` jump files. **Shell → Send Git Diff to Pane** on macOS or **File → Send Git Diff to Pane** on Windows/Linux still pastes the raw patch.
- **Run Ledger** — open the redacted command-run overlay with `⌘⇧L` on macOS or `Ctrl+Shift+L` on Windows/Linux; click a row to jump to that pane and its OSC 133 Anchor. Persisted to `runs.json` by default (`run_ledger`: `off` / `memory` / `persist`). Pane gutter triangles mark command start/end (hidden on the alternate screen). After restore, a chrome tombstone banner names the last prior-launch command (not scrollback; dismisses on type; `show_tombstone: false` hides it).
- **Control surface** — available on macOS and Linux, off by default. Set `control_surface: true` or `SLEIPNIR_CONTROL=1` to bind `~/.config/sleipnir/control.sock`; `sleipnir-ctl ls|capture|send|wait` drives live panes ([ADR-0011](docs/adr/0011-control-surface.md)).
- **Find in scrollback** — use `⌘F` on macOS or `Ctrl+Shift+F` on Windows/Linux for highlighted search with regex (`.*`) and match-case (`Aa`) toggles; export to a file through **Shell → Export Scrollback…** on macOS or **File → Export Scrollback…** on Windows/Linux.
- **Path links & bell** - ⌘-click on macOS or Ctrl-click on Windows/Linux opens paths in the default app; hover shows a URL/path preview tooltip; optional system/visual bell.
- **Close confirm** — prompt when a non-shell job is running (`confirm_close`: dirty/always/never).
- **Shell collaboration** - OSC 133 prompt jump; new tabs inherit the workspace git root (splits inherit the active pane cwd); optional desktop notification when a long command finishes while unfocused. Search shell history from **Shell** on macOS or **File** on Windows/Linux (`⌘⇧;` / `Ctrl+Shift+;`). Send Selection / Send Git Diff pastes into the focused pane from the same platform menu; `pipe_selection_command` runs the selection through an external command.
- **Quick Terminal / Quick Select** — open a spare window fast; link-oriented mode.
- **Attention** — failed runs wash the tab on every platform. On macOS, the optional menu-bar item (`show_tray_icon`, default on) and Dock badge also show the failed Attention count.

## Install

### macOS 14+

Latest GitHub Release:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

The script downloads `Sleipnir-<ver>-macos.dmg`, checks it against the published
`.dmg.sha256` sidecar, mounts it, copies the app to `/Applications`, and runs `xattr -cr`
to drop the quarantine flag. CI builds are ad-hoc signed (no Developer ID), so
without that step Gatekeeper shows “unidentified developer” on first launch.

Prefer a different folder, or skip launching the app:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh \
  | PREFIX="$HOME/Applications" SLEIPNIR_NO_OPEN=1 bash
```

Or grab the `.dmg` from [Releases](https://github.com/Maidang1/sleipnir/releases)
and, if macOS still blocks it, run:

```bash
xattr -cr /Applications/Sleipnir.app
```

### Windows 10 1809+

Download `Sleipnir-<ver>-windows-x64.exe` from
[Releases](https://github.com/Maidang1/sleipnir/releases) and run it. Or build from source:

```powershell
cargo run -p sleipnir
```

Settings live in `%APPDATA%\sleipnir\`. Default font is Cascadia Mono. The app
never binds a bare `Ctrl+<key>`; those stay with the shell/TUI (`Ctrl+C` /
`Ctrl+D` / `Ctrl+V` / `Ctrl+1`…`9`, etc.). App shortcuts live on `Ctrl+Shift+*`
(primary, mirrors macOS `⌘`) and `Ctrl+Alt+*` (pane geometry, mirrors macOS
`⌘⌥`); copy/paste also accept `Ctrl+Insert` / `Shift+Insert`. **Check for
Updates** opens the Releases page for a manual install.

### Linux

Ubuntu 22.04 and newer are officially supported on Wayland and X11. Other desktop
distributions with glibc 2.35 or newer are best effort through the portable
tarball. Both x86_64 and ARM64 are native builds:

| Architecture | Debian package | Portable tarball |
|---|---|---|
| x86_64 | `sleipnir_<ver>_amd64.deb` | `Sleipnir-<ver>-linux-x86_64.tar.gz` |
| ARM64 | `sleipnir_<ver>_arm64.deb` | `Sleipnir-<ver>-linux-aarch64.tar.gz` |

The shared installer detects Linux, selects the host architecture, verifies the
published SHA-256 sidecar, and installs the `.deb` with `apt`:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

For a rootless user-local tarball install under `~/.local`, put the environment
assignment on the `bash` side of the pipe:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | SLEIPNIR_TARBALL=1 bash
```

The portable tarball is not self-contained. It requires glibc 2.35+, a Vulkan
driver, Wayland or X11, fontconfig, `xdg-open` from `xdg-utils`, and `notify-send`
from `libnotify-bin`. Settings and sessions live under `~/.config/sleipnir/`, and
the default font is Ubuntu Mono with DejaVu Sans Mono and Liberation Mono
fallbacks. Linux uses the same Ctrl-based desktop keymap as Windows:
`Ctrl+Shift+*` for primary app actions, `Ctrl+Alt+*` for pane geometry, and
`Ctrl+Shift+1`…`9` for tab selection. Bare `Ctrl+<key>` chords remain available
to the shell. **Check for Updates** opens Releases for a manual update.

## Requirements

To **run** a release build:

- macOS 14.0+ (Sonoma) with Metal;
- Windows 10 1809+ with a Direct3D 11 GPU; or
- Ubuntu 22.04+ with glibc 2.35+, Vulkan, and a Wayland or X11 desktop. Other
  glibc 2.35+ desktop distributions are supported on a best-effort basis.

Linux packages expect fontconfig, `xdg-utils`, and `libnotify-bin` in addition to
the display and Vulkan runtime libraries listed above.

To **build** from source, also install:

- Rust **1.95.0** (see `rust-toolchain.toml`);
- macOS: Xcode + Metal Toolchain (`xcodebuild -downloadComponent MetalToolchain`);
- Linux: a C/C++ build toolchain, `pkg-config`, and development packages for
  fontconfig, FreeType, X11/XCB/XRandR/XInput, xkbcommon, Wayland, GLib, and
  Vulkan. On Ubuntu 22.04, install them with:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config \
  libfontconfig-dev libfreetype-dev libx11-dev libx11-xcb-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxrandr-dev libxi-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libglib2.0-dev libvulkan1 mesa-vulkan-drivers
```

Creating Linux release packages additionally requires `dpkg-dev` and
`python3-pil`:

```bash
sudo apt-get install -y dpkg-dev python3-pil
```

## Build & run

```bash
cargo run -p sleipnir
# binary: target/debug/sleipnir
```

On Linux, build `.deb` and portable tarball artifacts from the native release
binary with:

```bash
cargo build --release -p sleipnir
./scripts/make-linux-package.sh --binary target/release/sleipnir --out build
```

## Config

Zed-compatible `terminal.*` keys plus Sleipnir extensions. Settings live in
`~/.config/sleipnir/settings.json` on macOS and Linux, or
`%APPDATA%\sleipnir\settings.json` on Windows; session restore is `session.json`
next to it. Default font is **Menlo** on macOS, **Cascadia Mono** on Windows, and
**Ubuntu Mono** on Linux.

| Key | Meaning |
|-----|---------|
| `theme` | `auto` / `mocha` / … (see example); `auto` follows system appearance; any name from `themes.json` also works |
| `custom_theme` | Optional hex palette (`background`/`foreground`/`ansi`…) that overrides `theme` |
| `restore_session` | Restore tabs/splits/cwd on launch (default `true`) |
| `confirm_close` | `dirty` / `always` / `never` — prompt before closing a busy pane (default `dirty`) |
| `path_links` | Open path-like targets on ⌘-click (macOS) or Ctrl-click (Windows/Linux); default `true` |
| `key_bindings` | Extra chords (`{ "key": "cmd-alt-t", "action": "new_tab" }`). Actions: `new_tab`, `close_tab`, `next_tab`, `prev_tab`, `split_right`, `split_down`, `new_window`, `open_settings`, `reload_settings`, `cycle_theme`, `find`, `toggle_command_palette`, `increase_font_size`, `decrease_font_size`, `reset_font_size`, `toggle_pane_zoom`, `toggle_broadcast`, `jump_prev_prompt`, `jump_next_prompt`, `toggle_quick_select`, `open_quick_terminal`, `export_scrollback`, `check_for_updates`, `clear_run_ledger`, `toggle_run_ledger`, `mark_tab_seen`, `toggle_pane_facts`, `send_selection`, `pipe_selection`, `send_git_diff`, `toggle_diff`, `toggle_history_search`. Optional `context`: `AppShell` / `Terminal`. Restart to apply. |
| `terminal.font_ligatures` | Enable OpenType ligatures (default `false`) |
| `terminal.copy_on_select` | Copy selection on mouse-up (default `false`; toggle in Settings) |
| `terminal.bell` | `off` / `system` / `visual` (default `off`) |
| `background_opacity` | 0.15–1.0 content opacity (default `1.0` opaque) |
| `notify_on_command_finish_secs` | Long-job notify threshold seconds (default `5`; `0` off) |
| `notify_on_command_finish_mode` | `never` / `unfocused` / `always` (default `unfocused`) |
| `run_ledger` | `off` / `memory` / `persist` — collect and show the Run Ledger (default `persist`) |
| `run_ledger_retention_days` | How long to keep persisted runs (default `7`) |
| `run_ledger_max_runs` | Cap on persisted runs; oldest dropped first (default `500`) |
| `run_ledger_redact` | Redact command lines at capture (default `true`; heuristic, not a guarantee) |
| `agent_icons` | Letter monograms for known coding-agent processes (default `true`) |
| `control_surface` | Bind the local control socket (default `false`). Also on with `SLEIPNIR_CONTROL=1` |
| `show_tray_icon` | Menu-bar Attention item (default `true`). Dock badge is independent |
| `pipe_selection_command` | External command that receives the current selection; empty disables |
| `keybinding_preset` | `default` / `tmux` (`ctrl-b` then `c` / `%` / `"` / arrows / `z`) |
| `show_tombstone` | Chrome restore banner from prior-launch Run metadata (default `true`) |
| `terminal.inject_osc133` | Inject OSC 133 A/B/C/D into zsh/bash/fish (default `true`; was `false`) |

Open the in-app theme picker with `⌘,` on macOS or `Ctrl+,` on Windows/Linux,
or edit the file and reload with `⌘⇧R` / `Ctrl+Shift+R` (key binding overrides
apply on next launch).
See [`docs/settings.example.json`](docs/settings.example.json).

## Paste

| Clipboard | Result |
|-----------|--------|
| Image (screenshot, etc.) | Write to a temp file and paste a quoted absolute path |
| File-manager selection | Paste space-separated quoted paths |
| Text | Normal paste (bracketed paste when the app enables it) |

Paste with `⌘V` / `⌃⇧V` on macOS or `Ctrl+Shift+V` / `Shift+Insert` on
Windows/Linux. Force **text-only** paste with `⌃⌘V` on macOS or `Ctrl+Alt+V` on
Windows/Linux.

## Shortcuts

The macOS shortcuts are shown below. Windows and Linux share the desktop keymap:
primary application actions use `Ctrl+Shift+*`, pane geometry uses
`Ctrl+Alt+*`, tab selection uses `Ctrl+Shift+1`…`9`, and bare Ctrl chords stay
available to the shell. The command palette shows the exact binding on the
current platform.

| Action | macOS shortcut | Windows/Linux shortcut |
|--------|----------------|------------------------|
| Copy | `⌘C` / `⌃⇧C` | `Ctrl+Shift+C` / `Ctrl+Insert` |
| Paste (image → path) | `⌘V` / `⌃⇧V` | `Ctrl+Shift+V` / `Shift+Insert` |
| Paste text only | `⌃⌘V` | `Ctrl+Alt+V` |
| Select all | `⌘A` | `Ctrl+Shift+A` |
| Clear | `⌘K` | `Ctrl+Shift+K` |
| New tab / close pane | `⌘T` / `⌘W` | `Ctrl+Shift+T` / `Ctrl+Shift+W` |
| New window | `⌘N` | `Ctrl+Shift+N` |
| Jump to tab N | `⌘1`…`⌘9` | `Ctrl+Shift+1`…`Ctrl+Shift+9` |
| Split pane right / down | `⌘D` / `⌘⇧D` | `Ctrl+Alt+D` / `Ctrl+Alt+Shift+D` |
| Move focus between panes | `⌘⌥←↑↓→` | `Ctrl+Alt+←↑↓→` |
| Next / previous tab | `⌃Tab` / `⌃⇧Tab` | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Next / previous tab (alt) | `⌘⇧]` / `⌘⇧[` | `Ctrl+Shift+]` / `Ctrl+Shift+[` |
| Increase / decrease / reset font | `⌘+` `⌘=` `⌘-` `⌘0` | `Ctrl+Shift++` `Ctrl+Shift+=` `Ctrl+Shift+-` `Ctrl+Shift+0` |
| Toggle pane zoom | `⌘⇧Enter` | `Ctrl+Shift+Enter` |
| Toggle broadcast | `⌘⇧B` | `Ctrl+Shift+B` |
| Jump prev / next prompt | `⌘⇧↑` / `⌘⇧↓` | `Ctrl+Shift+↑` / `Ctrl+Shift+↓` |
| Quick Select | `⌘⇧O` | `Ctrl+Shift+O` |
| Quick Terminal | `⌘⇧N` | `Ctrl+Alt+N` |
| Settings | `⌘,` | `Ctrl+,` |
| Reload settings | `⌘⇧R` | `Ctrl+Shift+R` |
| Cycle theme | `⌘⇧P` | `Ctrl+Shift+Y` |
| Command palette | `⌘⇧K` | `Ctrl+Shift+P` |
| Run Ledger | `⌘⇧L` | `Ctrl+Shift+L` |
| Diff inspector | `⌥⌘G` | `Ctrl+Alt+Shift+G` |
| Search shell history | `⌘⇧;` | `Ctrl+Shift+;` |
| Find in scrollback | `⌘F` | `Ctrl+Shift+F` |
| Next / previous find match | `⌘G` / `⌘⇧G` | `Ctrl+Shift+G` / `Ctrl+Alt+G` |
| Check for updates | `⌘⇧U` | `Ctrl+Shift+U` |
| Scroll line | `⇧↑` / `⇧↓` | `Shift+↑` / `Shift+↓` |
| Word back / forward | `⌥←` / `⌥→` | `Alt+←` / `Alt+→` |
| Toggle vi mode | `⌃⇧Space` | `Ctrl+Shift+Space` |
| Quit | `⌘Q` | `Alt+F4` / `Ctrl+Shift+Q` |

Scroll shortcuts are ignored on the alternate screen (full-screen TUI apps).

## Auto-update

Sleipnir can update itself from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases).

- Open the update dialog via **Sleipnir → Check for Updates…** (`⌘⇧U`) on
  macOS, or **File → Check for Updates…** (`Ctrl+Shift+U`) on Windows/Linux.
  Sleipnir does **not** check for updates automatically on launch.
- If a newer version is found, choosing **Download & Install** fetches the
  `Sleipnir-<ver>-macos.dmg` artifact and verifies it against the published
  `.dmg.sha256` sidecar before staging. Because CI builds are ad-hoc signed (no Apple
  Developer certificate), this SHA-256 check is the integrity guarantee — the download is
  rejected on any mismatch.
- **Restart & Update** on macOS swaps the running `.app` in place and relaunches.
  If the bundle is not writable, it falls back to opening Releases.
- Windows and Linux do not replace the running application in place. **Check for
  Updates** opens GitHub Releases so you can install the appropriate `.exe`, `.deb`,
  or tarball manually.

## Scope

Sleipnir is built for **people who run coding agents in a terminal**: the human is
the user, the agent is the workload. That sets the boundaries:

- **No built-in AI.** No model calls, no chat panel, no API-key management. Sleipnir
  is the terminal your agent runs *in*, not an agent. If you want to talk to an AI
  inside your terminal, use [Warp](https://warp.dev) or [Wave](https://waveterm.dev).
  ([ADR-0008](docs/adr/0008-no-builtin-ai.md))
- **No persisted scrollback.** Session restore brings back tabs, splits and working
  directories — never terminal output, because output routinely contains tokens and
  passwords. Export from **Shell → Export Scrollback…** on macOS or
  **File → Export Scrollback…** on Windows/Linux when you want to keep a transcript.
- **No process restore.** Restarting does not resurrect running commands; that is
  what `tmux` / `zellij` are for, and Sleipnir does not reimplement them.
- **No plugin system.** Configuration, key bindings, and piping to external commands
  are the extension points.

## Status

Shipped through M15 plus the Unreleased chrome, Run Ledger, and control-surface
work — see [CHANGELOG](CHANGELOG.md). Still open: scrollback byte-budget /
compression, a fuller VoiceOver tree, and kitty graphics (tracked, not
implemented: [ADR-0004](docs/adr/0004-kitty-graphics-track-not-implement.md)).

Performance is measured, not asserted: methodology in
[`scripts/bench/README.md`](scripts/bench/README.md), numbers in
[`scripts/bench/results.md`](scripts/bench/results.md). Input latency and a
same-machine comparison against Ghostty / kitty / Alacritty are still open, so treat
the throughput figures as internal baselines rather than competitive claims.

## Upstream

The GPUI stack is **not** vendored. Root `Cargo.toml` pins `zed-industries/zed` at a
fixed `rev` (`gpui`, `gpui_macos`, `gpui_windows`, `gpui_linux`, `collections`,
`util`, …). Local forks:
`terminal`, a slim `gpui_platform`, and the Sleipnir app crates. See [`UPSTREAM.md`](UPSTREAM.md)
to bump the pin.

```bash
./scripts/upstream-diff.sh /path/to/zed
```

## Packaging & release

### Local build

```bash
# Build and package as .app + .dmg (macOS)
./scripts/make-app.sh

# With developer certificate signing (macOS)
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)"

# Notarize the .dmg (requires signing)
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)" --notarize

# Package an existing native Linux release binary as .deb + portable tarball
./scripts/make-linux-package.sh --binary target/release/sleipnir --out build
```

The macOS bundle uses [`resources/AppIcon.icns`](resources/AppIcon.icns), generated from
[`resources/appicon.svg`](resources/appicon.svg).

### Publish to GitHub Releases (via `gh` CLI)

```bash
# Tag a release and publish
git tag v0.2.0
git push origin v0.2.0

# Or manually: build then publish
./scripts/make-app.sh --sign "..." --dmg
./scripts/publish-release.sh 0.2.0 ./build
```

### CI (GitHub Actions)

Triggered automatically on git tags (`v*`). Also supports manual dispatch.
The macOS job builds and packages the `.dmg`; the Windows job builds and tests,
then packages the x64 `.exe`. Both publish SHA-256 sidecars. Native Ubuntu 22.04
jobs build and test on x86_64 and ARM64, run packaging checks, smoke an X11
window, and attach one `.deb`, one portable tarball, and both sidecars for each
architecture.

```bash
gh workflow run build-and-release.yml \
  -f version=0.2.0
```

**Required GitHub Secrets** (repo Settings → Secrets and variables → Actions):

| Secret | Description |
|--------|-------------|
| `CODE_SIGNING_CERT_P12` | Base64-encoded `.p12` certificate file |
| `CODE_SIGNING_CERT_PASSWORD` | Password for the `.p12` file |
| `APPLE_ID` | Apple ID email for notarization |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password for notarization |
| `APPLE_TEAM_ID` | Apple Developer team ID |

See [`.github/workflows/build-and-release.yml`](.github/workflows/build-and-release.yml)
for the full pipeline.

## Credits & license

Sleipnir reuses and adapts code from [Zed](https://github.com/zed-industries/zed):

| Component | License |
|-----------|---------|
| GPUI and related UI crates | Apache-2.0 |
| Terminal crates (M1+) | GPL-3.0-or-later |

Because GPL terminal code is included, **distribution of the combined work is
GPL-3.0-or-later**. See [`LICENSE-GPL`](LICENSE-GPL) and per-crate license files under
`crates/`.
