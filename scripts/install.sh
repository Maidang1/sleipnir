#!/usr/bin/env bash
# install.sh — Download the latest Sleipnir macOS release into /Applications
# and clear Gatekeeper quarantine (ad-hoc signed CI builds).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
#   PREFIX="$HOME/Applications" ./scripts/install.sh
#
# Environment:
#   PREFIX           Install directory (default: /Applications)
#   SLEIPNIR_NO_OPEN Set to 1 to skip launching the app after install

set -euo pipefail

REPO="${SLEIPNIR_REPO:-Maidang1/sleipnir}"
APP_NAME="Sleipnir"
PREFIX="${PREFIX:-/Applications}"
DEST="${PREFIX}/${APP_NAME}.app"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "ERROR: Sleipnir's prebuilt installer is macOS-only." >&2
    exit 1
fi

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "ERROR: missing required command: $1" >&2
        exit 1
    }
}
need curl
need ditto
need shasum
need hdiutil
need xattr

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install.XXXXXX")"
cleanup() { rm -rf "${WORKDIR}"; }
trap cleanup EXIT

echo "=== Sleipnir install ==="
echo "  repo:   ${REPO}"
echo "  dest:   ${DEST}"

echo "  fetching latest release…"
# Follow /releases/latest → /releases/tag/vX.Y.Z. Avoids the unauthenticated
# GitHub API (easy to 403 from a shared IP) and needs no python/jq.
LATEST_URL="$(curl -fsSL -o /dev/null -w '%{url_effective}' \
    -H "User-Agent: sleipnir-install" \
    "https://github.com/${REPO}/releases/latest")"
TAG="${LATEST_URL##*/}"
VERSION="${TAG#v}"
if [[ -z "${VERSION}" || "${VERSION}" == "${LATEST_URL}" ]]; then
    echo "ERROR: could not resolve latest tag from ${LATEST_URL}" >&2
    exit 1
fi
DMG_URL="https://github.com/${REPO}/releases/download/${TAG}/${APP_NAME}-${VERSION}-macos.dmg"
SHA_URL="${DMG_URL}.sha256"

echo "  version: ${VERSION} (${TAG})"

DMG="${WORKDIR}/${APP_NAME}-${VERSION}-macos.dmg"
echo "  downloading $(basename "${DMG}")…"
curl -fL --retry 3 --retry-delay 1 -o "${DMG}" "${DMG_URL}"

if [[ -n "${SHA_URL}" ]]; then
    SHA_FILE="${DMG}.sha256"
    curl -fL --retry 3 --retry-delay 1 -o "${SHA_FILE}" "${SHA_URL}"
    WANT="$(tr -d ' \n\r\t' < "${SHA_FILE}")"
    GOT="$(shasum -a 256 "${DMG}" | awk '{print $1}')"
    if [[ "${WANT}" != "${GOT}" ]]; then
        echo "ERROR: SHA-256 mismatch" >&2
        echo "  expected: ${WANT}" >&2
        echo "  got:      ${GOT}" >&2
        exit 1
    fi
    echo "  sha256: ${GOT}  ok"
else
    echo "  warning: no .dmg.sha256 sidecar — skipping integrity check" >&2
fi

echo "  mounting…"
MOUNT="${WORKDIR}/mnt"
mkdir -p "${MOUNT}"
hdiutil attach "${DMG}" -nobrowse -noautoopen -mountpoint "${MOUNT}" >/dev/null
detach_dmg() { hdiutil detach "${MOUNT}" -force >/dev/null 2>&1 || true; }
trap 'detach_dmg; cleanup' EXIT

MOUNTED_APP="${MOUNT}/${APP_NAME}.app"
if [[ ! -d "${MOUNTED_APP}" ]]; then
    echo "ERROR: disk image did not contain ${APP_NAME}.app" >&2
    exit 1
fi
APP="${WORKDIR}/${APP_NAME}.app"
ditto "${MOUNTED_APP}" "${APP}"
detach_dmg

# Ad-hoc CI builds trip Gatekeeper via the download quarantine flag.
# Clear all xattrs (including com.apple.quarantine) before first launch.
xattr -cr "${APP}"

mkdir -p "${PREFIX}"
if [[ ! -w "${PREFIX}" ]]; then
    echo "  ${PREFIX} is not writable — using sudo"
    sudo mkdir -p "${PREFIX}"
    sudo rm -rf "${DEST}"
    sudo ditto "${APP}" "${DEST}"
    sudo chown -R "$(id -un):staff" "${DEST}"
    sudo xattr -cr "${DEST}"
else
    rm -rf "${DEST}"
    ditto "${APP}" "${DEST}"
    xattr -cr "${DEST}"
fi

echo "  installed: ${DEST}"
echo "  quarantine cleared (xattr -cr) — Gatekeeper will not block this copy"

if [[ "${SLEIPNIR_NO_OPEN:-0}" != "1" ]]; then
    open "${DEST}"
fi

echo "=== done ==="
