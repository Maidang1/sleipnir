<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir app icon" width="160" height="160" />

# Sleipnir

**A fast, native terminal emulator for macOS — GPU-rendered, tab- and split-aware.**

Built on [GPUI](https://gpui.rs) (the UI framework behind [Zed](https://github.com/zed-industries/zed))
with a forked terminal backend.

[Features](#features) · [Install](#build--run) · [Config](#config) · [Shortcuts](#shortcuts) · [Roadmap](#roadmap)

</div>

---

## About

Sleipnir is a standalone terminal for macOS that renders on the GPU through GPUI, so
scrolling and redraw stay smooth even under heavy output. It ships with a real PTY,
IME support, multi-tab and split panes, adaptive theming that follows the system
appearance, and a Finder-friendly clipboard that turns pasted images into file paths.

The name comes from Norse myth — Odin's eight-legged steed, the fastest horse in the
nine worlds. The app icon abstracts that into a minimal horse-head mark over a terminal
prompt.

## Features

- **GPU rendering** — smooth scrollback and redraw via GPUI + Metal.
- **Tabs, splits & panes** — split right/down, move focus between panes, jump to any tab.
- **Adaptive themes** — Catppuccin flavors plus Tokyo Night, Nord, Gruvbox, Solarized,
  GitHub Dark/Light; `auto` follows the system light/dark appearance.
- **Smart paste** — paste an image to get a shell-quoted temp-file path; paste Finder
  selections as quoted paths; force text-only paste when you need it.
- **Zed-compatible config** — reuse your `terminal.*` settings; hot-reload with `⌘⇧R`.
- **vi mode** — keyboard-driven selection and navigation.

## Requirements

- macOS
- Rust **1.95.0** (see `rust-toolchain.toml`)
- Xcode + Metal Toolchain: `xcodebuild -downloadComponent MetalToolchain`

## Build & run

```bash
cargo run -p sleipnir
# binary: target/debug/sleipnir
```

## Config

`~/.config/sleipnir/settings.json` — Zed-compatible `terminal.*` keys plus a `theme`
key (`auto` / `mocha` / `macchiato` / `frappe` / `latte` / `tokyo_night` / `nord` /
`gruvbox_dark` / `solarized_light` / `github_dark` / `github_light`). `auto` follows
the system light/dark appearance.
Open the in-app theme picker with `⌘,`, or edit the file and reload with `⌘⇧R`.
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
| `⌘1`…`⌘9` | Jump to tab N |
| `⌘D` / `⌘⇧D` | Split pane right / down |
| `⌘⌥←↑↓→` | Move focus between panes |
| `⌃Tab` / `⌃⇧Tab` | Next / previous tab |
| `⌘⇧]` / `⌘⇧[` | Next / previous tab |
| `⌘,` | Open settings (theme picker) |
| `⌘⇧R` | Reload settings |
| `⌘⇧P` | Cycle theme (persists) |
| `⌘⇧U` | Check for updates |
| `⌘↑` / `⌘↓` | Scroll page |
| `⇧↑` / `⇧↓` | Scroll line |
| `⌘←` / `⌘→` | Line start / end |
| `⌥←` / `⌥→` | Word back / forward |
| `⌘⌫` | Clear line |
| `⌃⇧Space` | Toggle vi mode |

See [`docs/M6.md`](docs/M6.md) for the full list.

## Auto-update

Sleipnir can update itself from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases).

- On launch it silently checks for a newer version (when `auto_update` is enabled).
  A notification bar appears only if an update is available.
- Trigger a check manually via **Sleipnir → Check for Updates…** or `⌘⇧U`.
- Choosing **Download & Install** fetches the `Sleipnir-<ver>-macos.zip` artifact and
  verifies it against the published `.zip.sha256` sidecar before staging. Because CI
  builds are ad-hoc signed (no Apple Developer certificate), this SHA-256 check is the
  integrity guarantee — the download is rejected on any mismatch.
- **Restart & Update** swaps the running `.app` in place and relaunches. If the bundle
  lives somewhere the app can't write (e.g. a protected `/Applications` install owned by
  another user), it falls back to opening the releases page for a manual install.

Disable the launch check by adding to `~/.config/sleipnir/settings.json`:

```json
{ "auto_update": false }
```

## Roadmap

**Status:** M6 complete — image paste-as-path + Zed Terminal shortcuts.

| Milestone | Goal | Status |
|-----------|------|:------:|
| **M0** | GPUI empty window | ✅ |
| **M1** | Display-only terminal grid paint | ✅ |
| **M2** | Real PTY + IME + selection/clipboard | ✅ |
| **M3** | Multi-tab + http(s) open | ✅ |
| **M4** | Theme/font settings polish | ✅ |
| **M5** | Upstream port checklist (`UPSTREAM.md`) | ✅ |
| **M6** | Image paste-as-path + Zed Terminal shortcuts | ✅ |
| **M7** | Signed & notarized release builds + auto-update | 🔜 |
| **M8** | Session persistence (restore tabs/splits on launch) | 🔜 |
| **M9** | Configurable keymap + command palette | 🧭 |
| **M10** | Ligatures, per-pane fonts, search-in-scrollback | 🧭 |

Legend: ✅ done · 🔜 next · 🧭 planned.

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
