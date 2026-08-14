<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir app icon" width="160" height="160" />

# Sleipnir

**A fast, native terminal emulator for macOS, Windows and Linux — GPU-rendered, tab- and split-aware.**

Built on [GPUI](https://gpui.rs) (the UI framework behind [Zed](https://github.com/zed-industries/zed))
with a forked terminal backend.

[Features](#features) · [Install](#install) · [Config](#config) · [Shortcuts](#shortcuts) · [Roadmap](#roadmap)

</div>

---

## About

Sleipnir is a standalone terminal that renders on the GPU through GPUI, so
scrolling and redraw stay smooth even under heavy output. It ships with a real PTY
(ConPTY on Windows, the host PTY on Linux/macOS), IME support, multi-tab and split
panes, multi-window sessions, adaptive theming that follows the system appearance,
and a file-manager-friendly clipboard that turns pasted images into quoted paths.

Prebuilt downloads: macOS `.dmg` / `.zip`, a Windows x64 `.zip`
(`Sleipnir-<ver>-windows-x64.zip`), and a Linux `.deb` / `.tar.gz`
(`Sleipnir-<ver>-linux-x86_64.tar.gz`). All ship on
[GitHub Releases](https://github.com/Maidang1/sleipnir/releases).

The name comes from Norse myth — Odin's eight-legged steed, the fastest horse in the
nine worlds. The app icon abstracts that into a minimal horse-head mark over a terminal
prompt.

## Features

- **GPU rendering** — smooth scrollback and redraw via GPUI (Metal on macOS, Direct3D on Windows, Vulkan on Linux); ease-in-out cursor blink.
- **Tabs, splits & panes** — split right/down, jump tabs, move focus; pane zoom and unfocused dim.
- **Multi-window** — `⌘N` / `Ctrl+Shift+N` opens an independent window with its own tabs and shells.
- **Font zoom** — `⌘+` / `Ctrl++` (and `-` / `0`) resize the grid for the current window (not persisted).
- **Adaptive themes** — Catppuccin flavors plus Tokyo Night, Nord, Gruvbox, Solarized,
  GitHub Dark/Light; `auto` follows the system light/dark appearance.
- **Smart paste** — paste an image to get a shell-quoted temp-file path; paste Finder / Explorer /
  Nautilus selections as quoted paths; force text-only paste when you need it.
- **Zed-compatible config** — reuse your `terminal.*` settings; hot-reload with `⌘⇧R` / `Ctrl+Shift+R`.
- **vi mode** — keyboard-driven selection and navigation.
- **Session restore** — tabs, splits, and working directories survive relaunch.
- **Command palette** — discover actions with `⌘⇧K` / `Ctrl+Shift+P`; optional key binding overrides in settings.
- **Find in scrollback** — `⌘F` / `Ctrl+Shift+F` search with match highlights.
- **Path links & bell** — ⌘-click / Ctrl-click paths open in the default app; optional system/visual bell.
- **Close confirm** — prompt when a non-shell job is running (`confirm_close`: dirty/always/never).
- **Shell collaboration** — OSC 133 prompt jump; optional notify when a long command finishes
  while unfocused (macOS notification; Windows logs only; Linux via libnotify).
- **Quick Terminal / Quick Select** — open a spare window fast; link-oriented mode.

## Install

### macOS

Latest GitHub Release:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

The script downloads `Sleipnir-<ver>-macos.zip`, checks it against the published
`.zip.sha256` sidecar, copies the app to `/Applications`, and runs `xattr -cr`
to drop the quarantine flag. CI builds are ad-hoc signed (no Developer ID), so
without that step Gatekeeper shows “unidentified developer” on first launch.

Prefer a different folder, or skip launching the app:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh \
  | PREFIX="$HOME/Applications" SLEIPNIR_NO_OPEN=1 bash
```

Or grab the `.dmg` / `.zip` from [Releases](https://github.com/Maidang1/sleipnir/releases)
and, if macOS still blocks it, run:

```bash
xattr -cr /Applications/Sleipnir.app
```

### Windows

PowerShell, latest GitHub Release (installs to `%LOCALAPPDATA%\Sleipnir`):

```powershell
irm https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.ps1 | iex
```

Or grab `Sleipnir-<ver>-windows-x64.zip` from
[Releases](https://github.com/Maidang1/sleipnir/releases), unzip, and run
`sleipnir.exe`. The default shell is the system shell (usually PowerShell).
Set `terminal.shell` to `wsl.exe` in settings if you want WSL.

Closing the last window exits the process (unlike macOS, where the app can stay
in the Dock with no windows).

Or build from source (see below).

### Linux (Ubuntu 22.04+ / Debian 12+)

Latest GitHub Release, as a native package:

```bash
# Download sleipnir_<ver>_amd64.deb from Releases, then:
sudo apt install ./sleipnir_<ver>_amd64.deb
sleipnir
```

Or use the portable tarball `Sleipnir-<ver>-linux-x86_64.tar.gz`, which bundles the
binary plus a `.desktop` file and README. It needs a Vulkan driver:

```bash
sudo apt install libvulkan1 mesa-vulkan-drivers
tar -xzf Sleipnir-<ver>-linux-x86_64.tar.gz
cd Sleipnir-<ver>-linux-x86_64 && ./sleipnir
```

The `.deb` installs a desktop entry and hicolor icons, so you can launch Sleipnir
from the app menu. Sleipnir prefers Wayland when `WAYLAND_DISPLAY` is set and falls
back to X11 otherwise (set `WAYLAND_DISPLAY=` to force X11).

## Requirements

To **run** a release build:

- macOS 14.0+ (Sonoma)
- Windows 10 1809+ (ConPTY) with a Direct3D 11 GPU
- Linux (Ubuntu 22.04+ / Debian 12+) with a Vulkan driver, X11 or Wayland,
  and `xdg-open` (from `xdg-utils`) for opening links/paths

To **build** from source, also:

- Rust **1.95.0** (see `rust-toolchain.toml`)
- macOS: Xcode + Metal Toolchain (`xcodebuild -downloadComponent MetalToolchain`)
- Linux (Ubuntu): `pkg-config`, `libfontconfig-dev`, `libfreetype-dev`,
  `libxkbcommon-dev`, `libwayland-dev`, and `libvulkan1` at runtime

## Build & run

```bash
cargo run -p sleipnir
# macOS binary: target/debug/sleipnir
# Windows binary: target/debug/sleipnir.exe
# Linux binary:  target/debug/sleipnir
```

## Config

Zed-compatible `terminal.*` keys plus Sleipnir extensions:

| OS | Settings / session |
|----|--------------------|
| macOS | `~/.config/sleipnir/settings.json` and `session.json` |
| Windows | `%APPDATA%\sleipnir\settings.json` and `session.json` |
| Linux | `~/.config/sleipnir/settings.json` and `session.json` |

Default font is **Menlo** on macOS, **Cascadia Mono** (then Consolas, Courier New)
on Windows, and **Ubuntu Mono** (then DejaVu Sans Mono, Liberation Mono) on Linux.

| Key | Meaning |
|-----|---------|
| `theme` | `auto` / `mocha` / … (see example); `auto` follows system appearance |
| `restore_session` | Restore tabs/splits/cwd on launch (default `true`) |
| `confirm_close` | `dirty` / `always` / `never` — prompt before closing a busy pane (default `dirty`) |
| `path_links` | Open path-like targets on ⌘-click / Ctrl-click (default `true`) |
| `key_bindings` | Extra key bindings (see [`docs/M9.md`](docs/M9.md)) |
| `terminal.font_ligatures` | Enable OpenType ligatures (default `false`) |
| `terminal.copy_on_select` | Copy selection on mouse-up (default `false`; toggle in Settings) |
| `terminal.bell` | `off` / `system` / `visual` (default `off`) |
| `background_opacity` | 0.15–1.0 content opacity (default `1.0` opaque) |
| `notify_on_command_finish_secs` | Unfocused long-job notify threshold seconds (default `5`; `0` off) |

Open the in-app theme picker with `⌘,` / `Ctrl+,`, or edit the file and reload
with `⌘⇧R` / `Ctrl+Shift+R` (key binding overrides apply on next launch).
See [`docs/settings.example.json`](docs/settings.example.json).

## Paste

| Clipboard | Paste behavior (`⌘V` / `Ctrl+V` / `Ctrl+Shift+V`) |
|-----------|----------------|
| Image (screenshot, etc.) | Write to a temp file; paste a quoted absolute path (POSIX on macOS/Linux, PowerShell on Windows) |
| Finder / Explorer / Nautilus file paths | Paste space-separated quoted paths |
| Text | Normal paste (bracketed paste when the app enables it) |

Force **text-only** paste (skip image→path conversion) with `⌃⌘V` on macOS or
`Ctrl+Alt+V` on Windows/Linux.

On Windows and Linux, plain `Ctrl+C` still goes to the PTY (interrupt). Copy is
`Ctrl+Shift+C`.

## Shortcuts

Windows and Linux follow Windows Terminal / Zed conventions: app chords use
Ctrl+Shift so `Ctrl+C`, `Ctrl+W`, and `Ctrl+D` stay with the shell.

| Action | macOS | Windows |
|--------|-------|---------|
| Copy | `⌘C` / `⌃⇧C` | `Ctrl+Shift+C` / `Ctrl+Insert` |
| Paste (image → path) | `⌘V` / `⌃⇧V` | `Ctrl+V` / `Ctrl+Shift+V` / `Shift+Insert` |
| Paste text only | `⌃⌘V` | `Ctrl+Alt+V` |
| Select all | `⌘A` | `Ctrl+Shift+A` |
| Clear | `⌘K` | `Ctrl+Shift+L` |
| New tab / close pane | `⌘T` / `⌘W` | `Ctrl+Shift+T` / `Ctrl+Shift+W` |
| New window | `⌘N` | `Ctrl+Shift+N` |
| Jump to tab N | `⌘1`…`⌘9` | `Ctrl+1`…`Ctrl+9` |
| Split pane right / down | `⌘D` / `⌘⇧D` | `Alt+Shift+D` / `Alt+Shift+-` |
| Move focus between panes | `⌘⌥←↑↓→` | `Ctrl+Alt+←↑↓→` |
| Next / previous tab | `⌃Tab` / `⌃⇧Tab` | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Next / previous tab (alt) | `⌘⇧]` / `⌘⇧[` | (use Ctrl+Tab) |
| Increase / decrease / reset font | `⌘+` `⌘=` `⌘-` `⌘0` | `Ctrl++` `Ctrl+=` `Ctrl+-` `Ctrl+0` |
| Toggle pane zoom | `⌘⇧Enter` | `Ctrl+Shift+Enter` |
| Toggle broadcast | `⌘⇧B` | `Ctrl+Shift+B` |
| Jump prev / next prompt | `⌘⇧↑` / `⌘⇧↓` | `Ctrl+Shift+↑` / `Ctrl+Shift+↓` |
| Quick Select | `⌘⇧O` | `Ctrl+Shift+O` |
| Quick Terminal | `⌘⇧N` | `Ctrl+Alt+N` |
| Settings | `⌘,` | `Ctrl+,` |
| Reload settings | `⌘⇧R` | `Ctrl+Shift+R` |
| Cycle theme | `⌘⇧P` | `Ctrl+Shift+Alt+P` |
| Command palette | `⌘⇧K` | `Ctrl+Shift+P` |
| Find in scrollback | `⌘F` | `Ctrl+Shift+F` |
| Next / previous find match | `⌘G` / `⌘⇧G` | `Ctrl+Shift+G` / `Ctrl+Shift+Alt+G` |
| Check for updates | `⌘⇧U` | `Ctrl+Shift+U` |
| Scroll page | `⌘↑` / `⌘↓` | `Shift+PageUp` / `Shift+PageDown` |
| Scroll line | `⇧↑` / `⇧↓` | `Shift+↑` / `Shift+↓` |
| Line start / end | `⌘←` / `⌘→` | (send to PTY) |
| Word back / forward | `⌥←` / `⌥→` | `Alt+←` / `Alt+→` |
| Clear line | `⌘⌫` | (send to PTY) |
| Toggle vi mode | `⌃⇧Space` | `Ctrl+Shift+Space` |
| Quit | `⌘Q` | `Alt+F4` / `Ctrl+Q` |

See [`docs/M6.md`](docs/M6.md) for the full macOS terminal list.

Linux uses the same Ctrl+Shift app chords as Windows (so `Ctrl+C`/`Ctrl+V` stay
with the shell), with these differences:

| Action | Linux |
|--------|-------|
| Copy | `Ctrl+Shift+C` / `Ctrl+Insert` |
| Paste | `Ctrl+Shift+V` / `Shift+Insert` |
| Paste text only | `Ctrl+Alt+V` |
| Quit | `Ctrl+Shift+Q` |
| Scroll to top / bottom | `Shift+Home` / `Shift+End` |
| Word back / forward | `Alt+←` / `Alt+→` (also `Alt+B` / `Alt+F`) |
| Delete word / rest of line | `Alt+Delete` / `Ctrl+Delete` (sent to the PTY as escape sequences) |

## Auto-update

Sleipnir can update itself from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases).

In-place install is **macOS-only**. On Windows and Linux, Check for Updates still
queries GitHub, then opens the releases page.

- Open the update dialog via **Sleipnir → Check for Updates…** (`⌘⇧U` / `Ctrl+Shift+U`).
  Sleipnir does **not** check for updates automatically on launch.
- If a newer version is found, choosing **Download & Install** fetches the
  `Sleipnir-<ver>-macos.zip` artifact and verifies it against the published
  `.zip.sha256` sidecar before staging. Because CI builds are ad-hoc signed (no Apple
  Developer certificate), this SHA-256 check is the integrity guarantee — the download is
  rejected on any mismatch.
- **Restart & Update** swaps the running `.app` in place and relaunches. If the bundle
  lives somewhere the app can't write (e.g. a protected `/Applications` install owned by
  another user), it falls back to opening the releases page for a manual install.

## Roadmap

**Status:** M0–M15 complete (per-pane fonts still deferred inside M10).

| Milestone | Goal | Status |
|-----------|------|:------:|
| **M0** | GPUI empty window | ✅ |
| **M1** | Display-only terminal grid paint | ✅ |
| **M2** | Real PTY + IME + selection/clipboard | ✅ |
| **M3** | Multi-tab + http(s) open | ✅ |
| **M4** | Theme/font settings polish | ✅ |
| **M5** | Upstream port checklist (`UPSTREAM.md`) | ✅ |
| **M6** | Image paste-as-path + Zed Terminal shortcuts | ✅ |
| **M7** | Signed & notarized release builds + auto-update | ✅ |
| **M8** | Session persistence (restore tabs/splits on launch) | ✅ |
| **M9** | Configurable keymap + command palette | ✅ |
| **M10** | Ligatures + search-in-scrollback (per-pane fonts deferred) | ✅ |
| **M11** | Visual polish (cursor fade, URL hover, optional scroll) | ✅ |
| **M12** | Daily gaps (font zoom, multi-window, close confirm, path open, bell) | ✅ |
| **M13** | Split professionalism (pane zoom, unfocused dim) | ✅ |
| **M14** | Shell collaboration (OSC 133, jump prompt, notify) | ✅ |
| **M15** | Optional differentiators (Quick Terminal, Quick Select) | ✅ |

Legend: ✅ done · 📋 planned.  
Shipped notes: [`docs/M7.md`](docs/M7.md)–[`docs/M15.md`](docs/M15.md).  
Competitive research: [`docs/competitive-research-features.md`](docs/competitive-research-features.md).  
Implementation plan: [`docs/superpowers/plans/2026-08-12-post-m10-feature-roadmap.md`](docs/superpowers/plans/2026-08-12-post-m10-feature-roadmap.md).

## Upstream

The GPUI stack is **not** vendored. Root `Cargo.toml` pins `zed-industries/zed` at a
fixed `rev` (`gpui`, `gpui_macos`, `gpui_linux`, `gpui_windows`, `collections`, `util`, …). Local forks:
`terminal`, a slim `gpui_platform`, and the Sleipnir app crates. See [`UPSTREAM.md`](UPSTREAM.md)
to bump the pin.

```bash
./scripts/upstream-diff.sh /path/to/zed
```

## Packaging & release

### Local build

```bash
# Build and package as .app + .zip (macOS)
./scripts/make-app.sh

# With developer certificate signing (macOS)
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)"

# Create a .dmg (requires signing)
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)" --dmg

# Linux: build and package as .tar.gz + .deb
./scripts/make-linux-package.sh
```

The macOS bundle uses [`resources/AppIcon.icns`](resources/AppIcon.icns), generated from
[`resources/appicon.svg`](resources/appicon.svg). The Linux packages embed hicolor icons
resized from [`resources/appicon_preview.png`](resources/appicon_preview.png) and a
[`.desktop`](resources/linux/sleipnir.desktop) entry.

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
The same workflow builds and tests `-p sleipnir` on `windows-latest` (`.zip`) and
`ubuntu-latest` (`.tar.gz` + `.deb`); the macOS job creates the GitHub Release and
each job attaches its assets to it.

```bash
gh workflow run build-and-release.yml \
  -f version=0.2.0 \
  -f draft=false
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
