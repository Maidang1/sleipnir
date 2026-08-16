#!/usr/bin/env bash
# Same-machine end-to-end drain comparison (B1 / B5a), fully automated.
#
# Method: launch a terminal, find the pty of the shell it spawned, and write the
# corpus straight into that pty's slave (`/dev/ttysNNN`). That is byte-for-byte
# what a program running in the pane would emit, and the write blocks until the
# emulator drains the master — so wall time is the emulator's ingest time.
#
# Why not `time cat` typed into each terminal (the manual runbook): typing needs
# Accessibility permission, and the shell adds its own prompt/OSC work. Writing to
# the pty is identical for every terminal under test, which is what makes the
# comparison fair.
#
# Fairness notes:
#   * every terminal is put frontmost and composited before measuring — an
#     occluded window is not drawn, which would flatter it (the run is skipped,
#     not silently accepted, if it will not come forward);
#   * scrollback is forced to a comparable size where the CLI allows it.
#     Retention policy matters: a terminal that drops lines early "finishes"
#     sooner. Ghostty budgets scrollback in *bytes*, not lines, so it cannot be
#     matched exactly — noted rather than hidden.
#
# Usage: scripts/bench/drain-compare.sh [reps]
set -euo pipefail

REPS="${1:-2}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CORPUS="$HERE/corpus"
APP="${SLEIPNIR_APP:-$ROOT/build/Sleipnir.app}"
LINES=10000

now() { perl -MTime::HiRes=time -e 'print time'; }
frontmost_name() { lsappinfo info -only name "$(lsappinfo front)" 2>/dev/null || true; }

wait_front() {
  for _ in 1 2 3 4 5 6; do
    case "$(frontmost_name)" in *"$1"*) return 0 ;; esac
    sleep 1
  done
  return 1
}

# All ttys owned by the app's children (the shells it spawned).
ttys_of() {
  ps -o tty= -p "$(pgrep -P "$1" | tr '\n' ',' | sed 's/,$//')" 2>/dev/null |
    tr -d ' ' | grep -v '^??$' | sort -u
}

drain() { # $1 = label, $2 = tty
  local label="$1" tty="$2" f file bytes t0 t1 dt
  for f in ascii unicode agent; do
    file="$CORPUS/bench-$f.txt"
    [ -f "$file" ] || continue
    bytes=$(wc -c < "$file" | tr -d ' ')
    t0=$(now); cat "$file" > "/dev/$tty"; t1=$(now)
    dt=$(perl -e "print $t1 - $t0")
    printf '%-10s %-8s %7.3f s  %7.1f MB/s\n' \
      "$label" "$f" "$dt" "$(perl -e "printf '%.1f', $bytes / 1e6 / $dt")"
    sleep 2
  done
}

# label | process name | frontmost fragment | launch command
SPECS=(
  "sleipnir|sleipnir|Sleipnir|open '$APP'"
  "ghostty|ghostty|Ghostty|/Applications/Ghostty.app/Contents/MacOS/ghostty"
  "kitty|kitty|kitty|/Applications/kitty.app/Contents/MacOS/kitty --override scrollback_lines=$LINES"
  "alacritty|alacritty|Alacritty|/Applications/Alacritty.app/Contents/MacOS/alacritty -o scrolling.history=$LINES"
)

echo "corpus: $CORPUS"
printf '%-10s %-8s %9s  %11s\n' terminal corpus wall throughput
echo

for rep in $(seq "$REPS"); do
  echo "--- rep $rep ---"
  for spec in "${SPECS[@]}"; do
    IFS='|' read -r label proc front cmd <<< "$spec"
    bin="${cmd%% *}"
    [ "$bin" = "open" ] || [ -x "$bin" ] || { echo "$label: not installed"; continue; }

    pkill -x "$proc" 2>/dev/null || true
    sleep 1
    if [ "$bin" = "open" ]; then
      eval "$cmd"
    else
      (nohup $cmd >"/tmp/$label-bench.log" 2>&1 &)
    fi
    sleep 5

    if ! wait_front "$front"; then
      echo "$label: SKIPPED (frontmost is $(frontmost_name), window would not be drawn)"
      pkill -x "$proc" 2>/dev/null || true
      continue
    fi
    pid=$(pgrep -x "$proc" | head -1 || true)
    tty=$(ttys_of "$pid" | head -1)
    if [ -z "$tty" ]; then
      echo "$label: SKIPPED (no pty found for pid $pid)"
    else
      drain "$label" "$tty"
    fi
    pkill -x "$proc" 2>/dev/null || true
    sleep 1
  done
  echo
done

echo "scrollback: kitty/alacritty forced to $LINES lines; Sleipnir uses"
echo "max_scroll_history_lines (default 10000); Ghostty budgets bytes, not lines."
