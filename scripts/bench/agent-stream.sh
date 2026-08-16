#!/usr/bin/env bash
# B5b — emulate a coding agent's *write pattern* inside a Sleipnir pane.
#
# Why this exists alongside `corpus/bench-agent.txt`:
#   `cat file` always reads in 64 KiB blocks, so a static corpus can only
#   reproduce agent-shaped *content* (cursor addressing, in-place redraw, SGR
#   churn, huge lines), never the agent-shaped *write pattern*: many small
#   flushed writes a few milliseconds apart. That pattern is what stresses the
#   repaint path (PTY event coalescing + damage tracking), not the parser.
#
# Run this INSIDE a Sleipnir pane. To measure the repaint cost of a *background*
# pane, run it in one tab, switch to another tab, and sample CPU with
# `scripts/bench/measure-cpu.sh` from an outside shell.
#
# Usage:
#   scripts/bench/agent-stream.sh [seconds] [delay_ms]
# Defaults: 30 seconds, 4 ms between writes (matches the PTY event loop's 4 ms
# coalescing window, i.e. the worst realistic case: one write per batch).
#
# Env:
#   CHUNK_MIN / CHUNK_MAX  payload bytes per write (default 32 / 256)
set -euo pipefail

SECONDS_TO_RUN="${1:-30}"
DELAY_MS="${2:-4}"
CHUNK_MIN="${CHUNK_MIN:-32}"
CHUNK_MAX="${CHUNK_MAX:-256}"

echo "agent-stream: ${SECONDS_TO_RUN}s, one write every ${DELAY_MS}ms, payload ${CHUNK_MIN}-${CHUNK_MAX}B"
echo "(Ctrl-C to stop early)"

SECONDS_TO_RUN="$SECONDS_TO_RUN" DELAY_MS="$DELAY_MS" \
CHUNK_MIN="$CHUNK_MIN" CHUNK_MAX="$CHUNK_MAX" perl -e '
  use Time::HiRes qw(time sleep);
  $| = 1;                                    # unbuffered: one write == one PTY write
  my $run   = $ENV{SECONDS_TO_RUN};
  my $delay = $ENV{DELAY_MS} / 1000;
  my $cmin  = $ENV{CHUNK_MIN};
  my $cmax  = $ENV{CHUNK_MAX};
  srand(20260813);

  my @spin = qw(- \ | /);
  my @tok  = qw(fn let const async await return struct impl match Some None Ok Err);

  my $t0 = time;
  my ($writes, $bytes, $frame) = (0, 0, 0);

  # Pad a payload up to a realistic small-write size without changing its shape.
  sub pad {
    my ($s, $want) = @_;
    $s .= " " while length($s) < $want;
    return $s;
  }

  while (time - $t0 < $run) {
    my $want = $cmin + int(rand($cmax - $cmin + 1));
    my $out;
    my $kind = $frame % 10;
    if ($kind < 5) {
      # Spinner / status line: rewrite the same row.
      $out = sprintf("\r\e[K\e[36m%s\e[0m streaming  t=%.1fs  writes=%d",
                     $spin[$frame % 4], time - $t0, $writes);
      $out = pad($out, $want);
    } elsif ($kind < 8) {
      # Streaming tokens appended to the current line, colour churn per token.
      $out = "";
      $out .= sprintf("\e[3%dm%s\e[0m ", 1 + (length($out) % 6), $tok[int(rand(scalar @tok))])
        while length($out) < $want;
    } elsif ($kind < 9) {
      # Commit the line, then rewrite the 2 rows above (TUI diff behaviour).
      $out = sprintf("\n\e[2A\e[K\e[1mstep %d\e[0m ok\n\e[K  \e[90m-> done\e[0m\n", $frame);
    } else {
      # In-place progress bar.
      my $p = $frame % 11;
      $out = sprintf("\r\e[K\e[32m[%s%s]\e[0m %3d%%",
                     "#" x ($p * 4), "." x (40 - $p * 4), $p * 10);
      $out = pad($out, $want);
    }
    print $out;
    $writes++;
    $bytes += length($out);
    $frame++;
    sleep($delay);
  }

  my $dt = time - $t0;
  printf("\n--- agent-stream done ---\n");
  printf("wall            %.2f s\n", $dt);
  printf("writes          %d (%.0f/s)\n", $writes, $writes / $dt);
  printf("bytes           %d (%.1f KiB/s)\n", $bytes, $bytes / 1024 / $dt);
  printf("avg write size  %.0f B\n", $bytes / $writes);
'
