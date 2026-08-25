#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS transactional update tests: SKIP (not Darwin)"
  exit 0
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

cargo test -p sleipnir-update-helper \
  swap::tests::swaps_two_directories_atomically -- --exact
cargo test -p sleipnir-update-helper \
  process::tests::kqueue_observes_child_exit -- --exact
cargo test -p sleipnir-update-helper --test supervisor

# The integration suite uses fake process/filesystem/launcher adapters to prove
# success, timeout, launch failure, rollback failure, and unsafe termination.
# Packaging verification below proves the production helper is embedded and
# covered by the application bundle signature.
if [[ -d build/Sleipnir.app ]]; then
  test -x build/Sleipnir.app/Contents/MacOS/sleipnir-update-helper
  codesign --verify --deep --strict build/Sleipnir.app
fi

echo "macOS transactional update tests: PASS"
