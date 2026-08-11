#!/usr/bin/env bash
# publish-release.sh — Create or update a GitHub Release and upload sleipnir artifacts.
#
# Usage:
#   ./scripts/publish-release.sh [VERSION] [ARTIFACT_DIR]
#
# Examples:
#   ./scripts/publish-release.sh                  # uses last git tag or --version
#   ./scripts/publish-release.sh 0.2.0 ./build
#
# Requires: `gh` CLI, logged in (`gh auth login`)
#
# Environment:
#   GH_TOKEN           (optional, also read from gh auth)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

VERSION="${1:-}"
ARTIFACT_DIR="${2:-${ROOT}/build}"

# ── Determine version ─────────────────────────────────────────────────────────
if [[ -z "${VERSION}" ]]; then
    # Try git tag
    VERSION=$(git describe --tags --abbrev=0 2>/dev/null || true)
fi
if [[ -z "${VERSION}" ]]; then
    # Fallback: Cargo.toml
    VERSION=$(grep '^version' crates/sleipnir/Cargo.toml | head -1 | awk '{print $3}' | tr -d '"')
fi

if [[ -z "${VERSION}" ]]; then
    echo "ERROR: could not determine version. Pass as arg 1, or set a git tag." >&2
    exit 1
fi

echo "=== publish-release  v${VERSION} ==="

# ── Validate artifacts ────────────────────────────────────────────────────────
ZIP="${ARTIFACT_DIR}/${APP_NAME:-Sleipnir}-${VERSION}-macos.zip"
SHA="${ZIP}.sha256"
DMG="${ARTIFACT_DIR}/${APP_NAME:-Sleipnir}-${VERSION}-macos.dmg"

if [[ ! -f "${ZIP}" ]]; then
    echo "ERROR: zip not found at ${ZIP}" >&2
    exit 1
fi

# SHA-256 sidecar for the auto-updater to verify downloads.
( cd "${ARTIFACT_DIR}" && shasum -a 256 "$(basename "${ZIP}")" \
    | awk '{print $1}' > "$(basename "${SHA}")" )
echo "  sha256: $(cat "${SHA}")"

# ── Create / update GitHub Release ───────────────────────────────────────────
TAG="v${VERSION}"

# Check if tag already exists
if gh release view "${TAG}" --json tagName 2>/dev/null | grep -q '"tagName"'; then
    echo "  release ${TAG} already exists — updating..."
    RELEASE_ARGS="--notes-update"
else
    echo "  creating release ${TAG}..."
    RELEASE_ARGS=""
fi

# Generate release notes from CHANGELOG if available, otherwise brief notes
NOTES_FILE=$(mktemp)
trap 'rm -f "${NOTES_FILE}"' EXIT
if [[ -f "${ROOT}/CHANGELOG.md" ]]; then
    # Grab the section for this version (between this version header and the next)
    awk -v ver="## ${VERSION}" '
        $0 ~ ver {found=1; next}
        found && /^## / {exit}
        found {print}
    ' "${ROOT}/CHANGELOG.md" > "${NOTES_FILE}" 2>/dev/null || true
fi

if [[ ! -s "${NOTES_FILE}" ]]; then
    cat > "${NOTES_FILE}" <<EOF
## Sleipnir v${VERSION}

### What's new
- Initial macOS release build.

### Install
Download \`Sleipnir-${VERSION}-macos.dmg\` (recommended) or \`Sleipnir-${VERSION}-macos.zip\`.
Open the .dmg and drag Sleipnir to Applications.

### Requirements
- macOS 14.0+ (Sonoma)
- Rust 1.95+ (only needed for building from source)

### Source
https://github.com/Maidang1/sleipnir
EOF
fi

gh release create "${TAG}" \
    --target main \
    --title "Sleipnir v${VERSION}" \
    --notes-file "${NOTES_FILE}" \
    ${RELEASE_ARGS:-} \
    --draft=false \
    "${ZIP}" \
    "${SHA}" \
    "${DMG:+${DMG}}"

echo ""
echo "=== Published ==="
gh release view "${TAG}" --json url --jq '.url'
echo "  artifacts:"
gh release download "${TAG}" --dir /tmp/sleipnir-release-${VERSION} 2>/dev/null || true
ls -lh /tmp/sleipnir-release-${VERSION}/ 2>/dev/null || true
rm -rf /tmp/sleipnir-release-${VERSION}
