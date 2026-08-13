#!/bin/bash
# Run INSIDE Sleipnir's shell (typed into it) to measure end-to-end `cat` throughput.
#
# The output of `cat` flows into Sleipnir's PTY (the terminal drains it), so the
# wall time = how fast Sleipnir ingests the corpus — exactly the B1 metric.
# Results are written to a file so the osascript driver can read them back.
#
# Usage (type into Sleipnir):
#   /Users/bytedance/codes/myself/harbor/scripts/bench/bench-in-shell.sh

OUT="${1:-/tmp/sleipnir-bench-results.txt}"
CORPUS="$(cd "$(dirname "$0")" && pwd)/corpus"

: > "$OUT"

measure() {
    name="$1"
    file="$2"
    t0=$(perl -MTime::HiRes=time -e 'print time')
    cat "$file"
    t1=$(perl -MTime::HiRes=time -e 'print time')
    dt=$(perl -e "print $t1 - $t0")
    printf '%s %.3f s\n' "$name" "$dt" >> "$OUT"
}

measure "unicode" "$CORPUS/bench-unicode.txt"
measure "ascii"   "$CORPUS/bench-ascii.txt"
echo "BENCH_DONE" >> "$OUT"
echo "wrote results to $OUT" >> "$OUT"
