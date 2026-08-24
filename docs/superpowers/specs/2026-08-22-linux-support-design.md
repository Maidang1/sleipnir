# Linux Support Design

**Status:** Approved in conversation

**Goal:** Restore Linux as a fully released Sleipnir platform, with native x86_64 and ARM64 builds, Wayland/X11 support, Ubuntu/Debian packages, portable tarballs, one-line installation, CI, documentation, and website downloads.

## Context

Sleipnir currently ships on macOS and Windows. The repository previously shipped Linux support in commit `518d5b2`, then removed it in `a6e17a1`. Since that removal, the UI, terminal notification path, updater, website, packaging workflow, and release format have changed substantially. The old Linux implementation is therefore a behavioral reference, not code to cherry-pick wholesale.

The implementation will semantically forward-port the old behavior into the current module boundaries. It will not introduce a repository-wide platform abstraction or restore FreeBSD support.

## Decisions

- Restore complete Linux release support, not source-only support.
- Ship native `x86_64` and `aarch64` builds.
- Build against Ubuntu 22.04 / glibc 2.35.
- Use native Linux runners for both architectures; do not cross-compile or use QEMU as the release validation path.
- Enable Wayland and X11 and let GPUI select the available display backend.
- Keep in-place auto-update macOS-only. Linux **Check for Updates** opens GitHub Releases.
- Publish SHA-256 sidecars, without GPG signatures.
- Officially support Ubuntu 22.04 and newer. Other glibc 2.35+ desktop distributions are best-effort through the portable tarball.
- Keep tarballs system-dependency-based rather than bundling shared graphics libraries.
- The one-line installer defaults to the architecture-appropriate `.deb`; `SLEIPNIR_TARBALL=1` selects a user-local tarball installation.
- Deliver desktop notifications through `notify-send` / libnotify.

## Scope

### In scope

- GPUI Linux platform startup with Vulkan, Wayland, and X11.
- Linux keybindings, menus, window controls, titlebar geometry, default font, configuration location, path opening, notifications, and last-window lifecycle.
- Linux-safe update behavior.
- Native x86_64 and ARM64 CI builds and tests.
- `.deb` and `.tar.gz` packages for both architectures, each with a SHA-256 sidecar.
- Cross-platform one-line installer behavior.
- Release notes, English and Chinese READMEs, upstream notes, changelog, website messaging, and architecture-specific downloads.
- Automated package, installer, and X11 smoke validation plus a documented Wayland manual test.

### Out of scope

- FreeBSD support.
- musl artifacts.
- AppImage, Flatpak, Snap, or a hosted apt repository.
- GPG/package signing.
- Bundling Vulkan, Wayland, X11, fontconfig, or notification shared libraries.
- Linux in-place self-update.
- A new global `PlatformCapabilities` or `DesktopIntegration` framework.
- Cross-compilation or emulation as a substitute for native ARM64 release validation.

## Architecture

Linux support is restored through four bounded areas: platform startup, desktop behavior, distribution, and release presentation. Existing small policy functions should be generalized only where Linux and Windows genuinely share behavior. Platform-specific process execution remains local to the relevant desktop operation.

### 1. Platform startup

The root workspace will add `gpui_linux` at exactly the same pinned Zed revision as `gpui`, `gpui_macos`, and `gpui_windows`. GPUI will enable the `wayland` and `x11` features needed by the Linux backend.

`gpui_platform` will:

- keep `MacPlatform` on macOS;
- keep `WindowsPlatform` on Windows;
- call `gpui_linux::current_platform(headless)` only on Linux;
- retain a compile error for unsupported operating systems;
- expose Linux feature forwarding from its Cargo manifest.

Linux initialization errors must retain their underlying GPUI context. If neither Wayland nor X11 can initialize, the final diagnostic must tell the user to inspect `WAYLAND_DISPLAY`, `DISPLAY`, and the documented graphics dependencies. Vulkan initialization failures must direct users to install `libvulkan1`, Mesa Vulkan drivers, or their GPU vendor driver. Sleipnir will not add an untested rendering fallback.

### 2. Desktop behavior

#### Keybindings

Linux uses the current non-macOS shortcut family:

- application shortcuts use `Ctrl+Shift+*` and `Ctrl+Alt+*`;
- copy/paste use `Ctrl+Shift+C` / `Ctrl+Shift+V`;
- text-only paste uses `Ctrl+Alt+V`;
- bare `Ctrl+C`, `Ctrl+W`, `Ctrl+D`, and `Ctrl+V` remain available to shells and TUIs;
- bare `Ctrl+1..9` remain available to terminal applications;
- tab activation uses `Ctrl+Shift+1..9`.

Existing policy functions currently named around `windows: bool` will be renamed or replaced with explicit macOS versus non-macOS intent where Linux shares the same behavior. Shortcut display labels and actual bindings must consume the same policy.

#### Menus

Linux uses the non-macOS **File / Edit / View / Window** menu arrangement. It must not expose macOS-only Services, Hide, Hide Others, or Show All entries. The File menu includes terminal/window actions, Check for Updates, and Exit, matching the current Windows discoverability surface.

#### Window lifecycle and custom chrome

Closing the last Linux window exits the process, as on Windows. macOS keeps its Dock reopen behavior unchanged.

The current custom titlebar renders caption buttons only for Windows. It will be locally generalized so Linux and Windows share:

- right-side minimize, maximize/restore, and close buttons;
- trailing titlebar space reserved for those buttons;
- no caption reservation in fullscreen;
- a draggable blank titlebar region;
- double-click maximize/restore behavior.

Function/module names should describe non-macOS desktop controls rather than claim the implementation is Windows-only. GPUI remains responsible for the actual window operation and display-server differences. This is a local generalization, not a new platform framework.

#### Font, configuration, and Unix behavior

- Linux default terminal font: `Ubuntu Mono`.
- Linux config directory: `~/.config/sleipnir` through the project’s current config-path behavior.
- Missing `Ubuntu Mono` falls through to fontconfig/GPUI fallback; packages do not bundle fonts.
- PTY handling, cwd inheritance, OSC 133, control surface socket, and Run Ledger use existing Unix code paths.

#### Path opening

Linux opens URLs and filesystem paths with `xdg-open`. Calls must use `std::process::Command` arguments rather than constructing shell command strings. Failure or absence logs a warning and does not interrupt the terminal.

#### Notifications

OSC 9/777 and long-running-command completion notifications share the existing `notify_message` entry point. On Linux it asynchronously invokes:

```text
notify-send --app-name Sleipnir <title> <message>
```

Arguments are passed separately. A missing executable, unavailable notification service, or nonzero exit logs a warning and does not affect terminal I/O or OSC parsing. The Debian package declares `libnotify-bin` as a runtime dependency.

#### Update checks

Linux **Check for Updates** opens the GitHub Releases page. It must not enter the macOS bundle replacement path. Existing macOS update behavior and Windows release-page behavior stay unchanged.

## Distribution

### Release assets

Every release publishes these Linux assets:

```text
Sleipnir-<version>-linux-x86_64.tar.gz
Sleipnir-<version>-linux-x86_64.tar.gz.sha256
sleipnir_<version>_amd64.deb
sleipnir_<version>_amd64.deb.sha256
Sleipnir-<version>-linux-aarch64.tar.gz
Sleipnir-<version>-linux-aarch64.tar.gz.sha256
sleipnir_<version>_arm64.deb
sleipnir_<version>_arm64.deb.sha256
```

Each `.sha256` file contains only the lowercase hexadecimal digest so it remains compatible with the existing installer convention. Validation reads that digest and compares it with `sha256sum <artifact>`; when using `sha256sum -c`, the test constructs the required `<digest>  <filename>` input rather than passing the digest-only sidecar directly.

### Packaging script

`scripts/make-linux-package.sh` will build or consume the current native release binary and produce both package formats. Architecture mapping is strict:

| Native/Debian name | Portable artifact name | Debian package name |
|---|---|---|
| `x86_64` / `amd64` | `x86_64` | `amd64` |
| `aarch64` / `arm64` | `aarch64` | `arm64` |

Any other architecture fails before packaging.

The portable tarball contains:

- executable `sleipnir`;
- `sleipnir.desktop`;
- PNG application icon;
- `README.txt` with runtime dependencies and desktop-install instructions;
- GPL license.

The Debian package installs:

- `/usr/bin/sleipnir`;
- `/usr/share/applications/sleipnir.desktop`;
- hicolor icons under `/usr/share/icons/hicolor/`;
- documentation and license material under `/usr/share/doc/sleipnir` and the repository’s established license location.

The package metadata identifies its native architecture and declares dynamically linked requirements plus runtime-loaded desktop dependencies. At minimum this includes libc/X11/XCB/xkbcommon requirements discovered by Debian tooling and explicit Vulkan, Wayland, fontconfig, `xdg-utils`, and `libnotify-bin` dependencies appropriate for Ubuntu 22.04. Dependency classification must favor required dependencies for functionality promised by this specification; notification support cannot be only a recommendation.

The script supports a CI mode that consumes an already-built release binary so the release job does not compile twice.

### Desktop entry

`resources/linux/sleipnir.desktop` registers Sleipnir as a terminal emulator, launches `sleipnir`, references the `sleipnir` icon, uses `Terminal=false` (the emulator is itself the GUI application rather than a program to launch inside another terminal), and uses standard desktop categories suitable for validation with `desktop-file-validate`.

## Installer

The public command remains:

```bash
curl -fsSL https://raw.githubusercontent.com/Maidang1/sleipnir/main/scripts/install.sh | bash
```

`scripts/install.sh` dispatches by `uname -s`:

- Darwin runs the existing DMG installation unchanged.
- Linux resolves the latest release, maps the native architecture, selects the correct artifact, downloads its sidecar, verifies SHA-256, and only then installs.
- Other kernels fail with a supported-platform message.

Default Linux behavior requires `apt`, downloads the matching `.deb`, and invokes `sudo apt install ./<package>`. If `apt` is absent, the script exits with a direct retry command using `SLEIPNIR_TARBALL=1`; it does not guess another package manager.

With `SLEIPNIR_TARBALL=1`, the script installs without root into:

- `${XDG_BIN_HOME:-$HOME/.local/bin}` for the executable;
- `${XDG_DATA_HOME:-$HOME/.local/share}/applications` for the desktop file;
- `${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/...` for the icon.

The tarball path must create needed directories, replace files atomically where practical, and print a warning if the selected bin directory is absent from `PATH`. Unsupported CPUs, failed requests, missing assets, and digest mismatches abort before `apt` or filesystem installation. Temporary files are always cleaned by a trap.

## CI and release orchestration

The workflow adds a Linux matrix with two native jobs:

| Architecture | Native runner | Build userspace |
|---|---|---|
| x86_64 | GitHub-hosted x86_64 Linux runner | Ubuntu 22.04 |
| ARM64 | GitHub-hosted ARM64 Linux runner | Ubuntu 22.04 ARM64 container when required to preserve glibc 2.35 |

The implementation plan must verify the exact available GitHub Actions ARM64 runner label before editing the workflow. The job must assert `uname -m` matches its expected architecture before compiling. If a native ARM64 runner is unavailable to the repository, the ARM64 release job fails with a clear requirement; it must not silently cross-compile or emulate.

Each matrix entry:

1. installs Rust 1.95 and the GPUI/Vulkan/Wayland/X11/fontconfig/libnotify/build/package dependencies;
2. runs all workspace tests that are supported on Linux;
3. builds `sleipnir` in release mode;
4. invokes the packaging script without rebuilding;
5. verifies ELF architecture with `file` or `readelf`;
6. verifies tar contents;
7. verifies Debian metadata and contents with `dpkg-deb`;
8. verifies all SHA-256 sidecars;
9. uploads an architecture-specific Actions artifact;
10. for releases, waits for the macOS job to create the GitHub Release and uploads exactly that architecture’s four files.

Release upload must fail when any expected package or sidecar is absent. A partial Linux matrix must not pass as a complete Linux release. macOS and Windows jobs remain required and must keep passing.

## Website and documentation

The website release model discovers architecture-specific Linux `.deb` and tarball assets. The download menu names each architecture explicitly. Its existing Windows asset matching is corrected to the current `*-windows-x64.exe` release format while this release surface is being changed.

Website copy will state that Sleipnir supports macOS, Windows, and Linux, with Metal, Direct3D 11, and Vulkan respectively. Linux copy includes Ubuntu 22.04+, Wayland/X11, both CPU architectures, system dependency expectations, and the fact that Check for Updates opens Releases rather than replacing the application.

The shared install command remains cross-platform and its hint no longer describes only `/Applications` and Gatekeeper.

Documentation updates include:

- `README.md` and `README.zh.md`: support statement, Linux installation modes, source prerequisites, shortcuts, config path, Vulkan/display notes, package names, and troubleshooting;
- `CHANGELOG.md`: restored Linux support and both architectures;
- `UPSTREAM.md`: `gpui_linux`, Wayland/X11 features, and platform entry details;
- `docs/glossary.md` and other current platform assertions: remove stale macOS-only wording;
- release notes: enumerate both `.deb` and tarball choices and state the support boundary.

No documentation may continue to claim Linux is unsupported.

## Error handling

- GPUI/display/Vulkan initialization errors are actionable and preserve their source error.
- `xdg-open` and `notify-send` failures are warnings only.
- Unsupported install architectures and kernels fail before download/install.
- HTTP, asset-resolution, and SHA failures abort installation.
- SHA mismatch never reaches `apt` or destination writes.
- Packaging fails for unknown architectures or absent binaries.
- CI fails for wrong native architecture, missing package contents, invalid metadata, bad hashes, or missing release assets.

## Testing and validation

Implementation follows TDD: policy tests are written and observed failing before behavior changes.

### Rust unit tests

Tests cover:

- Linux GPUI backend wiring and cfg boundaries;
- macOS, Windows, and Linux platform selection remaining distinct;
- Linux non-macOS bindings and preservation of shell-owned Ctrl keys;
- File-style Linux menus;
- Linux default font and config-path policy;
- Linux `xdg-open` and `notify-send` selection;
- last-window close behavior;
- release-page update behavior;
- Linux/non-macOS titlebar spacing and caption controls;
- unchanged macOS and Windows policy outputs.

Platform policy tests should use explicit platform enums or focused parameters rather than assertions that only inspect the host OS.

### Script tests

Shell behavior is factored enough to test with temporary directories and mock executables. Tests cover:

- all four architecture aliases;
- rejection of unknown architecture;
- `.deb` versus `SLEIPNIR_TARBALL=1` asset selection;
- architecture-correct URLs and filenames;
- digest success and mismatch;
- proof that a mismatch never invokes mocked `apt` or writes installation targets;
- apt absence guidance;
- Linux/macOS dispatch.

`shellcheck` runs on changed shell scripts.

### Package validation

On both architectures CI checks:

- ELF machine type;
- required tarball files and executable mode;
- Debian `Architecture`, dependency metadata, and installed paths;
- desktop-file validity;
- `sha256sum -c` success for every artifact.

### Runtime smoke and manual validation

- X11 CI smoke runs under Xvfb. It must prove the process initializes the GPUI window before being terminated by the test harness; a timeout that merely kills a pre-window process is not success.
- Wayland remains compile-covered in CI unless a deterministic compositor harness is introduced during planning. Release readiness requires a manual smoke on Ubuntu 22.04 GNOME Wayland recorded in the GitHub Release checklist or release-tracking issue linked from the release.
- Manual release checks cover GNOME Wayland, GNOME Xorg/XWayland, `.deb` install/uninstall, tarball user-local install, notifications, path opening, caption buttons, shortcuts, and Check for Updates.

## Acceptance criteria

Linux support is complete when:

- native x86_64 and native ARM64 Linux jobs pass tests and release builds against Ubuntu 22.04 userspace;
- all eight Linux files (two packages plus two sidecars per architecture) are present and validated;
- X11 initialization smoke passes on both architecture jobs, or any architecture-specific smoke limitation is explicitly resolved before release rather than waived;
- Ubuntu 22.04 Wayland manual smoke is recorded as passed;
- `.deb` and tarball installation paths work and desktop integration appears;
- `notify-send`, `xdg-open`, window controls, non-macOS shortcuts, and release-page update checks work on Linux;
- macOS and Windows CI remain green;
- website, release notes, and English/Chinese documentation accurately present Linux support;
- no current product documentation says Linux is unsupported;
- the support promise is Ubuntu 22.04+ official and other glibc 2.35+ desktop distributions best-effort.
