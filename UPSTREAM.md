# Upstream sync (git pin + local forks)

harbor **does not copy** the GPUI stack into this repo.

GPUI and shared Zed utilities come from the [Zed monorepo](https://github.com/zed-industries/zed) via **Cargo git dependencies** pinned to a single commit (`rev` in root `Cargo.toml`).

What stays **local** (forked or original):

| Crate | Why local |
|-------|-----------|
| `terminal` | Heavily forked: settings/theme/task rewired to `harbor_settings` / `task_types` |
| `gpui_platform` | Slim macOS-only application entry |
| `harbor`, `harbor_ui`, `harbor_settings`, `task_types`, `release_channel` | Product code |

---

## Baseline pins

| Item | Value |
|------|--------|
| Source | `https://github.com/zed-industries/zed` |
| **Zed `rev`** | `371a7d4ba2fd0064b79a0bc67d28e57a906779dc` (2026-08-09) |
| Packages from that rev | `gpui`, `gpui_macos`, `collections`, `util`, `util_macros` (+ their transitive Zed crates) |
| `alacritty_terminal` | `git = "https://github.com/zed-industries/alacritty"` **rev `4c129667ce56611becdc82de6e28218c80e2e88f`** |
| Rust toolchain | `1.95.0` (`rust-toolchain.toml`, match Zed) |
| License | GPUI stack Apache-2.0; terminal path GPL-3.0-or-later |

After each successful upgrade, update this table and the `rev` values in root `Cargo.toml` (keep every Zed package on the **same** `rev`).

---

## How dependency works

```toml
# root Cargo.toml (pattern)
gpui = { git = "https://github.com/zed-industries/zed", rev = "<SAME>", default-features = false, features = ["font-kit"] }
gpui_macos = { git = "https://github.com/zed-industries/zed", rev = "<SAME>", ... }
collections = { git = "https://github.com/zed-industries/zed", rev = "<SAME>" }
util = { git = "https://github.com/zed-industries/zed", rev = "<SAME>" }
```

Cargo clones the monorepo once per rev and resolves workspace/path deps inside Zed. You do **not** need to list every transitive crate (`scheduler`, `sum_tree`, `media`, …) in harbor’s workspace unless a **local** crate depends on it via `workspace = true`.

Required `[patch.crates-io]` entries (aligned with Zed) live in root `Cargo.toml` (`async-process`, `async-task`, …).

---

## Upgrade checklist

1. Pick a new Zed commit:  
   `git -C /path/to/zed rev-parse HEAD`
2. Compare Zed’s `alacritty_terminal` rev with ours; bump if needed.
3. In root `Cargo.toml`, replace **all** Zed `rev = "…"` with the new commit (same string everywhere).
4. Build:  
   `cargo build -p harbor`
5. Fix **local** breaks only (`terminal`, `harbor_ui`, `gpui_platform`, settings). Do not vendor GPUI back unless you must patch upstream.
6. Smoke:  
   `cargo run -p harbor` — shell, tabs (`⌘T`), settings reload (`⌘⇧R`).
7. Commit with Zed rev + alacritty rev in the message; update this table.

Optional dry-run against a local Zed checkout (forks only):

```bash
./scripts/upstream-diff.sh /path/to/zed
```

---

## Known harbor-specific forks (API break hotspots)

| Area | Why different |
|------|----------------|
| `terminal/src/terminal_settings.rs` | Thin re-export of `harbor_settings`, not Zed `Settings` trait |
| `terminal/src/terminal.rs` imports | `task_types`, `harbor_settings::TerminalPalette` instead of `task`/`theme` |
| `get_color_at_index` | Takes palette, not full `Theme` |
| Integration tests in terminal | Stubbed / disabled |
| `gpui_platform` | macOS-only application entry (not multi-platform Zed crate) |
| UI | `harbor_ui` is original; not Zed `terminal_view` |

---

## When to re-vendor (exception)

Prefer git pin. Copy a crate into `crates/` only if you need a long-lived private patch to GPUI that cannot live upstream or in a personal Zed fork + alternate `git` URL.

---

## Alacritty rev change

If PTY/behavior bugs track alacritty, bump `alacritty_terminal` rev independently (still record both pins). Prefer matching whatever Zed uses at the GPUI `rev` you choose.
