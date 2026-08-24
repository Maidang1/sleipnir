#!/usr/bin/env bash
# Build deterministic native Linux tar and Debian packages for Sleipnir.
set -euo pipefail

# Staging modes must not inherit the caller's umask. Individual packaged files
# are normalized below as well, but this fixes every intermediate directory.
umask 022

portable_arch_for() {
    case "${1:-}" in
        x86_64|amd64) printf '%s\n' x86_64 ;;
        aarch64|arm64) printf '%s\n' aarch64 ;;
        *) echo "ERROR: unsupported Linux architecture: ${1:-unknown}" >&2; return 1 ;;
    esac
}

debian_arch_for() {
    case "${1:-}" in
        x86_64|amd64) printf '%s\n' amd64 ;;
        aarch64|arm64) printf '%s\n' arm64 ;;
        *) echo "ERROR: unsupported Linux architecture: ${1:-unknown}" >&2; return 1 ;;
    esac
}

tarball_name() { printf 'Sleipnir-%s-linux-%s.tar.gz\n' "$1" "$2"; }
deb_name() { printf 'sleipnir_%s_%s.deb\n' "$1" "$2"; }

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print tolower($1)}'
    else
        shasum -a 256 "$1" | awk '{print tolower($1)}'
    fi
}

merge_debian_dependencies() {
    python3 - "$1" <<'PY'
import re, sys
explicit = [
    "libx11-6", "libxcb1", "libxkbcommon0", "libxkbcommon-x11-0",
    "libvulkan1", "libwayland-client0", "libfontconfig1", "xdg-utils", "libnotify-bin",
]
items = [item.strip() for item in sys.argv[1].split(",") if item.strip()]
present = {re.split(r"[ (]", item, maxsplit=1)[0] for item in items}
for dependency in explicit:
    if dependency not in present:
        items.append(dependency)
print(", ".join(items))
PY
}

if [[ "${SLEIPNIR_PACKAGE_SOURCE_ONLY:-0}" == "1" ]]; then
    # `return` handles sourcing; `exit` handles direct execution.
    # shellcheck disable=SC2317
    return 0 2>/dev/null || exit 0
fi

usage() {
    cat <<'EOF'
Usage: scripts/make-linux-package.sh [options]
  --binary <path>  package an existing native release binary
  --out <dir>      output directory (default: ./build)
  --no-deb         skip the Debian package
  --no-tar         skip the portable tarball
  --no-strip       retain symbols in packaged binaries
EOF
}

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${ROOT}/build"
BINARY=""
MAKE_DEB=1
MAKE_TAR=1
STRIP_BINARY=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary)
            [[ $# -ge 2 ]] || { echo "ERROR: --binary requires a path" >&2; exit 2; }
            BINARY="$2"; shift 2 ;;
        --out)
            [[ $# -ge 2 ]] || { echo "ERROR: --out requires a directory" >&2; exit 2; }
            OUT_DIR="$2"; shift 2 ;;
        --no-deb) MAKE_DEB=0; shift ;;
        --no-tar) MAKE_TAR=0; shift ;;
        --no-strip) STRIP_BINARY=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ "${MAKE_DEB}" == "0" && "${MAKE_TAR}" == "0" ]]; then
    echo "ERROR: --no-deb and --no-tar cannot be combined" >&2
    exit 2
fi

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "ERROR: missing required command: $1" >&2
        exit 1
    }
}

need python3
if [[ -z "${SLEIPNIR_VERSION:-}" ]]; then
    need cargo
fi

VERSION="${SLEIPNIR_VERSION:-$(
    cargo metadata --no-deps --format-version 1 \
        | python3 -c 'import json,sys; data=json.load(sys.stdin); print(next(p["version"] for p in data["packages"] if p["name"] == "sleipnir"))'
)}"

if [[ -n "${SLEIPNIR_PACKAGE_ARCH:-}" ]]; then
    NATIVE_ARCH="${SLEIPNIR_PACKAGE_ARCH}"
elif command -v dpkg >/dev/null 2>&1; then
    NATIVE_ARCH="$(dpkg --print-architecture)"
else
    NATIVE_ARCH="$(uname -m)"
fi
PORTABLE_ARCH="$(portable_arch_for "${NATIVE_ARCH}")"
DEBIAN_ARCH="$(debian_arch_for "${NATIVE_ARCH}")"

if [[ -z "${BINARY}" ]]; then
    need cargo
    cargo build --release -p sleipnir
    BINARY="${ROOT}/target/release/sleipnir"
fi
[[ -f "${BINARY}" && -x "${BINARY}" ]] || {
    echo "ERROR: executable binary not found: ${BINARY}" >&2
    exit 1
}

if [[ "${SLEIPNIR_PACKAGE_SKIP_ELF_CHECK:-0}" != "1" ]]; then
    need file
    ELF_DESCRIPTION="$(file -b "${BINARY}")"
    [[ "${ELF_DESCRIPTION}" == *ELF* ]] || {
        echo "ERROR: existing binary is not an ELF executable: ${BINARY}" >&2
        exit 1
    }
    case "${PORTABLE_ARCH}" in
        x86_64) [[ "${ELF_DESCRIPTION}" == *x86-64* || "${ELF_DESCRIPTION}" == *x86_64* ]] ;;
        aarch64) [[ "${ELF_DESCRIPTION}" == *aarch64* || "${ELF_DESCRIPTION}" == *ARM\ aarch64* ]] ;;
    esac || {
        echo "ERROR: binary architecture does not match ${PORTABLE_ARCH}: ${ELF_DESCRIPTION}" >&2
        exit 1
    }
fi

if [[ "${STRIP_BINARY}" == "1" ]]; then
    need strip
fi
if [[ "${MAKE_DEB}" == "1" ]]; then
    need dpkg-deb
    need dpkg-shlibdeps
    python3 -c 'from PIL import Image' >/dev/null 2>&1 || {
        echo "ERROR: Python Pillow is required to build the Debian icons" >&2
        exit 1
    }
fi

SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "${ROOT}" log -1 --format=%ct 2>/dev/null || printf '0')}"
[[ "${SOURCE_DATE_EPOCH}" =~ ^[0-9]+$ ]] || {
    echo "ERROR: SOURCE_DATE_EPOCH must be an integer" >&2
    exit 1
}
export SOURCE_DATE_EPOCH

mkdir -p "${OUT_DIR}"
OUT_DIR="$(cd "${OUT_DIR}" && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-package.XXXXXX")"
cleanup() { rm -rf "${WORK}"; }
trap cleanup EXIT

copy_binary() {
    local destination="$1"
    cp "${BINARY}" "${destination}"
    chmod 755 "${destination}"
    if [[ "${STRIP_BINARY}" == "1" ]]; then
        strip "${destination}"
    fi
}

write_readme() {
    cat > "$1" <<'EOF'
Sleipnir - Linux portable build
===============================

Run Sleipnir with ./sleipnir. This build targets Ubuntu 22.04+ and requires
glibc 2.35+, a Vulkan driver, Wayland or X11, fontconfig, xdg-open (xdg-utils),
and notify-send (libnotify-bin).

Optional per-user desktop installation:

  BIN_HOME="${XDG_BIN_HOME:-$HOME/.local/bin}"
  DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
  install -Dm755 sleipnir "$BIN_HOME/sleipnir"
  install -Dm644 sleipnir.desktop "$DATA_HOME/applications/sleipnir.desktop"
  install -Dm644 sleipnir.png "$DATA_HOME/icons/hicolor/512x512/apps/sleipnir.png"
  command -v update-desktop-database >/dev/null && update-desktop-database "$DATA_HOME/applications"

Ensure BIN_HOME is in PATH before launching `sleipnir` from the desktop entry.
EOF
}

normalize_tree() {
    python3 - "$1" "${SOURCE_DATE_EPOCH}" <<'PY'
import os, stat, sys
root, epoch = sys.argv[1], int(sys.argv[2])
for directory, dirs, files in os.walk(root):
    os.chmod(directory, 0o755, follow_symlinks=False)
    os.utime(directory, (epoch, epoch), follow_symlinks=False)
    for name in dirs:
        path = os.path.join(directory, name)
        os.chmod(path, 0o755, follow_symlinks=False)
        os.utime(path, (epoch, epoch), follow_symlinks=False)
    for name in files:
        path = os.path.join(directory, name)
        mode = os.lstat(path).st_mode
        os.chmod(path, 0o755 if mode & stat.S_IXUSR else 0o644, follow_symlinks=False)
        os.utime(path, (epoch, epoch), follow_symlinks=False)
os.chmod(root, 0o755, follow_symlinks=False)
os.utime(root, (epoch, epoch), follow_symlinks=False)
PY
}

create_reproducible_tarball() {
    local stage_parent="$1" stage_name="$2" output="$3"
    python3 - "${stage_parent}" "${stage_name}" "${output}" "${SOURCE_DATE_EPOCH}" <<'PY'
import gzip, os, sys, tarfile
parent, root_name, output, epoch = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
root = os.path.join(parent, root_name)
paths = [root]
for directory, dirs, files in os.walk(root):
    dirs.sort()
    files.sort()
    paths.extend(os.path.join(directory, item) for item in dirs + files)
with open(output, "wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, compresslevel=9, mtime=0) as gz:
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.PAX_FORMAT) as tf:
            for path in sorted(paths, key=lambda p: os.path.relpath(p, parent)):
                arcname = os.path.relpath(path, parent)
                info = tf.gettarinfo(path, arcname)
                info.uid = info.gid = 0
                info.uname = info.gname = ""
                info.mtime = epoch
                if info.isfile():
                    with open(path, "rb") as source:
                        tf.addfile(info, source)
                else:
                    tf.addfile(info)
PY
}

write_sidecar() {
    local artifact="$1" expected
    sha256_file "${artifact}" > "${artifact}.sha256"
    expected="$(cat "${artifact}.sha256")"
    [[ "${expected}" =~ ^[0-9a-f]{64}$ && "${expected}" == "$(sha256_file "${artifact}")" ]] || {
        echo "ERROR: failed to verify generated checksum: ${artifact}" >&2
        exit 1
    }
}

README_FILE="${WORK}/README.txt"
write_readme "${README_FILE}"

if [[ "${MAKE_TAR}" == "1" ]]; then
    TAR_ROOT_NAME="Sleipnir-${VERSION}-linux-${PORTABLE_ARCH}"
    TAR_ROOT="${WORK}/tar/${TAR_ROOT_NAME}"
    mkdir -p "${TAR_ROOT}"
    copy_binary "${TAR_ROOT}/sleipnir"
    cp "${ROOT}/resources/linux/sleipnir.desktop" "${TAR_ROOT}/sleipnir.desktop"
    cp "${ROOT}/resources/appicon_preview.png" "${TAR_ROOT}/sleipnir.png"
    cp "${README_FILE}" "${TAR_ROOT}/README.txt"
    cp "${ROOT}/LICENSE-GPL" "${TAR_ROOT}/LICENSE"
    chmod 644 "${TAR_ROOT}/sleipnir.desktop" "${TAR_ROOT}/sleipnir.png" \
        "${TAR_ROOT}/README.txt" "${TAR_ROOT}/LICENSE"
    normalize_tree "${WORK}/tar"
    TARBALL="${OUT_DIR}/$(tarball_name "${VERSION}" "${PORTABLE_ARCH}")"
    create_reproducible_tarball "${WORK}/tar" "${TAR_ROOT_NAME}" "${TARBALL}"
    write_sidecar "${TARBALL}"
    echo "created ${TARBALL}"
fi

if [[ "${MAKE_DEB}" == "1" ]]; then
    DEB_ROOT="${WORK}/deb-root"
    mkdir -p \
        "${DEB_ROOT}/DEBIAN" \
        "${DEB_ROOT}/usr/bin" \
        "${DEB_ROOT}/usr/share/applications" \
        "${DEB_ROOT}/usr/share/doc/sleipnir" \
        "${DEB_ROOT}/usr/share/licenses/sleipnir"
    copy_binary "${DEB_ROOT}/usr/bin/sleipnir"
    install -m 644 "${ROOT}/resources/linux/sleipnir.desktop" \
        "${DEB_ROOT}/usr/share/applications/sleipnir.desktop"
    install -m 644 "${README_FILE}" "${DEB_ROOT}/usr/share/doc/sleipnir/README.txt"
    install -m 644 "${ROOT}/LICENSE-GPL" "${DEB_ROOT}/usr/share/licenses/sleipnir/LICENSE"
    cat > "${DEB_ROOT}/usr/share/doc/sleipnir/copyright" <<'EOF'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: Sleipnir
Source: https://github.com/Maidang1/sleipnir
Files: *
License: GPL-3.0-or-later
 See /usr/share/licenses/sleipnir/LICENSE for the complete license text.
EOF

    python3 - "${ROOT}/CHANGELOG.md" "${DEB_ROOT}/usr/share/doc/sleipnir/changelog.gz" <<'PY'
import gzip, sys
source, output = sys.argv[1:]
with open(source, "rb") as src, open(output, "wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, compresslevel=9, mtime=0) as dst:
        dst.write(src.read())
PY

    for size in 48 64 128 256 512; do
        icon="${DEB_ROOT}/usr/share/icons/hicolor/${size}x${size}/apps/sleipnir.png"
        mkdir -p "$(dirname "${icon}")"
        python3 - "${ROOT}/resources/appicon_preview.png" "${icon}" "${size}" <<'PY'
import sys
from PIL import Image
source, output, size = sys.argv[1], sys.argv[2], int(sys.argv[3])
with Image.open(source) as image:
    image.convert("RGBA").resize((size, size), Image.Resampling.LANCZOS).save(output, "PNG", optimize=False)
PY
    done

    SHLIB_WORK="${WORK}/shlibdeps"
    mkdir -p "${SHLIB_WORK}/debian"
    cat > "${SHLIB_WORK}/debian/control" <<'EOF'
Source: sleipnir
Section: x11
Priority: optional
Maintainer: Sleipnir Developers <noreply@example.com>
Standards-Version: 4.6.2

Package: sleipnir
Architecture: any
Description: GPU-accelerated terminal emulator
EOF
    SHLIB_OUTPUT="$(cd "${SHLIB_WORK}" && dpkg-shlibdeps -e"${DEB_ROOT}/usr/bin/sleipnir" -O)"
    SHLIB_DEPS="${SHLIB_OUTPUT#shlibs:Depends=}"
    [[ -n "${SHLIB_DEPS}" && "${SHLIB_DEPS}" != "${SHLIB_OUTPUT}" ]] || {
        echo "ERROR: dpkg-shlibdeps returned no dependencies" >&2
        exit 1
    }
    DEPENDS="$(merge_debian_dependencies "${SHLIB_DEPS}")"
    INSTALLED_SIZE="$(du -sk "${DEB_ROOT}/usr" | awk '{print $1}')"
    cat > "${DEB_ROOT}/DEBIAN/control" <<EOF
Package: sleipnir
Version: ${VERSION}
Section: x11
Priority: optional
Architecture: ${DEBIAN_ARCH}
Depends: ${DEPENDS}
Installed-Size: ${INSTALLED_SIZE}
Maintainer: Sleipnir Developers <noreply@example.com>
Homepage: https://github.com/Maidang1/sleipnir
Description: GPU-accelerated terminal emulator with tabs and splits
 Sleipnir is a native terminal emulator rendered through Vulkan on Linux.
 It supports Wayland and X11 desktops, tabs, split panes, and PTY sessions.
EOF
    chmod 644 "${DEB_ROOT}/DEBIAN/control" "${DEB_ROOT}/usr/share/doc/sleipnir/"*
    normalize_tree "${DEB_ROOT}"
    # dpkg-deb also reads filesystem timestamps directly; touch every staged
    # entry from SOURCE_DATE_EPOCH before constructing the archive.
    find "${DEB_ROOT}" -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} + 2>/dev/null \
        || normalize_tree "${DEB_ROOT}"
    DEB_PATH="${OUT_DIR}/$(deb_name "${VERSION}" "${DEBIAN_ARCH}")"
    dpkg-deb --build --root-owner-group --uniform-compression \
        -Zgzip -z9 "${DEB_ROOT}" "${DEB_PATH}" >/dev/null
    write_sidecar "${DEB_PATH}"
    echo "created ${DEB_PATH}"
fi
