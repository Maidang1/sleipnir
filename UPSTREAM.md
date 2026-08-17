# Upstream sync (git pin + local forks)

sleipnir **does not copy** the GPUI stack into this repo.

GPUI and shared Zed utilities come from the [Zed monorepo](https://github.com/zed-industries/zed) via **Cargo git dependencies** pinned to a single commit (`rev` in root `Cargo.toml`).

What stays **local** (forked or original):

| Crate | Why local |
|-------|-----------|
| `terminal` | Heavily forked: settings/theme rewired to `sleipnir_settings` |
| `gpui_platform` | Slim macOS application entry |
| `sleipnir`, `sleipnir_ui`, `sleipnir_settings`, `release_channel` | Product code |
| `alacritty_terminal` | **Git pin** to [Maidang1/alacritty](https://github.com/Maidang1/alacritty) (`sleipnir-osc-custom`): zed alacritty fork + OSC 133/9/777 (see [ADR-0005](docs/adr/0005-vendored-alacritty-term.md)) |
| `vte` | **Git pin** to [Maidang1/vte](https://github.com/Maidang1/vte) (`sleipnir-osc-custom`): vte 0.15.0 + `Handler::osc_custom`; `[patch.crates-io]` forces every crate onto that rev |

---

## Baseline pins

| Item | Value |
|------|--------|
| Source | `https://github.com/zed-industries/zed` |
| **Zed `rev`** | `371a7d4ba2fd0064b79a0bc67d28e57a906779dc` (2026-08-09) |
| Packages from that rev | `gpui`, `gpui_macos`, `collections`, `util`, `util_macros` (+ their transitive Zed crates) |
| `alacritty_terminal` | `git = "https://github.com/Maidang1/alacritty"` **rev `561594caa275f00914a039356816fe70467a2d44`** (zed `4c129667` + OSC patch) |
| `vte` | `git = "https://github.com/Maidang1/vte"` **rev `94ce0d5fb89392da3b1b243b43e401068fb54937`** (`v0.15.0` + `osc_custom`) |
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

Cargo clones the monorepo once per rev and resolves workspace/path deps inside Zed. You do **not** need to list every transitive crate (`scheduler`, `sum_tree`, `media`, …) in sleipnir’s workspace unless a **local** crate depends on it via `workspace = true`.

Required `[patch.crates-io]` entries live in root `Cargo.toml`:
- `async-process` / `async-task` — aligned with Zed
- `vte = { git = "https://github.com/Maidang1/vte", rev = "…" }` — vte 0.15.0 + `Handler::osc_custom` so OSC 133/9/777 reach `alacritty_terminal` (ADR-0005)

---

## Divergence & upstream watch (frozen fork)

Policy: the VT fork is **frozen** — there is no routine upstream sync. See
[ADR-0007](docs/adr/0007-frozen-vt-fork.md). This section is the price of that
policy: a rebase must never depend on anyone's memory.

**What our forks add on top of their baselines:**

| Fork | Baseline | Our additions |
|------|----------|---------------|
| [Maidang1/vte](https://github.com/Maidang1/vte) `sleipnir-osc-custom` | `alacritty/vte` v0.15.0 | `Handler::osc_custom` hook; unhandled OSC is forwarded to it instead of dropped |
| [Maidang1/alacritty](https://github.com/Maidang1/alacritty) `sleipnir-osc-custom` | zed's alacritty fork `4c129667` | `osc_custom` impl on `Term`; emits `Event::Osc133` / `Event::DesktopNotification` through `EventProxy` (OSC 133 / 9 / 777) |

Nothing else. Any new divergence **must be appended here in the same commit that
introduces it** — that is the only reason this list is trustworthy.

**Watch obligation (every Zed rev bump):** the frozen crates parse *untrusted*
byte streams, so scan upstream `alacritty_terminal` / `vte` history for
**panic / hang / unbounded-allocation fixes in the parser** and cherry-pick those
specifically. Feature and cosmetic commits may be skipped indefinitely; a
malicious-escape-sequence crash or OOM may not.

```bash
# quick scan while doing an upgrade
git -C /path/to/vte       log --oneline v0.15.0..origin/master -i --grep='panic\|overflow\|oom\|unbounded\|hang\|fuzz'
git -C /path/to/alacritty log --oneline 4c129667..origin/master -i --grep='panic\|overflow\|oom\|unbounded\|hang\|fuzz'
```

---

## Upgrade checklist

0. Run the parser crash/OOM scan above (Divergence & upstream watch). Skipping it
   silently converts the freeze policy into a security regression.
1. Pick a new Zed commit:  
   `git -C /path/to/zed rev-parse HEAD`
2. Compare Zed’s `alacritty_terminal` rev with the baseline of our
   [Maidang1/alacritty](https://github.com/Maidang1/alacritty) pin; if Zed moved,
   merge that rev into `sleipnir-osc-custom` and re-apply the OSC patch.
3. In root `Cargo.toml`, replace **all** Zed `rev = "…"` with the new commit (same string everywhere).
4. Build:  
   `cargo build -p sleipnir`
5. Fix **local** breaks only (`terminal`, `sleipnir_ui`, `gpui_platform`, settings). Do not vendor GPUI back unless you must patch upstream.
6. Smoke:  
   `cargo run -p sleipnir` — shell, tabs (`⌘T`), settings reload (`⌘⇧R`).
7. Commit with Zed rev + alacritty rev in the message; update this table.

Optional dry-run against a local Zed checkout (forks only):

```bash
./scripts/upstream-diff.sh /path/to/zed
```

---

## Known sleipnir-specific forks (API break hotspots)

| Area | Why different |
|------|----------------|
| `terminal/src/terminal_settings.rs` | Thin re-export of `sleipnir_settings`, not Zed `Settings` trait |
| `terminal/src/terminal.rs` imports | `sleipnir_settings::TerminalPalette` instead of `task`/`theme` |
| `get_color_at_index` | Takes palette, not full `Theme` |
| Integration tests in terminal | Stubbed / disabled |
| `gpui_platform` | macOS application entry via `gpui_macos` |
| UI | `sleipnir_ui` is original; not Zed `terminal_view` |

---

## When to copy a crate into this repo (exception)

Prefer git pin. Copy a crate into `crates/` only if you need a long-lived private patch to GPUI that cannot live upstream or in a personal fork + alternate `git` URL.

`alacritty_terminal` / `vte` already follow that rule: the OSC 133/9/777 patch lives on
[Maidang1/alacritty](https://github.com/Maidang1/alacritty) and
[Maidang1/vte](https://github.com/Maidang1/vte). Do not recopy those trees here.

---

## Alacritty / vte rev change

If PTY/behavior bugs track alacritty, bump the **fork** `rev` independently (still record both pins). Prefer matching whatever Zed uses at the GPUI `rev` you choose, then merge that baseline into `sleipnir-osc-custom`. Bump `vte` in the same commit whenever the OSC hook changes.
