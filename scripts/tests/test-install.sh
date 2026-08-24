#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="${ROOT}/scripts/install.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_eq() {
    [[ "$1" == "$2" ]] || fail "expected '$2', got '$1'"
}

export SLEIPNIR_INSTALL_SOURCE_ONLY=1
# SCRIPT is an absolute path computed above.
# shellcheck disable=SC1090
source "${SCRIPT}"

assert_eq "$(linux_deb_arch_for x86_64)" amd64
assert_eq "$(linux_deb_arch_for amd64)" amd64
assert_eq "$(linux_deb_arch_for aarch64)" arm64
assert_eq "$(linux_deb_arch_for arm64)" arm64
assert_eq "$(linux_portable_arch_for x86_64)" x86_64
assert_eq "$(linux_portable_arch_for amd64)" x86_64
assert_eq "$(linux_portable_arch_for aarch64)" aarch64
assert_eq "$(linux_portable_arch_for arm64)" aarch64
if linux_deb_arch_for riscv64 >/dev/null 2>&1; then
    fail "unsupported Debian architecture accepted"
fi
if linux_portable_arch_for riscv64 >/dev/null 2>&1; then
    fail "unsupported portable architecture accepted"
fi

assert_eq "$(linux_asset_name 1.2.3 x86_64 0)" sleipnir_1.2.3_amd64.deb
assert_eq "$(linux_asset_name 1.2.3 amd64 1)" Sleipnir-1.2.3-linux-x86_64.tar.gz
assert_eq "$(linux_asset_name 1.2.3 aarch64 0)" sleipnir_1.2.3_arm64.deb
assert_eq "$(linux_asset_name 1.2.3 arm64 1)" Sleipnir-1.2.3-linux-aarch64.tar.gz
assert_eq "$(release_asset_url v1.2.3 sleipnir_1.2.3_amd64.deb)" \
    https://github.com/Maidang1/sleipnir/releases/download/v1.2.3/sleipnir_1.2.3_amd64.deb

# Cleanup also owns same-filesystem temporary destinations used by atomic moves.
CLEANUP_TMP="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install-cleanup.XXXXXX")"
WORKDIR="${CLEANUP_TMP}/work"
INSTALL_BINARY_TMP="${CLEANUP_TMP}/binary.tmp"
INSTALL_DESKTOP_TMP="${CLEANUP_TMP}/desktop.tmp"
INSTALL_ICON_TMP="${CLEANUP_TMP}/icon.tmp"
mkdir -p "${WORKDIR}"
touch "${INSTALL_BINARY_TMP}" "${INSTALL_DESKTOP_TMP}" "${INSTALL_ICON_TMP}"
# cleanup is defined by the sourced installer above.
# shellcheck disable=SC2218
cleanup
[[ ! -e "${WORKDIR}" ]] || fail "cleanup left the installer work directory"
[[ ! -e "${INSTALL_BINARY_TMP}" && ! -e "${INSTALL_DESKTOP_TMP}" && ! -e "${INSTALL_ICON_TMP}" ]] \
    || fail "cleanup left pending destination files"
rm -rf "${CLEANUP_TMP}"
WORKDIR=""
INSTALL_BINARY_TMP=""
INSTALL_DESKTOP_TMP=""
INSTALL_ICON_TMP=""

# Dispatcher tests replace installers, so they perform no network or filesystem I/O.
# The functions below are invoked indirectly by main.
dispatch_log=""
# shellcheck disable=SC2329
uname() { printf '%s\n' Darwin; }
# shellcheck disable=SC2329
install_macos() { dispatch_log=macos; }
# shellcheck disable=SC2329
install_linux() { dispatch_log=linux; }
main
assert_eq "${dispatch_log}" macos
# shellcheck disable=SC2329
uname() { printf '%s\n' Linux; }
main
assert_eq "${dispatch_log}" linux
# shellcheck disable=SC2329
uname() { printf '%s\n' FreeBSD; }
set +e
unsupported_output="$(main 2>&1)"
unsupported_status=$?
set -e
[[ "${unsupported_status}" -ne 0 ]] || fail "unsupported kernel succeeded"
[[ "${unsupported_output}" == *'support macOS and Linux'* ]] || fail "unsupported kernel message is unclear"
unset -f uname install_macos install_linux

# An unavailable apt must fail before release resolution and print executable
# guidance for the curl pipeline form.
APT_TMP="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install-no-apt.XXXXXX")"
cat > "${APT_TMP}/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) echo Linux ;;
  -m) echo x86_64 ;;
  *) echo Linux ;;
esac
EOF
cat > "${APT_TMP}/curl" <<'EOF'
#!/bin/sh
echo "curl must not run before apt validation" >&2
exit 88
EOF
chmod +x "${APT_TMP}/uname" "${APT_TMP}/curl"
set +e
no_apt_output="$(PATH="${APT_TMP}" SLEIPNIR_INSTALL_SOURCE_ONLY=0 /bin/bash "${SCRIPT}" 2>&1)"
no_apt_status=$?
set -e
rm -rf "${APT_TMP}"
[[ "${no_apt_status}" -ne 0 ]] || fail "Linux install without apt succeeded"
[[ "${no_apt_output}" == *'apt is required'* ]] || fail "missing apt error was not reported"
[[ "${no_apt_output}" == *'| SLEIPNIR_TARBALL=1 bash'* ]] || fail "missing apt guidance is not pipeline-safe"
[[ "${no_apt_output}" != *'curl must not run'* ]] || fail "installer fetched a release before checking apt"

# An unsupported CPU must fail before any network request in both package modes.
UNSUPPORTED_TMP="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install-unsupported.XXXXXX")"
cat > "${UNSUPPORTED_TMP}/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) echo Linux ;;
  -m) echo riscv64 ;;
  *) echo Linux ;;
esac
EOF
cat > "${UNSUPPORTED_TMP}/curl" <<'EOF'
#!/bin/sh
: > "$UNSUPPORTED_CURL_MARKER"
exit 88
EOF
cat > "${UNSUPPORTED_TMP}/apt" <<'EOF'
#!/bin/sh
exit 88
EOF
chmod +x "${UNSUPPORTED_TMP}/"*
for unsupported_mode in 0 1; do
    rm -f "${UNSUPPORTED_TMP}/curl-called"
    set +e
    unsupported_arch_output="$(
        PATH="${UNSUPPORTED_TMP}" \
        UNSUPPORTED_CURL_MARKER="${UNSUPPORTED_TMP}/curl-called" \
        SLEIPNIR_INSTALL_SOURCE_ONLY=0 \
        SLEIPNIR_TARBALL="${unsupported_mode}" \
            /bin/bash "${SCRIPT}" 2>&1
    )"
    unsupported_arch_status=$?
    set -e
    [[ "${unsupported_arch_status}" -ne 0 ]] || fail "unsupported CPU succeeded (tarball=${unsupported_mode})"
    [[ "${unsupported_arch_output}" == *'no prebuilt Linux package for riscv64'* ]] \
        || fail "unsupported CPU error is unclear (tarball=${unsupported_mode})"
    [[ ! -e "${UNSUPPORTED_TMP}/curl-called" ]] \
        || fail "unsupported CPU downloaded an asset (tarball=${unsupported_mode})"
done
rm -rf "${UNSUPPORTED_TMP}"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install-test.XXXXXX")"
cleanup() { rm -rf "${TMP}"; }
trap cleanup EXIT

make_mock_path() {
    local root="$1" kernel="$2" machine="$3"
    mkdir -p "${root}/bin" "${root}/calls" "${root}/tmp"
    cat > "${root}/bin/uname" <<EOF
#!/bin/sh
case "\${1:-}" in
  -s) echo ${kernel} ;;
  -m) echo ${machine} ;;
  *) echo ${kernel} ;;
esac
EOF
    cat > "${root}/bin/curl" <<'EOF'
#!/bin/sh
output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    -w) shift 2 ;;
    -H|--retry|--retry-delay) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
printf '%s\n' "$url" >> "$MOCK_CALLS/curl-urls"
case "$url" in
  */releases/latest)
    printf 'https://github.com/Maidang1/sleipnir/releases/tag/v1.2.3'
    ;;
  *.sha256)
    printf '%064d\n' 0 > "$output"
    ;;
  *)
    printf 'deliberately-corrupt-artifact\n' > "$output"
    ;;
esac
EOF
    for command in sudo apt install mv; do
        cat > "${root}/bin/${command}" <<EOF
#!/bin/sh
: > "\${MOCK_CALLS}/${command}"
exit 99
EOF
    done
    chmod +x "${root}/bin/"*
}

run_mismatch_case() {
    local mode="$1" root="${TMP}/mock-${1}"
    make_mock_path "${root}" Linux x86_64
    set +e
    PATH="${root}/bin:${PATH}" \
    MOCK_CALLS="${root}/calls" \
    TMPDIR="${root}/tmp" \
    HOME="${root}/home" \
    XDG_BIN_HOME="${root}/destination/bin" \
    XDG_DATA_HOME="${root}/destination/share" \
    SLEIPNIR_INSTALL_SOURCE_ONLY=0 \
    SLEIPNIR_TARBALL="${mode}" \
    SLEIPNIR_NO_OPEN=1 \
        bash "${SCRIPT}" >"${root}/stdout" 2>"${root}/stderr"
    local status=$?
    set -e
    [[ "${status}" -ne 0 ]] || fail "checksum mismatch unexpectedly succeeded (tarball=${mode})"
    grep -Fq 'SHA-256 mismatch' "${root}/stderr" || fail "checksum mismatch was not reported"
    [[ ! -e "${root}/calls/sudo" ]] || fail "sudo ran after checksum mismatch"
    [[ ! -e "${root}/calls/apt" ]] || fail "apt ran after checksum mismatch"
    [[ ! -e "${root}/calls/install" ]] || fail "install ran after checksum mismatch"
    [[ ! -e "${root}/calls/mv" ]] || fail "mv ran after checksum mismatch"
    [[ ! -e "${root}/destination/bin/sleipnir" ]] || fail "binary was written after checksum mismatch"
    [[ ! -e "${root}/destination/share/applications/sleipnir.desktop" ]] || fail "desktop file was written after checksum mismatch"
    if find "${root}/tmp" -mindepth 1 -print -quit | grep -q .; then
        fail "temporary installer files were not cleaned"
    fi
}

run_mismatch_case 0
run_mismatch_case 1

grep -Fq 'sleipnir_1.2.3_amd64.deb' "${TMP}/mock-0/calls/curl-urls" || fail "amd64 deb URL not selected"
grep -Fq 'Sleipnir-1.2.3-linux-x86_64.tar.gz' "${TMP}/mock-1/calls/curl-urls" || fail "x86_64 tar URL not selected"

# Successful URL selection on ARM64 uses the matching release names without
# requiring a real apt invocation.
ARM_ROOT="${TMP}/mock-arm64"
make_mock_path "${ARM_ROOT}" Linux aarch64
cat > "${ARM_ROOT}/bin/curl" <<'EOF'
#!/bin/sh
output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    -w) shift 2 ;;
    -H|--retry|--retry-delay) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
printf '%s\n' "$url" >> "$MOCK_CALLS/curl-urls"
case "$url" in
  */releases/latest) printf 'https://github.com/Maidang1/sleipnir/releases/tag/v1.2.3' ;;
  *.sha256) "$REAL_SHASUM" -a 256 "$ARTIFACT" | awk '{print $1}' > "$output" ;;
  *) printf 'valid-arm64-deb\n' > "$output"; cp "$output" "$ARTIFACT" ;;
esac
EOF
cat > "${ARM_ROOT}/bin/sudo" <<'EOF'
#!/bin/sh
: > "$MOCK_CALLS/sudo"
exit 0
EOF
chmod +x "${ARM_ROOT}/bin/curl" "${ARM_ROOT}/bin/sudo"
set +e
PATH="${ARM_ROOT}/bin:${PATH}" \
MOCK_CALLS="${ARM_ROOT}/calls" \
TMPDIR="${ARM_ROOT}/tmp" \
ARTIFACT="${ARM_ROOT}/artifact-path" \
REAL_SHASUM="$(command -v shasum)" \
SLEIPNIR_INSTALL_SOURCE_ONLY=0 \
SLEIPNIR_NO_OPEN=1 \
    bash "${SCRIPT}" >"${ARM_ROOT}/stdout" 2>"${ARM_ROOT}/stderr"
arm_status=$?
set -e
[[ "${arm_status}" -eq 0 ]] || fail "verified ARM64 deb selection failed: $(cat "${ARM_ROOT}/stderr")"
grep -Fq 'sleipnir_1.2.3_arm64.deb' "${ARM_ROOT}/calls/curl-urls" || fail "arm64 deb URL not selected"
[[ -e "${ARM_ROOT}/calls/sudo" ]] || fail "verified ARM64 deb did not invoke apt through sudo"

# A verified portable archive installs all three user-local desktop assets and
# SLEIPNIR_NO_OPEN=1 must not launch the installed executable.
TAR_ROOT="${TMP}/mock-tar-success"
mkdir -p "${TAR_ROOT}/bin" "${TAR_ROOT}/calls" "${TAR_ROOT}/tmp" "${TAR_ROOT}/source/Sleipnir-1.2.3-linux-x86_64"
cat > "${TAR_ROOT}/source/Sleipnir-1.2.3-linux-x86_64/sleipnir" <<'EOF'
#!/bin/sh
: > "$LAUNCH_MARKER"
EOF
chmod 755 "${TAR_ROOT}/source/Sleipnir-1.2.3-linux-x86_64/sleipnir"
printf '%s\n' '[Desktop Entry]' 'Exec=sleipnir' > "${TAR_ROOT}/source/Sleipnir-1.2.3-linux-x86_64/sleipnir.desktop"
printf 'png fixture\n' > "${TAR_ROOT}/source/Sleipnir-1.2.3-linux-x86_64/sleipnir.png"
tar -czf "${TAR_ROOT}/fixture.tar.gz" -C "${TAR_ROOT}/source" Sleipnir-1.2.3-linux-x86_64
shasum -a 256 "${TAR_ROOT}/fixture.tar.gz" | awk '{print $1}' > "${TAR_ROOT}/fixture.tar.gz.sha256"
cat > "${TAR_ROOT}/bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) echo Linux ;;
  -m) echo x86_64 ;;
  *) echo Linux ;;
esac
EOF
cat > "${TAR_ROOT}/bin/curl" <<'EOF'
#!/bin/sh
output=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    -w) shift 2 ;;
    -H|--retry|--retry-delay) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
printf '%s\n' "$url" >> "$MOCK_CALLS/curl-urls"
case "$url" in
  */releases/latest) printf 'https://github.com/Maidang1/sleipnir/releases/tag/v1.2.3' ;;
  *.sha256) cp "$TARBALL_FIXTURE.sha256" "$output" ;;
  *) cp "$TARBALL_FIXTURE" "$output" ;;
esac
EOF
chmod +x "${TAR_ROOT}/bin/"*
PATH="${TAR_ROOT}/bin:${PATH}" \
MOCK_CALLS="${TAR_ROOT}/calls" \
TMPDIR="${TAR_ROOT}/tmp" \
HOME="${TAR_ROOT}/home" \
XDG_BIN_HOME="${TAR_ROOT}/destination/bin" \
XDG_DATA_HOME="${TAR_ROOT}/destination/share" \
TARBALL_FIXTURE="${TAR_ROOT}/fixture.tar.gz" \
LAUNCH_MARKER="${TAR_ROOT}/calls/launched" \
SLEIPNIR_INSTALL_SOURCE_ONLY=0 \
SLEIPNIR_TARBALL=1 \
SLEIPNIR_NO_OPEN=1 \
    bash "${SCRIPT}" >"${TAR_ROOT}/stdout" 2>"${TAR_ROOT}/stderr"
[[ -x "${TAR_ROOT}/destination/bin/sleipnir" ]] || fail "verified tar install missing executable"
[[ -f "${TAR_ROOT}/destination/share/applications/sleipnir.desktop" ]] || fail "verified tar install missing desktop file"
[[ -f "${TAR_ROOT}/destination/share/icons/hicolor/512x512/apps/sleipnir.png" ]] || fail "verified tar install missing icon"
[[ ! -e "${TAR_ROOT}/calls/launched" ]] || fail "SLEIPNIR_NO_OPEN=1 launched the installed executable"
grep -Fq 'Sleipnir-1.2.3-linux-x86_64.tar.gz' "${TAR_ROOT}/calls/curl-urls" \
    || fail "verified tar install selected the wrong asset"

# The curl-pipeline hint must put the environment assignment on bash, where it
# survives piping. Assigning it to curl would not affect the installer process.
grep -Fq 'curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | SLEIPNIR_TARBALL=1 bash' "${SCRIPT}" \
    || fail "apt fallback guidance does not set SLEIPNIR_TARBALL on bash"

printf 'test-install: PASS\n'
