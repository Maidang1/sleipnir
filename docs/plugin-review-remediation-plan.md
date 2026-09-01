# Plugin programme remediation plan

**Scope:** the structural findings from the deep code-quality audit of the resident-plugin
branch (`origin/main...HEAD`, 2 commits, 80 files, +17,828 / −252).

**Status:** proposed. Two items are treated as blockers; the rest are sequenced behind them.

## Baseline, measured

Every number below was measured on the branch head, not estimated. Re-measure before
claiming an item is done.

| Metric | Baseline | Command |
| --- | --- | --- |
| `app_shell/mod.rs` | 3040 lines (was 2074 on `origin/main`) | `wc -l` |
| `accumulate_wheel` production call sites | 1 | `grep -n 'accumulate_wheel(' crates/terminal/src/terminal.rs` |
| `apply_pixel_delta` production call sites | 0 (13 test-only) | `grep -rn 'apply_pixel_delta'` |
| `viewport.row` production readers | 0 | `grep -rn '\.row\b' \| grep viewport` |
| `render_*_granted` helpers | 3 | `grep -rn 'fn render_.*_granted'` |
| `mark_plugin_stale` / `mark_missing_stale` definitions | 2 each (Panel, Block) | `grep -rn 'fn mark_plugin_stale'` |
| `action_at` definitions | 2 | `grep -rn '^pub fn action_at'` |
| Tests passing | 626 | `cargo test --workspace` |
| Clippy warnings | 0 | `cargo clippy --workspace --all-targets` |

**Per-stage gate, no exceptions:** `cargo build --workspace` + `cargo clippy --workspace
--all-targets` (zero warnings) + `cargo test --workspace` (**≥ 626 passing**; the count must
not drop). Every stage is independently committable and independently revertable.

## Correction to the original review

The first pass of this audit recommended "delete `accumulate_wheel`, call
`apply_pixel_delta` instead." **That advice was wrong and would invert scroll direction.**
The two functions do not agree on sign or coordinate space:

| | Return value | `row` field |
| --- | --- | --- |
| `accumulate_wheel` (live) | `spilled` moves **with** `display_offset` — `offset += 1` yields `spilled += 1`, i.e. positive means further into history | not maintained |
| `apply_pixel_delta` (dead) | `spilled = new_line - old_line`, an **absolute-line** delta | maintained in **absolute lines** |

`spilled` flows into `Scroll::Delta(delta_lines)`, whose semantics are pinned by
`jump_prompt` (`terminal.rs:1730-1735`) as a **`display_offset` delta**
(`target_offset - now`). Absolute lines and `display_offset` run in **opposite**
directions, because `top_abs = history - offset`.

The finding that `viewport.row` has conflicting writers still holds, and is worse than
first stated — three parties disagree:

- `terminal.rs:2613` writes a `display_offset`.
- `jump_to_anchor` (`terminal.rs:1607`, `1738`) writes an absolute line.
- `row_geometry/src/tests.rs` asserts absolute-line behaviour.

Nothing reads the field, so the contradiction cannot surface. ADR-0018 anticipated exactly
this: *"any code path that reads one without the other is wrong."*

So P0-a is **not** a simple deletion. It must first fix the meaning of `row`, then route the
single surviving implementation through one explicit conversion at the boundary.

## Guiding rules

1. **Behaviour-preserving by default.** Only P2-a intentionally changes visible behaviour.
2. **Lock behaviour before touching sign- or coordinate-sensitive code.** Add assertions
   that pin current output, then change the implementation.
3. **Conversions get one named home.** A sign/coordinate conversion must be a single
   documented function, never inlined at call sites — otherwise the bug just moves.
4. **Acceptance is quantified.** Use the metric table, not an impression.

---

## Stage 1 — Blockers

### P0-a Make `ViewportPosition` the single scroll source of truth

Addresses finding §2. First, because it fixes the meaning of `row`, which later geometry
work depends on.

**Step 1 — pin current behaviour without changing it.**
Add table-driven assertions in `crates/terminal/src/row_map.rs` covering the real output of
today's `accumulate_wheel`: positive and negative deltas, walking a tall Block, `offset`
clamping at top and bottom, and non-finite input. Tests only. Must be green before Step 2 —
this is the safety net for everything that follows.

**Step 2 — fix the meaning of `row`.**
Define `ViewportPosition::row` as an **absolute line**, matching `jump_to_anchor`,
`row_geometry/src/tests.rs`, and the `RowGeometry` coordinate system of ADR-0018. State this
in the doc comment, and state explicitly that it is **not** a `display_offset`.

**Step 3 — let the one implementation maintain both fields.**
Keep `apply_pixel_delta` as the sole implementation (better algorithm, no bounded loop).
The caller takes on one explicit conversion:

- before the call, derive `top_abs` from `history_size` and `display_offset`, and set `pos.row`;
- after the call, convert the absolute-line delta into a `display_offset` delta (negate)
  before handing it to `Scroll::Delta`.

That conversion is one named, documented helper. Not inlined.

**Step 4 — delete the duplicate.**
Remove `accumulate_wheel` and the patch-up assignment at `terminal.rs:2613`. Repoint the
Step 1 assertions at the new path; output must match item for item.

**Acceptance**

- [ ] `accumulate_wheel` call sites = 0; the `for _ in 0..10_000` loop is gone.
- [ ] `viewport.row` has at least one production **reader**; it is no longer write-only.
- [ ] Exactly one meaning of `row` across the workspace.
- [ ] Step 1 assertions pass against the new path → direction and feel unchanged.
- [ ] Manual check: wheel up/down, across a tall Block, at top and bottom of scrollback,
      and alt-screen toggle — direction and damping identical to before.

### P0-b Extract `app_shell/plugins.rs`

Addresses finding §1. Pure relocation, **zero logic change**.

`app_shell/` already has ten submodules following one convention: `use super::AppShell;`
plus a focused `impl AppShell`. The last six commits established it
(`refactor(ui): extract the palette and find bar from the shell`, `extract pane layout and
dragging`, `extract settings and panel rendering`, `delete incidental complexity in the
shell`). This PR bypassed it. Follow it.

Move into `app_shell/plugins.rs`:

`poll_plugin_events`, `poll_plugin_inbound`, `apply_panel_render`, `apply_block_render`,
`apply_chrome_status`, `mark_blocks_stale`, `mark_missing_blocks_stale`,
`set_all_blocks_frozen`, `insert_panel_leaf`, `handle_host_call`, `terminal_and_panel_keys`,
`execute_open_pane`, `start_resident_plugins`, `connect_resident`, `invoke_v2_command`,
`run_plugin_command`, `start_plugin_command`, `invoke_plugin_command`,
`refresh_plugin_commands`, `run_plugin_contribution`, `kill_plugin`,
`toggle_plugin_monitor`, `close_plugin_monitor`, `approve_plugin_consent`,
`deny_plugin_consent`, plus the free function `run_event_to_host` and its test.

Adjust field visibility to `pub(super)` where the existing submodules already do so.
**Visibility only — no type or signature changes.** If `mod.rs` still exceeds 2000 lines,
move the consent overlay state (`PluginConsentPending`, `PluginConsentKind`) as well.

**Acceptance**

- [ ] `wc -l crates/sleipnir_ui/src/app_shell/mod.rs` **< 2000**.
- [ ] `grep -c 'plugin\|Plugin' mod.rs` materially lower than 260.
- [ ] `git diff --stat` shows the method bodies as a pure move (insertions ≈ deletions).
- [ ] Test count and clippy output unchanged.

---

## Stage 2 — Pure deletions (mutually independent, parallelizable)

### P1-a Delete the `granted` round-trip

Addresses finding §3. `has_grant` already returns `bool` (`plugin_runtime.rs:271`), but all
three mount points synthesize a throwaway `&[Capability]` and call a helper whose body is
one `contains` — recovering the same `bool`. Double-shadowing `granted` hides it.

Delete `render_panel_granted`, `render_block_granted`, `render_status_granted` and their
three tests. Pass the `bool` from `has_grant(...)` directly.

**Acceptance**

- [ ] `render_.*_granted` definitions = 0.
- [ ] Each of the three call sites goes from ~6 lines to 1.
- [ ] `DeniedGrant` branch assertions retained — the denial path stays covered.

### P1-b Converge registry duplication

Addresses finding §4.

- Delete `plugin_block::action_at`; call `plugin_panel::action_at`. It already returns
  `plugin_panel::PanelAction`, so it depends on the module it duplicates.
- Converge the byte-identical `mark_plugin_stale` / `mark_missing_stale` in `PanelRegistry`
  and `BlockRegistry` onto one shared implementation (`Staleable` trait or a generic
  `SurfaceRegistry<K>`, whichever is the smaller change). Note `ChromeRegistry::sync_live`
  is a **different** policy (`retain` + drop, not mark-stale) — leave it alone.
- Have `poll_plugin_inbound` iterate mount points instead of naming each one twice.

**Acceptance**

- [ ] `fn mark_plugin_stale` definitions 2 → 1; same for `mark_missing_stale`.
- [ ] `action_at` definitions 2 → 1.
- [ ] Staleness behaviour tests green; `ChromeRegistry` semantics unchanged.

### P1-c Delete the no-op and the redundant state

Addresses finding §6.

- Remove `sleipnir_ui.rs:415-417`. `live` is built from `geometry_blocks()`, which maps over
  `self.blocks`' own values, so it is the full key set by construction and `retain_live` can
  never remove anything. Dead code that reads like a lifecycle guarantee.
- Remove `TermView::last_block_history`. It re-derives the history shrink that
  `Terminal::last_history_size` already tracks, and `Terminal` already calls
  `row_geometry.rebase_after_history_shrink` at `terminal.rs:2182`. History belongs to the
  terminal; drive the rebase from there.

**Acceptance**

- [ ] `last_block_history` references = 0.
- [ ] A new test covers Block rebase/drop after history shrink, so the deletion is proven
      safe rather than assumed safe.

---

## Stage 3 — Quality

### P2-a Fix the silently-dropped widget kinds

Addresses finding §5. **The only intentional behaviour change in this plan.**

Two painters render the same `Layout` and disagree:

| Kind | Panel (`app_shell/layout.rs`) | Block (`term_element.rs`) |
| --- | --- | --- |
| `Spark` | rendered | `=> return`, silently dropped |
| `Text { bold }` | `font_weight(BOLD)` | `bold` ignored |

`sleipnir_widget::layout` lays `Spark` out and reserves its cells, so a Block containing one
renders **correctly-sized blank space** — invisible failure, nothing logged. `Unknown =>
"[?]"` and `Truncated` show the schema's intent is that unrenderable content stays visible.

In `term_element.rs::paint_laid_node`, render `Spark` and honour `bold`, and drop the
`_ => palette.foreground` catch-all so the match is exhaustive.

**Acceptance**

- [ ] Both painters support an identical set of `LaidOutKind` variants.
- [ ] Adding a variant makes `cargo build` fail (verify temporarily, then revert).

### P2-b Move RFC3339 formatting to `plugin_grants`

Addresses finding §7. `plugin_runtime.rs:337-368` hand-rolls leap years, a month table, and
epoch day-walking to format `GrantRecord.granted_at` — a field defined, documented, and
tested in `plugin_grants`. As it stands, `plugin_grants` can produce a record it cannot
timestamp.

Move `rfc3339_now`, `rfc3339_from_unix`, and `is_leap` into `plugin_grants` beside the field,
ideally behind a `GrantRecord::new(...)` that stamps itself so the invariant cannot be
bypassed. Migrate the epoch/leap-year tests with them. (No `chrono`/`time` in the workspace,
so hand-rolling is defensible; the location is not.)

**Acceptance**

- [ ] Zero calendar arithmetic in `sleipnir_ui`.
- [ ] `plugin_grants` can produce a valid `granted_at` on its own.

### P2-c Remove the guard test's ordering dependency

Addresses finding §8. `terminal.rs:2914` locates the end of `process_terminal_event` by
searching for the literal `"\n    pub(crate) fn pointer_map("`, hard-coding that
`pointer_map` is the *next* function. The bug it guards is real — re-entering
`lock_unfair` under a held lock, a 100% CPU hang reproducible by opening Settings — and
source-scraping is an existing repo idiom (8 other sites), so the technique is not the issue.

Preferred: have the locked path take `&AlacrittyTerm` so `pointer_map(&self)` is unreachable
where a guard is already held. The hazard becomes unrepresentable and the test can be
deleted.

Fallback, if the type change is too invasive: scan the whole file for `pointer_map()`
occurrences excluding the two accessors, instead of depending on declaration order.

**Acceptance**

- [ ] Swapping the declaration order of `process_terminal_event` and `pointer_map` does not
      change the test result (today it panics).

---

## Sequencing and risk

| Stage | Item | Risk | Mitigation |
| --- | --- | --- | --- |
| 1 | P0-a scroll unification | **High** — sign/coordinate error can invert scroll direction | Pin behaviour with assertions first; one named conversion; manual scroll-feel check |
| 1 | P0-b module extraction | Low — pure relocation | Diff must read as a move; visibility-only changes |
| 2 | P1-a / P1-b / P1-c | Low — pure deletion | Keep Denied / staleness / rebase assertions |
| 3 | P2-a | Medium — the one behaviour change | Exhaustive match, no catch-all; compare variant-by-variant against the Panel painter |
| 3 | P2-b / P2-c | Low | Migrate tests alongside; prove P2-c by swapping declaration order |

Order: **P0-a → P0-b → Stage 2 → Stage 3.**

## Open questions

1. **`ViewportPosition::row` is defined as an absolute line.** This is the premise of P0-a,
   inferred from `jump_to_anchor`, the `row_geometry` tests, and ADR-0018. But
   `terminal.rs:2613` writes a `display_offset`. If `display_offset` was the intent, the
   conversion in P0-a Step 3 reverses direction. Confirm before starting.
2. **Whether P2-a belongs in this pass.** It is the only item that changes visible behaviour
   (a Spark inside a Block goes from blank to rendered). If this pass must be strictly
   behaviour-preserving, split it into its own commit or defer it.

## Notes for whoever picks this up

- The underlying architecture is sound. `row_geometry` (integer cell-row space, `y_for`/`hit`
  exact inverses by construction, saturating arithmetic, non-finite `line_height` degrading
  to zero) and the resident supervisor (closing the v1 stderr-deadlock and
  strict-request/response defects, transport-as-trait, `ManualClock` for sleep-free
  determinism) are the strongest parts of the branch. Do not rewrite them.
- The problems are all in the integration layer: it preserved incidental complexity the
  design had already eliminated, and the shell absorbed code the codebase has a well-worn
  place for.
- Behaviour was never the issue. Build, clippy, and 626 tests were green throughout the
  audit. Passing tests are therefore **not** evidence that an item in this plan is done —
  use its acceptance checklist.
