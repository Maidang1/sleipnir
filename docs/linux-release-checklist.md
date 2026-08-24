# Linux release checklist

Use this checklist for every Sleipnir release that publishes Linux artifacts. The release workflow always creates a draft and uploads Linux files only after the macOS release job and both native Linux builds succeed. Record screenshots, logs, package output, and any exception in the release-tracking issue before checking an item.

**Release-tracking issue:** [open or update the Linux release evidence issue](https://github.com/Maidang1/sleipnir/issues)

> Keep the workflow-created release in draft. Do not undraft or announce it until the Ubuntu 22.04 GNOME Wayland result and its evidence are recorded in the release-tracking issue.

## Environment and assets

- [ ] Record the release tag, test date, tester, machine or VM image, desktop session, GPU/driver, and links to CI runs in the release-tracking issue.
- [ ] Verify the synchronized upload attached exactly ten non-macOS files: the Windows `.exe` and sidecar plus one `.deb`, one portable `.tar.gz`, and their digest-only `.sha256` sidecars for each of x86_64 and ARM64.
- [ ] Verify all four checksums independently after downloading the release assets.

## Ubuntu 22.04 GNOME Wayland — required release gate

- [ ] Install and launch the x86_64 or ARM64 `.deb` in an Ubuntu 22.04 GNOME Wayland session.
- [ ] Confirm a visible Sleipnir window appears and terminal input/output works.
- [ ] Exercise tabs, horizontal and vertical splits, pane focus, pane zoom, font zoom, copy, paste, and window controls.
- [ ] Record the Wayland result and evidence in the release-tracking issue before undrafting or announcing the release.

## Ubuntu 22.04 GNOME Xorg / XWayland

- [ ] Launch Sleipnir in a GNOME Xorg or forced-X11/XWayland session.
- [ ] Confirm a visible window appears and terminal input/output works.
- [ ] Exercise tabs, splits, pane focus, pane zoom, shortcuts, and window controls.

## Debian packages

- [ ] Install `sleipnir_<version>_amd64.deb` on native x86_64 Ubuntu 22.04.
- [ ] Verify the application menu entry, icon, launch, terminal input, and clean uninstall on x86_64.
- [ ] Install `sleipnir_<version>_arm64.deb` on native ARM64 Ubuntu 22.04.
- [ ] Verify the application menu entry, icon, launch, terminal input, and clean uninstall on ARM64.

## User-local portable tarballs

- [ ] Run the `SLEIPNIR_TARBALL=1` installer with the x86_64 tarball and verify the executable, desktop entry, icon, launch, and replacement of a previous user-local install.
- [ ] Run the `SLEIPNIR_TARBALL=1` installer with the ARM64 tarball and verify the executable, desktop entry, icon, launch, and replacement of a previous user-local install.
- [ ] Verify both tarball installs require no root privileges and warn when the selected binary directory is absent from `PATH`.

## Desktop integration and updates

- [ ] Confirm a completed background command invokes `notify-send` and displays a desktop notification.
- [ ] Confirm path and URL activation invokes `xdg-open` and opens the expected target.
- [ ] Confirm minimize, maximize/restore, close, dragging, and double-click maximize behavior work for the custom window controls.
- [ ] Confirm Linux Ctrl-based shortcuts work and ordinary shell controls such as `Ctrl+C`, `Ctrl+D`, `Ctrl+V`, and `Ctrl+1` remain available to the terminal.
- [ ] Confirm **Check for Updates** opens the matching GitHub Releases page and does not enter the macOS in-place update flow.

## Final evidence gate

- [ ] Link all Wayland, Xorg/XWayland, x86_64, ARM64, package, installer, integration, and update evidence from the release-tracking issue.
- [ ] Confirm the Wayland result is recorded and all eight Linux files are attached to the draft, then manually undraft and announce the release.
