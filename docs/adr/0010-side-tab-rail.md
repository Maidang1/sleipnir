# Side tab rail, derived workspaces, owned agent marks

**Status:** accepted

## Context

The primary user runs coding agents in many tabs across a few git checkouts.
A horizontal tab strip hides status and does not show which repo a tab belongs
to. Run Ledger badges already say *what happened*; they do not say *who* is in
the foreground.

## Decision

1. **Default chrome is the top strip** (`tab_placement: top`). `side` is the
   same tab list as a left rail (grouping, drag). Width of the rail is
   `sidebar_width` (160–320, default 200). Not a live-drag divider — that
   would reflow the PTY on every mouse move.
2. **Workspace is derived**, not stored. Group by the git work tree of the
   active pane cwd (walk up for `.git`, file or directory). No `session.json`
   change. `cd` to another repo moves the tab. Drag-reorder only inside a group.
   Grouping is silent: no workspace header, no tab count.
3. **Agent marks are ours.** Match `foreground_process_command_name()` against a
   built-in catalog and draw a letter + fixed color. No vendor logos, no house
   placeholder for a plain shell, no model calls
   ([ADR-0008](0008-no-builtin-ai.md)). `agent_icons: false` hides them.
4. **Failed Attention is a wash**, not a run glyph. Running and Succeeded
   never draw on the chip.

## Consequences

- New tabs inherit the workspace root (git root or current cwd), not a nested
  subdirectory, so they stay in the same group.
- A missing `tab_placement` key means top. `"tab_placement": "side"` is the
  first-class left rail, not a reduced fallback.
- Rail rows are two lines (title, then branch + `+N −M`). Line counts come
  from `git diff --numstat HEAD` off the UI thread; untracked files are omitted.
- Top-strip chips show only the last two cwd components (`myself/harbor`).
  No branch, no dirty mark. A right-click rename still overrides either layout.
- Chrome grows a column; the VT grid is unchanged.
