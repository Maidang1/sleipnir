# Harbor

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
cargo run -p harbor
# binary: target/debug/harbor
```

## Config

`~/.config/harbor/settings.json` — Zed-compatible `terminal.*` keys + `theme` (`mocha`/`macchiato`/`frappe`/`latte`). See `docs/settings.example.json`. Reload with `⌘⇧R`.

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
| `⌘T` / `⌘W` | New / close tab |
| `⌃Tab` / `⌃⇧Tab` | Next / previous tab |
| `⌘⇧]` / `⌘⇧[` | Next / previous tab |
| `⌘⇧R` | Reload settings |
| `⌘⇧T` | Cycle theme (session) |
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
and harbor app crates. See `UPSTREAM.md` to bump the pin.

```bash
./scripts/upstream-diff.sh /path/to/zed
```
