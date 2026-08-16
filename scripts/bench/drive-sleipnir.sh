#!/bin/bash
# Drive Sleipnir via osascript: activate it, type the B1 benchmark command into
# its shell, and read the results back from the file that bench-in-shell.sh writes.
#
# PREREQUISITE (one-time): grant Accessibility ("辅助功能") to the terminal/harness
# that runs this script, otherwise `keystroke` is blocked by macOS
# (System Settings → Privacy & Security → Accessibility).
#
# Usage: scripts/bench/drive-sleipnir.sh
set -euo pipefail

RESULT="${RESULT_FILE:-/tmp/sleipnir-bench-results.txt}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CMD="${BENCH_CMD:-$SCRIPT_DIR/bench-in-shell.sh}"
rm -f "$RESULT"

echo "[1/3] activating Sleipnir..."
osascript <<'APPLESCRIPT'
tell application "Sleipnir" to activate
delay 0.6
APPLESCRIPT

echo "[2/3] typing benchmark command..."
osascript <<APPLESCRIPT
tell application "System Events"
    keystroke "$CMD"
    key code 36 -- Return
end tell
APPLESCRIPT

echo "[3/3] waiting for results..."
timeout=$(( ${TIMEOUT_SECS:-30} * 5 ))
for _ in $(seq 1 "$timeout"); do
    if grep -q BENCH_DONE "$RESULT" 2>/dev/null; then
        break
    fi
    sleep 0.2
done

if grep -q BENCH_DONE "$RESULT" 2>/dev/null; then
    echo "--- results ---"
    cat "$RESULT"
else
    echo "TIMEOUT: no BENCH_DONE marker in $RESULT" >&2
    echo "--- partial results ---"
    cat "$RESULT" 2>/dev/null || true
    exit 1
fi
