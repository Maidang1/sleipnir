#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="${ROOT}/scripts/make-linux-package.sh"
DESKTOP="${ROOT}/resources/linux/sleipnir.desktop"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_eq() {
    [[ "$1" == "$2" ]] || fail "expected '$2', got '$1'"
}

assert_file_contains() {
    grep -Fq -- "$2" "$1" || fail "$1 does not contain: $2"
}

export SLEIPNIR_PACKAGE_SOURCE_ONLY=1
# SCRIPT is an absolute path computed above.
# shellcheck disable=SC1090
source "${SCRIPT}"

# Source-only mode must also exit successfully when the script is executed
# directly rather than sourced by this test process.
SLEIPNIR_PACKAGE_SOURCE_ONLY=1 bash "${SCRIPT}"

# Architecture aliases are accepted, but package names are canonical.
assert_eq "$(portable_arch_for x86_64)" x86_64
assert_eq "$(portable_arch_for amd64)" x86_64
assert_eq "$(portable_arch_for aarch64)" aarch64
assert_eq "$(portable_arch_for arm64)" aarch64
assert_eq "$(debian_arch_for x86_64)" amd64
assert_eq "$(debian_arch_for amd64)" amd64
assert_eq "$(debian_arch_for aarch64)" arm64
assert_eq "$(debian_arch_for arm64)" arm64
if portable_arch_for riscv64 >/dev/null 2>&1; then
    fail "unknown portable architecture unexpectedly accepted"
fi
if debian_arch_for i686 >/dev/null 2>&1; then
    fail "unknown Debian architecture unexpectedly accepted"
fi

assert_eq "$(tarball_name 1.2.3 x86_64)" Sleipnir-1.2.3-linux-x86_64.tar.gz
assert_eq "$(tarball_name 1.2.3 aarch64)" Sleipnir-1.2.3-linux-aarch64.tar.gz
assert_eq "$(deb_name 1.2.3 amd64)" sleipnir_1.2.3_amd64.deb
assert_eq "$(deb_name 1.2.3 arm64)" sleipnir_1.2.3_arm64.deb
assert_eq "$(merge_debian_dependencies 'libc6 (>= 2.35), libx11-6 (>= 2:1.6.0)')" \
    'libc6 (>= 2.35), libx11-6 (>= 2:1.6.0), libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1, libwayland-client0, libfontconfig1, xdg-utils, libnotify-bin'

# Policy is kept visible in source-only tests, including metadata-derived versioning
# and deterministic Debian construction that cannot run on every developer host.
assert_file_contains "${SCRIPT}" 'cargo metadata --no-deps --format-version 1'
assert_file_contains "${SCRIPT}" 'dpkg-shlibdeps'
assert_file_contains "${SCRIPT}" 'dpkg-deb --build --root-owner-group --uniform-compression'
assert_file_contains "${SCRIPT}" '-Zgzip -z9'
assert_file_contains "${SCRIPT}" 'gzip.GzipFile'
assert_file_contains "${SCRIPT}" 'mtime=0'
assert_file_contains "${SCRIPT}" 'changelog.gz'
assert_file_contains "${SCRIPT}" 'SOURCE_DATE_EPOCH'
assert_file_contains "${SCRIPT}" "find \"\${DEB_ROOT}\" -exec touch"
for package_path in \
    '/usr/bin/sleipnir' \
    '/usr/share/applications/sleipnir.desktop' \
    '/usr/share/icons/hicolor/' \
    '/usr/share/doc/sleipnir/README.txt' \
    '/usr/share/doc/sleipnir/changelog.gz' \
    '/usr/share/doc/sleipnir/copyright' \
    '/usr/share/licenses/sleipnir/LICENSE'; do
    assert_file_contains "${SCRIPT}" "${package_path}"
done
for dependency in \
    libx11-6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libvulkan1 \
    libwayland-client0 libfontconfig1 xdg-utils libnotify-bin; do
    assert_file_contains "${SCRIPT}" "${dependency}"
done

for entry in \
    'Type=Application' \
    'GenericName=Terminal Emulator' \
    'Exec=sleipnir' \
    'Icon=sleipnir' \
    'Terminal=false' \
    'Categories=System;TerminalEmulator;Utility;'; do
    assert_file_contains "${DESKTOP}" "${entry}"
done

TMP="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-package-test.XXXXXX")"
cleanup() { rm -rf "${TMP}"; }
trap cleanup EXIT

FAKE_BIN="${TMP}/sleipnir"
printf '#!/bin/sh\necho sleipnir fixture\n' > "${FAKE_BIN}"
chmod 755 "${FAKE_BIN}"

build_fixture() {
    local output="$1" build_umask="$2"
    (
        umask "${build_umask}"
        SLEIPNIR_PACKAGE_SOURCE_ONLY=0 \
        SOURCE_DATE_EPOCH=1700000000 \
        SLEIPNIR_VERSION=1.2.3 \
        SLEIPNIR_PACKAGE_ARCH=x86_64 \
        SLEIPNIR_PACKAGE_SKIP_ELF_CHECK=1 \
            bash "${SCRIPT}" --binary "${FAKE_BIN}" --out "${output}" --no-deb --no-strip >/dev/null
    )
}

build_fixture "${TMP}/out-one" 022
build_fixture "${TMP}/out-two" 077

TARBALL="${TMP}/out-one/Sleipnir-1.2.3-linux-x86_64.tar.gz"
TARBALL_TWO="${TMP}/out-two/Sleipnir-1.2.3-linux-x86_64.tar.gz"
[[ -f "${TARBALL}" ]] || fail "portable tarball was not created"
[[ -f "${TARBALL}.sha256" ]] || fail "digest sidecar was not created"
assert_eq "$(sha256_file "${TARBALL}")" "$(cat "${TARBALL}.sha256")"
[[ "$(cat "${TARBALL}.sha256")" =~ ^[0-9a-f]{64}$ ]] || fail "sidecar is not a digest-only lowercase SHA-256"
assert_eq "$(sha256_file "${TARBALL}")" "$(sha256_file "${TARBALL_TWO}")"

# sha256_file must remain portable to macOS, where sha256sum is normally absent.
SHA_MOCK="${TMP}/sha-mock"
mkdir -p "${SHA_MOCK}"
cat > "${SHA_MOCK}/shasum" <<'EOF'
#!/bin/sh
printf 'ABCDEF%058d  %s\n' 0 "$3"
EOF
cat > "${SHA_MOCK}/awk" <<'EOF'
#!/bin/sh
read -r digest _
printf '%s\n' "$(printf '%s' "$digest" | tr '[:upper:]' '[:lower:]')"
EOF
cat > "${SHA_MOCK}/tr" <<'EOF'
#!/bin/sh
/usr/bin/tr "$@"
EOF
chmod +x "${SHA_MOCK}/"*
assert_eq "$(PATH="${SHA_MOCK}" sha256_file "${TARBALL}")" "abcdef$(printf '%058d' 0)"

python3 - "${TARBALL}" <<'PY'
import sys, tarfile
archive = sys.argv[1]
prefix = "Sleipnir-1.2.3-linux-x86_64/"
expected = {
    prefix,
    prefix + "sleipnir",
    prefix + "sleipnir.desktop",
    prefix + "sleipnir.png",
    prefix + "README.txt",
    prefix + "LICENSE",
}
with tarfile.open(archive, "r:gz") as tf:
    names = set(tf.getnames())
    # tarfile may omit the trailing slash on the root directory.
    names.add(prefix) if prefix[:-1] in names else None
    missing = expected - names
    if missing:
        raise SystemExit("missing tar members: " + ", ".join(sorted(missing)))
    binary = tf.getmember(prefix + "sleipnir")
    if binary.mode & 0o111 == 0:
        raise SystemExit("packaged binary is not executable")
    readme = tf.extractfile(prefix + "README.txt").read().decode()
    required = [
        "Ubuntu 22.04+", "glibc 2.35+", "Vulkan", "Wayland", "X11",
        "fontconfig", "xdg-open", "notify-send", "XDG_DATA_HOME",
        "applications", "hicolor", "update-desktop-database",
    ]
    for text in required:
        if text not in readme:
            raise SystemExit(f"README missing {text!r}")
PY

if [[ "$(uname -s)" == "Linux" ]]; then
    for required_command in dpkg dpkg-deb dpkg-shlibdeps; do
        command -v "${required_command}" >/dev/null 2>&1 \
            || fail "Linux Debian checks require ${required_command}"
    done
    python3 -c 'from PIL import Image' >/dev/null 2>&1 \
        || fail "Linux Debian checks require Python Pillow"
    [[ -x /bin/true ]] || fail "Linux Debian checks require /bin/true as a native ELF fixture"
    DEB_ARCH="$(debian_arch_for "$(dpkg --print-architecture)")"
    DEB_ONE="${TMP}/deb-one"
    DEB_TWO="${TMP}/deb-two"
    for spec in "${DEB_ONE}:022" "${DEB_TWO}:077"; do
        output="${spec%%:*}"
        build_umask="${spec##*:}"
        (
            umask "${build_umask}"
            SLEIPNIR_PACKAGE_SOURCE_ONLY=0 \
            SOURCE_DATE_EPOCH=1700000000 \
            SLEIPNIR_VERSION=1.2.3 \
            SLEIPNIR_PACKAGE_ARCH="${DEB_ARCH}" \
                bash "${SCRIPT}" --binary /bin/true --out "${output}" --no-tar --no-strip >/dev/null
        )
    done

    DEB_PATH="${DEB_ONE}/sleipnir_1.2.3_${DEB_ARCH}.deb"
    DEB_PATH_TWO="${DEB_TWO}/sleipnir_1.2.3_${DEB_ARCH}.deb"
    [[ -f "${DEB_PATH}" && -f "${DEB_PATH_TWO}" ]] || fail "Debian package was not created"
    assert_eq "$(sha256_file "${DEB_PATH}")" "$(sha256_file "${DEB_PATH_TWO}")"
    assert_eq "$(sha256_file "${DEB_PATH}")" "$(cat "${DEB_PATH}.sha256")"
    [[ "$(cat "${DEB_PATH}.sha256")" =~ ^[0-9a-f]{64}$ ]] \
        || fail "Debian sidecar is not a digest-only lowercase SHA-256"

    DEB_DEPENDS="$(dpkg-deb -f "${DEB_PATH}" Depends)"
    for dependency in \
        libx11-6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libvulkan1 \
        libwayland-client0 libfontconfig1 xdg-utils libnotify-bin; do
        [[ ",${DEB_DEPENDS}," == *",${dependency},"* \
            || ",${DEB_DEPENDS}," == *", ${dependency},"* \
            || "${DEB_DEPENDS}" == *"${dependency} ("* ]] \
            || fail "Debian Depends missing ${dependency}: ${DEB_DEPENDS}"
    done

    DEB_CONTENTS="$(dpkg-deb -c "${DEB_PATH}")"
    for package_path in \
        './usr/bin/sleipnir' \
        './usr/share/applications/sleipnir.desktop' \
        './usr/share/icons/hicolor/48x48/apps/sleipnir.png' \
        './usr/share/icons/hicolor/64x64/apps/sleipnir.png' \
        './usr/share/icons/hicolor/128x128/apps/sleipnir.png' \
        './usr/share/icons/hicolor/256x256/apps/sleipnir.png' \
        './usr/share/icons/hicolor/512x512/apps/sleipnir.png' \
        './usr/share/doc/sleipnir/README.txt' \
        './usr/share/doc/sleipnir/changelog.gz' \
        './usr/share/doc/sleipnir/copyright' \
        './usr/share/licenses/sleipnir/LICENSE'; do
        grep -Fq "${package_path}" <<<"${DEB_CONTENTS}" \
            || fail "Debian package missing ${package_path}"
    done
    echo "test-linux-package: Debian checks executed"
else
    echo "test-linux-package: SKIP Debian checks (non-Linux host)"
fi

printf 'test-linux-package: PASS\n'
