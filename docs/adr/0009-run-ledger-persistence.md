# Persist a Run Ledger of redacted command lines

**Status:** accepted (supersedes [ADR-0006](0006-tombstone-session-restore.md))

## Context

An audit of the install → first launch → start work → leave → come back → find
the result → restart journey found a single root cause at the three worst
breakpoints (come back, find the result, restart):

> The system already knows the facts. They do not appear in the UI.

The terminal already tracks `looks_busy()`, `busy_since`, the foreground program
name, exit status, and OSC 133 command boundaries (the vte fork in
[ADR-0005](0005-vendored-alacritty-term.md) exists so those markers reach the
real PTY). Tab chrome used that information only for a title and a bell flash.
`session.json` restored structure and a blank pane. The layout said nothing had
happened there.

[ADR-0006](0006-tombstone-session-restore.md) named that lie and proposed two
rules that this design cannot keep:

1. Render the tombstone as a generated grid line on the display-only output
   path, so `⌘F` can match it.
2. Never persist more than `argv[0]`. Any record of a command line must revisit
   that ADR.

A grid-line tombstone invades the VT grid ([Run Ledger design](../superpowers/specs/2026-08-16-run-ledger-design.md)
principle 2: the content area is inviolable). Persisting only a program name
cannot answer "what ran here, for how long, and how did it finish?" after a
restart. ADR-0006 also left open whether an idle pane (`last_program` unknown)
should show a tombstone at all.

## Decision

Persist a **Run Ledger**: one record per command execution, written as
`runs.json` next to `session.json` (`~/.config/sleipnir/runs.json`).

Each Run stores a **redacted command line** plus non-output metadata (`RunId`,
`PaneKey`, `LaunchId`, cwd, wall-clock start, duration, exit status, inferred
flag). Scrollback, output bytes, and environment values are never written.
Redaction runs at capture so memory and disk hold the same text.

**Persistence is on by default** (`run_ledger: "persist"`). `~/.zsh_history` and
`~/.bash_history` already store full plaintext command lines; this file stores a
redacted subset of that plus metadata, so the risk is not higher than the
status quo. Tombstones and "still remembered after restart" only have value if
they are on without a hunt through settings.

Default-on is allowed only with three hard requirements, all of which shipped
in P0:

1. **First-write notice** — the first time `runs.json` is created, tell the user
   once. The notice is itself persisted (`announced` in the file) so a second
   launch does not repeat it.
2. **One-click off** — `run_ledger: "off"` (or `"memory"`) is a documented
   settings key. Off stops collection and hides every Run Ledger surface;
   `"memory"` keeps in-process badges but neither reads nor writes the file.
3. **One-click clear** — `clear_run_ledger` in the command palette and menus
   deletes the in-memory ledger and the on-disk file after a confirm dialog.

This ADR **supersedes two decisions in ADR-0006**:

| ADR-0006 | This ADR |
|---|---|
| Tombstone is a generated grid line on the display-only path | Tombstone is a chrome banner above the grid (P1). It is not terminal content and is not searchable with `⌘F`. |
| The full command line is never persisted; `argv[0]` only | The redacted command line is persisted in `runs.json`. |

ADR-0006's intent — the restored layout must not pretend the pane was unused —
stands. Its open question is answered here: **a pane whose history Run count is
0 shows no tombstone banner.** No facts, no speech. New panes, and panes
restored from a session file that has no `pane_key`, also show none.

`terminal.inject_osc133` defaults to `true`. Without injection the ledger has
only the busy-probe fallback (inferred, no exit code). Existing skip rules
(another terminal already injects; `shell -c`) stay. `false` remains the
escape hatch.

## Consequences

- **Redaction is a heuristic, not a guarantee.** High-entropy tokens, known
  flags, `KEY=VALUE` prefixes, and URL userinfo/query patterns are replaced;
  anything the heuristic misses is written as captured. Documentation must not
  call `runs.json` "safe" or "secret-free".
- `restore_session: false` writes no `pane_key` and therefore has no tombstone.
  There is no second persistence path for that case.
- Multiple Sleipnir instances share `runs.json` by **merge-on-write** (re-read,
  union by `RunId`, prune, atomic temp+rename). There is no single-instance
  lock; the repo has never enforced one.
- The file is versioned JSON, Unix mode `0600`, retained for 7 days and 500
  runs (whichever binds first). A corrupt or unknown-version file is renamed
  `.bak` and the process starts empty; a write failure drops to in-memory only.
- Attention (unseen finished Runs) and Anchors (scrollback positions) are
  process-lifetime only. They are not fields in `runs.json`. Loaded history is
  marked seen; in-flight Runs become Abandoned.
- P0 writes `runs.json` and the first-write notice / one-click off / one-click
  clear. P1 consumption UI has since shipped: chrome tombstone banner (not a
  grid line), Ledger overlay (`⌘⇧L`, jump to pane + Anchor), and pane gutter
  triangles. Tab chrome no longer draws run glyphs; failed Attention is a wash
  (see [ADR-0010](0010-side-tab-rail.md)).
- ADR-0006 is **superseded, not accepted.** Do not promote it.
