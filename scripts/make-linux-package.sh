#!/usr/bin/env bash
# make-linux-package.sh — Build sleipnir for Linux and produce installable
# packages: a portable `.tar.gz` and a native Ubuntu/Debian `.deb`.
#
# Usage:
#   ./scripts/make-linux-package.sh [OPTIONS]
#
# Options:
#   --debug          Build a debug binary instead of release
#   --no-deb         Skip the .deb package (tar.gz only)
#   --no-tar         Skip the .tar.gz package (deb only)
#   --no-strip       Keep debug symbols in the packaged binary
#   --out <dir>      Output directory (default: ./build)
#
# Environment:
#   SLEIPNIR_VERSION     Version override (default: from crates/sleipnir/Cargo.toml)
#
# Requires (Ubuntu):
#   build-essential, pkg-config, libxkbcommon-dev, libx11-dev, libwayland-dev,
#   libvulkan1, python3-pil (for icon resizing), dpkg-dev (for the .deb).
#
# On a headless machine, install libvulkan1 + the Mesa Vulkan driver so the
# GPU/software renderer can initialize at runtime.

set -euo pipefail

# Map dpkg/uname arch names onto the trio used in portable tarball names.
# ubuntu `dpkg --print-architecture` reports amd64, not x86_64.
linux_trio_arch() {
    case "${1:-}" in
        x86_64|amd64) printf '%s\n' x86_64 ;;
        aarch64|arm64) printf '%s\n' aarch64 ;;
        *) printf '%s\n' "${1:-}" ;;
    esac
}

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

APP_NAME="sleipnir"
RELEASE=true
MAKE_DEB=true
MAKE_TAR=true
STRIP_BIN=true
OUT_DIR="${ROOT}/build"

# ── Parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)      RELEASE=false; shift ;;
        --no-deb)     MAKE_DEB=false; shift ;;
        --no-tar)     MAKE_TAR=false; shift ;;
        --no-strip)   STRIP_BIN=false; shift ;;
        --out)        OUT_DIR="$2"; shift 2 ;;
        --print-trio-arch)
            linux_trio_arch "${2:-}"
            exit 0
            ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

# ── Determine version ─────────────────────────────────────────────────────────
if [[ -n "${SLEIPNIR_VERSION:-}" ]]; then
    VERSION="${SLEIPNIR_VERSION}"
else
    VERSION=$(grep '^version' crates/sleipnir/Cargo.toml | head -1 | awk '{print $3}' | tr -d '"')
fi

ARCH="$(dpkg --print-architecture 2>/dev/null || uname -m)"
TRIO_ARCH="$(linux_trio_arch "${ARCH}")"

echo "=== sleipnir make-linux-package  v${VERSION} (${ARCH}) ==="
echo "  release=${RELEASE}  deb=${MAKE_DEB}  tar=${MAKE_TAR}  strip=${STRIP_BIN}  out=${OUT_DIR}"

# ── Build ─────────────────────────────────────────────────────────────────────
if [[ "${RELEASE}" == "true" ]]; then
    cargo build --release -p sleipnir
    BIN="${ROOT}/target/release/sleipnir"
else
    cargo build -p sleipnir
    BIN="${ROOT}/target/debug/sleipnir"
fi

if [[ ! -x "${BIN}" ]]; then
    echo "ERROR: binary not found at ${BIN}" >&2
    exit 1
fi

mkdir -p "${OUT_DIR}"
WORK="${OUT_DIR}/linux-work"
rm -rf "${WORK}"

# ── Resize icons (hicolor) ────────────────────────────────────────────────────
ICON_SRC="${ROOT}/resources/appicon_preview.png"
ICON_DIR="${WORK}/icons"
mkdir -p "${ICON_DIR}"
SIZES="48 64 128 256 512"
for s in ${SIZES}; do
    out="${ICON_DIR}/${s}x${s}/apps/sleipnir.png"
    mkdir -p "$(dirname "${out}")"
    python3 - "${ICON_SRC}" "${out}" "${s}" <<'PY'
import sys
from PIL import Image
src, dst, size = sys.argv[1], sys.argv[2], int(sys.argv[3])
im = Image.open(src).convert("RGBA")
im.thumbnail((size, size), Image.LANCZOS)
im.save(dst, "PNG")
PY
done
echo "  icons: ${SIZES} sizes"

# ── Portable .tar.gz ──────────────────────────────────────────────────────────
if [[ "${MAKE_TAR}" == "true" ]]; then
    TAR_DIR="${WORK}/Sleipnir-${VERSION}-linux-${TRIO_ARCH}"
    mkdir -p "${TAR_DIR}"
    if [[ "${STRIP_BIN}" == "true" ]]; then
        cp "${BIN}" "${TAR_DIR}/sleipnir"
        strip "${TAR_DIR}/sleipnir"
    else
        cp "${BIN}" "${TAR_DIR}/sleipnir"
    fi
    cp "${ROOT}/resources/linux/sleipnir.desktop" "${TAR_DIR}/sleipnir.desktop"
    cp "${ROOT}/resources/appicon_preview.png" "${TAR_DIR}/sleipnir.png"

    cat > "${TAR_DIR}/README.txt" <<'EOF'
Sleipnir - Linux portable build
===============================

Run the terminal with:

    ./sleipnir

Requirements (Ubuntu 22.04+ / Debian 12+):

  - Vulkan driver:      sudo apt install libvulkan1 mesa-vulkan-drivers
  - X11:                libx11-6 libxcb1 libxkbcommon0 libxkbcommon-x11-0
  - Wayland:            libwayland-client0
  - Desktop integration: sudo desktop-file-install sleipnir.desktop

Sleipnir prefers Wayland when WAYLAND_DISPLAY is set, otherwise X11.
Set WAYLAND_DISPLAY= to force X11.
EOF

    TARBALL="${OUT_DIR}/Sleipnir-${VERSION}-linux-${TRIO_ARCH}.tar.gz"
    rm -f "${TARBALL}"
    tar -C "${WORK}" -czf "${TARBALL}" "Sleipnir-${VERSION}-linux-${TRIO_ARCH}"
    echo "  tar: ${TARBALL}"
fi

# ── .deb package ──────────────────────────────────────────────────────────────
if [[ "${MAKE_DEB}" == "true" ]]; then
    PKG="sleipnir_${VERSION}_${ARCH}"
    DEB_ROOT="${WORK}/${PKG}"
    mkdir -p "${DEB_ROOT}/DEBIAN"
    mkdir -p "${DEB_ROOT}/usr/bin"
    mkdir -p "${DEB_ROOT}/usr/share/applications"
    mkdir -p "${DEB_ROOT}/usr/share/doc/sleipnir"
    mkdir -p "${DEB_ROOT}/usr/share/licenses/sleipnir"

    if [[ "${STRIP_BIN}" == "true" ]]; then
        cp "${BIN}" "${DEB_ROOT}/usr/bin/sleipnir"
        strip "${DEB_ROOT}/usr/bin/sleipnir"
    else
        cp "${BIN}" "${DEB_ROOT}/usr/bin/sleipnir"
    fi
    chmod 755 "${DEB_ROOT}/usr/bin/sleipnir"

    cp "${ROOT}/resources/linux/sleipnir.desktop" "${DEB_ROOT}/usr/share/applications/sleipnir.desktop"
    for s in ${SIZES}; do
        mkdir -p "${DEB_ROOT}/usr/share/icons/hicolor/${s}x${s}/apps"
        cp "${ICON_DIR}/${s}x${s}/apps/sleipnir.png" \
           "${DEB_ROOT}/usr/share/icons/hicolor/${s}x${s}/apps/sleipnir.png"
    done
    # Symlink the largest icon as the generic fallback too.
    mkdir -p "${DEB_ROOT}/usr/share/icons/hicolor/scalable/apps"
    cp "${ICON_DIR}/512x512/apps/sleipnir.png" \
       "${DEB_ROOT}/usr/share/icons/hicolor/512x512/apps/sleipnir.png"
    ln -sf "/usr/share/icons/hicolor/512x512/apps/sleipnir.png" \
        "${DEB_ROOT}/usr/share/icons/hicolor/scalable/apps/sleipnir.png"

    # LICENSE / copyright.
    cp "${ROOT}/LICENSE-GPL" "${DEB_ROOT}/usr/share/licenses/sleipnir/LICENSE"
    cat > "${DEB_ROOT}/usr/share/doc/sleipnir/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Sleipnir
Source: https://github.com/Maidang1/sleipnir

Files: *
License: GPL-3.0-or-later
 Sleipnir reuses and adapts code from Zed (https://github.com/zed-industries/zed).
 GPUI and related UI crates are Apache-2.0; the terminal crates are
 GPL-3.0-or-later. Because GPL terminal code is included, distribution of the
 combined work is GPL-3.0-or-later.
EOF

    # Changelog (best-effort from CHANGELOG.md).
    if [[ -f "${ROOT}/CHANGELOG.md" ]]; then
        gzip -9 -c "${ROOT}/CHANGELOG.md" > "${DEB_ROOT}/usr/share/doc/sleipnir/changelog.gz"
        chmod 644 "${DEB_ROOT}/usr/share/doc/sleipnir/changelog.gz"
    fi

    # Dependencies: dynamic libs from the binary plus dlopen'd runtimes (wgpu's
    # Vulkan, wayland-client, xdg-open) that ldd cannot see.
    if command -v dpkg-shlibdeps >/dev/null 2>&1; then
        SHLIB_DEPS="$(dpkg-shlibdeps -O "${DEB_ROOT}/usr/bin/sleipnir" 2>/dev/null \
            | sed 's/^Depends: //' || true)"
    fi
    EXTRA_DEPS="libvulkan1"
    RECOMMENDS="xdg-utils, libwayland-client0, libglib2.0-0, libfontconfig1"
    if [[ -n "${SHLIB_DEPS:-}" ]]; then
        DEPENDS="${SHLIB_DEPS}, ${EXTRA_DEPS}"
    else
        DEPENDS="libc6, libx11-6, libxcb1, libxkbcommon0, libxkbcommon-x11-0, ${EXTRA_DEPS}"
    fi

    cat > "${DEB_ROOT}/DEBIAN/control" <<EOF
Package: sleipnir
Version: ${VERSION}
Section: x11
Priority: optional
Architecture: ${ARCH}
Depends: ${DEPENDS}
Recommends: ${RECOMMENDS}
Installed-Size: $(du -sk "${DEB_ROOT}/usr" | cut -f1)
Maintainer: Sleipnir Developers <noreply@example.com>
Homepage: https://github.com/Maidang1/sleipnir
Description: GPU-accelerated terminal emulator with tabs and splits
 Sleipnir is a standalone terminal emulator built on GPUI. It renders through
 the GPU (Vulkan on Linux), ships a real PTY, supports multi-tab and split
 panes, multi-window sessions, adaptive theming and Zed-compatible
 terminal.* settings.
EOF

    DEB="${OUT_DIR}/${PKG}.deb"
    rm -f "${DEB}"
    dpkg-deb --build --root-owner-group "${DEB_ROOT}" "${DEB}" >/dev/null
    echo "  deb: ${DEB}"
fi

# ── Hash + summary ────────────────────────────────────────────────────────────
echo ""
echo "=== Done ==="
echo "  artifacts in: ${OUT_DIR}/"
for f in "${OUT_DIR}"/Sleipnir-${VERSION}-linux-*.tar.gz "${OUT_DIR}"/sleipnir_${VERSION}_${ARCH}.deb; do
    if [[ -f "${f}" ]]; then
        sha256sum "${f}" | tee "${f}.sha256"
    fi
done

rm -rf "${WORK}"