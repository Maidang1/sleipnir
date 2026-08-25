# Transactional macOS Auto-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the macOS shell updater with a signed, durable Rust supervisor transaction that atomically swaps app bundles, confirms healthy startup, and automatically rolls back failures.

**Architecture:** The `updater` library owns signed release discovery, preparation, transaction persistence, recovery policy, and its small application-facing interface. A new `sleipnir-update-helper` binary owns old-process observation, same-volume atomic swap, LaunchServices launch, health supervision, commit, and rollback. Sleipnir UI observes one process-wide update model and reports health only after first-window creation.

**Tech Stack:** Rust 2024, serde/serde_json, sha2, ed25519-dalek, ureq, libc/kqueue/renameatx_np, objc2/objc2-app-kit/block2, GPUI, GitHub Actions, macOS DMG/codesign tooling.

**Approved specification:** `docs/superpowers/specs/2026-08-25-transactional-auto-update-design.md`

**Execution constraint:** The user explicitly declined git worktrees and authorized implementation in the current `main` checkout. Use TDD and commit after every task. Do not push or publish releases.

---

## File map

### Updater library

- Create `crates/updater/src/lib.rs`: external interface and shared exports.
- Create `crates/updater/src/release.rs`: GitHub release query and asset identity.
- Create `crates/updater/src/manifest.rs`: canonical signed manifest parsing and Ed25519 verification.
- Create `crates/updater/src/download.rs`: bounded streaming artifact download and digest verification.
- Create `crates/updater/src/transaction.rs`: schema, phases, errors, atomic persistence, health marker.
- Create `crates/updater/src/recovery.rs`: pure filesystem-evidence recovery decisions and outcome interpretation.
- Create `crates/updater/src/prepare.rs`: DMG extraction and prepared-update orchestration.
- Create `crates/updater/src/platform/mod.rs`: platform adapter exports.
- Create `crates/updater/src/platform/macos.rs`: macOS bundle/path/signature/swap capability preflight.
- Remove `crates/updater/src/updater.rs` after callers and tests move.
- Modify `crates/updater/Cargo.toml`: library path and dependencies.

### Supervisor

- Create `crates/update_helper/Cargo.toml`.
- Create `crates/update_helper/src/main.rs`: strict CLI and production adapter wiring.
- Create `crates/update_helper/src/supervisor.rs`: transaction algorithm over internal seams.
- Create `crates/update_helper/src/swap.rs`: `RENAME_SWAP` adapter.
- Create `crates/update_helper/src/process.rs`: kqueue process watch/termination.
- Create `crates/update_helper/src/launch.rs`: NSWorkspace launch and PID return.
- Create `crates/update_helper/src/log.rs`: structured append-only diagnostics.
- Create `crates/update_helper/tests/supervisor.rs`: fake-adapter fault-injection integration suite.

### App integration

- Modify `Cargo.toml`: workspace member and dependencies.
- Modify `Cargo.lock`: resolved dependencies.
- Create `crates/sleipnir_ui/src/update_model.rs`: process-wide GPUI update state.
- Modify `crates/sleipnir_ui/src/sleipnir_ui.rs`: module exports/init surface.
- Modify `crates/sleipnir_ui/src/app_shell.rs`: UI observes model; remove local updater fields; outcome dialogs.
- Modify `crates/sleipnir/src/main.rs`: recovery/outcome startup and post-window health report.
- Modify `crates/sleipnir_ui/Cargo.toml` and `crates/sleipnir/Cargo.toml` as required.

### Packaging and documentation

- Modify `scripts/make-app.sh`: build/package/validate helper and manifest for local artifacts.
- Modify `.github/workflows/build-and-release.yml`: package helper, generate/sign/verify/upload manifest.
- Modify `scripts/publish-release.sh`: require signed manifest assets when used.
- Modify `README.md`, `README.zh.md`, and `CHANGELOG.md`: describe transactional behavior and manual fallback.

---

### Task 1: Transaction schema and durable persistence

**Files:**
- Create: `crates/updater/src/lib.rs`
- Create: `crates/updater/src/transaction.rs`
- Modify: `crates/updater/Cargo.toml`
- Modify: `Cargo.toml`

- [ ] Write tests for every allowed state transition, rejection of illegal transitions, stable error-code serialization, 256-bit nonce validation, atomic write/read, and corrupt/unknown schema handling.
- [ ] Run `cargo test -p updater transaction::tests -- --nocapture`; expect compile/test failure because the transaction module does not exist.
- [ ] Implement `Transaction`, `Phase`, `UpdateErrorCode`, `HealthMarker`, pure transition validation, strict path fields, and temp-write/fsync/rename persistence.
- [ ] Re-run the focused tests; expect all pass.
- [ ] Run `cargo test -p updater`; expect all updater tests pass.
- [ ] Commit: `feat(updater): add durable update transactions`.

### Task 2: Signed manifest and release discovery

**Files:**
- Create: `crates/updater/src/manifest.rs`
- Create: `crates/updater/src/release.rs`
- Modify: `crates/updater/src/lib.rs`
- Modify: `crates/updater/Cargo.toml`
- Remove/migrate tests from: `crates/updater/src/updater.rs`

- [ ] Add tests with a deterministic Ed25519 fixture for valid signature, one-byte mutation, malformed signature, unsupported schema, fixed artifact naming, same-version rejection, and downgrade rejection.
- [ ] Run `cargo test -p updater manifest::tests release::tests`; expect failure for missing modules.
- [ ] Implement `SignedManifest`, embedded public-key selection, exact-byte verification, fixed-repository URLs, version policy, and release query parsing.
- [ ] Preserve existing tag/version and platform-asset tests under the new modules.
- [ ] Run focused tests and `cargo test -p updater`; expect pass.
- [ ] Commit: `feat(updater): verify signed release manifests`.

### Task 3: Streaming download and macOS preparation

**Files:**
- Create: `crates/updater/src/download.rs`
- Create: `crates/updater/src/prepare.rs`
- Create: `crates/updater/src/platform/mod.rs`
- Create: `crates/updater/src/platform/macos.rs`
- Modify: `crates/updater/src/lib.rs`

- [ ] Add tests around a local reader adapter for exact size/hash, truncation, oversize, digest mismatch, `.part` cleanup, and verified artifact retention.
- [ ] Add pure preflight tests for bundle ID/version, symlink escape, helper presence, same-volume/capability decisions, and manual-fallback classification.
- [ ] Run focused tests; expect failure because implementations are missing.
- [ ] Implement streaming writes with incremental SHA-256 and atomic `.part` promotion.
- [ ] Implement macOS DMG mount/copy/detach, plist identity checks, executable/helper checks, canonical-path checks, `codesign --verify --deep --strict`, adjacent staging, and runtime swap-capability probe.
- [ ] Keep non-macOS preparation returning release-page behavior, and make non-writable or unsupported macOS installations retain the verified DMG for manual installation.
- [ ] Run `cargo test -p updater` and `cargo clippy -p updater --all-targets -- -D warnings`; expect pass.
- [ ] Commit: `feat(updater): prepare verified macOS candidates`.

### Task 4: Recovery policy and outcomes

**Files:**
- Create: `crates/updater/src/recovery.rs`
- Modify: `crates/updater/src/transaction.rs`
- Modify: `crates/updater/src/lib.rs`

- [ ] Encode the spec recovery table as table-driven tests covering pre-swap interruption, both bundle versions after swap, committed cleanup, rolling-back continuation, unknown schema, and inconsistent evidence.
- [ ] Run `cargo test -p updater recovery::tests`; expect failure for missing recovery module.
- [ ] Implement pure `RecoveryEvidence -> RecoveryAction`, final `UpdateOutcome`, active/history discovery, acknowledgement, and five-summary retention.
- [ ] Run focused and full updater tests; expect pass.
- [ ] Commit: `feat(updater): add deterministic update recovery`.

### Task 5: Supervisor core with fault injection

**Files:**
- Create: `crates/update_helper/Cargo.toml`
- Create: `crates/update_helper/src/main.rs`
- Create: `crates/update_helper/src/supervisor.rs`
- Create: `crates/update_helper/src/log.rs`
- Create: `crates/update_helper/tests/supervisor.rs`
- Modify: `Cargo.toml`

- [ ] Define test adapters for filesystem, process watcher, launcher, and clock; write failing happy-path test proving wait → swap → launch → health → 5-second stabilize → commit.
- [ ] Run `cargo test -p sleipnir-update-helper --test supervisor`; expect failure.
- [ ] Implement the smallest supervisor state loop to pass happy path.
- [ ] Add and observe failing tests one at a time for the 60-second old-exit timeout, swap failure, launch failure, early exit, invalid marker then valid marker, the 60-second health timeout, candidate termination failure, rollback success/failure, state-write failure, and lock contention.
- [ ] Implement each behavior minimally before moving to the next test.
- [ ] Add structured logger tests proving nonce/environment/terminal data are absent.
- [ ] Run helper tests; expect all pass.
- [ ] Commit: `feat(updater): add transactional update supervisor`.

### Task 6: Production macOS helper adapters

**Files:**
- Create: `crates/update_helper/src/swap.rs`
- Create: `crates/update_helper/src/process.rs`
- Create: `crates/update_helper/src/launch.rs`
- Modify: `crates/update_helper/src/main.rs`
- Modify: `crates/update_helper/Cargo.toml`

- [ ] Add macOS-only runtime tests using temporary APFS paths for swap in both directions and a child fixture for kqueue `NOTE_EXIT`.
- [ ] Run the focused tests; expect failure for missing adapters.
- [ ] Implement `renameatx_np(RENAME_SWAP)` with preflight/capability errors and no weak fallback.
- [ ] Implement kqueue registration before readiness, bounded old/candidate watches, and bounded termination.
- [ ] Add a fixture app and failing test that launches via `NSWorkspaceOpenConfiguration` and returns `NSRunningApplication.processIdentifier`.
- [ ] Implement the objc2 AppKit launcher with an appropriate run loop and `createsNewApplicationInstance`/activation configuration.
- [ ] Wire strict `supervise --transaction <absolute path>` parsing, ownership/path checks, detached logging, and production adapters.
- [ ] Run `cargo test -p sleipnir-update-helper` and `cargo clippy -p sleipnir-update-helper --all-targets -- -D warnings`; expect pass on macOS.
- [ ] Commit: `feat(updater): supervise macOS app replacement`.

### Task 7: Updater orchestration interface

**Files:**
- Modify: `crates/updater/src/lib.rs`
- Modify: `crates/updater/src/prepare.rs`
- Modify: `crates/updater/src/transaction.rs`
- Remove: `crates/updater/src/updater.rs`

- [ ] Add failing interface tests for `download_and_prepare`, `begin_install`, helper readiness timeout, `pending_outcome`, acknowledgement, and `report_healthy_if_candidate` race handling.
- [ ] Implement application-support paths, active transaction lock, helper copy/spawn/readiness, 10-second candidate transaction wait, atomic health marker, and exported result types.
- [ ] Ensure `begin_install` reports only `SafeToQuit`, never installed success.
- [ ] Run `cargo test -p updater`; expect pass.
- [ ] Run `cargo check -p sleipnir_ui -p sleipnir`; use compile failures to identify callers to migrate in Task 8, not compatibility shims.
- [ ] Commit: `refactor(updater): expose transactional install interface`.

### Task 8: Process-wide GPUI model and update UI

**Files:**
- Create: `crates/sleipnir_ui/src/update_model.rs`
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs`
- Modify: `crates/sleipnir_ui/src/app_shell.rs`
- Modify: `crates/sleipnir_ui/Cargo.toml`

- [ ] Add failing pure model tests for check, progress, prepare, durable ready, helper readiness, retryable failure, active-transaction suppression, and error-code actions.
- [ ] Implement a GPUI global wrapping the pure update core and background updater calls.
- [ ] Remove `AppShell.update_state`, `staged_dmg`, and artifact URL ownership; render the global state.
- [ ] Rename **Download & Install** to **Download Update**; add download bytes, preparing steps, terminal-loss confirmation, and **Restarting…**.
- [ ] Add final-outcome UI for success, rollback, manual install, and recovery required, with log/Finder/Releases actions.
- [ ] Run focused UI tests and `cargo test -p sleipnir_ui`; expect pass.
- [ ] Commit: `feat(ui): show transactional update lifecycle`.

### Task 9: Startup recovery and health report

**Files:**
- Modify: `crates/sleipnir_ui/src/app_shell.rs`
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs`
- Modify: `crates/sleipnir/src/main.rs`

- [ ] Add failing tests for window-open success signaling and health reporting only after first window creation.
- [ ] Refactor internal window creation to return a success handle while retaining existing public convenience entry points.
- [ ] Initialize update recovery/outcome state before normal UI actions.
- [ ] After first window success, spawn non-blocking candidate health reporting; do not delay presentation.
- [ ] Preserve normal launch behavior when no active candidate transaction exists.
- [ ] Run `cargo test -p sleipnir_ui -p sleipnir`; expect pass.
- [ ] Commit: `feat(updater): confirm healthy candidate startup`.

### Task 10: Package the helper

**Files:**
- Modify: `scripts/make-app.sh`
- Modify: `.github/workflows/build-and-release.yml`
- Modify: `scripts/publish-release.sh`

- [ ] Add shell/static tests or workflow assertions that fail when the helper is absent, non-executable, wrong architecture, or outside bundle signature coverage.
- [ ] Run those checks against the current packaging; expect failure because helper is not packaged.
- [ ] Build helper beside Sleipnir, copy both into `Contents/MacOS`, then sign the complete bundle.
- [ ] Generate canonical manifest after final DMG mutation, sign with the release Ed25519 secret, verify with the public key, and upload DMG/sidecar/manifest/signature as one required set.
- [ ] Keep local builds usable without the private key by generating unsigned local metadata that cannot pass production auto-update verification; publishing scripts must refuse it.
- [ ] Run `shellcheck scripts/make-app.sh scripts/publish-release.sh` and workflow/package assertions; expect pass.
- [ ] Build a local DMG and verify helper mode, architecture, bundle signature, and manifest hash fields.
- [ ] Commit: `build: package transactional update helper`.

### Task 11: End-to-end macOS qualification harness

**Files:**
- Create: `scripts/test-macos-update.sh`
- Create: `crates/update_helper/tests/fixtures/` as required
- Modify: `.github/workflows/build-and-release.yml`

- [ ] Write a harness that installs version N-1 and a fixture N under a writable temporary app parent, then drives the helper transaction.
- [ ] First run should fail because the harness scenarios are unsupported/incomplete.
- [ ] Cover successful commit, intentional candidate crash rollback, health timeout rollback, pre/post-swap helper termination recovery, non-writable fallback, and concurrent transaction rejection.
- [ ] Assert installed version, retained rollback bundle, final transaction phase, process launch, and log error code for each scenario.
- [ ] Run `scripts/test-macos-update.sh`; expect all scenarios pass.
- [ ] Commit: `test(updater): qualify transactional macOS updates`.

### Task 12: Documentation, compatibility, and full verification

**Files:**
- Modify: `README.md`
- Modify: `README.zh.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/superpowers/specs/2026-08-25-transactional-auto-update-design.md` only if implementation-confirmed details differ without changing approved behavior.

- [ ] Document signed manifests, no-elevation fallback, health confirmation, rollback, retained logs, and bootstrap/qualification release procedure.
- [ ] Verify Windows/Linux tests still enforce release-page behavior.
- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test -p updater -p sleipnir-update-helper -p sleipnir_ui -p sleipnir`.
- [ ] Run `cargo clippy -p updater -p sleipnir-update-helper -p sleipnir_ui -p sleipnir --all-targets -- -D warnings`.
- [ ] Run `scripts/test-macos-update.sh`.
- [ ] Run `./scripts/make-app.sh --release` and validate the produced DMG.
- [ ] Run `git diff --check` and inspect `git status --short`.
- [ ] Commit: `docs: document transactional auto updates`.

## Stop conditions

Stop and ask the user instead of guessing if:

- the Ed25519 release private key/secret naming requires an external repository decision;
- the current objc2 versions cannot safely expose the required NSWorkspace API without adding a second incompatible binding stack;
- GPUI quit hooks prevent the helper-ready ordering or first-window health point from being observed;
- runtime filesystem behavior contradicts `RENAME_SWAP` assumptions;
- implementation would require administrator elevation;
- more than three attempts fail for the same runtime behavior.

## Completion evidence

Do not claim completion without fresh output showing:

- all four Rust package test suites pass;
- clippy and rustfmt pass;
- macOS qualification harness passes each named scenario;
- local DMG contains both executables and verifies its bundle signature;
- a candidate fixture crash restores and launches the prior app;
- git diff contains no legacy shell updater production path;
- Windows/Linux still open Releases instead of entering replacement.
