#!/usr/bin/env bash
# Install the latest verified Sleipnir release on macOS or Linux.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | SLEIPNIR_TARBALL=1 bash
#
# Linux defaults to the native Debian package. SLEIPNIR_TARBALL=1 performs a
# rootless install under XDG_BIN_HOME/XDG_DATA_HOME. PREFIX and
# SLEIPNIR_NO_OPEN retain their existing macOS meanings.
set -euo pipefail

REPO="${SLEIPNIR_REPO:-Maidang1/sleipnir}"
APP_NAME="Sleipnir"
INSTALL_SCRIPT_URL="https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh"
WORKDIR=""
MOUNT=""
INSTALL_BINARY_TMP=""
INSTALL_DESKTOP_TMP=""
INSTALL_ICON_TMP=""

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "ERROR: missing required command: $1" >&2
        return 1
    }
}

linux_deb_arch_for() {
    case "${1:-}" in
        x86_64|amd64) printf '%s\n' amd64 ;;
        aarch64|arm64) printf '%s\n' arm64 ;;
        *) echo "ERROR: no prebuilt Linux package for ${1:-unknown}" >&2; return 1 ;;
    esac
}

linux_portable_arch_for() {
    case "${1:-}" in
        x86_64|amd64) printf '%s\n' x86_64 ;;
        aarch64|arm64) printf '%s\n' aarch64 ;;
        *) echo "ERROR: no prebuilt Linux package for ${1:-unknown}" >&2; return 1 ;;
    esac
}

linux_asset_name() {
    local version="$1" host_arch="$2" tarball="$3"
    if [[ "${tarball}" == "1" ]]; then
        printf 'Sleipnir-%s-linux-%s.tar.gz\n' \
            "${version}" "$(linux_portable_arch_for "${host_arch}")"
    else
        printf 'sleipnir_%s_%s.deb\n' \
            "${version}" "$(linux_deb_arch_for "${host_arch}")"
    fi
}

release_asset_url() {
    printf 'https://github.com/%s/releases/download/%s/%s\n' "${REPO}" "$1" "$2"
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print tolower($1)}'
    else
        shasum -a 256 "$1" | awk '{print tolower($1)}'
    fi
}

cleanup() {
    if [[ -n "${MOUNT}" && -d "${MOUNT}" ]] && command -v hdiutil >/dev/null 2>&1; then
        hdiutil detach "${MOUNT}" -force >/dev/null 2>&1 || true
    fi
    [[ -z "${INSTALL_BINARY_TMP}" ]] || rm -f "${INSTALL_BINARY_TMP}"
    [[ -z "${INSTALL_DESKTOP_TMP}" ]] || rm -f "${INSTALL_DESKTOP_TMP}"
    [[ -z "${INSTALL_ICON_TMP}" ]] || rm -f "${INSTALL_ICON_TMP}"
    if [[ -n "${WORKDIR}" ]]; then
        rm -rf "${WORKDIR}"
    fi
}

create_workdir() {
    WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/sleipnir-install.XXXXXX")"
    trap cleanup EXIT
}

resolve_latest_release() {
    local latest_url tag version
    latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' \
        -H 'User-Agent: sleipnir-install' \
        "https://github.com/${REPO}/releases/latest")"
    tag="${latest_url##*/}"
    version="${tag#v}"
    if [[ -z "${version}" || "${version}" == "${latest_url}" || "${tag}" == releases ]]; then
        echo "ERROR: could not resolve latest tag from ${latest_url}" >&2
        return 1
    fi
    printf '%s %s\n' "${tag}" "${version}"
}

download() {
    local url="$1" destination="$2"
    curl -fL --retry 3 --retry-delay 1 -o "${destination}" "${url}"
}

verify_sha() {
    local artifact="$1" sidecar="$2" want got
    [[ -s "${sidecar}" ]] || {
        echo "ERROR: required checksum sidecar is missing or empty" >&2
        return 1
    }
    want="$(tr -d ' \n\r\t' < "${sidecar}" | tr '[:upper:]' '[:lower:]')"
    [[ "${want}" =~ ^[0-9a-f]{64}$ ]] || {
        echo "ERROR: invalid SHA-256 sidecar" >&2
        return 1
    }
    got="$(sha256_file "${artifact}")"
    if [[ "${want}" != "${got}" ]]; then
        echo "ERROR: SHA-256 mismatch" >&2
        echo "  expected: ${want}" >&2
        echo "  got:      ${got}" >&2
        return 1
    fi
    echo "  sha256: ${got}  ok"
}

install_macos() {
    local prefix dest tag version dmg_name dmg_url dmg sha_file mounted_app app
    need curl
    need ditto
    need shasum
    need hdiutil
    need xattr
    create_workdir

    prefix="${PREFIX:-/Applications}"
    dest="${prefix}/${APP_NAME}.app"
    echo "=== Sleipnir install ==="
    echo "  repo:   ${REPO}"
    echo "  dest:   ${dest}"
    echo "  fetching latest release…"
    read -r tag version < <(resolve_latest_release)
    dmg_name="${APP_NAME}-${version}-macos.dmg"
    dmg_url="$(release_asset_url "${tag}" "${dmg_name}")"
    dmg="${WORKDIR}/${dmg_name}"
    sha_file="${dmg}.sha256"
    echo "  version: ${version} (${tag})"
    echo "  downloading ${dmg_name}…"
    download "${dmg_url}" "${dmg}"
    download "${dmg_url}.sha256" "${sha_file}"
    verify_sha "${dmg}" "${sha_file}"

    echo "  mounting…"
    MOUNT="${WORKDIR}/mnt"
    mkdir -p "${MOUNT}"
    hdiutil attach "${dmg}" -nobrowse -noautoopen -mountpoint "${MOUNT}" >/dev/null
    mounted_app="${MOUNT}/${APP_NAME}.app"
    [[ -d "${mounted_app}" ]] || {
        echo "ERROR: disk image did not contain ${APP_NAME}.app" >&2
        return 1
    }
    app="${WORKDIR}/${APP_NAME}.app"
    ditto "${mounted_app}" "${app}"
    hdiutil detach "${MOUNT}" -force >/dev/null
    MOUNT=""

    # Ad-hoc CI builds carry quarantine when downloaded; clear it before launch.
    xattr -cr "${app}"
    mkdir -p "${prefix}"
    if [[ ! -w "${prefix}" ]]; then
        echo "  ${prefix} is not writable — using sudo"
        sudo mkdir -p "${prefix}"
        sudo rm -rf "${dest}"
        sudo ditto "${app}" "${dest}"
        sudo chown -R "$(id -un):staff" "${dest}"
        sudo xattr -cr "${dest}"
    else
        rm -rf "${dest}"
        ditto "${app}" "${dest}"
        xattr -cr "${dest}"
    fi
    echo "  installed: ${dest}"
    echo "  quarantine cleared (xattr -cr) — Gatekeeper will not block this copy"
    if [[ "${SLEIPNIR_NO_OPEN:-0}" != "1" ]]; then
        open "${dest}"
    fi
    echo "=== done ==="
}

install_linux_tarball() {
    local tag="$1" version="$2" host_arch="$3"
    local asset url archive sidecar root extracted bin_home data_home
    need tar
    need install
    need mv
    asset="$(linux_asset_name "${version}" "${host_arch}" 1)"
    url="$(release_asset_url "${tag}" "${asset}")"
    archive="${WORKDIR}/${asset}"
    sidecar="${archive}.sha256"
    download "${url}" "${archive}"
    download "${url}.sha256" "${sidecar}"
    verify_sha "${archive}" "${sidecar}"

    root="${asset%.tar.gz}"
    extracted="${WORKDIR}/unpacked"
    mkdir -p "${extracted}"
    tar -xzf "${archive}" -C "${extracted}"
    [[ -x "${extracted}/${root}/sleipnir" \
        && -f "${extracted}/${root}/sleipnir.desktop" \
        && -f "${extracted}/${root}/sleipnir.png" ]] || {
        echo "ERROR: portable archive is missing required files" >&2
        return 1
    }

    bin_home="${XDG_BIN_HOME:-${HOME}/.local/bin}"
    data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
    mkdir -p "${bin_home}" "${data_home}/applications" \
        "${data_home}/icons/hicolor/512x512/apps"
    INSTALL_BINARY_TMP="${bin_home}/.sleipnir.install.$$"
    INSTALL_DESKTOP_TMP="${data_home}/applications/.sleipnir.desktop.install.$$"
    INSTALL_ICON_TMP="${data_home}/icons/hicolor/512x512/apps/.sleipnir.png.install.$$"
    install -m 755 "${extracted}/${root}/sleipnir" "${INSTALL_BINARY_TMP}"
    install -m 644 "${extracted}/${root}/sleipnir.desktop" "${INSTALL_DESKTOP_TMP}"
    install -m 644 "${extracted}/${root}/sleipnir.png" "${INSTALL_ICON_TMP}"
    mv -f "${INSTALL_BINARY_TMP}" "${bin_home}/sleipnir"
    INSTALL_BINARY_TMP=""
    mv -f "${INSTALL_DESKTOP_TMP}" "${data_home}/applications/sleipnir.desktop"
    INSTALL_DESKTOP_TMP=""
    mv -f "${INSTALL_ICON_TMP}" "${data_home}/icons/hicolor/512x512/apps/sleipnir.png"
    INSTALL_ICON_TMP=""

    case ":${PATH}:" in
        *":${bin_home}:"*) ;;
        *) echo "WARNING: ${bin_home} is not in PATH; add it before running sleipnir." >&2 ;;
    esac
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${data_home}/applications" >/dev/null 2>&1 || true
    fi
    echo "  installed: ${bin_home}/sleipnir"
    if [[ "${SLEIPNIR_NO_OPEN:-0}" != "1" ]]; then
        "${bin_home}/sleipnir" >/dev/null 2>&1 &
    fi
}

install_linux_deb() {
    local tag="$1" version="$2" host_arch="$3"
    local asset url package sidecar
    if ! command -v apt >/dev/null 2>&1; then
        echo "ERROR: apt is required for the default Linux install." >&2
        echo "Retry with: curl -fsSL ${INSTALL_SCRIPT_URL} | SLEIPNIR_TARBALL=1 bash" >&2
        return 1
    fi
    need sudo
    asset="$(linux_asset_name "${version}" "${host_arch}" 0)"
    url="$(release_asset_url "${tag}" "${asset}")"
    package="${WORKDIR}/${asset}"
    sidecar="${package}.sha256"
    download "${url}" "${package}"
    download "${url}.sha256" "${sidecar}"
    verify_sha "${package}" "${sidecar}"
    (cd "${WORKDIR}" && sudo apt install -y "./${asset}")
}

install_linux() {
    local host_arch tag version
    need curl
    host_arch="$(uname -m)"
    # Validate architecture and installation mode before allocating or downloading.
    linux_deb_arch_for "${host_arch}" >/dev/null
    linux_portable_arch_for "${host_arch}" >/dev/null
    if [[ "${SLEIPNIR_TARBALL:-0}" != "1" ]] && ! command -v apt >/dev/null 2>&1; then
        echo "ERROR: apt is required for the default Linux install." >&2
        echo "Retry with: curl -fsSL ${INSTALL_SCRIPT_URL} | SLEIPNIR_TARBALL=1 bash" >&2
        return 1
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        :
    else
        need shasum
    fi
    create_workdir
    echo "=== Sleipnir Linux install ==="
    echo "  fetching latest release…"
    read -r tag version < <(resolve_latest_release)
    echo "  version: ${version} (${tag})"
    if [[ "${SLEIPNIR_TARBALL:-0}" == "1" ]]; then
        install_linux_tarball "${tag}" "${version}" "${host_arch}"
    else
        install_linux_deb "${tag}" "${version}" "${host_arch}"
    fi
    echo "=== done ==="
}

main() {
    case "$(uname -s)" in
        Darwin) install_macos ;;
        Linux) install_linux ;;
        *) echo "ERROR: Sleipnir prebuilt installs support macOS and Linux." >&2; return 1 ;;
    esac
}

if [[ "${SLEIPNIR_INSTALL_SOURCE_ONLY:-0}" != "1" ]]; then
    main "$@"
fi
