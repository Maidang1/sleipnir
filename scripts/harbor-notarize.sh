#!/usr/bin/env bash
# harbor-notarize.sh — Notarize a harbor .app bundle using xcrun notarytool.
#
# Usage:
#   ./scripts/harbor-notarize.sh <path/to/Harbor.app>
#
# Environment variables (all required):
#   APPLE_ID                    Your Apple ID email
#   APPLE_APP_SPECIFIC_PASSWORD App-specific password (not your Apple ID password)
#   APPLE_TEAM_ID               Your Apple Developer team ID

set -euo pipefail

APP_PATH="${1:-}"
if [[ -z "${APP_PATH}" ]]; then
    echo "usage: $0 <path/to/Harbor.app>" >&2
    exit 2
fi
if [[ ! -d "${APP_PATH}" ]]; then
    echo "ERROR: not a directory: ${APP_PATH}" >&2
    exit 1
fi

if [[ -z "${APPLE_ID:-}" || -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
    echo "ERROR: APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, and APPLE_TEAM_ID must be set" >&2
    exit 1
fi

echo "=== notarize  $(basename "${APP_PATH}") ==="

# Staple first (works if already notarized; otherwise no-op)
echo "  stapling..."
xcrun stapler staple "${APP_PATH}" || true

# Package as zip (notarytool needs a zip, not a .app directly)
ZIP="/tmp/harbor-notarize-$(basename "${APP_PATH}" .app).zip"
rm -f "${ZIP}"
ditto -c -k --sequesterRsrc --keepParent "${APP_PATH}" "${ZIP}"

echo "  submitting to notarytool..."
xcrun notarytool submit "${ZIP}" \
    --apple-id "${APPLE_ID}" \
    --password "${APPLE_APP_SPECIFIC_PASSWORD}" \
    --team-id "${APPLE_TEAM_ID}" \
    --wait

echo "  stapling after notarization..."
xcrun stapler staple "${APP_PATH}"

rm -f "${ZIP}"
echo "  done ✓"
