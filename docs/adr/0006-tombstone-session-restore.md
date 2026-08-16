# Session restore shows a tombstone, never restored output

> **Superseded by [ADR-0009](0009-run-ledger-persistence.md).** The intent
> here — a restored layout must not pretend the pane was unused, and
> scrollback must never be written to disk — still stands. The grid-line
> tombstone and the "full command line is never persisted" rule are replaced
> there. Do not treat this ADR as accepted.

**Status:** superseded by [ADR-0009](0009-run-ledger-persistence.md)

## Context

`session.json` restores structure only — tabs, splits, pane ids, cwd
(`crates/sleipnir_ui/src/session.rs`). Processes are gone and no output is
restored, which is honest but produces a misleading screen: the user relaunches,
sees the exact layout they left, and every pane is blank. The layout says
"nothing happened here", which is false — three long commands may have run in it.

The obvious fix ("also restore scrollback") is the wrong one:

- **It creates a long-lived credential-leak surface.** Scrollback routinely
  contains `export TOKEN=…` echoes, `psql` / `mysql` prompts, cloud CLI output
  with signed URLs, and agent output containing API keys. Persisting it turns a
  process-lifetime secret into an on-disk secret, in plaintext, in the config
  directory, indefinitely.
- **What it buys is small**: the pixels look familiar. It does not bring back a
  single running process, so it cannot restore the thing the user actually lost.
- Users who genuinely want output preserved already have an explicit,
  consent-based path: **Shell → Export Scrollback…**, where the user chooses the
  destination and owns the file.

## Decision

Restore **structure + a tombstone**, never output bytes.

1. `session.json` gains, per leaf, only **non-content metadata**:
   - `last_program: Option<String>` — the foreground program *name*, from
     `Terminal::foreground_process_command_name()` (already argv[0]-normalized,
     e.g. `cargo`, `npm`, and script-name for `node`/`python` wrappers).
   - `saved_at: Option<u64>` — unix seconds of the snapshot.
2. **The full command line is never persisted.** `argv[0]` only. Arguments are
   themselves a secret surface (`curl -H "Authorization: Bearer …"`,
   `mysql -p<pass>`, `aws --profile … --session-token …`). Any future change that
   records more than the program name must revisit this ADR.
3. On restore, each pane shows one generated tombstone line, e.g.
   `── restored · was running cargo · 14:32 · nothing is running now ──`,
   written through the display-only output path. It is **generated locally from
   the metadata**, never replayed bytes.
4. Nothing is restored when `restore_session` is off, and no metadata is written
   in that case either.

## Consequences

- Relaunch stops lying: an empty pane that used to run `cargo` says so, and says
  the process is gone.
- The on-disk session file stays free of terminal output. Its worst-case leak is
  a program name and a cwd — the same information `ps` already exposes to the
  user's own uid.
- The tombstone occupies a real grid line, so `⌘F` can match it. Accepted: it is
  one short generated line per pane, not restored output. If tombstones ever
  become noisy in search, promote them to a chrome layer instead of grid content
  (a bigger `TermElement` change) rather than making them persistent output.
- `⌘F` and Export Scrollback keep operating on live session content only.
- **Open question before this moves to accepted:** should the tombstone appear at
  all when `last_program` is unknown (idle shell at save time)? Current lean:
  no tombstone for an idle pane — showing "nothing was running, nothing is
  running" is pure noise.
