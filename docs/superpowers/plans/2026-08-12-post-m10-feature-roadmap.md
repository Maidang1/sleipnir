# Post-M10 Feature Roadmap Implementation Plan

| Field | Value |
|-------|--------|
| **Status** | **Docs only** — do not implement until the user explicitly asks (e.g. “implement M12”) |
| **Date** | 2026-08-12 |

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Implement only the milestone the user names** (default: M12). Later milestones (M13–M15) are scoped here for sequencing; expand into a dedicated plan before coding them.

**Goal:** Close the daily-use feature gap versus Kaku / Kitty / Ghostty while preserving Sleipnir’s identity as a clean, native, maintainable macOS terminal (no AI, no shell-suite installer, no graphics protocol).

**Architecture:** Keep the three-layer stack (GPUI → `terminal`/alacritty → `sleipnir_ui` product shell). Prefer wiring existing hooks (`Event::Bell`, `PathLike`, `copy_on_select`, multi-window `AppShell` instances) over new subsystems. Each milestone is independently shippable.

**Tech Stack:** Rust 2024, GPUI (Zed pin), `alacritty_terminal`, JSON settings (`sleipnir_settings`), Cargo unit tests.

**Research input:** [`docs/competitive-research-features.md`](../../competitive-research-features.md)  
**Related design:** [`docs/superpowers/specs/2025-01-20-m11-visual-polish-design.md`](../specs/2025-01-20-m11-visual-polish-design.md)  
**Milestone notes:** [`M11`](../../M11.md) · [`M12`](../../M12.md) · [`M13`](../../M13.md) · [`M14`](../../M14.md) · [`M15`](../../M15.md)

---

## Product principles (do not violate)

1. **macOS-only** native terminal; chrome stays HIG-deferential to content.
2. **No built-in AI**, no auto-install shell plugins, no Kitty graphics / remote-control platform.
3. **Zed-compatible JSON** remains the config model (not Lua/Python).
4. **Ship incrementally:** each Mx leaves a usable app; no “big bang” that only works after M15.
5. **Defend differentiators:** smart paste, live divider reflow, session structure restore, GPUI/Zed pin strategy.

---

## Milestone map

| Milestone | Theme | Priority | Ship alone? |
|-----------|--------|----------|-------------|
| **M11** | Visual polish (cursor fade, URL hover, optional scroll momentum) | P0 polish | Yes |
| **M12** | Daily gaps (font zoom, multi-window, close confirm, path open, bell, copy_on_select UX) | **P0 next** | Yes |
| **M13** | Split professionalism (pane zoom, unfocused dim, optional broadcast) | P1 | Yes |
| **M14** | Shell collaboration (OSC 133, jump prompt, command-finish notify) | P1 | Yes |
| **M15** | Optional differentiators (Quick Terminal, Quick Select) | P1–P2 | Yes |

**Default execution target: M12** (sections “M12 tasks” below).  
M11 may run in parallel if visual-only and not blocking M12.

```
M10 done ──► M11 visual (optional parallel)
         └──► M12 daily gaps  ◄── implement first
                   └──► M13 splits
                          └──► M14 shell integration
                                 └──► M15 differentiators (optional)
```

---

## File structure (M12 focus)

| Path | Responsibility |
|------|----------------|
| `crates/sleipnir_settings/src/sleipnir_settings.rs` | New keys: `confirm_close`, `path_links`, maybe `bell: visual`; keep `copy_on_select` |
| `docs/settings.example.json` | Document new keys |
| `crates/sleipnir/src/main.rs` | Bind `⌘±`/`⌘0`, `⌘N`; open additional windows |
| `crates/sleipnir/src/app_menus.rs` | Menu items: New Window, larger/smaller font |
| `crates/sleipnir_ui/src/sleipnir_ui.rs` | Path open; bell event bubble; font zoom actions on TermView if needed |
| `crates/sleipnir_ui/src/term_element.rs` | Hover hyperlink underline (if bundled with M11/M12) |
| `crates/sleipnir_ui/src/app_shell.rs` | Close confirm dialog; font zoom state; tab visual bell; multi-window is per-shell |
| `crates/sleipnir_ui/src/command_palette.rs` | Register new actions |
| `crates/terminal/src/pty_info.rs` / `terminal.rs` | “Is close dirty?” query (foreground job / child alive) |
| `README.md` | Shortcuts + roadmap rows for M11–M12 when shipped |
| `docs/M12.md` | Milestone notes (create when M12 starts/lands) |

---

## Non-goals (all milestones)

- AI assistant, `#` natural-language commands, error auto-fix LLM
- `kaku init` style shell suite
- Kitty graphics protocol, remote control, Lua config
- Default translucent terminal content (opt-in only, earliest M15/P2)
- Cross-platform
- Per-pane fonts (still deferred from M10 unless explicitly reopened)

---

# M11 — Visual polish (summary; design already exists)

**Spec:** `docs/superpowers/specs/2025-01-20-m11-visual-polish-design.md`

| Item | Acceptance |
|------|------------|
| Cursor ease-in-out blink | Matches ~530ms half-period; resets solid on keystroke; respects `blinking` setting |
| URL / link hover underline | Hovered hyperlink paints underline; no false positives on plain text |
| Scroll momentum (optional) | Wheel scroll feels macOS-native; disable if janky |

**Files:** `term_element.rs`, `sleipnir_ui.rs` only.  
**Do not block M12** if M11 slips.

---

# M12 — Daily gaps (PRIMARY PLAN)

## M12 goals

| # | Feature | User-facing result |
|---|---------|-------------------|
| 1 | Font zoom | `⌘+` / `⌘-` / `⌘0` adjust size at runtime |
| 2 | New OS window | `⌘N` opens a second independent window |
| 3 | Close confirm | Closing pane/tab/window with a live shell asks once |
| 4 | Path open | Cmd-click / open path-like targets in Finder/default app |
| 5 | Bell | `terminal.bell` actually does something (`system` / `visual` / `off`) |
| 6 | copy_on_select UX | Setting visible in settings UI + docs (default stays `false`) |

**Out of M12:** pane zoom, shell integration, Quick Terminal, AI.

---

### Task 1: Settings surface for close confirm + path links + bell visual

**Files:**
- Modify: `crates/sleipnir_settings/src/sleipnir_settings.rs`
- Modify: `docs/settings.example.json`

- [ ] **Step 1: Extend schema**

```rust
// Suggested shapes (names may match serde snake_case JSON)
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmClose {
    #[default]
    Dirty,  // confirm only if process looks "busy"
    Always,
    Never,
}

// TerminalBell already has Off | System — add Visual
// pub enum TerminalBell { Off, System, Visual }

// On root SleipnirSettings (not only terminal.*):
// confirm_close: ConfirmClose  (default Dirty)
// path_links: bool             (default true)
```

Parse from `settings.json` with the same merge pattern as `restore_session`.

- [ ] **Step 2: Unit test load/save**

Add tests in `sleipnir_settings` for:

```json
{
  "confirm_close": "dirty",
  "path_links": true,
  "terminal": { "bell": "visual", "copy_on_select": true }
}
```

- [ ] **Step 3: Update example**

`docs/settings.example.json` documents the three keys + short comment.

- [ ] **Step 4: Commit**

```bash
git add crates/sleipnir_settings docs/settings.example.json
git commit -m "feat(settings): confirm_close, path_links, and visual bell options"
```

---

### Task 2: Runtime font zoom (`⌘+` / `⌘-` / `⌘0`)

**Files:**
- Modify: `crates/sleipnir_ui/src/app_shell.rs` (or window-level state held by AppShell)
- Modify: `crates/sleipnir_ui/src/term_element.rs` (read override when resolving font_size)
- Modify: `crates/sleipnir/src/main.rs` (key bindings)
- Modify: `crates/sleipnir/src/app_menus.rs` (View menu)
- Modify: `crates/sleipnir_ui/src/command_palette.rs` (actions)

**Design:**

- Store `font_size_override: Option<Pixels>` on `AppShell` (window-scoped; not written to disk by default).
- Effective size = `override.unwrap_or(settings.font_size.unwrap_or(14px))`, clamped e.g. `8..=72`.
- `IncreaseFontSize` / `DecreaseFontSize` step by `1.0` pt; `ResetFontSize` clears override.
- All panes in the window share the override (M10 deferred per-pane fonts stay deferred).

- [ ] **Step 1: Declare actions**

In the existing `actions!` module used by menus (e.g. `app_menus.rs` / `sleipnir_ui` exports):

```rust
// IncreaseFontSize, DecreaseFontSize, ResetFontSize
```

- [ ] **Step 2: Wire AppShell handlers + TermElement read path**

`TermElement::prepaint` currently:

```rust
let font_size = settings.font_size.unwrap_or(px(14.)).max(px(8.));
```

Change to read effective size from a global or entity-provided override (AppShell sets a `FontSizeOverride` GPUI global, or pass via `TermView` from shell). Prefer **GPUI global set by AppShell** only if multi-window isolation works; otherwise store on `TermView` when created and update all leaves on zoom.

Recommended: **AppShell field + on zoom, walk pane tree and set `TermView.font_size_override`**.

- [ ] **Step 3: Key bindings**

```rust
bind_both("cmd-=", IncreaseFontSize),      // and cmd-+ if GPUI distinguishes
bind_both("cmd-plus", IncreaseFontSize),
bind_both("cmd-minus", DecreaseFontSize),
bind_both("cmd-0", ResetFontSize),
```

- [ ] **Step 4: Manual accept**

Run: `cargo run -p sleipnir`  
Press `⌘+` several times → grid reflows larger; `⌘0` restores settings size; new tab inherits current override.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat: runtime font zoom with cmd-plus/minus/0"
```

---

### Task 3: New OS window (`⌘N`)

**Files:**
- Modify: `crates/sleipnir/src/main.rs` (`open_window` helper extract)
- Modify: `crates/sleipnir_ui/src/app_shell.rs` (action handler if shell-level)
- Modify: `crates/sleipnir/src/app_menus.rs` (Shell → New Window)
- Modify: `crates/sleipnir_ui/src/command_palette.rs`

**Design:**

- Extract current `cx.open_window(... AppShell::new ...)` into `fn open_sleipnir_window(cx: &mut App)`.
- `NewWindow` action calls that helper (no shared state between shells except settings globals).
- Session restore remains **per last-focused window** for M12 (document limitation); multi-window session is M12 non-goal unless trivial.

- [ ] **Step 1: Extract window open helper from `main.rs`**

Today (~L143–156):

```rust
cx.open_window(WindowOptions { ... }, |window, cx| {
    cx.new(|cx| AppShell::new(window, cx))
});
```

Make reusable; call once at startup and from `NewWindow`.

- [ ] **Step 2: Bind `cmd-n` → `NewWindow`**

Also File/Shell menu item. Avoid colliding with terminal “any key” — bind in `AppShell` + `Terminal` like other chrome actions.

- [ ] **Step 3: Manual accept**

`⌘N` → second window; each has independent tabs; quitting last window exits app (platform default).

- [ ] **Step 4: Commit**

```bash
git commit -m "feat: open additional OS windows with cmd-n"
```

---

### Task 4: Close confirmation when dirty

**Files:**
- Modify: `crates/terminal/src/pty_info.rs` and/or `terminal.rs` — expose `is_busy()` / `has_child_process()`
- Modify: `crates/sleipnir_ui/src/app_shell.rs` — intercept close tab/pane/window
- Possibly: small modal component in `app_shell.rs` (reuse update-dialog patterns)

**Design:**

```text
confirm_close = dirty | always | never

dirty  → confirm if PTY shell still alive OR foreground pgid has non-shell job
always → always confirm for pane/tab/window close
never  → current behavior
```

Use existing process signaling paths carefully: **query before** `terminate_processes_with_grace_period`.

UI: centered modal “Close this pane? A process is still running.” · **Cancel** · **Close**.

- [ ] **Step 1: Add pure `fn terminal_looks_busy(term: &Terminal) -> bool`**

Unit-test with mocks if possible; otherwise document manual test only and keep function thin around `PtyProcessInfo`.

- [ ] **Step 2: Gate `close_pane` / `close_tab` / window close**

Find `⌘W` path in `app_shell.rs` (~comment at L760). Insert confirm gate.

- [ ] **Step 3: Manual accept**

Start `sleep 1000` → `⌘W` → dialog; Cancel keeps session; Close kills as today. Idle shell with `never` or clean shell under `dirty` closes without dialog (define: idle shell-only = not dirty to avoid nagging on every tab).

**Policy decision (lock in):**

- **Idle interactive shell only → not dirty** (like iTerm “if processes other than shell”).
- Foreground job (sleep, npm, vim) → dirty.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat: confirm close when a foreground process is running"
```

---

### Task 5: Open path-like navigation targets

**Files:**
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs` (`open_navigation_target`, ~L568–580)
- Possibly: small pure helper module for `file.rs:12:3` parsing
- Settings: `path_links` (Task 1)

**Design:**

```rust
// Replace M3 ignore branch:
MaybeNavigationTarget::PathLike(path) if path_links_enabled => {
    if let Some(url) = resolve_path_like(path) {
        // Prefer file URL or editor scheme; fallback: open parent in Finder via `open`
        cx.open_url(&url);
    }
}
```

Resolution rules:

1. Strip optional `file://` (already partly handled in terminal).
2. Split `path:line:col` / `path:line` suffixes.
3. Join with `working_directory` if relative.
4. If path exists as file → `open` file (or `file://` absolute URL).
5. If path exists as dir → open dir.
6. If missing → log + no-op (no crash).
7. Guardrails (Kaku-inspired): skip targets that look like `ident.ident()` method calls when path doesn’t exist on disk.

- [ ] **Step 1: Pure unit tests for path parsing**

```rust
#[test]
fn splits_line_column_suffix() {
    assert_eq!(parse_path_line_col("src/main.rs:10:2"),
        ("src/main.rs", Some(10), Some(2)));
}
```

- [ ] **Step 2: Implement `open_navigation_target` PathLike branch**

Honor `path_links == false` → keep ignore.

- [ ] **Step 3: Manual accept**

`echo /tmp` and `touch /tmp/sleipnir-test.txt` → cmd-click opens. Relative path from a project cwd works.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat: open path-like terminal targets on click"
```

---

### Task 6: Make bell real (`system` / `visual` / `off`)

**Files:**
- Modify: `crates/sleipnir_settings` — `TerminalBell::Visual` if not present
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs` — `Event::Bell` (~L189)
- Modify: `crates/sleipnir_ui/src/app_shell.rs` — tab chip flash state

**Design:**

| Setting | Behavior |
|---------|----------|
| `off` | no-op (current effective behavior) |
| `system` | `NSBeep` / GPUI platform beep if available |
| `visual` | flash active tab chip ~300ms (and optional window attention) |

Bubble: `TermView` emits `TermViewEvent::Bell` → `AppShell` applies visual; system beep can run in TermView or shell.

- [ ] **Step 1: Handle `Event::Bell` with settings branch**
- [ ] **Step 2: Visual flash on tab strip**
- [ ] **Step 3: Manual accept** — `printf '\a'` with each setting
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: system and visual terminal bell"
```

---

### Task 7: copy_on_select discoverability

**Files:**
- Modify: settings panel UI in `app_shell.rs` (theme picker panel — add a toggles section)
- Modify: `README.md` Config / Features
- No default flip (`copy_on_select` stays `false` unless product decides otherwise)

- [ ] **Step 1: Add checkbox “Copy on select” bound to settings write + reload path**
- [ ] **Step 2: README one-liner under Config table**
- [ ] **Step 3: Manual accept** — toggle on, drag-select text, paste elsewhere
- [ ] **Step 4: Commit**

```bash
git commit -m "feat: expose copy_on_select in settings UI and docs"
```

---

### Task 8: M12 docs + README roadmap

**Files:**
- Create: `docs/M12.md` (milestone notes, same style as M9/M10)
- Modify: `README.md` Roadmap table
- Modify: `CHANGELOG.md` Unreleased section

- [ ] **Step 1: Write `docs/M12.md`** with shortcuts and settings keys
- [ ] **Step 2: Mark M12 ✅ when all Task 1–7 acceptance passes**
- [ ] **Step 3: Commit**

```bash
git commit -m "docs: M12 daily-gaps milestone notes and changelog"
```

---

### M12 acceptance checklist (release gate)

- [ ] `⌘+` / `⌘-` / `⌘0` change and reset font size; PTY reflows without crash
- [ ] `⌘N` opens independent window with working shell
- [ ] Dirty close (`sleep 999`) prompts; Cancel preserves pane
- [ ] Path click opens existing file; bad paths don’t panic
- [ ] `printf '\a'` respects `bell` setting
- [ ] Settings UI can toggle `copy_on_select`
- [ ] `cargo test -p sleipnir_settings -p sleipnir_ui -p terminal` green (or package-equivalent)
- [ ] `cargo build -p sleipnir` release-clean

---

# M13 — Split professionalism (scoped; plan later)

| Feature | Acceptance sketch | Effort |
|---------|-------------------|--------|
| **Pane zoom** | Action toggles active leaf full-content; second toggle restores tree ratios | M |
| **Unfocused split dim** | Non-active panes paint at ~0.7–0.85 opacity overlay | S |
| **Broadcast input (optional)** | Toggle mode; keystrokes fan-out to selected panes; banner visible | M |

**Files:** `pane_tree.rs`, `app_shell.rs`, `term_element.rs`.  
**Prerequisite:** M12 stable.  
**Write full task plan before coding.**

---

# M14 — Shell collaboration (scoped; plan later)

| Feature | Acceptance sketch | Effort |
|---------|-------------------|--------|
| **OSC 133 detect** | Parse prompt start/end markers from shell integration (opt-in inject or detect) | L |
| **Jump to prev/next prompt** | Key bindings scroll scrollback to markers | M |
| **Notify on command finish** | If unfocused and command longer than N seconds → system notification | M |

**Non-goal:** shipping a zsh plugin suite. Prefer detect-first, optional inject script in docs only.

**Prerequisite:** M12; M13 optional.

---

# M15 — Optional differentiators (scoped; plan later)

| Feature | Acceptance sketch | Effort |
|---------|-------------------|--------|
| **Quick Terminal** | Global hotkey; dropdown panel; doesn’t quit main app | L |
| **Quick Select** | Overlay labels on URLs/paths; type to copy/open | L |
| **Background opacity (opt-in)** | Setting only; default opaque | M |

Ship only if M12–M14 leave capacity and product still wants a “headline” feature.

---

## Risk register

| Risk | Mitigation |
|------|------------|
| Multi-window + session file races | M12: single writer / last-window-wins; document |
| Path open false positives | Existence check + reject method-call patterns |
| Close confirm too naggy | Idle shell = not dirty |
| Font zoom vs settings reload | Reload resets override or reapplies settings base then keeps override — **pick: reload clears override** |
| GPUI beep/API gaps | Visual bell always available as fallback |
| Scope creep into AI/shell suite | Reject in PR review using Non-goals section |

---

## Verification commands (every milestone)

```bash
cargo test -p sleipnir_settings -p sleipnir_ui -p terminal
cargo build -p sleipnir
cargo run -p sleipnir
# manual: checklist for the milestone
```

---

## Execution order recommendation

1. **M12 Tasks 1 → 7 → 8** in order (settings first so features have knobs).
2. **M11** in parallel only if a second engineer/agent is free.
3. After M12 ships: write **M13 plan** from the scoped section, then implement.
4. Do **not** start M15 before M14 is either done or explicitly skipped with a product decision.

---

## Handoff

**Plan saved to:** `docs/superpowers/plans/2026-08-12-post-m10-feature-roadmap.md`

**Research companion:** `docs/competitive-research-features.md`

To implement: say **“implement M12”** (or “implement this plan” for M12 only).  
For M13+, ask for a dedicated expanded plan first.
