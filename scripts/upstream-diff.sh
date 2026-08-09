#!/usr/bin/env bash
# Dry-run: summarize diffs between this repo's copied crates and a Zed checkout.
# Usage: ./scripts/upstream-diff.sh [ZED_ROOT]
# Default ZED_ROOT: $ZED_ROOT env, else ../open-source/zed relative to this monorepo layout.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ZED_ROOT="${1:-${ZED_ROOT:-}}"
if [[ -z "${ZED_ROOT}" ]]; then
  if [[ -d "${ROOT}/../open-source/zed" ]]; then
    ZED_ROOT="$(cd "${ROOT}/../open-source/zed" && pwd)"
  elif [[ -d "${HOME}/codes/open-source/zed" ]]; then
    ZED_ROOT="${HOME}/codes/open-source/zed"
  else
    echo "usage: $0 /path/to/zed" >&2
    exit 2
  fi
fi

if [[ ! -d "${ZED_ROOT}/crates" ]]; then
  echo "not a Zed checkout: ${ZED_ROOT}" >&2
  exit 2
fi

OUT="${ROOT}/docs/upstream-last-diff.txt"
mkdir -p "${ROOT}/docs"

{
  echo "jiajia-term upstream dry-run"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "jiajia: $(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo "zed:    $(git -C "${ZED_ROOT}" rev-parse --short HEAD) $(git -C "${ZED_ROOT}" log -1 --format='%ci')"
  echo "zed_root: ${ZED_ROOT}"
  echo

  echo "== alacritty_terminal rev =="
  echo -n "jiajia: "
  rg -N 'alacritty_terminal = \{ git' "${ROOT}/Cargo.toml" || true
  echo -n "zed:    "
  rg -N 'alacritty_terminal = \{ git' "${ZED_ROOT}/Cargo.toml" || true
  echo

  # Paths relative to crates/
  PATHS=(
    terminal/src
    gpui/src
    gpui_macos/src
    gpui_platform/src
    collections/src
    util/src
  )

  for rel in "${PATHS[@]}"; do
    local_p="${ROOT}/crates/${rel}"
    zed_p="${ZED_ROOT}/crates/${rel}"
    echo "== diff: crates/${rel} =="
    if [[ ! -d "${local_p}" ]]; then
      echo "(missing locally)"
      echo
      continue
    fi
    if [[ ! -d "${zed_p}" ]]; then
      echo "(missing in Zed)"
      echo
      continue
    fi
    # stat-only summary + short stat
    if diff -rq "${local_p}" "${zed_p}" >/tmp/jiajia-upstream-rq.txt 2>/dev/null; then
      echo "identical"
    else
      # count differing files
      changed=$(grep -c ' differ$' /tmp/jiajia-upstream-rq.txt || true)
      only_local=$(grep -c "^Only in ${local_p}" /tmp/jiajia-upstream-rq.txt || true)
      only_zed=$(grep -c "^Only in ${zed_p}" /tmp/jiajia-upstream-rq.txt || true)
      echo "files differ: ${changed:-0}  only-local: ${only_local:-0}  only-zed: ${only_zed:-0}"
      # top of rq list (cap)
      head -40 /tmp/jiajia-upstream-rq.txt | sed 's|^|  |'
      if [[ $(wc -l </tmp/jiajia-upstream-rq.txt) -gt 40 ]]; then
        echo "  ... (truncated)"
      fi
    fi
    echo
  done

  echo "== jiajia-only crates (not from Zed wholesale) =="
  for c in jiajia_settings jiajia_term jiajia_term_ui task_types release_channel; do
    if [[ -d "${ROOT}/crates/${c}" ]]; then
      echo "  crates/${c}"
    fi
  done
  echo
  echo "Done. Review this file; apply ports manually per UPSTREAM.md."
} | tee "${OUT}"

echo "wrote ${OUT}"
