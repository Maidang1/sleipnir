#!/usr/bin/env bash
# Dry-run: compare local forks against a Zed checkout; print pinned revs.
# Usage: ./scripts/upstream-diff.sh [ZED_ROOT]

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
  echo "sleipnir upstream dry-run"
  echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "sleipnir: $(git -C "${ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo "zed:    $(git -C "${ZED_ROOT}" rev-parse --short HEAD) $(git -C "${ZED_ROOT}" log -1 --format='%ci')"
  echo "zed_root: ${ZED_ROOT}"
  echo

  echo "== pinned revs in sleipnir Cargo.toml =="
  rg -N 'rev = "' "${ROOT}/Cargo.toml" || true
  echo

  echo "== alacritty_terminal rev =="
  echo -n "sleipnir: "
  rg -N 'alacritty_terminal = \{ git' "${ROOT}/Cargo.toml" || true
  echo -n "zed:    "
  rg -N 'alacritty_terminal = \{ git' "${ZED_ROOT}/Cargo.toml" || true
  echo

  echo "== GPUI stack =="
  echo "Not vendored. Consumed via git pin from zed-industries/zed (see Cargo.toml)."
  echo "Local checkout tip vs pin is informational only."
  echo

  # Local forks only
  PATHS=(
    terminal/src
    gpui_platform/src
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
      echo "(missing in Zed — expected for sleipnir-only paths)"
      echo
      continue
    fi
    if diff -rq "${local_p}" "${zed_p}" >/tmp/sleipnir-upstream-rq.txt 2>/dev/null; then
      echo "identical"
    else
      changed=$(grep -c ' differ$' /tmp/sleipnir-upstream-rq.txt || true)
      only_local=$(grep -c "^Only in ${local_p}" /tmp/sleipnir-upstream-rq.txt || true)
      only_zed=$(grep -c "^Only in ${zed_p}" /tmp/sleipnir-upstream-rq.txt || true)
      echo "files differ: ${changed:-0}  only-local: ${only_local:-0}  only-zed: ${only_zed:-0}"
      head -40 /tmp/sleipnir-upstream-rq.txt | sed 's|^|  |'
      if [[ $(wc -l </tmp/sleipnir-upstream-rq.txt) -gt 40 ]]; then
        echo "  ... (truncated)"
      fi
    fi
    echo
  done

  echo "== sleipnir-only crates =="
  for c in sleipnir_settings sleipnir sleipnir_ui release_channel; do
    if [[ -d "${ROOT}/crates/${c}" ]]; then
      echo "  crates/${c}"
    fi
  done
  echo
  echo "Done. To upgrade GPUI: bump all Zed rev= pins in Cargo.toml (see UPSTREAM.md)."
} | tee "${OUT}"

echo "wrote ${OUT}"
