# jiajia-term

Standalone terminal emulator for **macOS**, built on a copied [GPUI](https://gpui.rs) stack
and (soon) Zed's terminal backend.

**Status:** M4 complete — themes + settings polish.

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
cd ~/codes/jiajia-term
cargo run -p jiajia_term
```

## Config (planned)

`~/.config/jiajia-term/settings.json` — Zed-compatible `terminal.*` keys + `theme` (`mocha`/`macchiato`/`frappe`/`latte`). See `docs/settings.example.json`. Reload with `⌘⇧R`.

## Roadmap (milestones)

| Milestone | Goal |
|-----------|------|
| **M0** | GPUI empty window ✅ |
| **M1** | Display-only terminal grid paint ✅ |
| **M2** | Real PTY + IME + selection/clipboard ✅ |
| **M3** | Multi-tab + http(s) open ✅ |
| **M4** | Theme/font settings polish ✅ |
| **M5** | Upstream port checklist (`UPSTREAM.md`) |

## Upstream

No git submodule of Zed. See `UPSTREAM.md` for ad-hoc porting.
