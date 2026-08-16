#!/usr/bin/env bash
# make-app.sh — Build sleipnir and package it as a macOS .app bundle.
#
# Usage:
#   ./scripts/make-app.sh [OPTIONS]
#
# Options:
#   --sign <identity>   Code-sign with the given identity (default: ad-hoc if none)
#   --notarize          Notarize after signing (requires APPLE_ID / APPLE_APP_SPECIFIC_PASSWORD)
#   --dmg               Also create a .dmg after packaging
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
    cargo build --release -p sleipnir 2>&1 | tail -5
    BIN="${ROOT}/target/release/sleipnir"
else
    cargo build -p sleipnir 2>&1 | tail -5
    BIN="${ROOT}/target/debug/sleipnir"
fi

if [[ ! -x "${BIN}" ]]; then
    echo "ERROR: binary not found at ${BIN}" >&2
    exit 1
fi

# ── Assemble .app bundle ──────────────────────────────────────────────────────
rm -rf "${BUILD_DIR}/${APP_BUNDLE}"
APP="${BUILD_DIR}/${APP_BUNDLE}"

mkdir -p "${APP}/Contents/MacOS"
mkdir -p "${APP}/Contents/Resources"

# Copy binary (must match CFBundleExecutable in Info.plist)
cp "${BIN}" "${APP}/Contents/MacOS/sleipnir"

# Copy Info.plist (patch version)
plist="${ROOT}/resources/Info.plist"
sed -e "s|0.1.0|${VERSION}|g" "${plist}" > "${APP}/Contents/Info.plist"

# Copy the AppleScript scripting definition (minimal read-only + quit suite).
cp "${ROOT}/resources/Sleipnir.sdef" "${APP}/Contents/Resources/Sleipnir.sdef"

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
elif [[ "${MAKE_DMG}" == "true" ]]; then
    # dmg distribution ideally needs a valid identity; fall back to ad-hoc if possible
    echo "  WARNING: no --sign identity and --dmg requested. Applying ad-hoc sign."
    codesign -s - --deep --force "${APP}"
fi

# ── Verify ────────────────────────────────────────────────────────────────────
codesign --verify --deep --strict "${APP}" 2>&1 | head -5 || true
spctl --assess --type execute --verbose=4 "${APP}/Contents/MacOS/${APP_NAME}" 2>&1 || true

# ── Package .zip ──────────────────────────────────────────────────────────────
ZIP="${BUILD_DIR}/${APP_NAME}-${VERSION}-macos.zip"
rm -f "${ZIP}"
ditto -c -k --sequesterRsrc --keepParent "${APP}" "${ZIP}"
echo "  zip: ${ZIP} ($(du -sh "${ZIP}" | cut -f1))"

# ── (Optional) Notarize ──────────────────────────────────────────────────────
if [[ "${NOTARIZE}" == "true" ]]; then
    if [[ -z "${APPLE_ID:-}" || -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]]; then
        echo "ERROR: APPLE_ID and APPLE_APP_SPECIFIC_PASSWORD required for notarization" >&2
        exit 1
    fi
    echo "  notarizing..."
    xcrun notarytool submit "${ZIP}" \
        --apple-id "${APPLE_ID}" \
        --password "${APPLE_APP_SPECIFIC_PASSWORD}" \
        --team-id "692T845H5K" \
        --wait
    xcrun stapler staple "${APP}"
    echo "  notarized ✓"
fi

# ── (Optional) Create .dmg ───────────────────────────────────────────────────
if [[ "${MAKE_DMG}" == "true" ]]; then
    DMG="${BUILD_DIR}/${APP_NAME}-${VERSION}-macos.dmg"
    rm -f "${DMG}"
    # Volume name for the dmg
    VOL_NAME="${APP_NAME}-${VERSION}"
    hdiutil create -volname "${VOL_NAME}" \
        -srcfolder "${BUILD_DIR}" \
        -ov -format UDZO \
        "${DMG}"
    echo "  dmg: ${DMG} ($(du -sh "${DMG}" | cut -f1))"
fi

echo ""
echo "=== Done ==="
echo "  artifacts in: ${BUILD_DIR}/"
ls -lh "${BUILD_DIR}"/*.zip "${BUILD_DIR}"/*.dmg 2>/dev/null || true
