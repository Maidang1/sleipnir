<div align="center">

<img src="resources/appicon_preview.png" alt="Sleipnir app icon" width="160" height="160" />

# Sleipnir

**A fast, native terminal emulator for macOS — GPU-rendered, tab- and split-aware.**

Built on [GPUI](https://gpui.rs) (the UI framework behind [Zed](https://github.com/zed-industries/zed))
with a forked terminal backend.

[Features](#features) · [Install](#install) · [Config](#config) · [Shortcuts](#shortcuts)

</div>

---

## About

Sleipnir is a standalone macOS terminal that renders on the GPU through GPUI, so
scrolling and redraw stay smooth even under heavy output. It ships with a real
host PTY, IME support, multi-tab and split panes, multi-window sessions,
adaptive theming that follows the system appearance, and a file-manager-friendly
clipboard that turns pasted images into quoted paths.

Prebuilt downloads are macOS `.dmg` / `.zip` on
[GitHub Releases](https://github.com/Maidang1/sleipnir/releases). Windows and
Linux are not supported.

The name comes from Norse myth — Odin's eight-legged steed, the fastest horse in the
nine worlds. The app icon abstracts that into a minimal horse-head mark over a terminal
prompt.

## Features

- **GPU rendering** — smooth scrollback and redraw via GPUI (Metal); ease-in-out cursor blink.
- **Tabs, splits & panes** — side tab rail grouped by git workspace (or `tab_placement: top` for the historical strip); each row shows the title plus a branch subtitle and a dirty `+N −M` mark; split right/down, jump tabs, move focus, drag tabs to reorder or drop onto the terminal to detach into a new window; drag a background tab onto the visible panes to merge it as a split; drag a pane grip onto the tab list to extract it as a tab; pane zoom and unfocused dim. Known coding agents show a letter mark on the tab. Mark Tab as Seen clears Attention without deleting Run records.
- **Multi-window** — `⌘N` opens an independent window with its own tabs and shells.
- **Font zoom** — `⌘+` (and `-` / `0`) resize the grid for the current window (not persisted).
- **Adaptive themes** — Catppuccin flavors plus Tokyo Night, Nord, Gruvbox, Solarized,
  GitHub Dark/Light, Dracula, One Dark; `auto` follows the system light/dark appearance.
  Extra palettes go in `themes.json` in the config dir (`"theme": "kanagawa"`, see
  `docs/themes.example.json`).
- **Smart paste** — paste an image to get a shell-quoted temp-file path; paste Finder
  selections as quoted paths; force text-only paste when you need it.
- **Zed-compatible config** — reuse your `terminal.*` settings; hot-reload with `⌘⇧R`.
- **vi mode** — keyboard-driven selection and navigation.
- **Accessibility** — the terminal exposes the visible screen as a read-only accessible value (VoiceOver can read the current output), like Ghostty's read-only AX.
- **Session restore** — tabs, splits, and working directories survive relaunch.
- **Command palette** — discover actions with `⌘⇧K`; optional key binding overrides in settings. `keybinding_preset: tmux` adds `ctrl-b` tab/pane chords. **Pane Facts** shows the focused pane's directory, process tree, and listen ports.
- **Find in scrollback** — `⌘F` search with match highlights, regex (`.*`) and match-case (`Aa`) toggles; export scrollback to a file via **Shell → Export Scrollback…** (opens in your default editor).
- **Path links & bell** — ⌘-click paths open in the default app; hover shows a URL/path preview tooltip; optional system/visual bell.
- **Close confirm** — prompt when a non-shell job is running (`confirm_close`: dirty/always/never).
- **Shell collaboration** — OSC 133 prompt jump; new tabs/splits inherit the active pane's working directory; optional notify when a long command finishes
  while unfocused (macOS notification).
- **Quick Terminal / Quick Select** — open a spare window fast; link-oriented mode.

## Install

Sleipnir is **macOS 14+ only**. Latest GitHub Release:

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

To **run** a release build:

- macOS 14.0+ (Sonoma)

To **build** from source, also:

- Rust **1.95.0** (see `rust-toolchain.toml`)
- Xcode + Metal Toolchain (`xcodebuild -downloadComponent MetalToolchain`)

The crate fails to compile on Windows and Linux.

## Build & run

```bash
cargo run -p sleipnir
# binary: target/debug/sleipnir
```

## Config

Zed-compatible `terminal.*` keys plus Sleipnir extensions. Settings live in
`~/.config/sleipnir/settings.json` and session restore in `session.json`.
Default font is **Menlo**.

| Key | Meaning |
|-----|---------|
| `theme` | `auto` / `mocha` / … (see example); `auto` follows system appearance; any name from `themes.json` also works |
| `custom_theme` | Optional hex palette (`background`/`foreground`/`ansi`…) that overrides `theme` |
| `restore_session` | Restore tabs/splits/cwd on launch (default `true`) |
| `confirm_close` | `dirty` / `always` / `never` — prompt before closing a busy pane (default `dirty`) |
| `path_links` | Open path-like targets on ⌘-click (default `true`) |
| `key_bindings` | Extra chords (`{ "key": "cmd-alt-t", "action": "new_tab" }`). Actions: `new_tab`, `close_tab`, `next_tab`, `prev_tab`, `split_right`, `split_down`, `new_window`, `open_settings`, `reload_settings`, `cycle_theme`, `find`, `toggle_command_palette`, `increase_font_size`, `decrease_font_size`, `reset_font_size`, `toggle_pane_zoom`, `toggle_broadcast`, `jump_prev_prompt`, `jump_next_prompt`, `toggle_quick_select`, `open_quick_terminal`, `export_scrollback`, `check_for_updates`, `clear_run_ledger`, `toggle_run_ledger`. Optional `context`: `AppShell` / `Terminal`. Restart to apply. |
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
| `tab_placement` | `side` (default) left rail grouped by git workspace / `top` historical strip |
| `sidebar_width` | Left rail width in px, 160–320 (default `200`) |
| `agent_icons` | Letter monograms for known coding-agent processes (default `true`) |
| `control_surface` | Bind the local control socket (default `false`). Also on with `SLEIPNIR_CONTROL=1` |
| `show_tray_icon` | Menu-bar Attention item (default `true`). Dock badge is independent |
| `terminal.inject_osc133` | Inject OSC 133 A/B/C/D into zsh/bash/fish (default `true`; was `false`) |

Open the in-app theme picker with `⌘,`, or edit the file and reload
with `⌘⇧R` (key binding overrides apply on next launch).
See [`docs/settings.example.json`](docs/settings.example.json).

## Paste

| Clipboard | Paste behavior (`⌘V` / `⌃⇧V`) |
|-----------|----------------|
| Image (screenshot, etc.) | Write to a temp file; paste a quoted absolute POSIX path |
| Finder file paths | Paste space-separated quoted paths |
| Text | Normal paste (bracketed paste when the app enables it) |

Force **text-only** paste (skip image→path conversion) with `⌃⌘V`.

## Shortcuts

| Action | Shortcut |
|--------|----------|
| Copy | `⌘C` / `⌃⇧C` |
| Paste (image → path) | `⌘V` / `⌃⇧V` |
| Paste text only | `⌃⌘V` |
| Select all | `⌘A` |
| Clear | `⌘K` |
| New tab / close pane | `⌘T` / `⌘W` |
| New window | `⌘N` |
| Jump to tab N | `⌘1`…`⌘9` |
| Split pane right / down | `⌘D` / `⌘⇧D` |
| Move focus between panes | `⌘⌥←↑↓→` |
| Next / previous tab | `⌃Tab` / `⌃⇧Tab` |
| Next / previous tab (alt) | `⌘⇧]` / `⌘⇧[` |
| Increase / decrease / reset font | `⌘+` `⌘=` `⌘-` `⌘0` |
| Toggle pane zoom | `⌘⇧Enter` |
| Toggle broadcast | `⌘⇧B` |
| Jump prev / next prompt | `⌘⇧↑` / `⌘⇧↓` |
| Quick Select | `⌘⇧O` |
| Quick Terminal | `⌘⇧N` |
| Settings | `⌘,` |
| Reload settings | `⌘⇧R` |
| Cycle theme | `⌘⇧P` |
| Command palette | `⌘⇧K` |
| Find in scrollback | `⌘F` |
| Next / previous find match | `⌘G` / `⌘⇧G` |
| Find: match case / regex | `⌥⌘C` / `⌥⌘R` |
| Check for updates | `⌘⇧U` |
| Scroll page | `⌘↑` / `⌘↓` |
| Scroll line | `⇧↑` / `⇧↓` |
| Line start / end | `⌘←` / `⌘→` |
| Word back / forward | `⌥←` / `⌥→` |
| Clear line | `⌘⌫` |
| Delete to end of line | `⌘⌦` |
| Character palette | `⌃⌘Space` |
| Toggle vi mode | `⌃⇧Space` |
| Quit | `⌘Q` |

Scroll shortcuts are ignored on the alternate screen (full-screen TUI apps).

## Auto-update

Sleipnir can update itself from [GitHub Releases](https://github.com/Maidang1/sleipnir/releases).

- Open the update dialog via **Sleipnir → Check for Updates…** (`⌘⇧U`).
  Sleipnir does **not** check for updates automatically on launch.
- If a newer version is found, choosing **Download & Install** fetches the
  `Sleipnir-<ver>-macos.zip` artifact and verifies it against the published
  `.zip.sha256` sidecar before staging. Because CI builds are ad-hoc signed (no Apple
  Developer certificate), this SHA-256 check is the integrity guarantee — the download is
  rejected on any mismatch.
- **Restart & Update** swaps the running `.app` in place and relaunches. If the bundle
  lives somewhere the app can't write (e.g. a protected `/Applications` install owned by
  another user), it falls back to opening the releases page for a manual install.

## Scope

Sleipnir is built for **people who run coding agents in a terminal**: the human is
the user, the agent is the workload. That sets the boundaries:

- **No built-in AI.** No model calls, no chat panel, no API-key management. Sleipnir
  is the terminal your agent runs *in*, not an agent. If you want to talk to an AI
  inside your terminal, use [Warp](https://warp.dev) or [Wave](https://waveterm.dev).
  ([ADR-0008](docs/adr/0008-no-builtin-ai.md))
- **No persisted scrollback.** Session restore brings back tabs, splits and working
  directories — never terminal output, because output routinely contains tokens and
  passwords. Use **Shell → Export Scrollback…** when you want to keep a transcript.
- **No process restore.** Restarting does not resurrect running commands; that is
  what `tmux` / `zellij` are for, and Sleipnir does not reimplement them.
- **No plugin system.** Configuration, key bindings, and piping to external commands
  are the extension points.

## Status

Shipped through M15 — see [CHANGELOG](CHANGELOG.md). Still open: scrollback
byte-budget / compression, a fuller VoiceOver tree, and kitty graphics
(tracked, not implemented: [ADR-0004](docs/adr/0004-kitty-graphics-track-not-implement.md)).

Performance is measured, not asserted: methodology in
[`scripts/bench/README.md`](scripts/bench/README.md), numbers in
[`scripts/bench/results.md`](scripts/bench/results.md). Input latency and a
same-machine comparison against Ghostty / kitty / Alacritty are still open, so treat
the throughput figures as internal baselines rather than competitive claims.

## Upstream

The GPUI stack is **not** vendored. Root `Cargo.toml` pins `zed-industries/zed` at a
fixed `rev` (`gpui`, `gpui_macos`, `collections`, `util`, …). Local forks:
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
The workflow builds and tests on `macos-latest` and publishes the `.dmg`, `.zip`,
and `.zip.sha256` to the GitHub Release.

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
