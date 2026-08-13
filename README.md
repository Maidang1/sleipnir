<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir app icon" width="160" height="160" />

# Sleipnir

**A fast, native terminal emulator for macOS — GPU-rendered, tab- and split-aware.**

Built on [GPUI](https://gpui.rs) (the UI framework behind [Zed](https://github.com/zed-industries/zed))
with a forked terminal backend.

[Features](#features) · [Install](#install) · [Config](#config) · [Shortcuts](#shortcuts) · [Roadmap](#roadmap)

</div>

---

## About

Sleipnir is a standalone terminal for macOS that renders on the GPU through GPUI, so
scrolling and redraw stay smooth even under heavy output. It ships with a real PTY,
IME support, multi-tab and split panes, multi-window sessions, adaptive theming that
follows the system appearance, and a Finder-friendly clipboard that turns pasted images
into file paths.

The name comes from Norse myth — Odin's eight-legged steed, the fastest horse in the
nine worlds. The app icon abstracts that into a minimal horse-head mark over a terminal
prompt.

## Features

- **GPU rendering** — smooth scrollback and redraw via GPUI + Metal; ease-in-out cursor blink.
- **Tabs, splits & panes** — split right/down, jump tabs, move focus; pane zoom and unfocused dim.
- **Multi-window** — `⌘N` opens an independent window with its own tabs and shells.
- **Font zoom** — `⌘+` / `⌘-` / `⌘0` resize the grid for the current window (not persisted).
- **Adaptive themes** — Catppuccin flavors plus Tokyo Night, Nord, Gruvbox, Solarized,
  GitHub Dark/Light; `auto` follows the system light/dark appearance.
- **Smart paste** — paste an image to get a shell-quoted temp-file path; paste Finder
  selections as quoted paths; force text-only paste when you need it.
- **Zed-compatible config** — reuse your `terminal.*` settings; hot-reload with `⌘⇧R`.
- **vi mode** — keyboard-driven selection and navigation.
- **Session restore** — tabs, splits, and working directories survive relaunch.
- **Command palette** — discover actions with `⌘⇧K`; optional key binding overrides in settings.
- **Find in scrollback** — `⌘F` search with match highlights.
- **Path links & bell** — cmd-click paths open in the default app; optional system/visual bell.
- **Close confirm** — prompt when a non-shell job is running (`confirm_close`: dirty/always/never).
- **Shell collaboration** — OSC 133 prompt jump (`⌘⇧↑`/`⌘⇧↓`); optional notify when a long
  command finishes while unfocused.
- **Quick Terminal / Quick Select** — open a spare window fast (`⌘⇧N`); link-oriented mode (`⌘⇧O`).

## Install

macOS, latest GitHub Release:

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

## Requirements

- macOS
- Rust **1.95.0** (see `rust-toolchain.toml`) — only needed to build from source
- Xcode + Metal Toolchain: `xcodebuild -downloadComponent MetalToolchain`

## Build & run

```bash
cargo run -p sleipnir
# binary: target/debug/sleipnir
```

## Config

`~/.config/sleipnir/settings.json` — Zed-compatible `terminal.*` keys plus Sleipnir
extensions:

| Key | Meaning |
|-----|---------|
| `theme` | `auto` / `mocha` / … (see example); `auto` follows system appearance |
| `restore_session` | Restore tabs/splits/cwd on launch (default `true`) |
| `confirm_close` | `dirty` / `always` / `never` — prompt before closing a busy pane (default `dirty`) |
| `path_links` | Open path-like targets on cmd-click (default `true`) |
| `key_bindings` | Extra key bindings (see [`docs/M9.md`](docs/M9.md)) |
| `terminal.font_ligatures` | Enable OpenType ligatures (default `false`) |
| `terminal.copy_on_select` | Copy selection on mouse-up (default `false`; toggle in Settings) |
| `terminal.bell` | `off` / `system` / `visual` (default `off`) |
| `background_opacity` | 0.15–1.0 content opacity (default `1.0` opaque) |
| `notify_on_command_finish_secs` | Unfocused long-job notify threshold seconds (default `5`; `0` off) |

Open the in-app theme picker with `⌘,`, or edit the file and reload with `⌘⇧R`
(key binding overrides apply on next launch).
See [`docs/settings.example.json`](docs/settings.example.json).

## Paste

| Clipboard | `⌘V` behavior |
|-----------|----------------|
| Image (screenshot, etc.) | Write to a temp file; paste shell-quoted absolute path |
| Finder file paths | Paste space-separated quoted paths |
| Text | Normal paste (bracketed paste when the app enables it) |

Use `⌃⌘V` to force **text-only** paste (skip image→path conversion).

## Shortcuts

| Key | Action |
|-----|--------|
| `⌘C` / `⌃⇧C` | Copy |
| `⌘V` / `⌃⇧V` | Paste (image → path) |
| `⌃⌘V` | Paste text only |
| `⌘A` | Select all |
| `⌘K` | Clear |
| `⌘T` / `⌘W` | New tab / close active pane (then tab) |
| `⌘N` | New window |
| `⌘1`…`⌘9` | Jump to tab N |
| `⌘D` / `⌘⇧D` | Split pane right / down |
| `⌘⌥←↑↓→` | Move focus between panes |
| `⌃Tab` / `⌃⇧Tab` | Next / previous tab |
| `⌘⇧]` / `⌘⇧[` | Next / previous tab |
| `⌘+` / `⌘=` / `⌘-` / `⌘0` | Increase / decrease / reset font size (window) |
| `⌘⇧Enter` | Toggle pane zoom |
| `⌘⇧B` | Toggle broadcast input banner |
| `⌘⇧↑` / `⌘⇧↓` | Jump to previous / next shell prompt (OSC 133) |
| `⌘⇧O` | Toggle Quick Select mode |
| `⌘⇧N` | Open Quick Terminal (new window) |
| `⌘,` | Open settings (theme picker) |
| `⌘⇧R` | Reload settings |
| `⌘⇧P` | Cycle theme (persists) |
| `⌘⇧K` | Command palette |
| `⌘F` | Find in scrollback |
| `⌘G` / `⌘⇧G` | Next / previous find match |
| `⌘⇧U` | Check for updates |
| `⌘↑` / `⌘↓` | Scroll page |
| `⇧↑` / `⇧↓` | Scroll line |
| `⌘←` / `⌘→` | Line start / end |
| `⌥←` / `⌥→` | Word back / forward |
| `⌘⌫` | Clear line |
| `⌃⇧Space` | Toggle vi mode |

See [`docs/M6.md`](docs/M6.md) for the full terminal list.

## Auto-update

Sleipnir can update itself from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases).

- Open the update dialog via **Sleipnir → Check for Updates…** or `⌘⇧U`. Sleipnir does
  **not** check for updates automatically on launch.
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
fixed `rev` (`gpui`, `gpui_macos`, `collections`, `util`, …). Local forks: `terminal`,
a slim `gpui_platform`, and the Sleipnir app crates. See [`UPSTREAM.md`](UPSTREAM.md) to
bump the pin.

```bash
./scripts/upstream-diff.sh /path/to/zed
```

## Packaging & release

### Local build

```bash
# Build and package as .app + .zip
./scripts/make-app.sh

# With developer certificate signing
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)"

# Create a .dmg (requires signing)
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)" --dmg
```

The bundle uses [`resources/AppIcon.icns`](resources/AppIcon.icns), generated from
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

Triggered automatically on git tags (`v*`). Also supports manual dispatch:

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
