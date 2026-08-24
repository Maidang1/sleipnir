# Diff inspector is an overlay, not a Pane

**Status:** accepted

## Context

Harbor already shows work-tree dirtiness as rail `+N −M` (`git diff --numstat HEAD`)
and can paste a raw `git diff HEAD` into the focused pane (`send_git_diff`).
People running coding agents also need to *read* that patch. [ellie/lgtm](https://github.com/ellie/lgtm)
does this as a dedicated review app (GPUI + parse unified patch + word-level intra).

A Pane in Sleipnir is one PTY ([ADR-0001](0001-pane-tree-splits.md)). Session restore,
the control surface ([ADR-0011](0011-control-surface.md)), broadcast, and Run Ledger
anchors all assume a `TermView` leaf. Putting a Diff surface in `PaneNode` would
re-litigate that invariant.

## Decision

The diff inspector is a **window-level overlay** (same family as Settings / Run Ledger).
It is not a Pane and is not persisted in `session.json`.

- Source: `git -C <work-tree> diff --no-color --no-ext-diff HEAD` — the same
  tracked-vs-HEAD story as the rail badge. Untracked files are out of v1.
- Model: `diff_core` parses the unified patch and computes word-level intra-line
  ranges on equal-length removed/added runs (heuristics from lgtm, MIT).
- Render: fixed-height `uniform_list` + `StyledText`. Split is the default
  (removed/added runs paired into two cells; leftover sides are void). A left
  file tree jumps to that file's header. `v` toggles unified. Theme colors
  come from `TerminalPalette`, not a hardcoded Catppuccin table.
- `send_git_diff` stays. The overlay can reuse the fetched patch via
  "Send to pane".
- No GitHub PR fetch, no review comments, no model calls
  ([ADR-0008](0008-no-builtin-ai.md)).

Default chord is `⌥⌘G` (`cmd-alt-g`). `⌘⇧G` remains Find Previous.
The chrome **Diff** button in the tab band also opens it.

## Consequences

- Opening the overlay covers the terminal. Esc / the same chord returns focus
  to the active pane. Non-⌘ keys are swallowed so `n` / `p` cannot leak into
  an agent's PTY.
- Phase 3–4 shipped: after the patch view lands, eligible files are re-diffed
  from `git show HEAD:path` + worktree bytes (`diff_texts`, context 3). Hidden
  shared lines become clickable `⋯ N hidden lines` gaps. tree-sitter highlights
  Rust / Python / JavaScript / JSON (unknown extensions stay plain). A right-edge
  canvas minimap (`m` or the header chip) reduces display rows to coalesced
  runs. Binary, non-UTF-8, and >1 MB blobs stay on the patch-derived hunks.
- Word-diff heuristics originated in ellie/lgtm (MIT). This crate reimplements
  them; it does not vendor lgtm's UI.
