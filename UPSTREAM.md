# Upstream sync (ad-hoc)

jiajia-term **copies** selected code from [Zed](https://github.com/zed-industries/zed).
There is **no** git submodule, subtree, or path dependency on the Zed monorepo.

Policy: **no fixed calendar**. Port when a bug is fixed upstream or you intentionally refresh GPUI/terminal.

---

## Baseline pins (record at fork / last sync)

| Item | Value |
|------|--------|
| Source | Zed monorepo (local or clone) |
| Approx. Zed commit at M0 copy | `371a7d4` (2026-08-09) — update when you re-copy |
| `alacritty_terminal` | `git = "https://github.com/zed-industries/alacritty"` **rev `4c129667ce56611becdc82de6e28218c80e2e88f`** |
| Rust toolchain | `1.95.0` (`rust-toolchain.toml`, match Zed) |
| License | GPUI stack Apache-2.0; terminal path GPL-3.0-or-later |

After each successful port, update this table and mention pins in the commit message.

---

## What we track

### High value (port carefully)

| Path in Zed | Path here | Notes |
|-------------|-----------|--------|
| `crates/terminal/src/**` | `crates/terminal/src/**` | **Heavily forked**: settings/theme/task rewired to `jiajia_settings` / `task_types`. Do **not** wholesale overwrite `terminal_settings.rs` or settings `use` lines. |
| `crates/gpui/**` | `crates/gpui/**` | Prefer small patches; watch workspace deps. |
| `crates/gpui_macos/**` | `crates/gpui_macos/**` | Metal shaders / macOS platform. |
| `crates/gpui_platform/**` | `crates/gpui_platform/**` | Here: **macOS-only** slim `gpui_platform.rs`. |

### Medium (as needed)

`collections`, `util`, `sum_tree`, `scheduler`, `http_client`, `refineable`, `gpui_*` helpers.

### Do **not** re-import

`workspace`, `editor`, `project`, full `settings` / `settings_content`, `theme` IDE stack, agent/collab.

---

## Port checklist

1. **Locate Zed checkout** (outside this repo), e.g.  
   `export ZED_ROOT=~/codes/open-source/zed`
2. **Record versions**  
   `git -C "$ZED_ROOT" rev-parse HEAD`  
   Compare Zed’s `alacritty_terminal` rev in `$ZED_ROOT/Cargo.toml` with ours.
3. **Dry-run diffs** (no edits):  
   `./scripts/upstream-diff.sh "$ZED_ROOT"`
4. **Read the diff**. For each interesting hunk:
   - Apply by hand into `crates/…`
   - Preserve jiajia forks (`terminal_settings`, palette colors, `task_types`, slim platform).
5. **Build**  
   `cargo build -p jiajia_term`  
   `cargo build -p jiajia_term --release` (optional)
6. **Smoke**  
   `cargo run -p jiajia_term` — shell, tabs (`⌘T`), settings reload (`⌘⇧R`).
7. **Commit** with Zed commit + alacritty rev in the message.  
   Update the pin table above.

---

## Known jiajia-specific forks (merge conflict hotspots)

| Area | Why different |
|------|----------------|
| `terminal/src/terminal_settings.rs` | Thin re-export of `jiajia_settings`, not Zed `Settings` trait |
| `terminal/src/terminal.rs` imports | `task_types`, `jiajia_settings::TerminalPalette` instead of `task`/`theme` |
| `get_color_at_index` | Takes palette, not full `Theme` |
| Integration tests in terminal | Stubbed / disabled |
| `gpui_platform` | macOS-only application entry |
| UI | `jiajia_term_ui` is original; do not expect Zed `terminal_view` drop-in |

---

## Alacritty rev change

If Zed bumps `alacritty_terminal`:

1. Copy the new `rev` into root `Cargo.toml` (`workspace.dependencies.alacritty_terminal`).
2. `cargo update -p alacritty_terminal` (or rebuild and let lock refresh).
3. Fix compile breaks in `crates/terminal` (API drift is rare but real).
4. Smoke PTY + scrollback + colors.

---

## Dry-run

```bash
# from jiajia-term root
./scripts/upstream-diff.sh /path/to/zed
# writes summary under docs/upstream-last-diff.txt
```

A dry-run with **no code port** still counts as a successful M5 exercise if the script runs and pins are recorded.
