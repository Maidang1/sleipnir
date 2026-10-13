# Code quality fix plan

Action list from the 2026-10-01 thermo-nuclear review of `main` @ `2f0db80`
("Bezel p0" `798c5de` + `9037d02` + `2f0db80`). Every item below was verified
against the tree by reading the cited lines, grepping for consumers, and — for
item A1 — reproducing the failure in a standalone program. This is a model
change list, not a changelog: each batch deletes a concept or collapses
duplicated mechanisms, it does not move code around.

**Ground rule: everything is behavior-preserving except A1**, where the current
behavior *is* a panic, so the only honest change is panic → no match.

**Bar before starting:** `cargo build --workspace && cargo test --workspace` green.
Record the baseline. Every batch ends with `cargo test` + `cargo clippy
--all-targets` on the touched crates.

---

## Batch A — pure deletion (do this first; lowest risk, biggest line win)

Each item removes code with **zero reachable consumers**. Nothing in this batch
changes control flow.

- [x] **A1 `sleipnir_syntax::registry::of_path` panics on non-ASCII names.**
  `crates/sleipnir_syntax/src/registry.rs:166-171` — `name[name.len() -
  candidate.len() - 1..]` is an unguarded byte index. Reproduced for `資料`,
  `日本語` (`byte index N is not a char boundary`). Replace with
  `checked_sub` + `is_char_boundary` + `as_bytes()[start] == b'.'`
  + `name[start + 1..].eq_ignore_ascii_case(candidate)`. Also removes ~170
  `format!` allocations per path lookup. Add a regression test over a name table
  (CJK, Cyrillic, accented, ASCII, boundary lengths).
  *Accept:* the new `matches` never panics and returns the old value for every
  input the old one did not panic on.

- [x] **A2 `chrome/tombstone.rs` is 266 lines that nothing compiles.**
  `crates/sleipnir_ui/src/chrome/mod.rs:3-14` declares 12 modules and this is not
  one of them; no `#[path]`; last touched before the deletion commit. Delete the
  file, and with it `run_ledger/src/run.rs:210-234 Run::for_display` (its only
  caller is `tombstone.rs:128`) and the `show_tombstone` setting
  (`sleipnir_settings/src/lib.rs:207` decl, `:322` default, `:597` file field,
  `:719-720` merge, `:832` scaffold). Tests: trim the `show_tombstone` assertion
  from `new_config_does_not_advertise_removed_persistence_settings`
  (`:1338-1346` — the rest of that test covers `restore_session`, `ui_style`,
  `tab_placement` and must stay) and the `assert!(settings.show_tombstone)` from
  `legacy_settings_still_load_after_removing_their_product_surfaces`
  (`:1348-1360` — keep it as the legacy-keys-are-ignored test, drop the
  dead-field assertions).

- [x] **A3 two more settings keys are write-only.** `run_ledger_max_runs`
  (`lib.rs:197,317,587,636,703-704,791-792,827`) and `minimum_contrast`
  (`lib.rs:164,301,623,767-768,843`) have zero readers in the workspace. Delete
  the fields, the four merge branches, and `default_settings_file()`'s
  `:843 minimum_contrast: Some(45.0)`.
  **The asymmetry matters:** `minimum_contrast` is the one that lies — it is
  written into every fresh `settings.json` and advertised at
  `docs/settings.example.json:60`, so a user sets it and nothing happens,
  silently. `run_ledger_max_runs` and `show_tombstone` are already `None` in
  `default_settings_file()` (`:827`, `:832`), so those are inert scaffolding
  rather than advertising.
  Tests: keep `run_ledger_defaults_to_persist` (`:1330-1336`) minus the
  `run_ledger_max_runs == 500` assertion (it also covers the live
  `run_ledger` / `run_ledger_redact` defaults); trim
  `new_config_does_not_advertise_removed_persistence_settings` (`:1338-1346`)
  of its `run_ledger_max_runs` assertions; drop
  `assert_eq!(settings.run_ledger_max_runs, 120)` from
  `legacy_settings_still_load_after_removing_their_product_surfaces`
  (`:1348-1360`). *Note:* users' existing files are already ignored
  (`#[serde(default)]`, no `deny_unknown_fields`), so nothing regresses — only
  the advertising goes away.

- [x] **A4 `theme_color.rs` is ~70% orb-engine residue.** `2f0db80` deleted the
  orbs and kept the module. Dead: `neutral`, `oklch`, `oklch_to_srgb`,
  `oklch_to_linear`, `gamma_encode`, `gamma_decode`, `lightness`, `fit_chroma`,
  `tint`, `rgb_to_hsl`, `flatten`, `mix` (its own comment says it paints a
  spinner that no longer exists). Keep `relative_luminance`, `hsl_to_rgb`,
  `contrast_ratio`, `grey`; consider moving them next to their only consumer,
  `chrome/chrome_tokens.rs`. ~155 lines out.

- [x] **A5 identity wrapper.** `chrome/chrome_tokens.rs:139-140`
  `pub fn contrast_ratio` delegates to `theme_color::contrast_ratio` while the
  same file calls the real one at `:35/46/54/75`. Delete the wrapper; fix the
  re-export at `lib.rs:54` and `chrome/mod.rs:16` to name the real function.

- [x] **A6 `plugin_grants::Tier::BuiltIn` has no producer.** `plugin_grants/
  src/lib.rs:38-46` — writers hard-code `Tier::Local`
  (`sleipnir_ui/src/plugin_runtime.rs:533`, `app_shell/plugins.rs:592`,
  `panels.rs:367`); the only remaining textual hit is the badge arm
  `plugin_monitor_panel.rs:212`). Delete the variant and the arm. A hand-edited
  grant carrying `"tier":"built_in"` then fails to deserialize, and `load`
  (`plugin_grants/src/lib.rs:230-248`) is fail-closed — it quarantines the file to
  `.bak` and treats it as empty, forcing re-consent. That is this crate's
  documented contract, not a regression; say so in a comment.

- [x] **A7 `updater` tests grep their own source.** `crates/updater/src/lib.rs:
  931-986` — four `include_str!("lib.rs")` assertions, one of which pins an exact
  function signature spelling (breaks on the next `cargo fmt` wrap). Delete them
  and the identity function they exist for, `lib.rs:366-368
  in_place_update_supported_for(macos: bool) -> bool { macos }`; inline
  `cfg!(target_os = "macos")` at `:362-364`. Keep the two real unit tests
  (`minimum_macos_gate_compares_dotted_versions`,
  `verified_artifact_carries_path_version_and_minimum_macos`). Same disease at
  `gpui_platform/src/lib.rs:145-170` — delete that test too.

- [x] **A8 a second, unused line framer.** `plugin_host/src/resident/transport.rs:
  367-399 read_line_limited` (`#[allow(dead_code)]`, only caller is its own test
  at `:1259-1267`) re-implements the three rules of `PipeSource::recv_line`
  (`:253-300`). Delete it, its test, and the `BufRead` import at `:16`.

- [x] **A9 `#[allow(dead_code)]` state machines that only tests read.**
  `terminal/src/scroll_follow.rs:52-63` (`following`, `follow`) and
  `kitty/store.rs:104-108 Display::at_cursor` (its `#[allow(dead_code)]` comment
  reads "sixel and iTerm placement, deferred until those protocols land"),
  `:250 Store::hold` (same comment, same non-existent feature) — three items of
  API surface for code that does not exist. Delete all three; if the scroll pin
  must stay drivable in tests, move it into the test module.

- [x] **A10 the same disease at module scale.** `terminal/src/scanner.rs`:
  `Segment::Sixel`, `Segment::Iterm` (+ `Iterm` enum, `OscKind`, `OSC_HEADERS`),
  `Segment::Directory`, `Segment::CellSizeQuery` are all discarded by
  `graphics.rs:857-863`. Full detail in B2 — if B2 lands in the same pass, skip
  the separate entry here.

*Accept for Batch A:* `cargo test --workspace` unchanged; net line count down by
~600; no new `#[allow(dead_code)]`.

---

## Batch B — model unification (the code-judo work)

- [x] **B1 `graphics.rs`: three placement tables → one.**
  `graphics.rs:88-121` `Placed`/`Virtual`/`Relative` duplicate `cols, rows, frame,
  source, z` verbatim; `:131-133` three parallel `HashMap`s; `exists()`/
  `shown()` three-way `||`; `place()` `:321-336` and `relate()` `:372-386` open
  with the same 5-line "evict the other two" preamble (with *different* eviction
  sets); `locate()` `:537-662` builds three parallel `Vec`s behind a `Located`
  wrapper (`:826-830`) only so `placements()` `:187-199` can `.extend` three
  times; `delete()` `:698-814` (116 lines) walks the three tables separately.
  Target shape: one `Held { key, anchor, cols, rows, frame, source, z }` with
  `enum Anchor { Grid(u32) | Virtual | Relative { parent, offset } }`, plus
  `anchors: HashMap<u32, (u32,u32)>` for the reverse walk. `exists` →
  `contains_key`; `shown` → `values().any`; three preambles → one `insert`;
  `locate` → one `Vec`; `delete` → one `retain`. Deletes `Located`, ~100 lines.
  **Decision to make explicit and pin with a test:** `anchor()` `:473-477`
  evicts only `placed`, while `place()`/`relate()` evict the other two kinds — so
  today a `U=1` virtual placement under `(image, placement)` survives a later
  ordinary `a=p` display of the same pair. Untested. Pick one rule; one map makes
  the divergence unrepresentable.

- [x] **B2 `scanner.rs`: keep the swallow, drop the payload.**
  `scanner.rs` is 644 lines of which ~250 parse values nobody reads (Sixel
  `:370-449`; the whole iTerm/OSC 1337 payload path `:55-68,70-81,273-369`;
  `file_url`/`quoted_path`/`path_from` `:584-638`; `CellSizeQuery`
  `:36-40,509-520`). It also *buffers* them: an OSC 1337 File command stalls up
  to 96 MiB into `self.run` before being discarded, a sixel up to 32 MiB. Keep
  only the byte-stripping that keeps raw image bytes off the screen. Delete the
  OSC 7/9 path entirely — the crate already derives cwd from `CwdTimeline`
  (`cwd_timeline.rs`, fed by `record_cwd_change` at `lib.rs:2717`), so the
  scanner holds a third copy of that parse. Characterization test per surviving
  path first (the suite already pins the byte-level behavior).

- [x] **B3 `term_element.rs::layout_grid`: back to one pass.**
  Bezel replaced the single-pass batcher (flush on style / column discontinuity,
  `can_append_run` `:112-117`) with `glyphs: Vec<RowGlyph>` and `runs:
  Vec<BatchedTextRun>` in lockstep, a 48-line `segment_row` (`:259-306`), and a
  backward index-reconciliation walk (`:1188-1214`) that re-derives run
  boundaries by counting `column_span`. Evidence of over-building: the
  `wide_spacer` push at `:1140-1147` is immediately skipped by `:284` — provably
  dead. Fold "this cell is pinned" (`!ch.is_ascii() || column_span > 1`) into the
  existing flush condition so pinned cells always shape alone. Removes
  `RowGlyph`, `RowSegment`, `pins_its_column`, `segment_row`, the reconcile
  block, and ~80 lines from a file that is already 1982.

- [x] **B4 one widget-tree walk, one home.**
  `plugin_protocol/src/v2.rs:516-531 measure()` has no production caller (only
  its own tests and `sleipnir_widget/tests.rs:191,212`);
  `widget/src/layout.rs:264-283 bounded_measure()` is the one actually used;
  `plugin_runledger/src/view.rs:108-114 node_count()` has none. Move the bounded
  walk next to `MAX_WIDGET_NODES` / `MAX_WIDGET_DEPTH` (`v2.rs:500-501`),
  returning `TreeStats`; `layout` calls it and drops `bounded_measure`; add
  `measure`/`TreeStats`/`MAX_*` to `sdk/src/v2.rs`'s re-export list and delete
  `node_count` (its test asserts the same quantity the walk reports).

- [x] **B5 `kitty::Display` stops being a 17-field hand-copy of `Command`.**
  `store.rs:70-133` restates `placement, columns, rows, source_x(=x), source_y(=
  y), source_width, source_height, offset_x(=X), offset_y(=Y), z,
  cursor_movement, unicode, parent_*, quiet` one-for-one, and `Display::of`
  `:110-132` is 20 lines of assignment whose only job is copying. Introduce one
  `Placement` struct shared by both sides; `Display { image, placement }`.
  `Display::of` becomes one line, and the `#[non_exhaustive]` on both types
  (`:71`, `:98`) stops papering over the copy.

- [x] **B6 `delete()` states its enum partition three times.** `graphics.rs:
  713-722` (`names` with its `Some` arms and its `_ => None`) and `:740-746`
  (`hit` with its `_ => true`) plus `:723-727 by_name`, then `:764` a negated
  `matches!` **and** a redundant `&& by_name` guarding the virtual path (it
  exists only because `Virtual` has no `z`). Put the two questions on the enum —
  `Target::names_image()`, `Target::matches(image, placement, z)`,
  `Target::covers(p, cursor)` — with no `_ =>` arms anywhere, and take
  `z: Option<i32>` so virtuals pass `None`.

- [x] **B7 the chunk merge gets a name.** `store.rs:444-471` — nine `.max()`
  folds, one `if != 0`, one `|=`, three merge idioms in one block inside an
  already 105-line function, none explained and none directly tested. Make it
  `Command::absorb(&mut self, chunk)` with a comment on which side states the
  control keys; `CursorMovement` derives `PartialOrd, Ord` (`After < None`) to
  collapse its three-line special case. `transmit` shrinks by ~25 lines.

- [x] **B8 `standard.rs` stops duplicating `registry.rs`'s table.**
  `standard.rs:34-43` is character-identical to the third column of `NAMED`
  (`registry.rs:198-230`), plus `files_for` re-deriving them by linear scan
  (`:25-31`). Expose `registry::files_of(name)` over `NAMED` and delete `FILES`
  and `files_for`.

- [x] **B9 one capture→kind mapping.**
  `document.rs:104-128` + `:194-207 recognized()` re-implement the "most
  dot-separated parts wins" rule that `lang.rs:163 configure(NAMES)` owns for
  the `session.rs` highlighter path, on top of hand-rolled `flatten`/`merge`/
  `point_at`. The test that enforces the invariant (`tests/document.rs:54-64`)
  normalizes away exactly the divergence through `joined()`'s "the two painters
  may split a run differently". Hoist `capture_kinds(query) -> Vec<Option<HighlightKind>>`
  into `lang.rs` and call it from both.

- [x] **B10 `sleipnir_motion`: 417 lines for one call site.** The only consumer
  in the workspace is `sleipnir_ui/src/chrome/tab_strip.rs:9,319` (`CubicBezier::eval`
  + `TAB_SLIDE`). Delete `phase.rs` (197 lines, all of it — and it references
  `bezel::loaders`, a crate that does not exist here), `Lease`, `phase()`,
  `wall_progress()`, `MotionSpec::with_delay`, and the six unused constants.
  Keep ~140 lines that earn their keep.

---

## Batch C — boundary, lifetime, and the crate-shape decisions

- [x] **C1 decide `sleipnir_syntax`'s status.** *Decision: wire it in — the
  diff inspector's paint site now resolves through `registry::of_path` and
  paints `HighlightKind` spans, and the superseded `crates/syntax` crate (zero
  consumers after the switch) is out of the workspace.* The crate is ~4.2k lines
  (1,496 src + 720 tests + 1,901 lines of `.scm`) with **zero dependents** —
  `grep -rn sleipnir_syntax --include=*.rs crates` outside the crate is empty, and
  `798c5de` wired in `sleipnir_motion` but never this crate. Its injection
  subsystem has no built-in grammar using it. Either land the paint site into
  `sleipnir_ui` in the same release, or stop carrying it as a workspace member.
  Do not ship 4.2k lines of unpublished API in between.

- [x] **C2 make the dead surface visible.** `terminal/src/lib.rs:12-18` exports
  `pub mod kitty / scanner / graphics / emulator` while nothing outside the crate
  uses `terminal::kitty` — which is why `Image::frame_revisions` (`store.rs:28-31`,
  doc-commented as written *for* the cache in C3), `Store::len/is_empty`,
  `Rgba::opaque`, and `frame_at` never trip a warning. Change to `pub(crate)`,
  let the compiler enumerate what is dead, then delete it.

- [x] **C3 key the animation decode cache by frame, not by image.**
  `graphics.rs:136 decoded: HashMap<u32, (u64, usize, usize, Arc<RenderImage>)>` —
  one entry per image id, so an animation that alternates frames misses on every
  other step and re-decodes the whole frame plus a per-pixel R/B swap (1080p ≈
  8 MB + 8 MB per gap). Key it `(image id, frame)` and validate with
  `image.frame_revisions` — which also removes the quietly-wrong use of frame 0's
  byte length as the key for every frame (`graphics.rs:294`). While there: hoist
  `Instant::now()` out of `painted()`'s per-placement loop (`:213`) and drop the
  double dispatch in `Rgba::decode` (`pixels.rs:47-57`).

- [x] **C4 decide what to do with the computed-and-dropped signal.** *Decision:
  delete it — `frame_index` answers the frame, the `0..10_000` bound is the
  elapsed-over-shortest-gap division, and `Emulator::frame_at` returns the frame
  only.*
  `frame_index` returns `(usize, bool)` where the bool is discarded at the one
  production call site (`graphics.rs:212-215`) and read only by
  `tests/kitty.rs:1540-1573`; `graphics.rs:263-289` backs the catch-up loop with
  a magic `for _ in 0..10_000`. Either propagate the bool through `PaintedImage`
  so the view can request animation frames (this **does** change behavior — today
  a kitty animation with no pty traffic freezes, because `term_element.rs:775-777`
  requests frames only for cursor blink), or delete it and `Emulator::frame_at`.
  Replace the loop bound with the arithmetic division it is standing in for.

- [x] **C5 stop redacting events that are then discarded.**
  `plugin_host/src/resident/session.rs:362-366` redacts `RunStarted` *before* the
  `is_dead` / `is_shutting_down` / `SubscribeEvents` gates, and `fan_out`
  (`event_bus.rs:84-86`) calls it once per session — so a broadcast clones and
  fully rewrites N+..M times per event, `redact_command` allocating a
  `Vec<String>` per token each time. Hoist the redaction into `fan_out` (the
  single dispatcher) or at minimum move the three cheap gates above it.
  `redact_command` is pure, so the delivered payload is unchanged.

- [x] **C6 one OnDemand teardown helper.**
  `plugin_host/src/resident/supervisor.rs` writes the same
  `cancel()` + `teardown(grace)` block at `:138-144` and `:169-175` (byte-identical)
  and `:179-183` again, with `lifecycle == OnDemand` special-cased at `:87,120,
  138,154,169,179,186`. Extract `connect_active` / `retire_active`; the missing
  concept is "an OnDemand session is a temporary that must be retired on every
  exit".

- [x] **C7 one `apply_retention` invariant, one mechanism.**
  `run_ledger/src/ledger.rs:391-419` uses a mirrored `Vec<bool>`, a third branch
  for `protected_count >= max_runs`, and a side-effecting `retain` closure that
  mutates `to_drop` while iterating. Replace with a single `retain` and an
  explicit budget (`budget = max_runs.saturating_sub(running)`). Covered by
  `prune_caps_total_runs` (`:638`), `prune_keeps_current_launch_running_runs`
  (`:663`), `prune_allows_current_launch_running_runs_to_exceed` (`:689`),
  `prune_keeps_running_rows_from_other_launches_too` (`:756`).

- [x] **C8 updater: one publication discipline and no hand-rolled flock.**
  `crates/updater/src/install.rs:52-63,86-90` — two `unsafe` blocks calling
  `libc::flock` that `File::try_lock()` / `File::lock()` provide on a stable
  toolchain; drop both blocks and the `AsRawFd` import. The crate currently
  carries three publication disciplines: `install.rs:111-145` (`create_new` +
  `hard_link`, no parent fsync), `transaction.rs:230-239`
  (`atomic_write::save_atomic_with`, correct), `download.rs:89` (bare
  `fs::rename`, no parent fsync) — and depends on `atomic_write` at
  `Cargo.toml:13`. Add `atomic_write::publish_new(path, bytes)` (staged
  `create_new` + `hard_link` + `sync_parent`) and route all three through it; the
  durability gap closes as a side effect.

- [x] **C9 `BROWSER_PANEL_ENABLED` is a runtime `false` const.**
  `sleipnir_ui/src/lib.rs:6` gates user-facing entries from shared builders:
  `command_palette.rs:352 items.retain(|i| i.id != ToggleBrowser)`,
  `app_shell/mod.rs:2330 .when(cfg!(...) && BROWSER_PANEL_ENABLED, …)`. The whole
  WebView implementation still compiles and runs everywhere; the flag hides two
  entries. Choose a compile-time `#[cfg]` or gate registration so the palette
  and menu never see the item — do not keep dead-but-compiled code plus
  special-cases in shared paths.

- [x] **C10 `automation.rs` dead branches.**
  `sleipnir_ui/src/browser/automation.rs:43-53 authorize()` already returns
  `Err(not_ready)` without a host, so the re-checks at `:93-96` and `:118-121`
  are unreachable and duplicate the message a third time. Stop round-tripping
  `Option<Rc<NativeHost>>`; bind the host once.

- [x] **C11 `media.rs` states its security posture twice.**
  `media.rs:59-64 resolve()` and `:91-105 is_temporary()` encode different root
  lists (`/proc,/sys,/dev` vs `/tmp,/dev/shm,temp_dir,$TMPDIR`), so a reader must
  diff two functions to know the posture; `is_temporary` also rebuilds four
  `PathBuf`s and canonicalizes three of them per temporary read. One
  `SENSITIVE_ROOTS` const and one `is_under(root, path)` used by both.

- [x] **C12 `PartialEq` by convention and poisoned locks.**
  `sleipnir_syntax/src/registry.rs:29-48` hand-writes `PartialEq`/`Debug`
  comparing `&'static Lang` by pointer identity — safe only by the leak-and-retain
  convention at `:124`; derive on `(name, aliases)` instead. `:114,121,137,151,
  175,188` swallow poison with `let Ok(mut e) = … else { return }`, silently
  turning the registry into an empty one; `lib.rs:58` already uses
  `.unwrap_or_else(|err| err.into_inner())` in the same crate.

- [x] **C13 `sleipnir_syntax` panic surface is latent only because nothing calls
  it.** Once C1 lands, re-run A1's regression test as part of the caller's tests.

---

## Batch D — cheap wins (do only if a file is open anyway)

- [x] `row_geometry/src/geometry.rs:163-168` no-op guard: `if self.spans.is_empty()
  { return; }` before `clear()` — `Vec::clear()` on an empty vec is already a
  no-op.
- [x] `sleipnir_ui/src/app_shell/query.rs:67-70` bind `clamped_byte_range` once
  (two calls + a clone on an unchanged string).
- [x] `terminal/src/kitty/store.rs:386-396` call `png_size` once — build the
  `Image` first, then check `image.size()`; the raw branch's `Some` is
  unconditional and carries no information.
- [x] `terminal/src/media.rs` single root predicate (see C11).
- [x] `sleipnir_settings/src/lib.rs:405-420,423-435,438-452,455-462` unify the
  four refresh policies of the setters on `update()`'s existing return value.
- [x] `sleipnir_plugin_sdk/src/v2.rs:390-408` and `:411-425` differ only in error
  handling (log+continue vs. propagate); one reader with a mode.
- [x] `sleipnir_plugin_runledger/src/state.rs:113-117` `host_id_for` /
  `local_id_for` are one predicate; collapse to `is_jumpable`.
- [x] `terminal/src/kitty/store.rs:517-530` reply policy is implemented twice
  across the module boundary (vs `graphics.rs:171-176`'s `display.quiet < 2`);
  own it in one place.
- [x] `terminal/src/kitty/animation.rs:195-209` `ensure_rgba` mutates `self.gaps`
  before it can fail; move the push after the decode so state is never
  half-applied.
- [x] `sleipnir_browser_control/src/transport.rs:147-205` idle-polls the accept
  socket at 50 Hz per window to observe a `Drop` flag; use a blocking `accept`
  with a wake-up connect instead.
- [x] `terminal/src/placeholder.rs:101-106` allocates a `Vec<u32>` per cell per
  paint and `diacritic` (`:46-51`) linearly scans 297 entries; use a lookahead
  iterator plus `binary_search_by_key` (the table is already sorted).
- [x] `terminal/src/alacritty/byte_loop.rs:63-97` `ByteLoopSendError` — 35 lines
  for two variants; `thiserror` (already a dependency at `lib.rs:81`) gives 7
  lines, `?`, and `Error::source()` back. **Do not touch the rest of
  `byte_loop.rs`**: it is a near-verbatim upstream fork; the genuinely new delta
  is `NoSync` + `Scanner` in `LoopState` and the mode-2026 hold, whose lifecycle
  is restated at `:254`, `:268-273`, `:382-390`, `:403-410` and should become one
  `hold_after_read(edges, processed) -> bool`.

---

## Definition of done

1. `cargo build --workspace && cargo test --workspace` green, and
   `cargo clippy --all-targets` clean on every touched crate.
2. No file in `crates/` was pushed past 1000 lines by any batch; `term_element.rs`
   (1982 → ~1900 after B3) and `v2.rs` (1020 → ~690 after the test split) go the
   other way.
3. No new `#[allow(dead_code)]`, no new `Option`-as-mode, no new runtime feature
   consts in shared flows.
4. Every deleted concept has a grep proving zero consumers recorded in the
   commit message (the pattern that went wrong in `2f0db80` was deleting the
   feature and keeping the scaffolding).
5. `sleipnir_syntax` is either wired in or out of the workspace — not both.
