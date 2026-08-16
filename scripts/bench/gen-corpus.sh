#!/usr/bin/env bash
# Generate deterministic benchmark corpora for the Sleipnir perf baseline.
# See scripts/bench/README.md.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${BENCH_DIR:-$HERE/corpus}"
mkdir -p "$OUT"

# ~150 MiB of realistic ~106-char ASCII lines (matches Mitchell's `cat 150MB_ascii.txt`).
ASCII_LINE="0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz________"
ASCII_LINES="${ASCII_LINES:-1470000}"

# ~16 MiB of mixed Unicode/CJK/ZWJ/emoji content (matches Mitchell's Unicode file).
UNICODE_LINE="αβγδ 中文测试 line 🧑‍🌾 emoji 🚀 mixed-content εθικ 终端 渲染 benchmark 0123456789"
UNICODE_LINES="${UNICODE_LINES:-180000}"

echo "generating ASCII corpus -> $OUT/bench-ascii.txt"
perl -e 'my $l=$ARGV[0]; my $n=$ARGV[1]; for(1..$n){print $l,"\n"}' "$ASCII_LINE" "$ASCII_LINES" > "$OUT/bench-ascii.txt"

echo "generating Unicode corpus -> $OUT/bench-unicode.txt"
perl -e 'my $l=$ARGV[0]; my $n=$ARGV[1]; for(1..$n){print $l,"\n"}' "$UNICODE_LINE" "$UNICODE_LINES" > "$OUT/bench-unicode.txt"

# ---------------------------------------------------------------------------
# Agent corpus (B5): what Sleipnir's actual primary workload looks like.
#
# A coding agent running in a pane does NOT stream a flat 150 MiB of text. It
# emits redraw-heavy, cursor-addressed output: spinners overwriting themselves
# with \r, progress bars using ESC[K, streaming tokens, SGR colour churn, and
# the occasional huge single line (JSON / base64 / stack trace). Almost none of
# it becomes scrollback.
#
# This corpus reproduces that *content*. It cannot reproduce the *write pattern*
# (small writes a few ms apart) because `cat` always reads in 64 KiB blocks --
# for that see `agent-stream.sh`, which drives the same shapes through small
# flushed writes.
# ---------------------------------------------------------------------------
AGENT_CYCLES="${AGENT_CYCLES:-20000}"

echo "generating agent corpus -> $OUT/bench-agent.txt"
AGENT_CYCLES="$AGENT_CYCLES" perl -e '
  my $n = $ENV{AGENT_CYCLES};
  my @spin = qw(- \ | /);
  my @tok = qw(fn let const async await return struct impl match Some None Ok Err);
  # Deterministic pseudo-randomness: same corpus on every machine.
  srand(20260813);
  for my $i (1 .. $n) {
    # 1. Spinner: same cell rewritten, no newline (\r + ESC[K).
    for my $s (0 .. 7) {
      printf "\r\e[K\e[36m%s\e[0m thinking (%d/%d) ", $spin[$s % 4], $i, $n;
    }
    # 2. Progress bar: full-width in-place redraw with colour churn.
    for my $p (0 .. 9) {
      my $done = "#" x ($p * 4);
      my $rest = "." x (40 - $p * 4);
      printf "\r\e[K\e[32m[%s%s]\e[0m %3d%% \e[90m%s\e[0m", $done, $rest, $p * 10, "downloading";
    }
    print "\r\e[K";
    # 3. Streaming tokens: many tiny colourful appends on one line, then commit.
    for my $t (0 .. 11) {
      printf "\e[3%dm%s\e[0m ", 1 + ($t % 6), $tok[($i + $t) % scalar(@tok)];
    }
    print "\n";
    # 4. Cursor-addressed rewrite of the 2 lines above (TUI diff behaviour).
    printf "\e[2A\e[K\e[1mstep %d\e[0m done\n\e[K  \e[90m-> ok\e[0m\n", $i;
    # 5. Occasional huge single line (JSON-ish / base64-ish), ~4-16 KiB.
    if ($i % 50 == 0) {
      my $len = 4096 + int(rand(12288));
      my $blob = "";
      $blob .= substr("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/", int(rand(64)), 1)
        while length($blob) < $len;
      printf "{\"tool\":\"read_file\",\"bytes\":%d,\"data\":\"%s\"}\n", $len, $blob;
    }
  }
' > "$OUT/bench-agent.txt"

echo "--- generated ---"
wc -c "$OUT/bench-ascii.txt" "$OUT/bench-unicode.txt" "$OUT/bench-agent.txt"
ls -lh "$OUT/bench-ascii.txt" "$OUT/bench-unicode.txt" "$OUT/bench-agent.txt"
