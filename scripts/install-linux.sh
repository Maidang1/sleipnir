#!/usr/bin/env bash
# install-linux.sh — Download the latest Sleipnir Linux release.
#
# Default (Ubuntu/Debian): download the .deb and install it with apt.
# Portable tarball: SLEIPNIR_TARBALL=1 PREFIX="$HOME/.local" ./scripts/install-linux.sh
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install-linux.sh | bash
#
# Environment:
#   PREFIX           Tarball install prefix (default: $HOME/.local)
#   SLEIPNIR_TARBALL Set to 1 to install the portable .tar.gz instead of the .deb
#   SLEIPNIR_NO_OPEN Set to 1 to skip launching after install
#   SLEIPNIR_REPO    GitHub owner/repo (default: Maidang1/sleipnir)

set -euo pipefail

REPO="${SLEIPNIR_REPO:-Maidang1/sleipnir}"
APP_NAME="Sleipnir"
PREFIX="${PREFIX:-${HOME}/.local}"
USER_AGENT="sleipnir-install"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "ERROR: this installer is Linux-only." >&2
    exit 1
fi

HOST_ARCH="$(uname -m)"
case "${HOST_ARCH}" in
    x86_64|amd64)
        DEB_ARCH="amd64"
        TRIO_ARCH="x86_64"
        ;;
    *)
        echo "ERROR: no prebuilt Linux package for ${HOST_ARCH} (x86_64 only)." >&2
        echo "Build from source: https://github.com/${REPO}#build--run" >&2
        exit 1
        ;;
esac

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "ERROR: missing required command: $1" >&2
        exit 1
    }
}
need curl

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "ERROR: need sha256sum or shasum to verify the download." >&2
        exit 1
    fi
}

verify_sha() {
    local file="$1"
    local sha_url="$2"
    local sha_file="${file}.sha256"
    if ! curl -fL --retry 3 --retry-delay 1 -A "${USER_AGENT}" -o "${sha_file}" "${sha_url}"; then
        echo "  warning: no .sha256 sidecar — skipping integrity check" >&2
        return 0
    fi
    local want got
    want="$(tr -d ' \n\r\t' < "${sha_file}")"
    got="$(sha256_file "${file}")"
    if [[ "${want}" != "${got}" ]]; then
        echo "ERROR: SHA-256 mismatch" >&2
        echo "  expected: ${want}" >&2
        echo "  got:      ${got}" >&2
        exit 1
    fi
    echo "  sha256: ${got}  ok"
}

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install.XXXXXX")"
cleanup() { rm -rf "${WORKDIR}"; }
trap cleanup EXIT

echo "=== Sleipnir install ==="
echo "  repo:   ${REPO}"

echo "  fetching latest release…"
LATEST_URL="$(curl -fsSL -o /dev/null -w '%{url_effective}' \
    -A "${USER_AGENT}" \
    "https://github.com/${REPO}/releases/latest")"
TAG="${LATEST_URL##*/}"
VERSION="${TAG#v}"
if [[ -z "${VERSION}" || "${VERSION}" == "${LATEST_URL}" ]]; then
    echo "ERROR: could not resolve latest tag from ${LATEST_URL}" >&2
    exit 1
fi
echo "  version: ${VERSION} (${TAG})"

use_tarball() {
    [[ "${SLEIPNIR_TARBALL:-0}" == "1" ]] || ! command -v apt-get >/dev/null 2>&1
}

if use_tarball; then
    TAR_NAME="${APP_NAME}-${VERSION}-linux-${TRIO_ARCH}.tar.gz"
    TAR_URL="https://github.com/${REPO}/releases/download/${TAG}/${TAR_NAME}"
    TAR="${WORKDIR}/${TAR_NAME}"
    echo "  dest:   ${PREFIX}/bin/sleipnir"
    echo "  downloading ${TAR_NAME}…"
    curl -fL --retry 3 --retry-delay 1 -A "${USER_AGENT}" -o "${TAR}" "${TAR_URL}"
    verify_sha "${TAR}" "${TAR_URL}.sha256"

    echo "  unpacking…"
    tar -xzf "${TAR}" -C "${WORKDIR}"
    SRC="${WORKDIR}/${APP_NAME}-${VERSION}-linux-${TRIO_ARCH}/sleipnir"
    if [[ ! -f "${SRC}" ]]; then
        echo "ERROR: archive did not contain sleipnir" >&2
        exit 1
    fi
    mkdir -p "${PREFIX}/bin"
    install -m 755 "${SRC}" "${PREFIX}/bin/sleipnir"
    DEST="${PREFIX}/bin/sleipnir"
    echo "  installed: ${DEST}"
    if [[ ":${PATH}:" != *":${PREFIX}/bin:"* ]]; then
        echo "  note: add ${PREFIX}/bin to PATH to run sleipnir from any shell"
    fi
else
    DEB_NAME="sleipnir_${VERSION}_${DEB_ARCH}.deb"
    DEB_URL="https://github.com/${REPO}/releases/download/${TAG}/${DEB_NAME}"
    DEB="${WORKDIR}/${DEB_NAME}"
    echo "  dest:   apt package ${DEB_NAME}"
    echo "  downloading ${DEB_NAME}…"
    curl -fL --retry 3 --retry-delay 1 -A "${USER_AGENT}" -o "${DEB}" "${DEB_URL}"
    # .deb has no published sidecar today; the tarball path above is the
    # checksum-verified alternative (SLEIPNIR_TARBALL=1).
    echo "  installing (sudo apt)…"
    sudo apt-get install -y "${DEB}"
    DEST="$(command -v sleipnir || true)"
    echo "  installed: ${DEST:-sleipnir (on PATH after apt)}"
fi

if [[ "${SLEIPNIR_NO_OPEN:-0}" != "1" ]]; then
    if command -v sleipnir >/dev/null 2>&1; then
        nohup sleipnir >/dev/null 2>&1 &
    elif [[ -x "${DEST:-}" ]]; then
        nohup "${DEST}" >/dev/null 2>&1 &
    fi
fi

echo "=== done ==="
echo "  Vulkan driver required: sudo apt install libvulkan1 mesa-vulkan-drivers"
