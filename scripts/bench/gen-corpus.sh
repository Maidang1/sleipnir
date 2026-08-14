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

echo "--- generated ---"
wc -c "$OUT/bench-ascii.txt" "$OUT/bench-unicode.txt"
ls -lh "$OUT/bench-ascii.txt" "$OUT/bench-unicode.txt"
