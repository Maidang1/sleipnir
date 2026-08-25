#!/usr/bin/env bash
# make-app.sh — Build sleipnir and package it as a macOS .app bundle.
#
# Usage:
#   ./scripts/make-app.sh [OPTIONS]
#
# Options:
#   --sign <identity>   Code-sign with the given identity (default: ad-hoc if none)
#   --notarize          Notarize after signing (requires APPLE_ID / APPLE_APP_SPECIFIC_PASSWORD)
#   --dmg               No-op (a .dmg is always produced); kept for compatibility
#   --release           Build in release mode (default: release)
#
# Environment:
#   SLEIPNIR_APP_NAME    App bundle name (default: Sleipnir)
#   SLEIPNIR_VERSION     CFBundleVersion override (default: from Cargo.toml)
#   APPLE_ID           Apple ID for notarization
#   APPLE_APP_SPECIFIC_PASSWORD  App-specific password for notarization
#   GH_TOKEN           GitHub token (needed for --dmg with gh upload)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

APP_NAME="${SLEIPNIR_APP_NAME:-Sleipnir}"
APP_BUNDLE="${APP_NAME}.app"
BUILD_DIR="${ROOT}/build"
SIGN_IDENTITY=""
NOTARIZE=false
MAKE_DMG=false
RELEASE=true

# ── Parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --sign)  SIGN_IDENTITY="$2"; shift 2 ;;
        --notarize) NOTARIZE=true; shift ;;
        --dmg)   MAKE_DMG=true; shift ;;
        --debug) RELEASE=false; shift ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

# ── Determine version ─────────────────────────────────────────────────────────
if [[ -n "${SLEIPNIR_VERSION:-}" ]]; then
    VERSION="${SLEIPNIR_VERSION}"
else
    VERSION=$(grep '^version' crates/sleipnir/Cargo.toml | head -1 | awk '{print $3}' | tr -d '"')
fi

echo "=== sleipnir make-app  v${VERSION} ==="
echo "  release=${RELEASE}  sign=${SIGN_IDENTITY:-'(ad-hoc)'}  notarize=${NOTARIZE}  dmg=${MAKE_DMG}"

# ── Build ─────────────────────────────────────────────────────────────────────
if [[ "${RELEASE}" == "true" ]]; then
    cargo build --release -p sleipnir -p sleipnir-update-helper 2>&1 | tail -5
    BIN="${ROOT}/target/release/sleipnir"
    UPDATE_HELPER="${ROOT}/target/release/sleipnir-update-helper"
else
    cargo build -p sleipnir -p sleipnir-update-helper 2>&1 | tail -5
    BIN="${ROOT}/target/debug/sleipnir"
    UPDATE_HELPER="${ROOT}/target/debug/sleipnir-update-helper"
fi

if [[ ! -x "${BIN}" || ! -x "${UPDATE_HELPER}" ]]; then
    echo "ERROR: application or update helper binary is missing" >&2
    exit 1
fi

# ── Assemble .app bundle ──────────────────────────────────────────────────────
rm -rf "${BUILD_DIR}/${APP_BUNDLE}"
APP="${BUILD_DIR}/${APP_BUNDLE}"

mkdir -p "${APP}/Contents/MacOS"
mkdir -p "${APP}/Contents/Resources"

# Copy application and transactional update supervisor.
cp "${BIN}" "${APP}/Contents/MacOS/sleipnir"
cp "${UPDATE_HELPER}" "${APP}/Contents/MacOS/sleipnir-update-helper"
chmod 755 "${APP}/Contents/MacOS/sleipnir" "${APP}/Contents/MacOS/sleipnir-update-helper"

# Copy Info.plist (patch version)
plist="${ROOT}/resources/Info.plist"
sed -e "s|0.1.0|${VERSION}|g" "${plist}" > "${APP}/Contents/Info.plist"

# Copy the AppleScript scripting definition (minimal read-only + quit suite).
cp "${ROOT}/resources/Sleipnir.sdef" "${APP}/Contents/Resources/Sleipnir.sdef"
cp "${ROOT}/resources/update-signing-public.pem" \
    "${APP}/Contents/Resources/update-signing-public.pem"

# Copy app icon (.icns preferred; fall back to building from iconset)
if [[ -f "${ROOT}/resources/AppIcon.icns" ]]; then
    cp "${ROOT}/resources/AppIcon.icns" "${APP}/Contents/Resources/AppIcon.icns"
elif [[ -d "${ROOT}/resources/AppIcon.iconset" ]]; then
    iconutil -c icns "${ROOT}/resources/AppIcon.iconset" \
        -o "${APP}/Contents/Resources/AppIcon.icns"
fi

echo "  bundle: ${APP}"
ls -la "${APP}/Contents/"

# ── Code sign ─────────────────────────────────────────────────────────────────
if [[ -n "${SIGN_IDENTITY}" ]]; then
    echo "  signing with: ${SIGN_IDENTITY}"
    codesign --sign "${SIGN_IDENTITY}" \
        --entitlements "${ROOT}/resources/sleipnir.entitlements" \
        --deep --force --options runtime \
        "${APP}"
else
    echo "  WARNING: no --sign identity. Applying ad-hoc sign."
    codesign -s - --deep --force "${APP}"
fi

# ── Verify ────────────────────────────────────────────────────────────────────
if ! codesign --verify --deep --strict "${APP}" 2>&1 | head -5; then
    echo "ERROR: app bundle signature verification failed" >&2
    exit 1
fi
spctl --assess --type execute --verbose=4 "${APP}" 2>&1 || true

# ── Package .dmg ──────────────────────────────────────────────────────────────
# Stage only the .app so the disk image contains nothing else.
DMG_SRC="${BUILD_DIR}/dmg-src"
rm -rf "${DMG_SRC}"
mkdir -p "${DMG_SRC}"
ditto "${APP}" "${DMG_SRC}/${APP_BUNDLE}"

DMG="${BUILD_DIR}/${APP_NAME}-${VERSION}-macos.dmg"
rm -f "${DMG}"
VOL_NAME="${APP_NAME}-${VERSION}"
hdiutil create -volname "${VOL_NAME}" \
    -srcfolder "${DMG_SRC}" \
    -ov -format UDZO \
    "${DMG}"
rm -rf "${DMG_SRC}"
echo "  dmg: ${DMG} ($(du -sh "${DMG}" | cut -f1))"

# ── (Optional) Notarize ──────────────────────────────────────────────────────
if [[ "${NOTARIZE}" == "true" ]]; then
    if [[ -z "${APPLE_ID:-}" || -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]]; then
        echo "ERROR: APPLE_ID and APPLE_APP_SPECIFIC_PASSWORD required for notarization" >&2
        exit 1
    fi
    echo "  notarizing..."
    xcrun notarytool submit "${DMG}" \
        --apple-id "${APPLE_ID}" \
        --password "${APPLE_APP_SPECIFIC_PASSWORD}" \
        --team-id "692T845H5K" \
        --wait
    xcrun stapler staple "${DMG}"
    echo "  notarized ✓"
fi

# SHA-256 sidecar for the auto-updater / install.sh to verify downloads.
# Computed after any notarization/stapling so the digest matches the shipped bytes.
( cd "${BUILD_DIR}" && shasum -a 256 "$(basename "${DMG}")" \
    | awk '{print $1}' > "$(basename "${DMG}").sha256" )
echo "  sha256: $(cat "${DMG}.sha256")"
"${ROOT}/scripts/sign-update-manifest.py" \
    --version "${VERSION}" \
    --dmg "${DMG}" \
    --output "${BUILD_DIR}/sleipnir-update-v1.json" \
    --signature "${BUILD_DIR}/sleipnir-update-v1.json.sig"

echo ""
echo "=== Done ==="
echo "  artifacts in: ${BUILD_DIR}/"
ls -lh "${BUILD_DIR}"/*.dmg "${BUILD_DIR}"/*.dmg.sha256 \
    "${BUILD_DIR}/sleipnir-update-v1.json" \
    "${BUILD_DIR}/sleipnir-update-v1.json.sig" 2>/dev/null || true
