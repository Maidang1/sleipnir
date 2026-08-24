# Linux Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore Linux as a fully released Sleipnir platform with native x86_64 and ARM64 binaries, Wayland/X11 runtime support, `.deb` and portable tarball packages, one-line installation, CI validation, and accurate product documentation.

**Architecture:** Forward-port the behavior from historical commit `518d5b2` into the current code rather than cherry-picking it. Keep platform selection in `gpui_platform`, generalize only the existing small macOS/non-macOS policy seams, keep Linux process integrations local to notification/path-opening code, and make packaging/installation independently testable shell programs.

**Tech Stack:** Rust 1.95, GPUI (`gpui_linux`, Vulkan, Wayland, X11), Bash, Debian packaging tools, GitHub Actions (`ubuntu-22.04`, `ubuntu-22.04-arm`), React 19, TypeScript, Vite, Vitest.

**Specification:** `docs/superpowers/specs/2026-08-22-linux-support-design.md`

**Implementation disciplines:** Use @superpowers:test-driven-development for every behavior change, @systematic-debugging for unexpected build/test failures, and @verification-before-completion before claiming the work is complete. Do not revive FreeBSD branches from the historical implementation.

---

## File structure map

### Platform and runtime

- Modify `Cargo.toml` — add the pinned `gpui_linux` dependency and Linux display features.
- Modify `Cargo.lock` — capture the resolved Linux GPUI dependency graph.
- Modify `crates/gpui_platform/Cargo.toml` — expose and forward `wayland`/`x11` features only to the Linux backend.
- Modify `crates/gpui_platform/src/gpui_platform.rs` — select `gpui_linux` on Linux, add actionable display-startup diagnostics, and preserve macOS/Windows and unsupported-target behavior.
- Modify `crates/sleipnir/Cargo.toml` — enable both Linux display backends for the application target.
- Modify `crates/updater/src/updater.rs` — make all bundle replacement and DMG code macOS-only; non-macOS opens Releases.

### Desktop behavior

- Modify `crates/sleipnir_settings/src/sleipnir_settings.rs` — add the Linux default font/fallback policy while retaining the existing Unix config path.
- Modify `crates/sleipnir_ui/src/keymap.rs` — make the current Ctrl-based desktop keymap explicitly shared by Windows and Linux.
- Modify `crates/sleipnir/src/app_menus.rs` — make the File-style menu explicitly shared by Windows and Linux.
- Modify `crates/sleipnir_ui/src/sleipnir_ui.rs` — select `xdg-open` and `notify-send` on Linux and test command construction without spawning real desktop services.
- Modify `crates/sleipnir_ui/src/chrome/geometry.rs` — reserve custom caption space on all non-macOS desktops.
- Rename `crates/sleipnir_ui/src/chrome/window_controls.rs` to `crates/sleipnir_ui/src/chrome/desktop_window_controls.rs` — own Windows/Linux caption rendering under an accurate name.
- Modify `crates/sleipnir_ui/src/chrome/mod.rs` — register the renamed module.
- Modify `crates/sleipnir_ui/src/app_shell.rs` — use non-macOS titlebar geometry/controls and keep Linux update actions manual.

### Linux packaging and installation

- Create `resources/linux/sleipnir.desktop` — freedesktop application entry (`Terminal=false`).
- Create `scripts/make-linux-package.sh` — architecture-strict native `.deb`/tarball builder and SHA generator.
- Create `scripts/tests/test-linux-package.sh` — pure architecture/asset tests plus package-content assertions.
- Modify `scripts/install.sh` — retain DMG installation on Darwin and add verified `.deb`/user-local tarball installation on Linux.
- Create `scripts/tests/test-install.sh` — source-mode unit tests and mock-command installer safety tests.

### CI, release, website, and documentation

- Modify `.github/workflows/build-and-release.yml` — native `ubuntu-22.04` and `ubuntu-22.04-arm` jobs, package checks, X11 smoke, release upload, and Linux release notes.
- Modify `website/package.json` and `website/package-lock.json` — add Vitest and a test script.
- Create `website/src/lib/release.test.ts` — release-asset parser coverage for macOS, Windows EXE, and both Linux architectures.
- Modify `website/src/lib/release.ts` — expose current asset names and cross-platform install copy.
- Modify `website/src/components/download-menu.tsx` — show architecture-labelled Linux packages.
- Modify `website/src/components/install-command.tsx` — continue copying the shared installer command with cross-platform wording.
- Modify `website/src/App.tsx`, `website/index.html`, and `website/README.md` — advertise the shipped three-platform surface.
- Create `docs/linux-release-checklist.md` — required Wayland/X11/package/installer release evidence and recording instructions.
- Modify `README.md`, `README.zh.md`, `UPSTREAM.md`, `docs/glossary.md`, and `CHANGELOG.md` — document Linux support, prerequisites, packages, shortcuts, and support boundaries.

---

### Task 1: Wire the GPUI Linux backend

**Files:**
- Modify: `Cargo.toml:13-39`
- Modify: `Cargo.lock`
- Modify: `crates/gpui_platform/Cargo.toml:13-28`
- Modify: `crates/gpui_platform/src/gpui_platform.rs:1-60`
- Modify: `crates/sleipnir/Cargo.toml:14-23`

- [ ] **Step 1: Change the platform-entry test so Linux is required**

In `crates/gpui_platform/src/gpui_platform.rs`, replace the negative Linux assertion with explicit backend/cfg assertions:

```rust
assert!(
    impl_src.contains("gpui_linux::current_platform"),
    "Linux backend must stay wired"
);
assert!(
    impl_src.contains("target_os = \"linux\""),
    "Linux constructor must be cfg-gated"
);
assert!(
    impl_src.contains("macOS, Windows, and Linux only"),
    "unsupported-target diagnostic must name the shipped platforms"
);
```

- [ ] **Step 2: Run the focused test and verify the red state**

Run:

```bash
cargo test -p gpui_platform platform_entry_selects_a_backend_per_os
```

Expected: FAIL because `gpui_linux::current_platform` is absent and the compile-error text only names macOS and Windows.

- [ ] **Step 3: Add the pinned Linux dependencies and features**

In root `Cargo.toml`, keep every Zed crate on revision `371a7d4ba2fd0064b79a0bc67d28e57a906779dc` and use:

```toml
gpui = { git = "https://github.com/zed-industries/zed", rev = "371a7d4ba2fd0064b79a0bc67d28e57a906779dc", default-features = false, features = ["font-kit", "wayland", "x11"] }
gpui_macos = { git = "https://github.com/zed-industries/zed", rev = "371a7d4ba2fd0064b79a0bc67d28e57a906779dc", default-features = false, features = ["font-kit"] }
gpui_windows = { git = "https://github.com/zed-industries/zed", rev = "371a7d4ba2fd0064b79a0bc67d28e57a906779dc", default-features = false }
gpui_linux = { git = "https://github.com/zed-industries/zed", rev = "371a7d4ba2fd0064b79a0bc67d28e57a906779dc", default-features = false, features = ["wayland", "x11"] }
```

Update the workspace comment and the `gpui_platform` workspace dependency so Linux is not described as absent.

In `crates/gpui_platform/Cargo.toml`, add:

```toml
wayland = ["gpui_linux/wayland"]
x11 = ["gpui_linux/x11"]

[target.'cfg(target_os = "linux")'.dependencies]
gpui_linux.workspace = true
```

In `crates/sleipnir/Cargo.toml`, add:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
gpui_platform = { workspace = true, features = ["wayland", "x11"] }
```

- [ ] **Step 4: Implement the Linux platform branch**

Add this branch before the unsupported-target branch in `current_platform`:

```rust
#[cfg(target_os = "linux")]
{
    gpui_linux::current_platform(headless)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
)))]
{
    let _ = headless;
    compile_error!("sleipnir gpui_platform supports macOS, Windows, and Linux only");
}
```

Do not include historical `freebsd` cfgs.

- [ ] **Step 5: Add actionable Linux startup diagnostics**

Add pure formatting coverage in `gpui_platform.rs`:

```rust
#[test]
fn linux_startup_diagnostic_keeps_source_and_actionable_hints() {
    let message = linux_startup_diagnostic("Failed to initialize X11 client");
    assert!(message.contains("Failed to initialize X11 client"));
    assert!(message.contains("WAYLAND_DISPLAY"));
    assert!(message.contains("DISPLAY"));
    assert!(message.contains("libvulkan1"));
    assert!(message.contains("mesa-vulkan-drivers"));
    assert!(message.contains("vendor Vulkan driver"));
}
```

Implement:

```rust
fn linux_startup_diagnostic(source: &str) -> String {
    format!(
        "{source}\nLinux startup failed. Check WAYLAND_DISPLAY or DISPLAY, \
         install libvulkan1 and mesa-vulkan-drivers, or install the vendor \
         Vulkan driver for your GPU."
    )
}
```

The pinned `gpui_linux::current_platform` currently unwraps compositor connection errors. Wrap only the Linux call with `std::panic::catch_unwind(std::panic::AssertUnwindSafe(...))`; on panic, extract `&str`/`String` payload text and panic with `linux_startup_diagnostic(source)`. Do not swallow the original source text or return a headless platform after a real display failure.

Also change both `open_window` error logs in `crates/sleipnir_ui/src/app_shell.rs` under Linux to append the same environment/Vulkan guidance while retaining `{err:#}`. This covers renderer/Vulkan failures that occur after platform construction.

Run:

```bash
cargo test -p gpui_platform linux_startup_diagnostic_keeps_source_and_actionable_hints
```

Expected: PASS after the formatter and Linux wrapper are implemented.

- [ ] **Step 6: Resolve the lockfile and rerun the focused test**

Run:

```bash
cargo update -p gpui_linux
cargo test -p gpui_platform platform_entry_selects_a_backend_per_os
```

Expected: Cargo resolves `gpui_linux` and its Wayland/X11 graph into `Cargo.lock`; test PASS.

- [ ] **Step 7: Verify the current host still builds the platform crate and UI**

Run:

```bash
cargo check -p gpui_platform
cargo check -p sleipnir_ui
```

Expected: PASS on the current host; target-only Linux dependencies must not break macOS.

- [ ] **Step 8: Commit the backend wiring and diagnostics**

```bash
git add Cargo.toml Cargo.lock crates/gpui_platform crates/sleipnir/Cargo.toml crates/sleipnir_ui/src/app_shell.rs
git commit -m "feat: wire GPUI Linux platform backend"
```

---

### Task 2: Keep in-place updates macOS-only

**Files:**
- Modify: `crates/updater/src/updater.rs:1-15, 238-456, 491-564`
- Modify: `crates/sleipnir_ui/src/app_shell.rs:2385-2396, 2683-2695`

- [ ] **Step 1: Write the failing explicit-platform updater test**

Add a pure helper and test contract in `crates/updater/src/updater.rs`:

```rust
fn in_place_update_supported_for(macos: bool) -> bool {
    macos
}

#[test]
fn in_place_update_is_macos_only() {
    assert!(in_place_update_supported_for(true));
    assert!(!in_place_update_supported_for(false));
    assert_eq!(
        in_place_update_supported(),
        cfg!(target_os = "macos")
    );
}
```

Initially leave `in_place_update_supported()` unchanged.

- [ ] **Step 2: Run the test and verify the Unix/macos mismatch**

Run:

```bash
cargo test -p updater in_place_update_is_macos_only
```

Expected: FAIL on the current macOS host because the test requires capability detection to be keyed to `target_os = "macos"`, while the existing function still returns `cfg!(unix)`.

Include this source-boundary assertion in the test so the red state is observable on macOS and remains a permanent regression guard:

```rust
let src = include_str!("updater.rs");
assert!(
    src.contains("in_place_update_supported_for(cfg!(target_os = \"macos\"))"),
    "capability detection must be macOS-specific"
);
assert!(
    !src.contains("pub fn in_place_update_supported() -> bool {\n    cfg!(unix)"),
    "Linux is Unix but must never enter DMG replacement"
);
```

- [ ] **Step 3: Change capability detection to macOS**

Implement:

```rust
pub fn in_place_update_supported() -> bool {
    in_place_update_supported_for(cfg!(target_os = "macos"))
}
```

Change every updater-only `#[cfg(unix)]` / `#[cfg(not(unix))]` guarding DMG bundle installation, `hdiutil`, `ditto`, helper scripts, `Write`, and `PermissionsExt` to `#[cfg(target_os = "macos")]` / `#[cfg(not(target_os = "macos"))]`. Rename `install_and_relaunch_unix` to `install_and_relaunch_macos` so the boundary remains obvious.

- [ ] **Step 4: Keep the UI’s two update entry points on the same capability check**

In both `run_command(CommandId::CheckForUpdates, ...)` and `on_check_for_updates`, retain this exact gate:

```rust
if !updater::in_place_update_supported() {
    cx.open_url(updater::RELEASES_PAGE);
    return;
}
```

The command-palette path currently lacks the explicit `return`; add it so Linux never opens the update dialog after opening Releases.

- [ ] **Step 5: Run updater and UI tests**

Run:

```bash
cargo test -p updater
cargo test -p sleipnir_ui
```

Expected: PASS; updater tests describe macOS rather than Unix, and existing macOS update parsing remains green.

- [ ] **Step 6: Commit the update boundary**

```bash
git add crates/updater/src/updater.rs crates/sleipnir_ui/src/app_shell.rs
git commit -m "fix: keep in-place updates macOS-only"
```

---

### Task 3: Add Linux settings defaults

**Files:**
- Modify: `crates/sleipnir_settings/src/sleipnir_settings.rs:214-263, 582-605, 1068-1115`

- [ ] **Step 1: Replace the two-family font test with a three-platform contract**

Introduce focused policy parameters and this failing test:

```rust
#[test]
fn default_font_is_platform_specific() {
    assert_eq!(default_font_family_for(false, false), "Menlo");
    assert_eq!(default_font_family_for(true, false), "Cascadia Mono");
    assert_eq!(default_font_family_for(false, true), "Ubuntu Mono");

    let linux_fallbacks = default_font_fallbacks_for(false, true).unwrap();
    assert_eq!(
        linux_fallbacks.fallback_list(),
        &["DejaVu Sans Mono".to_string(), "Liberation Mono".to_string()]
    );
    assert!(default_font_fallbacks_for(false, false).is_none());
}
```

- [ ] **Step 2: Run the focused settings test and verify it does not compile**

Run:

```bash
cargo test -p sleipnir_settings default_font_is_platform_specific
```

Expected: FAIL to compile because the existing helpers accept only `windows: bool`.

- [ ] **Step 3: Implement the three-platform font policy**

Use:

```rust
pub fn default_font_family() -> &'static str {
    default_font_family_for(cfg!(windows), cfg!(target_os = "linux"))
}

pub fn default_font_family_for(windows: bool, linux: bool) -> &'static str {
    if linux {
        "Ubuntu Mono"
    } else if windows {
        "Cascadia Mono"
    } else {
        "Menlo"
    }
}

pub fn default_font_fallbacks() -> Option<FontFallbacks> {
    default_font_fallbacks_for(cfg!(windows), cfg!(target_os = "linux"))
}

pub fn default_font_fallbacks_for(
    windows: bool,
    linux: bool,
) -> Option<FontFallbacks> {
    if linux {
        Some(FontFallbacks::from_fonts(vec![
            "DejaVu Sans Mono".into(),
            "Liberation Mono".into(),
        ]))
    } else if windows {
        Some(FontFallbacks::from_fonts(vec![
            "Consolas".into(),
            "Courier New".into(),
        ]))
    } else {
        None
    }
}
```

Update all local test/helper call sites to pass both booleans. Keep `config_dir_for(false)` unchanged: it already yields `~/.config/sleipnir`, which is the Linux contract.

- [ ] **Step 4: Add a Linux config-path regression assertion**

Extend the config test:

```rust
let linux = config_path_for(false);
assert!(linux.ends_with(".config/sleipnir/settings.json"));
```

Name the false branch “Unix/macOS/Linux” rather than “macOS”.

- [ ] **Step 5: Run the settings crate tests**

Run:

```bash
cargo test -p sleipnir_settings
```

Expected: PASS.

- [ ] **Step 6: Commit settings defaults**

```bash
git add crates/sleipnir_settings/src/sleipnir_settings.rs
git commit -m "feat: add Linux settings defaults"
```

---

### Task 4: Generalize shortcuts and application menus

**Files:**
- Modify: `crates/sleipnir_ui/src/keymap.rs:96-380, 390-525`
- Modify: `crates/sleipnir/src/app_menus.rs:30-204`

- [ ] **Step 1: Add an explicit Linux keymap regression test**

Add:

```rust
#[test]
fn linux_bindings_use_desktop_ctrl_chords_without_stealing_shell_keys() {
    let keys: Vec<_> = builtin_bindings_for(false, true)
        .into_iter()
        .map(|binding| binding.key)
        .collect();
    for expected in [
        "ctrl-shift-t",
        "ctrl-shift-c",
        "ctrl-shift-v",
        "ctrl-alt-v",
        "ctrl-shift-1",
        "ctrl-shift-9",
    ] {
        assert!(keys.iter().any(|key| key == expected), "missing {expected}");
    }
    for reserved in ["ctrl-c", "ctrl-w", "ctrl-d", "ctrl-v", "ctrl-1", "ctrl-9"] {
        assert!(!keys.iter().any(|key| key == reserved), "stole {reserved}");
    }
    assert!(!keys.iter().any(|key| key.starts_with("cmd-")));
}
```

- [ ] **Step 2: Run the keymap test and verify the signature failure**

Run:

```bash
cargo test -p sleipnir_ui linux_bindings_use_desktop_ctrl_chords_without_stealing_shell_keys
```

Expected: FAIL to compile because `builtin_bindings_for` accepts one boolean.

- [ ] **Step 3: Generalize keymap helpers without duplicating a Linux table**

Change the host selector to:

```rust
pub fn builtin_bindings() -> Vec<BuiltinBinding> {
    builtin_bindings_for(cfg!(target_os = "macos"), cfg!(target_os = "linux"))
}
```

Change the helper to `builtin_bindings_for(macos: bool, linux: bool)`. Use `macos_static_bindings()` only when `macos`; otherwise use a renamed `desktop_static_bindings()` containing the current Windows table. The `linux` parameter is intentionally used in tests/documentation to prove that Linux takes the desktop branch; add `debug_assert!(!(macos && linux))`.

Use `non_macos = !macos` for tab activation, font zoom, and display strings. Rename:

- `windows_static_bindings` → `desktop_static_bindings`
- `font_zoom_key_bindings_for(windows)` → `font_zoom_key_bindings_for(non_macos)`
- `display_shortcut_for(id, windows)` → `display_shortcut_for(id, non_macos)`

Update comments so they say Windows/Linux. Do not add historical bare `ctrl-1..9` bindings.

Also make last-window lifecycle use macOS polarity and test all shipped policies:

```rust
pub fn last_window_close_quits() -> bool {
    last_window_close_quits_for(cfg!(target_os = "macos"))
}

pub fn last_window_close_quits_for(macos: bool) -> bool {
    !macos
}

#[test]
fn last_window_quits_on_windows_and_linux_only() {
    assert!(!last_window_close_quits_for(true));  // macOS
    assert!(last_window_close_quits_for(false));  // Windows/Linux
    assert_eq!(last_window_close_quits(), cfg!(not(target_os = "macos")));
}
```

This keeps `main.rs`'s existing `cx.on_window_closed` path active on Linux while preserving Dock reopen on macOS.

- [ ] **Step 4: Add Linux menu tests**

Generalize the menu-title helper to `app_menu_bar_titles_for(macos: bool)` and add:

```rust
#[test]
fn linux_menu_bar_uses_file_layout() {
    assert_eq!(
        app_menu_bar_titles_for(false),
        &["File", "Edit", "View", "Window"]
    );
}
```

Add a pure `uses_macos_app_menu_for(macos: bool) -> bool` only if needed to keep the builder selection directly testable.

- [ ] **Step 5: Run the menu test and verify it fails under the old polarity**

Run:

```bash
cargo test -p sleipnir linux_menu_bar_uses_file_layout
```

Expected: FAIL until the helper and builder are changed from `windows` semantics to `macos` semantics.

- [ ] **Step 6: Reuse one File-style desktop menu builder**

Rename `windows_menus()` to `desktop_menus()` and select it on every non-macOS host:

```rust
pub fn app_menu_bar_titles() -> &'static [&'static str] {
    app_menu_bar_titles_for(cfg!(target_os = "macos"))
}

pub fn app_menu_bar_titles_for(macos: bool) -> &'static [&'static str] {
    if macos {
        &["Sleipnir", "Shell", "Edit", "View", "Window"]
    } else {
        &["File", "Edit", "View", "Window"]
    }
}

pub fn app_menus() -> Vec<Menu> {
    let menus = if cfg!(target_os = "macos") {
        macos_menus()
    } else {
        desktop_menus()
    };
    debug_assert_eq!(menus.len(), app_menu_bar_titles().len());
    menus
}
```

Keep the current File-menu contents, including Check for Updates and Exit; do not clone a separate Linux menu.

- [ ] **Step 7: Run all keymap and menu tests**

Run:

```bash
cargo test -p sleipnir_ui keymap::tests
cargo test -p sleipnir app_menus::tests
```

Expected: PASS for macOS, Windows policy parameters, and Linux policy parameters; duplicate-key tests stay green.

- [ ] **Step 8: Commit desktop input/menu policy**

```bash
git add crates/sleipnir_ui/src/keymap.rs crates/sleipnir/src/app_menus.rs
git commit -m "feat: add Linux shortcuts and menus"
```

---

### Task 5: Add Linux path opening and notifications

**Files:**
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs:923-963, 1140-1175, 1450-1532`

- [ ] **Step 1: Add pure command-selection tests**

Add focused helper tests:

```rust
#[test]
fn desktop_commands_are_platform_specific() {
    assert_eq!(path_opener_program_for(false, false), Some("open"));
    assert_eq!(path_opener_program_for(true, false), None);
    assert_eq!(path_opener_program_for(false, true), Some("xdg-open"));
    assert_eq!(notification_program_for(false, false), "osascript");
    assert_eq!(notification_program_for(true, false), "powershell");
    assert_eq!(notification_program_for(false, true), "notify-send");
}

#[test]
fn linux_notification_arguments_do_not_use_a_shell() {
    assert_eq!(
        linux_notification_args("Sleipnir", "build; rm -rf /"),
        ["--app-name", "Sleipnir", "Sleipnir", "build; rm -rf /"]
    );
}
```

- [ ] **Step 2: Run the tests and verify missing helper failures**

Run:

```bash
cargo test -p sleipnir_ui desktop_commands_are_platform_specific
cargo test -p sleipnir_ui linux_notification_arguments_do_not_use_a_shell
```

Expected: FAIL to compile because the explicit helpers do not exist.

- [ ] **Step 3: Implement path-opener selection**

Use:

```rust
pub fn path_opener_program() -> Option<&'static str> {
    path_opener_program_for(cfg!(windows), cfg!(target_os = "linux"))
}

pub fn path_opener_program_for(
    windows: bool,
    linux: bool,
) -> Option<&'static str> {
    if windows {
        None
    } else if linux {
        Some("xdg-open")
    } else {
        Some("open")
    }
}
```

Make `open_existing_path` call `path_opener_program()` for non-Windows instead of hard-coding `open`. Preserve `cmd /C start` on Windows. Log spawn errors with the selected program and candidate path.

- [ ] **Step 4: Implement Linux notification command construction**

Add:

```rust
fn notification_program_for(windows: bool, linux: bool) -> &'static str {
    if windows {
        "powershell"
    } else if linux {
        "notify-send"
    } else {
        "osascript"
    }
}

fn linux_notification_args<'a>(
    title: &'a str,
    message: &'a str,
) -> [&'a str; 4] {
    ["--app-name", "Sleipnir", title, message]
}
```

Under `#[cfg(target_os = "linux")]`, spawn with separate arguments:

```rust
match std::process::Command::new("notify-send")
    .args(linux_notification_args(title, message))
    .spawn()
{
    Ok(mut child) => {
        std::thread::spawn(move || match child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => log::warn!("notify-send exited with {status}"),
            Err(err) => log::warn!("failed waiting for notify-send: {err}"),
        });
    }
    Err(err) => log::warn!("failed to start notify-send: {err}"),
}
```

Keep macOS and Windows behavior intact. Remove the old no-op non-macOS branch. Keep `notify_uses_osascript()` or replace it with the explicit selector in tests.

- [ ] **Step 5: Update existing font-zoom/clipboard/path tests for Linux policy**

Where current host assertions assume “not Windows means macOS”, use explicit selectors or include `cfg!(target_os = "linux")`. Linux must expect Ctrl-based font zoom, `xdg-open`, and no bare Ctrl clipboard interception.

- [ ] **Step 6: Run UI tests**

Run:

```bash
cargo test -p sleipnir_ui
```

Expected: PASS; command tests prove Linux uses argument-safe `notify-send` and `xdg-open`.

- [ ] **Step 7: Commit Linux desktop integrations**

```bash
git add crates/sleipnir_ui/src/sleipnir_ui.rs
git commit -m "feat: add Linux desktop integrations"
```

---

### Task 6: Generalize custom titlebar controls

**Files:**
- Rename: `crates/sleipnir_ui/src/chrome/window_controls.rs` → `crates/sleipnir_ui/src/chrome/desktop_window_controls.rs`
- Modify: `crates/sleipnir_ui/src/chrome/mod.rs:1-18`
- Modify: `crates/sleipnir_ui/src/chrome/geometry.rs:1-119`
- Modify: `crates/sleipnir_ui/src/app_shell.rs:406-460, 4740-4970` (also appends actionable Linux Vulkan/display guidance to window-open failures)
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs:1595-1636` (source-boundary tests)

- [ ] **Step 1: Add geometry tests for macOS and non-macOS desktops**

Replace Windows-only naming with:

```rust
#[test]
fn desktop_chrome_reserves_caption_buttons_only_when_windowed() {
    let desktop = ChromeGeometry::for_window(true, false);
    assert_eq!(desktop.leading_pad, px(8.0));
    assert_eq!(desktop.trailing_pad, px(138.0));

    let desktop_fullscreen = ChromeGeometry::for_window(true, true);
    assert_eq!(desktop_fullscreen.trailing_pad, px(0.0));

    let macos = ChromeGeometry::for_window(false, false);
    assert!(macos.leading_pad >= px(71.0));
    assert_eq!(macos.trailing_pad, px(8.0));
}
```

- [ ] **Step 2: Run the geometry test before renaming the API**

Run:

```bash
cargo test -p sleipnir_ui desktop_chrome_reserves_caption_buttons
```

Expected: FAIL because the test/name/API do not yet exist in the desired form.

- [ ] **Step 3: Rename the caption module and cfg boundary**

Run:

```bash
git mv crates/sleipnir_ui/src/chrome/window_controls.rs \
  crates/sleipnir_ui/src/chrome/desktop_window_controls.rs
```

In `chrome/mod.rs`, use `pub(crate) mod desktop_window_controls;`.

Within the renamed file:

- replace “Windows” prose with “non-macOS desktop”;
- rename `render_windows_titlebar_end` → `render_desktop_titlebar_end`;
- rename `render_windows_caption_buttons` → `render_desktop_caption_buttons`;
- use `#[cfg(any(windows, target_os = "linux"))]` for real controls;
- use `#[cfg(target_os = "macos")]` for the empty element;
- rename element IDs from `windows-*` to `desktop-*`;
- keep the same `WindowControlArea::{Min, Max, Close}` hit areas.

- [ ] **Step 4: Change geometry to an explicit desktop-controls boolean**

Implement:

```rust
impl ChromeGeometry {
    pub fn standard() -> Self {
        Self::standard_for(cfg!(not(target_os = "macos")))
    }

    pub fn standard_for(desktop_controls: bool) -> Self {
        Self::for_window(desktop_controls, false)
    }

    pub fn for_window(desktop_controls: bool, fullscreen: bool) -> Self {
        Self {
            // keep all existing dimensions
            leading_pad: if fullscreen {
                Self::fullscreen_leading_pad()
            } else {
                leading_pad_for(desktop_controls)
            },
            trailing_pad: if fullscreen {
                px(0.0)
            } else {
                trailing_pad_for(desktop_controls)
            },
            // remaining existing fields unchanged
        }
    }
}

pub fn leading_pad_for(desktop_controls: bool) -> Pixels {
    if desktop_controls {
        px(8.0)
    } else if cfg!(macos_sdk_26_or_later) {
        px(78.0)
    } else {
        px(71.0)
    }
}

pub fn trailing_pad_for(desktop_controls: bool) -> Pixels {
    if desktop_controls { px(138.0) } else { px(8.0) }
}
```

- [ ] **Step 5: Use non-macOS titlebar options in both window constructors**

In `open_sleipnir_window` and `open_sleipnir_window_with_tab`:

```rust
traffic_light_position: if cfg!(target_os = "macos") {
    Some(geo.traffic_light_position)
} else {
    None
},
```

- [ ] **Step 6: Render controls and reserve drag space on Linux**

In `Render::render`, replace the separate fullscreen leading calculation with:

```rust
let fullscreen = window.is_fullscreen();
let geo = ChromeGeometry::for_window(
    cfg!(not(target_os = "macos")),
    fullscreen,
);
let leading = geo.leading_pad;
```

Then in the chrome band:

- set the trailing drag minimum from `geo.trailing_pad`, which is `0px` in fullscreen instead of retaining the windowed 138px reservation;
- call `render_desktop_titlebar_end`;
- keep controls absent in fullscreen by wrapping the `render_desktop_titlebar_end` child in `.when(!fullscreen, ...)`; do not hide them inside the renderer so fullscreen behavior remains visible at the chrome composition site.

Update source-boundary tests in `sleipnir_ui.rs` to require the new function/module names and permit Windows/Linux cfgs.

- [ ] **Step 7: Run chrome and UI tests**

Run:

```bash
cargo test -p sleipnir_ui chrome::geometry::tests
cargo test -p sleipnir_ui chrome_band
cargo test -p sleipnir_ui
```

Expected: PASS; caption width equals the windowed desktop trailing pad, fullscreen trailing pad is zero, and source tests find the renamed desktop renderer.

- [ ] **Step 8: Commit custom chrome support**

```bash
git add crates/sleipnir_ui/src/chrome crates/sleipnir_ui/src/app_shell.rs crates/sleipnir_ui/src/sleipnir_ui.rs
git commit -m "feat: add Linux custom window controls"
```

---

### Task 7: Build deterministic Linux packages

**Files:**
- Create: `resources/linux/sleipnir.desktop`
- Create: `scripts/make-linux-package.sh`
- Create: `scripts/tests/test-linux-package.sh`

- [ ] **Step 1: Create the failing packaging-policy test**

Create executable `scripts/tests/test-linux-package.sh` with source-only mode:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export SLEIPNIR_PACKAGE_SOURCE_ONLY=1
# shellcheck source=../make-linux-package.sh
source "${ROOT}/scripts/make-linux-package.sh"

assert_eq() {
  [[ "$1" == "$2" ]] || {
    printf 'expected %q, got %q\n' "$2" "$1" >&2
    exit 1
  }
}

assert_eq "$(portable_arch_for x86_64)" "x86_64"
assert_eq "$(portable_arch_for amd64)" "x86_64"
assert_eq "$(portable_arch_for aarch64)" "aarch64"
assert_eq "$(portable_arch_for arm64)" "aarch64"
assert_eq "$(debian_arch_for x86_64)" "amd64"
assert_eq "$(debian_arch_for aarch64)" "arm64"

if portable_arch_for riscv64 >/dev/null 2>&1; then
  echo "unknown architecture unexpectedly accepted" >&2
  exit 1
fi

assert_eq "$(tarball_name 1.2.3 x86_64)" \
  "Sleipnir-1.2.3-linux-x86_64.tar.gz"
assert_eq "$(tarball_name 1.2.3 aarch64)" \
  "Sleipnir-1.2.3-linux-aarch64.tar.gz"
assert_eq "$(deb_name 1.2.3 amd64)" "sleipnir_1.2.3_amd64.deb"
assert_eq "$(deb_name 1.2.3 arm64)" "sleipnir_1.2.3_arm64.deb"
```

- [ ] **Step 2: Run the script and verify the missing implementation**

Run:

```bash
bash scripts/tests/test-linux-package.sh
```

Expected: FAIL because `scripts/make-linux-package.sh` does not exist.

- [ ] **Step 3: Add the desktop entry**

Create `resources/linux/sleipnir.desktop`:

```ini
[Desktop Entry]
Type=Application
Version=1.0
Name=Sleipnir
GenericName=Terminal Emulator
Comment=A fast native GPU-rendered terminal emulator
Exec=sleipnir
Icon=sleipnir
Terminal=false
Categories=System;TerminalEmulator;Utility;
Keywords=terminal;console;shell;tabs;split;pane;
StartupNotify=true
```

- [ ] **Step 4: Implement strict packaging helpers and source-only mode**

Start `scripts/make-linux-package.sh` with:

```bash
#!/usr/bin/env bash
set -euo pipefail

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

if [[ "${SLEIPNIR_PACKAGE_SOURCE_ONLY:-0}" == "1" ]]; then
  return 0 2>/dev/null || exit 0
fi
```

The executable main body must support:

```text
--binary <path>   consume an existing native release binary
--out <dir>       output directory (default ./build)
--no-deb          skip Debian package
--no-tar          skip tarball
--no-strip        retain symbols
```

When `--binary` is omitted, run `cargo build --release -p sleipnir`. Determine the version from `SLEIPNIR_VERSION` or Cargo metadata, never by grepping a literal crate-manifest line:

```bash
VERSION="${SLEIPNIR_VERSION:-$(
  cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json,sys; data=json.load(sys.stdin); print(next(p["version"] for p in data["packages"] if p["name"] == "sleipnir"))'
)}"
```

This continues to work if the crate later changes to `version.workspace = true`. Determine native architecture from `dpkg --print-architecture` when available, otherwise `uname -m`, then map it through both strict helpers.

- [ ] **Step 5: Assemble the portable tarball**

Stage exactly:

```text
Sleipnir-<version>-linux-<portable-arch>/
  sleipnir
  sleipnir.desktop
  sleipnir.png
  README.txt
  LICENSE
```

`README.txt` must state Ubuntu 22.04+, glibc 2.35+, Vulkan, Wayland/X11, fontconfig, `xdg-open`, and `notify-send` requirements. It must include commands to copy `sleipnir.desktop` into `${XDG_DATA_HOME:-$HOME/.local/share}/applications`, copy the icon into the matching hicolor directory, and run `update-desktop-database` when available. Preserve executable mode on `sleipnir`. Name the archive with `tarball_name`.

For reproducible packaging, derive and export `SOURCE_DATE_EPOCH` from the environment or `git log -1 --format=%ct`, normalize staged file mtimes to it, sort tar members by name, and force numeric owner/group `0`. Create the tarball with a small Python standard-library `tarfile` + `gzip.GzipFile(mtime=0)` block so this package test behaves identically with BSD tar on macOS and GNU tar on Linux; do not depend on GNU-only `tar --sort/--mtime` flags. Apply the same normalized mtimes before `dpkg-deb`. In the Linux package test, build twice with the same `SOURCE_DATE_EPOCH` and assert matching SHA-256 digests for both tarballs and both Debian packages.

- [ ] **Step 6: Assemble the Debian package**

Stage these paths:

```text
/usr/bin/sleipnir
/usr/share/applications/sleipnir.desktop
/usr/share/icons/hicolor/{48x48,64x64,128x128,256x256,512x512}/apps/sleipnir.png
/usr/share/doc/sleipnir/README.txt
/usr/share/doc/sleipnir/changelog.gz
/usr/share/doc/sleipnir/copyright
/usr/share/licenses/sleipnir/LICENSE
```

Generate PNG sizes from `resources/appicon_preview.png` using Pillow. Generate `changelog.gz` with `gzip -n -9` (or Python gzip with `mtime=0`) so it does not embed the build clock. Build `DEBIAN/control` with native `Architecture`, `Installed-Size`, homepage, description, and dependencies.

Run `dpkg-shlibdeps` in a temporary packaging workspace that includes a minimal `debian/control` source-package stanza, and pass `-e"${DEB_ROOT}/usr/bin/sleipnir" -O` explicitly; do not assume the repository itself is a Debian source tree. Fail if it returns no dependencies. Then append and de-duplicate all runtime-loaded integrations explicitly so `dlopen` does not make package metadata incomplete:

```text
libx11-6, libxcb1, libxkbcommon0, libxkbcommon-x11-0,
libvulkan1, libwayland-client0, libfontconfig1, xdg-utils, libnotify-bin
```

The package test and CI must assert every explicit name is present in `Depends`; do not rely on `dpkg-shlibdeps` to discover display libraries loaded at runtime. Build with:

```bash
dpkg-deb --build --root-owner-group "${DEB_ROOT}" "${DEB_PATH}"
```

- [ ] **Step 7: Generate digest-only sidecars**

Add a portable helper:

```bash
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print tolower($1)}'
  else
    shasum -a 256 "$1" | awk '{print tolower($1)}'
  fi
}
```

For each generated artifact:

```bash
sha256_file "${artifact}" > "${artifact}.sha256"
```

Then compare the stored digest with a fresh `sha256_file` result and fail on mismatch. Linux CI still validates independently with `sha256sum`.

- [ ] **Step 8: Make the pure packaging test pass**

Run:

```bash
chmod +x scripts/make-linux-package.sh scripts/tests/test-linux-package.sh
bash scripts/tests/test-linux-package.sh
```

Expected: PASS.

- [ ] **Step 9: Extend the package test with a fake native ELF fixture hook**

Add `SLEIPNIR_PACKAGE_SKIP_ELF_CHECK=1` only for the shell unit test, create a temporary executable, invoke `--binary ... --no-deb --no-strip`, and assert tar members and `.sha256` shape. `--no-deb` keeps the portable test independent of Debian tooling on macOS, and `--no-strip` prevents `strip` from rejecting the intentional text fixture. Keep real `file`/`readelf`, stripping, and Debian checks mandatory outside these explicit test arguments. Do not allow the CI release job to set `SLEIPNIR_PACKAGE_SKIP_ELF_CHECK`.

- [ ] **Step 10: Run static checks available on the current host**

Run:

```bash
bash -n scripts/make-linux-package.sh scripts/tests/test-linux-package.sh
shellcheck scripts/make-linux-package.sh scripts/tests/test-linux-package.sh
```

Expected: PASS. If `shellcheck` is not installed locally, record that and rely on the required Linux CI step; do not suppress findings.

- [ ] **Step 11: Commit packaging**

```bash
git add resources/linux scripts/make-linux-package.sh scripts/tests/test-linux-package.sh
git commit -m "feat: package native Linux releases"
```

---

### Task 8: Extend the one-line installer to Linux

**Files:**
- Modify: `scripts/install.sh:1-123`
- Create: `scripts/tests/test-install.sh`

- [ ] **Step 1: Create source-mode architecture and asset tests**

Create executable `scripts/tests/test-install.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export SLEIPNIR_INSTALL_SOURCE_ONLY=1
# shellcheck source=../install.sh
source "${ROOT}/scripts/install.sh"

assert_eq() {
  [[ "$1" == "$2" ]] || {
    printf 'expected %q, got %q\n' "$2" "$1" >&2
    exit 1
  }
}

assert_eq "$(linux_deb_arch_for x86_64)" amd64
assert_eq "$(linux_deb_arch_for aarch64)" arm64
assert_eq "$(linux_portable_arch_for x86_64)" x86_64
assert_eq "$(linux_portable_arch_for aarch64)" aarch64
assert_eq "$(linux_asset_name 1.2.3 x86_64 0)" sleipnir_1.2.3_amd64.deb
assert_eq "$(linux_asset_name 1.2.3 aarch64 1)" Sleipnir-1.2.3-linux-aarch64.tar.gz

if linux_deb_arch_for riscv64 >/dev/null 2>&1; then
  echo "unsupported architecture accepted" >&2
  exit 1
fi
```

- [ ] **Step 2: Run the installer test and verify missing Linux helpers**

Run:

```bash
bash scripts/tests/test-install.sh
```

Expected: FAIL because the helpers do not exist.

- [ ] **Step 3: Refactor the installer into sourceable functions**

Keep one self-contained public script because `curl .../install.sh | bash` cannot source repository siblings. Add:

```bash
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
  if [[ "$tarball" == "1" ]]; then
    printf 'Sleipnir-%s-linux-%s.tar.gz\n' \
      "$version" "$(linux_portable_arch_for "$host_arch")"
  else
    printf 'sleipnir_%s_%s.deb\n' \
      "$version" "$(linux_deb_arch_for "$host_arch")"
  fi
}
```

Extract shared `resolve_latest_release`, `download`, `sha256_file`, and `verify_sha` functions. `verify_sha` must require the sidecar; remove the old “warning, skip integrity” behavior. Create one installer temp directory with `mktemp -d` and register `trap cleanup EXIT` before any download so Darwin and Linux clean temporary files on both success and failure.

At the end:

```bash
if [[ "${SLEIPNIR_INSTALL_SOURCE_ONLY:-0}" != "1" ]]; then
  main "$@"
fi
```

- [ ] **Step 4: Preserve Darwin behavior behind `install_macos`**

Move the existing DMG flow into `install_macos`. Preserve `PREFIX=/Applications`, `SLEIPNIR_NO_OPEN`, DMG mounting, SHA verification, quarantine removal, sudo handling, and launch behavior. Do not change artifact naming.

- [ ] **Step 5: Implement verified `.deb` installation**

`install_linux` must:

1. map `uname -m` before downloading;
2. resolve the latest version once;
3. default to the `.deb` when `SLEIPNIR_TARBALL != 1`;
4. require `apt` (accept `apt` or `apt-get`, choose one consistently);
5. if apt is missing, fail with the exact retry guidance:

```text
Retry with: curl ... | SLEIPNIR_TARBALL=1 bash
```

6. download the matching `.deb` and digest-only sidecar;
7. verify SHA-256;
8. only then call `sudo apt install -y ./<package>` (or `apt-get install -y` with an absolute local path).

- [ ] **Step 6: Implement user-local tarball installation**

With `SLEIPNIR_TARBALL=1`, use:

```bash
BIN_HOME="${XDG_BIN_HOME:-${HOME}/.local/bin}"
DATA_HOME="${XDG_DATA_HOME:-${HOME}/.local/share}"
```

After verification and extraction, install:

```text
${BIN_HOME}/sleipnir
${DATA_HOME}/applications/sleipnir.desktop
${DATA_HOME}/icons/hicolor/512x512/apps/sleipnir.png
```

Use `install -m` through temporary destination names followed by `mv` so existing files are not truncated on failure. Warn when `BIN_HOME` is not in `PATH`. Launch only when `SLEIPNIR_NO_OPEN != 1`.

- [ ] **Step 7: Dispatch only Darwin and Linux**

Implement:

```bash
main() {
  case "$(uname -s)" in
    Darwin) install_macos ;;
    Linux) install_linux ;;
    *) echo "ERROR: Sleipnir prebuilt installs support macOS and Linux." >&2; exit 1 ;;
  esac
}
```

- [ ] **Step 8: Add a SHA mismatch safety test with mocked commands**

In `scripts/tests/test-install.sh`, create a temp `PATH` containing mock `curl`, `uname`, `sudo`, `apt`, `install`, and `mv`. Make `curl` emit an artifact plus a deliberately wrong sidecar.

For `.deb` mode, invoke `install_linux` in a subprocess and assert:

```bash
[[ "$status" -ne 0 ]]
[[ ! -e "${calls}/sudo" ]]
[[ ! -e "${calls}/apt" ]]
```

For `SLEIPNIR_TARBALL=1`, point `XDG_BIN_HOME` and `XDG_DATA_HOME` at empty temporary directories, repeat the digest mismatch, and assert:

```bash
[[ "$status" -ne 0 ]]
[[ ! -e "${calls}/install" ]]
[[ ! -e "${calls}/mv" ]]
[[ ! -e "${XDG_BIN_HOME}/sleipnir" ]]
[[ ! -e "${XDG_DATA_HOME}/applications/sleipnir.desktop" ]]
```

Add successful URL-selection cases for both `x86_64` and `aarch64`, an apt-missing case whose stderr contains the effective `curl ... | SLEIPNIR_TARBALL=1 bash` form, and a dispatcher test that mocks `uname -s` as `Darwin` and `Linux` and verifies `main` calls `install_macos` and `install_linux` respectively without performing network I/O.

- [ ] **Step 9: Run installer tests and shell lint**

Run:

```bash
chmod +x scripts/tests/test-install.sh
bash scripts/tests/test-install.sh
bash -n scripts/install.sh scripts/tests/test-install.sh
shellcheck scripts/install.sh scripts/tests/test-install.sh
```

Expected: PASS; no mocked installer or destination write runs after a digest mismatch.

- [ ] **Step 10: Commit the installer**

```bash
git add scripts/install.sh scripts/tests/test-install.sh
git commit -m "feat: install verified Linux packages"
```

---

### Task 9: Add native x86_64 and ARM64 Linux CI

**Files:**
- Modify: `.github/workflows/build-and-release.yml:1-281`
- Create: `docs/linux-release-checklist.md`

- [ ] **Step 1: Add a workflow structure check to the packaging test**

Extend `scripts/tests/test-linux-package.sh` with source assertions for:

```bash
grep -q 'ubuntu-22.04-arm' "${ROOT}/.github/workflows/build-and-release.yml"
grep -q 'expected-arch: aarch64' "${ROOT}/.github/workflows/build-and-release.yml"
grep -q 'scripts/tests/test-install.sh' "${ROOT}/.github/workflows/build-and-release.yml"
grep -q 'xvfb-run' "${ROOT}/.github/workflows/build-and-release.yml"
```

- [ ] **Step 2: Run the check and verify the workflow is red**

Run:

```bash
bash scripts/tests/test-linux-package.sh
```

Expected: FAIL because no Linux matrix exists.

- [ ] **Step 3: Add the native runner matrix**

The official GitHub-hosted runner reference (verified 2026-08-22) lists both `ubuntu-22.04` x64 and `ubuntu-22.04-arm` arm64. Add one `linux-check` job:

```yaml
strategy:
  fail-fast: false
  matrix:
    include:
      - runner: ubuntu-22.04
        expected-arch: x86_64
        deb-arch: amd64
        artifact-suffix: x86_64
      - runner: ubuntu-22.04-arm
        expected-arch: aarch64
        deb-arch: arm64
        artifact-suffix: aarch64
runs-on: ${{ matrix.runner }}
```

Do not add QEMU, `cross`, or cross targets. First assert:

```bash
test "$(uname -m)" = "${{ matrix.expected-arch }}"
```

- [ ] **Step 4: Install Linux build, package, lint, and smoke dependencies**

Install at least:

```text
pkg-config
libfontconfig-dev
libfreetype-dev
libx11-dev
libxkbcommon-dev
libxkbcommon-x11-dev
libwayland-dev
libglib2.0-dev
libvulkan1
mesa-vulkan-drivers
python3-pil
dpkg-dev
desktop-file-utils
libnotify-bin
xdg-utils
xvfb
xdotool
shellcheck
```

If the pinned GPUI graph reports another missing native dependency, diagnose it with @systematic-debugging and add only that dependency.

- [ ] **Step 5: Run Linux unit and shell tests**

Use:

```yaml
- run: cargo test --workspace
- run: bash scripts/tests/test-linux-package.sh
- run: bash scripts/tests/test-install.sh
- run: shellcheck scripts/install.sh scripts/make-linux-package.sh scripts/tests/*.sh
```

- [ ] **Step 6: Build once and package the existing ELF**

Run:

```bash
cargo build --release -p sleipnir
SLEIPNIR_VERSION="${VER}" ./scripts/make-linux-package.sh \
  --binary target/release/sleipnir \
  --out build
```

Assert the two package names match the matrix architecture.

- [ ] **Step 7: Validate architecture, metadata, contents, desktop entry, and hashes**

Use commands equivalent to:

```bash
readelf -h target/release/sleipnir | grep -F "$EXPECTED_MACHINE"
tar -tzf "build/${TARBALL}" | grep -F '/sleipnir'
dpkg-deb --field "build/${DEB}" Architecture | grep -Fx "${DEB_ARCH}"
depends="$(dpkg-deb --field "build/${DEB}" Depends)"
for package in \
  libx11-6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 \
  libvulkan1 libwayland-client0 libfontconfig1 xdg-utils libnotify-bin
do
  grep -F "$package" <<<"$depends"
done
dpkg-deb --contents "build/${DEB}" | grep -F './usr/bin/sleipnir'
desktop-file-validate resources/linux/sleipnir.desktop
for artifact in "build/${TARBALL}" "build/${DEB}"; do
  expected="$(tr -d '[:space:]' < "${artifact}.sha256")"
  actual="$(sha256sum "${artifact}" | awk '{print $1}')"
  test "$expected" = "$actual"
done
```

Map `x86_64` to ELF machine `Advanced Micro Devices X86-64` and `aarch64` to `AArch64` in the matrix or a shell case.

- [ ] **Step 8: Add an X11 window-initialization smoke**

Force X11 by unsetting Wayland and run under Xvfb with Mesa software Vulkan:

```bash
unset WAYLAND_DISPLAY
export LIBGL_ALWAYS_SOFTWARE=1
xvfb-run -a -s '-screen 0 1280x720x24' bash -euo pipefail <<'SH'
target/release/sleipnir >sleipnir-smoke.log 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
found=0
for _ in $(seq 1 40); do
  if xdotool search --onlyvisible --name 'Sleipnir' >/dev/null 2>&1; then
    found=1
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    cat sleipnir-smoke.log >&2
    exit 1
  fi
  sleep 0.25
done
test "$found" = 1
SH
```

A live process without a discoverable window is failure.

- [ ] **Step 9: Upload exactly four files per architecture**

Upload architecture-specific Actions artifacts containing one tarball, one `.deb`, and their two sidecars. In release mode, wait for the macOS job’s Release and verify the array length is exactly four before `gh release upload --clobber`.

- [ ] **Step 10: Update generated release notes**

Replace “Linux is not supported” with both architecture package names, Ubuntu 22.04+/Vulkan requirements, Wayland/X11 support, and the manual-update rule. State that Linux assets may appear after the macOS release is created.

- [ ] **Step 11: Add the manual release checklist**

Create `docs/linux-release-checklist.md` with checkboxes for:

- Ubuntu 22.04 GNOME Wayland startup and terminal input;
- GNOME Xorg/XWayland startup;
- x86_64 and ARM64 `.deb` install/uninstall;
- both user-local tarball installs;
- `notify-send`, `xdg-open`, window controls, shortcuts, and Check for Updates;
- link to the release-tracking issue where evidence is recorded;
- instruction not to undraft/announce a release until the Wayland result is recorded.

- [ ] **Step 12: Run local structural checks**

Run:

```bash
bash scripts/tests/test-linux-package.sh
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/build-and-release.yml")' \
  2>/dev/null || true
git diff --check
```

Expected: packaging structure check PASS. If Ruby rejects GitHub’s `on:` key semantics, use `actionlint` when available rather than treating Ruby as authoritative.

- [ ] **Step 13: Commit CI and release validation**

```bash
git add .github/workflows/build-and-release.yml docs/linux-release-checklist.md scripts/tests/test-linux-package.sh
git commit -m "ci: build and validate native Linux releases"
```

---

### Task 10: Expose Linux assets on the website

**Files:**
- Modify: `website/package.json`
- Modify: `website/package-lock.json`
- Create: `website/src/lib/release.test.ts`
- Modify: `website/src/lib/release.ts:1-52`
- Modify: `website/src/components/download-menu.tsx:1-136`
- Modify: `website/src/components/install-command.tsx:1-55`
- Modify: `website/src/App.tsx:1-333`
- Modify: `website/index.html:1-22`
- Modify: `website/README.md:1-35`

- [ ] **Step 1: Add Vitest to the website**

Run:

```bash
cd website
npm install --save-dev vitest
npm pkg set scripts.test='vitest run'
```

Expected: `package.json` and `package-lock.json` update with a reproducible test command.

- [ ] **Step 2: Write the failing release parser test**

Create `website/src/lib/release.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { parseLatestRelease } from './release'

describe('parseLatestRelease', () => {
  it('discovers current Windows and both Linux architecture assets', () => {
    const release = parseLatestRelease({
      tag_name: 'v0.3.0',
      html_url: 'https://example.test/v0.3.0',
      assets: [
        { name: 'Sleipnir-0.3.0-macos.dmg', browser_download_url: 'dmg' },
        { name: 'Sleipnir-0.3.0-windows-x64.exe', browser_download_url: 'exe' },
        { name: 'sleipnir_0.3.0_amd64.deb', browser_download_url: 'deb-x64' },
        { name: 'Sleipnir-0.3.0-linux-x86_64.tar.gz', browser_download_url: 'tar-x64' },
        { name: 'sleipnir_0.3.0_arm64.deb', browser_download_url: 'deb-arm' },
        { name: 'Sleipnir-0.3.0-linux-aarch64.tar.gz', browser_download_url: 'tar-arm' },
      ],
    })

    expect(release).toMatchObject({
      version: '0.3.0',
      windowsExeUrl: 'exe',
      linuxX64DebUrl: 'deb-x64',
      linuxX64TarUrl: 'tar-x64',
      linuxArm64DebUrl: 'deb-arm',
      linuxArm64TarUrl: 'tar-arm',
    })
  })
})
```

- [ ] **Step 3: Run the test and verify the missing parser**

Run:

```bash
npm test -- release.test.ts
```

Expected: FAIL because `parseLatestRelease` and the new fields do not exist.

- [ ] **Step 4: Extract and implement a pure release parser**

In `release.ts`, replace stale `zipUrl`/`windowsZipUrl` fields with:

```ts
export interface LatestRelease {
  version: string
  dmgUrl: string | null
  windowsExeUrl: string | null
  linuxX64DebUrl: string | null
  linuxX64TarUrl: string | null
  linuxArm64DebUrl: string | null
  linuxArm64TarUrl: string | null
  htmlUrl: string
}
```

Export `parseLatestRelease(data: GitHubReleasePayload): LatestRelease | null` and match exact suffixes while excluding `.sha256`. Let `fetchLatestRelease` do only network/error handling and delegate parsing.

Set:

```ts
export const INSTALL_HINT =
  'Detects macOS or Linux, verifies SHA-256, and installs the matching release.'
```

- [ ] **Step 5: Add architecture-labelled menu items**

In `downloadItems`, order items:

```text
macOS (.dmg)
Windows x64 (.exe)
Linux x86_64 (.deb)
Linux x86_64 (.tar.gz)
Linux ARM64 (.deb)
Linux ARM64 (.tar.gz)
All releases
```

Only render an asset when its URL is present.

- [ ] **Step 6: Update product copy and metadata**

In `App.tsx` and `index.html`:

- say macOS, Windows, and Linux in hero/metadata;
- mention Metal, Direct3D 11, and Vulkan;
- mention Wayland/X11 and Ubuntu 22.04+;
- use Ctrl-based Linux shortcut copy where platform-specific examples appear;
- state in-place update is macOS-only and Linux opens Releases;
- remove stale `.zip` and “Linux is not supported” text.

Update `website/README.md` to match actual EXE/DMG/Linux assets.

- [ ] **Step 7: Run website tests and production build**

Run:

```bash
npm test
npm run build
```

Expected: Vitest PASS, TypeScript PASS, Vite build PASS.

- [ ] **Step 8: Smoke the built website**

Run `npm run dev -- --host 127.0.0.1` and use @chrome-use to verify the Download menu exposes the expected labels (the GitHub API may omit future Linux assets; use a test fixture or component test if the live latest release does not yet contain them). Check browser console for errors.

- [ ] **Step 9: Commit website support**

```bash
git add website
git commit -m "feat: publish Linux downloads on website"
```

---

### Task 11: Update English and Chinese product documentation

**Files:**
- Modify: `README.md:1-343`
- Modify: `README.zh.md:1-317`
- Modify: `UPSTREAM.md:1-133`
- Modify: `docs/glossary.md:1-15`
- Modify: `CHANGELOG.md:1-20`

- [ ] **Step 1: Add a documentation assertion for stale support claims**

Run and save the expected failing search in the task notes:

```bash
rg -n -S \
  'Linux is not supported|不支持 Linux|macOS only|macOS-only' \
  README.md README.zh.md UPSTREAM.md docs/glossary.md website \
  -g '!node_modules/**'
```

Expected: FAIL the acceptance gate by printing current stale statements.

- [ ] **Step 2: Update the English README**

Document:

- three supported platforms and renderers;
- Linux x86_64/ARM64 `.deb` and tarball names;
- the shared one-line command and `SLEIPNIR_TARBALL=1` mode;
- Ubuntu 22.04+ official support and other glibc 2.35+ best effort;
- Vulkan, Wayland/X11, fontconfig, xdg-utils, and libnotify prerequisites;
- Linux Ctrl-based shortcuts and `Ubuntu Mono` default;
- `~/.config/sleipnir` config/session path;
- source build packages and `cargo run -p sleipnir`;
- Linux package build command;
- manual Check for Updates behavior;
- native x86_64/ARM64 CI coverage.

Do not describe tarballs as self-contained.

- [ ] **Step 3: Mirror all support facts in the Chinese README**

Keep commands and filenames identical to `README.md`. Translate support boundaries and troubleshooting rather than shortening away ARM64, Wayland/X11, dependency, or update behavior.

- [ ] **Step 4: Update upstream and glossary boundaries**

In `UPSTREAM.md`, add `gpui_linux` to the pin/package table and describe `gpui_platform` as a macOS/Windows/Linux entry. Add `wayland`/`x11` feature expectations to the upgrade checklist.

In `docs/glossary.md`, replace “macOS only” and `AppKit NSWindow` with platform-neutral OS-window wording, optionally listing AppKit/Win32/Wayland-X11 examples.

- [ ] **Step 5: Add a new changelog entry**

At the top unreleased/current section, state restored Linux support, native architectures, package formats, Wayland/X11/Vulkan, installer, notifications/path opener, and CI. Preserve historical entries that accurately say Linux was removed in that old release; the stale-claim gate should exclude historical changelog text rather than rewriting history.

- [ ] **Step 6: Rerun stale-claim and link checks**

Run:

```bash
rg -n -S \
  'Linux is not supported|不支持 Linux|A standalone terminal emulator built on GPUI \(macOS only\)' \
  README.md README.zh.md UPSTREAM.md docs/glossary.md website \
  -g '!node_modules/**'

test -f resources/linux/sleipnir.desktop
test -f scripts/make-linux-package.sh
test -f docs/linux-release-checklist.md
git diff --check
```

Expected: `rg` returns no matches (exit 1), all `test -f` commands pass, and `git diff --check` is clean.

- [ ] **Step 7: Commit documentation**

```bash
git add README.md README.zh.md UPSTREAM.md docs/glossary.md CHANGELOG.md
git commit -m "docs: document Linux support and packages"
```

---

### Task 12: Run the release-readiness verification matrix

**Files:**
- Verify only; fix the owning task’s files if a check fails.

- [ ] **Step 1: Format Rust code**

Run:

```bash
cargo fmt --all -- --check
```

Expected: PASS. If it fails, run `cargo fmt --all`, inspect the diff, and recommit the affected task rather than making a formatting-only mystery commit.

- [ ] **Step 2: Run the complete Rust test suite**

Run:

```bash
cargo test --workspace
```

Expected: PASS on the current host.

- [ ] **Step 3: Run Clippy on the workspace**

Run:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 4: Build the application on the current host**

Run:

```bash
cargo build -p sleipnir
```

Expected: PASS.

- [ ] **Step 5: Run all shell tests and lint**

Run:

```bash
bash scripts/tests/test-linux-package.sh
bash scripts/tests/test-install.sh
bash -n scripts/install.sh scripts/make-linux-package.sh scripts/tests/*.sh
shellcheck scripts/install.sh scripts/make-linux-package.sh scripts/tests/*.sh
```

Expected: PASS.

- [ ] **Step 6: Run website tests/build**

Run:

```bash
npm --prefix website test
npm --prefix website run build
```

Expected: PASS.

- [ ] **Step 7: Validate repository-wide stale platform statements**

Run:

```bash
rg -n -S \
  'Linux is not supported|不支持 Linux|supports macOS and Windows only' \
  . \
  -g '!target/**' -g '!website/node_modules/**' -g '!.git/**' \
  -g '!CHANGELOG.md' -g '!docs/superpowers/**'
```

Expected: no matches.

- [ ] **Step 8: Validate both native GitHub Actions jobs**

Push the implementation branch and run `build-and-release.yml` as a draft/manual workflow. Verify both `ubuntu-22.04` and `ubuntu-22.04-arm` jobs pass unit tests, package validation, and X11 smoke. Download each Actions artifact and confirm exactly four files per architecture.

Expected files:

```text
Sleipnir-<ver>-linux-x86_64.tar.gz
Sleipnir-<ver>-linux-x86_64.tar.gz.sha256
sleipnir_<ver>_amd64.deb
sleipnir_<ver>_amd64.deb.sha256
Sleipnir-<ver>-linux-aarch64.tar.gz
Sleipnir-<ver>-linux-aarch64.tar.gz.sha256
sleipnir_<ver>_arm64.deb
sleipnir_<ver>_arm64.deb.sha256
```

- [ ] **Step 9: Complete and record manual Linux smoke**

Follow `docs/linux-release-checklist.md` on Ubuntu 22.04 GNOME Wayland and GNOME Xorg/XWayland. Record results in a release-tracking issue and link it from the draft Release. Do not mark release readiness complete with an unchecked Wayland startup.

- [ ] **Step 10: Verify macOS and Windows jobs remain green**

Confirm the same workflow’s macOS DMG and Windows EXE jobs pass and their existing artifacts/checksums remain present.

- [ ] **Step 11: Inspect the final change set**

Run:

```bash
git status --short
git log --oneline --decorate -15
git diff origin/main...HEAD --stat
git diff --check origin/main...HEAD
```

Expected: only planned files changed, no untracked build output, no whitespace errors.

- [ ] **Step 12: Commit any verification-only corrections**

If verification required corrections, commit them by owning concern, for example:

```bash
git add path/to/the/corrected/file path/to/its/test
git commit -m "fix: address Linux release verification"
```

If no files changed, do not create an empty commit.

---

## Handoff notes

- The historical Linux implementation at `518d5b2` is a reference for intent, not a patch source. In particular, do **not** restore its bare `Ctrl+1..9`, FreeBSD cfgs, `notify-rust` dependency, optional checksum behavior, or x86_64-only installer assumptions.
- GitHub’s official runner reference was checked while planning and lists `ubuntu-22.04-arm` as a standard native ARM64 label. Re-check the official documentation if Actions rejects the label; do not silently replace it with cross-compilation.
- Keep release artifacts system-dependency-based. Do not add AppImage, musl, GPG, apt repository publishing, or bundled graphics libraries.
- Use the plan checkboxes as the execution ledger and keep one focused commit per task.
