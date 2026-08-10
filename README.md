# Sleipnir

Standalone terminal emulator for **macOS**, built on [GPUI](https://gpui.rs) from the
[Zed](https://github.com/zed-industries/zed) monorepo (Cargo **git pin** by commit) plus a
forked terminal backend.

**Status:** M5 complete — upstream sync process.

## Credits & license

This project reuses and adapts code from [Zed](https://github.com/zed-industries/zed):

| Component | License |
|-----------|---------|
| GPUI and related UI crates | Apache-2.0 |
| Terminal crates (M1+) | GPL-3.0-or-later |

Once GPL terminal code is included, **distribution of the combined work is GPL-3.0-or-later**.
See `LICENSE-GPL` and per-crate license files under `crates/`.

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

`~/.config/sleipnir/settings.json` — Zed-compatible `terminal.*` keys + `theme` (`auto`/`mocha`/`macchiato`/`frappe`/`latte`/`tokyo_night`/`nord`/`gruvbox_dark`/`solarized_light`). `auto` follows the system light/dark appearance. See `docs/settings.example.json`. Reload with `⌘⇧R`.

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
| `⌘⇧R` | Reload settings |
| `⌘⇧P` | Cycle theme (session) |
| `⌘↑` / `⌘↓` | Scroll page |
| `⇧↑` / `⇧↓` | Scroll line |
| `⌘←` / `⌘→` | Line start / end |
| `⌥←` / `⌥→` | Word back / forward |
| `⌘⌫` | Clear line |
| `⌃⇧Space` | Toggle vi mode |

See `docs/M6.md` for the full list.

## Roadmap (milestones)

| Milestone | Goal |
|-----------|------|
| **M0** | GPUI empty window ✅ |
| **M1** | Display-only terminal grid paint ✅ |
| **M2** | Real PTY + IME + selection/clipboard ✅ |
| **M3** | Multi-tab + http(s) open ✅ |
| **M4** | Theme/font settings polish ✅ |
| **M5** | Upstream port checklist (`UPSTREAM.md`) ✅ |
| **M6** | Image paste-as-path + Zed Terminal shortcuts ✅ |

## Upstream

GPUI stack is **not** vendored. Root `Cargo.toml` pins `zed-industries/zed` at a fixed `rev`
(`gpui`, `gpui_macos`, `collections`, `util`, …). Local forks: `terminal`, slim `gpui_platform`,
and sleipnir app crates. See `UPSTREAM.md` to bump the pin.

```bash
./scripts/upstream-diff.sh /path/to/zed
```

## Packaging & Release

### Local build

```bash
# Build and package as .app + .zip
./scripts/make-app.sh

# With developer certificate signing
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)"

# Create a .dmg (requires signing)
./scripts/make-app.sh --sign "Apple Development: you@domain.com (TEAMID)" --dmg
```

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

**Required GitHub Secrets** (set in repo Settings → Secrets and variables → Actions):

| Secret | Description |
|--------|-------------|
| `CODE_SIGNING_CERT_P12` | Base64-encoded `.p12` certificate file |
| `CODE_SIGNING_CERT_PASSWORD` | Password for the `.p12` file |
| `APPLE_ID` | Apple ID email for notarization |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password for notarization |
| `APPLE_TEAM_ID` | Apple Developer team ID |

See `.github/workflows/build-and-release.yml` for the full pipeline.
