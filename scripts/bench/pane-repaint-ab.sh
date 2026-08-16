#!/usr/bin/env bash
# B5c, automated: what does output in a *background* pane cost the window?
#
# The manual runbook (README §5c) needs a human to switch tabs. This script needs
# nobody, and no Accessibility permission either, because it never sends
# keystrokes. Instead it writes the agent-shaped stream **directly into each
# pane's slave pty** (`/dev/ttysNNN`), which is byte-for-byte what a program
# running in that pane would emit, and samples Sleipnir's CPU while it does.
#
# It deliberately does *not* try to learn which tab is active (that needs UI
# introspection). It streams into every pane in turn and lets the numbers say it:
#
#   * healthy build   -> exactly one pane is expensive (the visible one),
#                        the others sit near idle
#   * regressed build -> every pane is expensive, visible or not
#
# ⚠️ VALIDITY GATE: the app must be **frontmost and composited**, otherwise macOS
# does not draw it, repaint cost is ~0 for *every* pane, and the test silently
# measures nothing. That is why this script launches a real .app bundle with
# `open` and refuses to measure unless `lsappinfo front` says Sleipnir is front.
# Launching the bare `target/release/sleipnir` from a background shell does NOT
# satisfy this.
#
# Usage:
#   scripts/bench/pane-repaint-ab.sh <sleipnir-binary> [stream_seconds]
#   scripts/bench/pane-repaint-ab.sh target/release/sleipnir 20
#
# Requires a bundle at build/Sleipnir.app (run scripts/make-app.sh once). The
# given binary is copied into that bundle and ad-hoc signed, so A/B-ing two
# builds needs no rebuild of the bundle.
#
# Safety: backs up ~/.config/sleipnir/session.json and restores it on exit; the
# seeded session is 2 tabs in $HOME.
set -euo pipefail

BIN="${1:?usage: pane-repaint-ab.sh <sleipnir-binary> [stream_seconds]}"
STREAM_SECS="${2:-20}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
APP="${SLEIPNIR_APP:-$ROOT/build/Sleipnir.app}"
SESSION="$HOME/.config/sleipnir/session.json"
BACKUP="$SESSION.grillbak.$$"

cleanup() {
  pkill -x sleipnir 2>/dev/null || true
  if [ -f "$BACKUP" ]; then
    mv -f "$BACKUP" "$SESSION"
    echo "restored $SESSION"
  fi
}
trap cleanup EXIT

# Sample CPU over `n` one-second intervals. `top` is used instead of
# `ps -o %cpu` because ps reports a decaying lifetime average that blurs bursts.
sample_cpu() {
  local pid="$1" n="$2"
  # Only bare single-field numbers are %CPU rows; top also prints a
  # `2026/08/15 02:28:13` timestamp per sample, which /^[0-9]/ would swallow.
  top -l "$((n + 1))" -s 1 -pid "$pid" -stats cpu 2>/dev/null |
    awk 'NF == 1 && $1 ~ /^[0-9]+(\.[0-9]+)?$/ { v[++i] = $1 } END {
      # drop the first sample: top reports a lifetime figure for it
      s = 0; c = 0
      for (j = 2; j <= i; j++) { s += v[j]; c++ }
      if (c > 0) printf "%.1f", s / c; else printf "n/a"
    }'
}

[ -d "$APP" ] || { echo "no bundle at $APP — run scripts/make-app.sh first" >&2; exit 1; }
[ -x "$BIN" ] || { echo "not executable: $BIN" >&2; exit 1; }

pkill -x sleipnir 2>/dev/null || true
sleep 1

[ -f "$SESSION" ] && cp -p "$SESSION" "$BACKUP"
mkdir -p "$(dirname "$SESSION")"
cat > "$SESSION" <<JSON
{
  "version": 1,
  "active_tab": 0,
  "tabs": [
    { "active_pane": 1, "tree": { "type": "leaf", "id": 1, "cwd": "$HOME" } },
    { "active_pane": 2, "tree": { "type": "leaf", "id": 2, "cwd": "$HOME" } }
  ]
}
JSON

echo "staging $BIN into $APP"
cp "$BIN" "$APP/Contents/MacOS/sleipnir"
# arm64 refuses to exec an unsigned Mach-O; ad-hoc is enough for a local run.
codesign -s - --force "$APP/Contents/MacOS/sleipnir" 2>/dev/null

open "$APP"
sleep 6
PID="$(pgrep -x sleipnir | head -1 || true)"
[ -n "$PID" ] || { echo "sleipnir did not start" >&2; exit 1; }

# Focus can bounce back to whatever launched us; retry `open` a few times.
front=""
for _ in 1 2 3 4 5; do
  front="$(lsappinfo info -only name "$(lsappinfo front)" 2>/dev/null || true)"
  case "$front" in *Sleipnir*) break ;; esac
  open "$APP"
  sleep 2
done
case "$front" in
  *Sleipnir*) : ;;
  *)
    echo "VALIDITY FAIL: frontmost app is $front, not Sleipnir." >&2
    echo "An occluded window is not drawn, so repaint cost would read as ~0 for" >&2
    echo "every pane and the comparison would be meaningless. Aborting." >&2
    exit 1
    ;;
esac
echo "frontmost: $front (pid $PID)"

TTYS=$(ps -o tty= -p "$(pgrep -P "$PID" | tr '\n' ',' | sed 's/,$//')" | tr -d ' ' | grep -v '^??$' | sort -u)
count=$(echo "$TTYS" | wc -l | tr -d ' ')
echo "panes: $(echo "$TTYS" | tr '\n' ' ')"
[ "$count" -ge 2 ] || { echo "need >= 2 panes; got $count" >&2; exit 1; }

echo
printf '%-16s %8s\n' "phase" "cpu%"
IDLE=$(sample_cpu "$PID" 5)
printf '%-16s %8s\n' "idle" "$IDLE"

is_front() { case "$(lsappinfo info -only name "$(lsappinfo front)" 2>/dev/null || true)" in *Sleipnir*) return 0 ;; *) return 1 ;; esac; }

for t in $TTYS; do
  "$HERE/agent-stream.sh" "$((STREAM_SECS + 6))" 4 > "/dev/$t" 2>/dev/null &
  streamer=$!
  sleep 3   # let the stream reach steady state before sampling
  # Focus must hold for the WHOLE sample, not just at startup. If another app
  # steals the foreground mid-sample, macOS stops drawing Sleipnir and the
  # visible pane reads as cheap as a hidden one — the exact false negative that
  # made an earlier run of this script "prove" a fix that does nothing.
  front_before=no; is_front && front_before=yes
  cpu=$(sample_cpu "$PID" "$STREAM_SECS")
  front_after=no; is_front && front_after=yes
  if [ "$front_before" = yes ] && [ "$front_after" = yes ]; then
    printf '%-16s %8s\n' "stream $t" "$cpu"
  else
    printf '%-16s %8s   INVALID (lost foreground during sample)\n' "stream $t" "$cpu"
  fi
  wait "$streamer" 2>/dev/null || true
  sleep 2
done

echo
echo "read it like this: the visible pane's number should stand out; every other"
echo "pane should sit near idle ($IDLE%). If they are all high, off-screen panes"
echo "are still driving repaints — see AppShell::is_pane_visible."
