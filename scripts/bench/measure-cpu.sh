#!/usr/bin/env bash
# Sample a running Sleipnir's CPU / RSS from an OUTSIDE shell.
#
# This is the measuring half of the B5c check: "output in a background tab must
# not cost the window anything". Procedure:
#
#   1. scripts/bench/measure-cpu.sh 10            # idle baseline
#   2. in Sleipnir tab A: scripts/bench/agent-stream.sh 120
#   3. switch to tab B, leave the stream running in the background
#   4. scripts/bench/measure-cpu.sh 10            # background-tab cost
#   5. switch back to tab A (stream now visible)
#   6. scripts/bench/measure-cpu.sh 10            # foreground cost
#
# Acceptance: step 4 must be within a few points of step 1. If step 4 looks like
# step 6, an off-screen pane is driving window repaints -- see
# `AppShell::is_pane_visible`.
#
# Usage: scripts/bench/measure-cpu.sh [samples] [interval_seconds]
set -euo pipefail

SAMPLES="${1:-10}"
INTERVAL="${2:-1}"
PROC="${PROC_NAME:-sleipnir}"

pids=$(pgrep -x "$PROC" || true)
if [ -z "$pids" ]; then
  echo "no '$PROC' process found (set PROC_NAME to override)" >&2
  exit 1
fi
count=$(echo "$pids" | wc -l | tr -d ' ')
if [ "$count" -gt 1 ]; then
  echo "warning: $count '$PROC' processes; summing all of them:" >&2
  echo "$pids" | tr '\n' ' ' >&2
  echo >&2
fi

echo "sampling $PROC: ${SAMPLES}x every ${INTERVAL}s"
: > /tmp/sleipnir-cpu-samples.txt
for _ in $(seq "$SAMPLES"); do
  # %cpu here is a short-interval average maintained by the kernel, not a
  # since-boot average, so repeated sampling reflects current load.
  line=$(ps -o %cpu=,rss= -p $(echo "$pids" | tr '\n' ',' | sed 's/,$//') |
    awk '{ cpu += $1; rss += $2 } END { printf "%.1f %.1f", cpu, rss / 1024 }')
  echo "$line" >> /tmp/sleipnir-cpu-samples.txt
  printf '  cpu %5s%%   rss %6s MB\n' $(echo "$line" | awk '{print $1, $2}')
  sleep "$INTERVAL"
done

awk '{ c += $1; if ($1 > cm) cm = $1; r += $2; if ($2 > rm) rm = $2; n++ }
     END { printf "\nmean cpu %.1f%%  max cpu %.1f%%  |  mean rss %.0f MB  max rss %.0f MB  (n=%d)\n",
                  c / n, cm, r / n, rm, n }' /tmp/sleipnir-cpu-samples.txt
