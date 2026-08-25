# Transactional macOS Auto-Update Design

**Status:** Approved in conversation

**Goal:** Replace Sleipnir's detached shell updater with a bundled Rust supervisor that performs a recoverable, observable macOS app-bundle transaction, verifies the new process reaches a healthy first window, and automatically restores the previous version when installation or startup fails.

## Context

The current updater in `crates/updater/src/updater.rs` checks GitHub Releases, downloads a macOS DMG and its SHA-256 sidecar, verifies the digest, mounts the image, copies out `Sleipnir.app`, writes a shell script, starts that script, and immediately asks GPUI to quit. The script waits for the old PID with `kill -0`, renames the installed application to `.bak`, moves the candidate into place, runs `open`, and suppresses nearly every command error.

That implementation has four structural problems:

1. `install_and_relaunch` returns after spawning the shell, so the caller cannot distinguish a completed update from a helper that merely started.
2. The shell emits no durable state or diagnostic log and does not verify the relaunched application initialized successfully.
3. Its 60-second old-process timeout falls through into replacement even if the process remains alive.
4. The published digest and artifact share the same GitHub Release trust domain, while ad-hoc code signing proves bundle consistency but not publisher identity.

A local update from 0.3.0 to 0.3.1 completed successfully, but only filesystem residue, bundle hashes, and system logs made that outcome reconstructable. The new design makes the result explicit and recoverable by construction.

## Decisions

- Bundle an independent Rust binary named `sleipnir-update-helper` inside `Sleipnir.app/Contents/MacOS`.
- Copy that helper outside the application bundle before replacing the bundle, then run it as the transaction supervisor.
- Do not request administrator privileges. If the installation directory is not writable, retain the verified DMG and offer manual installation.
- Require the candidate application to publish a valid health confirmation within 60 seconds and remain alive for a further 5-second stabilization period.
- Automatically swap the previous version back and relaunch it when the candidate cannot launch, exits early, or fails health confirmation.
- Use same-volume adjacent staging and `renameatx_np(..., RENAME_SWAP)` rather than a three-step move. If swap renaming is unavailable, use manual installation rather than a non-atomic fallback.
- Use a signed Ed25519 update manifest as the auto-updater trust root. Keep digest sidecars for manual verification, not as the updater's trust root.
- Keep in-place updates macOS-only. Windows and Linux continue opening GitHub Releases.
- Treat uncertainty conservatively: never delete or overwrite a bundle unless transaction state and filesystem evidence agree.

## Scope

### In scope

- A new Rust update-supervisor binary and its packaging in the macOS app bundle.
- Signed manifest generation, publication, parsing, signature verification, and artifact verification.
- Streaming DMG download, extraction, bundle identity/version/signature checks, and prepared-update persistence.
- Durable transaction state, structured logs, locking, health confirmation, atomic swap, rollback, and interrupted-transaction recovery.
- Process-wide update state and update-result UI.
- Unit, integration, macOS runtime, packaging, and release-upgrade tests.
- Bootstrap migration from the existing shell updater.

### Out of scope

- Privileged helpers, authorization dialogs, or automatic writes to non-writable installations.
- In-place updates on Windows or Linux.
- Background update checks or automatic download on application launch.
- Full runtime crash monitoring after the candidate has committed.
- Resumable HTTP range downloads in the first implementation.
- Delta patches.
- Replacing GitHub Releases as the artifact host.
- Guaranteeing rollback against a malicious candidate that already executes with the user's privileges.

## Architecture

The updater becomes a deep module with a small interface. UI code requests checks, preparation, installation, health reporting, and final outcomes; it does not manipulate artifact URLs, staging paths, or transaction JSON.

```text
┌──────────────────── Sleipnir ─────────────────────┐
│ check → download → verify → extract → preflight   │
│ create transaction → copy helper → request quit   │
└────────────────────────┬──────────────────────────┘
                         │ durable transaction
                         ▼
┌──────────── sleipnir-update-helper ───────────────┐
│ lock → wait old exit → atomic swap → launch       │
│ → await health 60s → observe 5s → commit          │
│                         └ failure → swap back     │
└────────────────────────┬──────────────────────────┘
                         │ final outcome + log
                         ▼
┌──────────────── next Sleipnir launch ─────────────┐
│ report success / rollback / manual action         │
└───────────────────────────────────────────────────┘
```

### Crate layout

The existing updater library is split by responsibility:

```text
crates/updater/src/
├── lib.rs
├── release.rs
├── manifest.rs
├── download.rs
├── prepare.rs
├── transaction.rs
├── recovery.rs
└── platform/
    └── macos.rs
```

A new workspace member contains the supervisor:

```text
crates/update_helper/
├── Cargo.toml
└── src/
    ├── main.rs
    ├── supervisor.rs
    ├── swap.rs
    ├── launch.rs
    └── log.rs
```

The helper depends on neither GPUI nor networking. Internally it has four test seams with production and fake adapters:

- `FileSystem`: durable writes, metadata, paths, locking, and swaps.
- `ProcessWatcher`: old-process and candidate-process exit observation.
- `AppLauncher`: LaunchServices launch and returned process identity.
- `Clock`: 60-second health deadline and 5-second stabilization period.

These are internal seams, not part of the updater library's external interface.

## Public updater interface

The exact Rust ownership details may follow existing executor patterns, but the semantic interface is fixed:

```text
check(current_version) -> CheckResult
download_and_prepare(release, progress_callback) -> PreparedUpdate
begin_install(prepared_update, current_pid) -> InstallLaunchResult
pending_outcome() -> Option<UpdateOutcome>
acknowledge_outcome(transaction_id)
report_healthy_if_candidate(current_version, current_executable) -> HealthReportResult
```

Interface guarantees:

- `check` returns only manifest-backed releases whose signatures and repository-derived asset identities are valid.
- `download_and_prepare` returns success only after artifact size/hash checks and candidate bundle preflight pass.
- `begin_install` returns success only after transaction state is durable, the copied helper is running, and the helper has published `supervisor_ready`. It means "safe to quit," not "installed."
- `pending_outcome` returns only durable final outcomes written by the supervisor or deterministic startup recovery.
- `report_healthy_if_candidate` is a no-op for ordinary launches and cannot acknowledge a transaction with a mismatched ID, nonce, process, path, or version.
- UI callers never read or write `transaction.json` directly.

## Storage layout

Durable control data lives under the macOS application-support directory:

```text
~/Library/Application Support/Sleipnir/updates/
├── active.json
├── lock
├── history/
│   └── <transaction-id>.json
└── <transaction-id>/
    ├── transaction.json
    ├── update-helper
    ├── artifact.dmg
    ├── health-ready.json
    └── update.log
```

The implementation must use the macOS application-support directory specifically; it must not reuse `sleipnir_settings::config_dir()`, because current configuration and session files intentionally use the platform configuration directory.

Application payload staging is adjacent to the installed app so both paths are on the same volume:

```text
<install-parent>/
├── Sleipnir.app
└── .sleipnir-update-<transaction-id>/
    └── candidate.app
```

After the first successful swap, `candidate.app` holds the complete previous version and serves as the rollback bundle. The DMG is retained until the transaction reaches a final acknowledged outcome. Final summaries are moved to `history/`; payloads and copied helpers are cleaned only after final state is durable. Keep at most five acknowledged summaries.

The staging directory name is derived from a validated UUID, created with mode `0700`, and verified not to be a symlink. On macOS `/Applications` commonly permits user-owned applications but not arbitrary new sibling directories for every installation; inability to create adjacent staging is therefore a supported `ManualInstallRequired` result, not a reason to elevate.

## Transaction schema

`transaction.json` is a versioned record containing at least:

```text
schema_version
transaction_id
nonce
phase
old_version
new_version
old_pid
helper_pid
candidate_pid
installed_bundle_path
adjacent_staging_path
artifact_path
created_at
updated_at
error_code
os_error
```

The nonce is 256 random bits encoded in a canonical textual form. It prevents accidental or cross-transaction health confirmation, not attacks from a process already running as the same user.

Every mutation is written to a new file, flushed, atomically renamed over `transaction.json`, and followed by a directory sync where the platform supports it. The implementation must never truncate the only copy in place. State transitions are validated in one pure transition function; helpers and recovery code cannot assign arbitrary phases.

## State machine

```text
Downloaded
   │ manifest, artifact, bundle, version, signature, path preflight
   ▼
Prepared
   │ supervisor copied, started, and ready
   ▼
WaitingForOldExit
   │ old process exited
   ▼
Swapping
   │ RENAME_SWAP(candidate, installed)
   ▼
LaunchingCandidate
   │ NSWorkspace returns NSRunningApplication + PID
   ▼
AwaitingHealth ───────────────────────────────┐
   │ valid marker + 5-second stable lifetime │ launch/exit/health failure
   ▼                                         ▼
Committed                               RollingBack
                                              │ RENAME_SWAP again
                                              ▼
                                         RolledBack
```

Additional terminal states:

- `Cancelled`: cancellation before swap.
- `ManualInstallRequired`: automatic installation preconditions are absent.
- `RecoveryRequired`: neither completion nor rollback can be proven safe.

Cancellation is forbidden after entering `Swapping`. The helper holds an exclusive user-level transaction lock from readiness through final state. Only one `active.json` may identify a non-final transaction.

## Preparation flow

### Signed manifest

Each release publishes:

```text
sleipnir-update-v1.json
sleipnir-update-v1.json.sig
```

The manifest contains:

```json
{
  "schema_version": 1,
  "version": "0.3.2",
  "tag": "v0.3.2",
  "artifact": "Sleipnir-0.3.2-macos.dmg",
  "size": 7012345,
  "sha256": "<64 lowercase hex>",
  "bundle_id": "com.maidang1.sleipnir",
  "minimum_macos": "14.0",
  "minimum_updater_schema": 1
}
```

The release workflow signs the exact manifest bytes with Ed25519. The public key is compiled into the updater; the private key is stored only as a release secret. Verification occurs before trusting any manifest field. Download URLs are constructed from the fixed `Maidang1/sleipnir` repository, the validated tag, and the validated artifact filename; the manifest cannot provide an arbitrary URL.

The updater rejects downgrade, same-version replacement, unsupported manifest schemas, target versions below the current version, and manifests requiring a newer updater schema. `.sha256` sidecars remain published for people and installation scripts but are no longer the updater's trust root.

### Streaming artifact verification

The DMG is streamed to `artifact.dmg.part` while SHA-256 and byte count are updated incrementally. It is accepted only when:

- the request uses HTTPS and the fixed repository-derived URL;
- the response does not exceed the manifest size or global artifact ceiling;
- the final byte count exactly equals `manifest.size`;
- the digest equals `manifest.sha256`.

Only then is `.part` atomically renamed to `artifact.dmg`. A failed partial download is removed. A verified DMG and prepared candidate are retained for retry so a restart does not require another download. HTTP range resume is explicitly deferred.

### Bundle preflight

Mount the DMG read-only and require exactly one top-level `.app`. Copy it to adjacent staging with metadata and code signatures preserved, then verify:

1. `CFBundleIdentifier` is `com.maidang1.sleipnir`.
2. `CFBundleShortVersionString` and `CFBundleVersion` match the manifest target.
3. `Contents/MacOS/sleipnir` exists, is a regular executable, and remains inside the bundle after canonical path validation.
4. `Contents/MacOS/sleipnir-update-helper` exists, is a regular executable, and remains inside the bundle.
5. `codesign --verify --deep --strict` succeeds.
6. Critical bundle paths contain no symlink that escapes the bundle.
7. The install parent and candidate support same-volume swap renaming.
8. The target install path is not a symlink and matches the currently running app bundle.
9. The current user can create and mutate the adjacent staging directory without elevation.

The helper copied into the durable transaction directory comes from the already-preflighted candidate bundle, not from an unverified download path.

## Supervisor protocol

The copied helper accepts one command:

```text
sleipnir-update-helper supervise \
  --transaction <absolute-path-to-transaction.json>
```

Paths, versions, IDs, and nonce are loaded from the protected transaction record rather than appearing in the process list. The transaction path must be absolute, underneath the expected application-support updates directory, non-symlinked, and owned by the current user.

The launcher redirects helper stdout and stderr to `update.log` and starts it in an independent process group. The helper acquires the transaction lock, validates state and filesystem evidence, and registers the old process's `kqueue` exit watch while the old process is guaranteed to still be alive. Only after that watch is active does it atomically persist its PID and `supervisor_ready`. The main app polls the durable readiness field with a bounded startup timeout; only a ready helper allows `cx.quit()`.

This ordering prevents PID reuse: the main process does not begin quitting until the kernel watch is attached to that specific live process. If readiness is not observed, the app stays open, reports `HelperStartFailed`, and leaves the prepared update retryable.

## Process exit and candidate launch

### Old-process exit

The supervisor observes the old PID using `kqueue` with `EVFILT_PROC | NOTE_EXIT`, with a 60-second deadline. This avoids periodic `kill -0` polling and receives a kernel process-exit event. The helper must register the watch before the main process quits; if registration proves the process already exited, it continues. If registration fails ambiguously or the deadline expires, it records `OldProcessExitTimeout` and ends without swapping.

The local macOS SDK exposes `EVFILT_PROC` and `NOTE_EXIT`. PID reuse risk is eliminated from this handoff by treating `supervisor_ready` as proof that the watch was registered while the main process was still waiting and alive; the helper refuses to proceed when that watch cannot be established deterministically.

### Atomic swap

The supervisor uses `renameatx_np` with `RENAME_SWAP` to exchange the installed bundle and adjacent candidate. The local macOS SDK exposes `RENAME_SWAP`, `renameatx_np`, and a volume capability indicating swap-renaming support. Both bundles remain present, and rollback is the same operation in reverse.

No three-step rename fallback is permitted. Unsupported filesystems, cross-volume paths, permission denial, or capability mismatch become `ManualInstallRequired` before the old app quits where possible; a swap-time failure preserves both paths and becomes a structured failure.

### Candidate launch

The helper launches the application bundle through `NSWorkspace.openApplication(at:configuration:completionHandler:)`, configured to create and activate the candidate instance. The completion handler returns an `NSRunningApplication` on success; its process identifier is persisted as `candidate_pid`. The local macOS SDK declares this API from macOS 10.15, below Sleipnir's macOS 14 minimum.

The helper must run the AppKit call on a thread/run-loop arrangement supported by the Objective-C framework bindings. The Rust implementation should reuse the already-resolved `objc2`, `objc2-app-kit`, and `block2` ecosystem versions where practical rather than add a second Objective-C binding stack.

Launch callback failure produces `CandidateLaunchFailed` and immediate rollback. A returned application that is already terminating or whose PID cannot be monitored is not considered launched.

## Health confirmation

An ordinary launch does nothing unless `active.json` identifies a candidate transaction whose installed path and target version match the running executable. Because `NSWorkspace` may start the candidate before its completion handler returns the `NSRunningApplication`, candidate startup and supervisor persistence can race. After the first window succeeds, `report_healthy_if_candidate` therefore runs off the UI thread and waits up to 10 seconds for the active transaction to reach `AwaitingHealth` with a matching persisted `candidate_pid`. That wait is contained within the helper's 60-second health deadline. It never delays first-window presentation. If no matching transaction appears, reporting ends as a normal no-op.

The candidate writes `health-ready.json` only after:

- transaction ID and nonce match;
- its PID matches `candidate_pid`;
- its executable canonicalizes inside the transaction's installed bundle;
- its version equals `new_version`;
- GPUI application initialization completed;
- settings and session completed their initial load path;
- the first `AppShell` window was successfully created;
- no startup path returned a fatal error.

The existing `open_sleipnir_window` returns no success signal, so the implementation must add an internal startup function that returns the created `WindowHandle`/success outcome while retaining the public convenience wrapper used elsewhere. Health reporting belongs in startup orchestration after this success is known, not in `AppShell::construct`, where window creation has not yet been proven.

The marker contains:

```json
{
  "schema_version": 1,
  "transaction_id": "<uuid>",
  "nonce": "<256-bit nonce>",
  "version": "0.3.2",
  "pid": 12345,
  "executable": "/Applications/Sleipnir.app/Contents/MacOS/sleipnir",
  "ready_at": "<timestamp>"
}
```

It is written atomically with mode `0600`. The helper validates every field and the marker's parent path without following unexpected symlinks.

The 60-second health deadline starts only after the `NSWorkspace` completion handler returns the candidate, `candidate_pid` is durable, and the transaction reaches `AwaitingHealth`. The helper concurrently watches candidate exit and that deadline:

- launch failure: rollback immediately;
- candidate exit before a valid marker: rollback immediately;
- no valid marker within 60 seconds: terminate candidate, wait for exit, then rollback;
- invalid marker: record the validation error and continue waiting for a valid marker until the deadline;
- valid marker: watch the candidate for another 5 seconds;
- candidate survives the stabilization period: commit.

After `Committed`, later application crashes are normal runtime failures and do not trigger update rollback.

## Commit and rollback

### Commit

After health confirmation and stabilization:

1. write and sync `Committed`;
2. remove the old bundle now held in adjacent staging;
3. remove the empty adjacent staging directory;
4. retain the final transaction summary and log until the user acknowledges the outcome;
5. clean the DMG and copied helper after final state is durable.

Cleanup failure does not reverse a committed update. It is logged as a cleanup warning and retried on the next launch.

### Rollback

Rollback performs:

1. request candidate termination and wait with a bounded deadline;
2. if the candidate cannot be confirmed stopped, do not swap its running bundle; enter `RecoveryRequired` and preserve both bundles;
3. atomically swap installed candidate and adjacent previous version;
4. validate the restored bundle's identifier, old version, executable, and code signature;
5. launch the restored bundle with `NSWorkspace`;
6. persist `RolledBack` with the original failure and rollback launch result;
7. retain the failed candidate and log until outcome acknowledgement, then clean them.

A rollback launch failure still leaves the old bundle installed. It is reported as `RecoveryRequired` with Finder actions and exact retained paths.

## Interrupted transaction recovery

Recovery is deterministic and conservative. On startup, the updater library acquires the transaction lock before interpreting an active transaction. If a live helper owns the transaction, the app does not race it; it waits for or presents the helper's final outcome.

| Durable phase | Filesystem evidence | Recovery action |
|---|---|---|
| `Downloaded` / `Prepared` | installed old app remains | retain as retryable or cancel; do not change install |
| `WaitingForOldExit` | installed old app remains, no live helper | mark interrupted and retain prepared update |
| `Swapping` | installed path and adjacent bundle both exist | identify versions; default to restoring old version unless final commit is durable |
| `LaunchingCandidate` / `AwaitingHealth` | installed new + adjacent old | terminate matching candidate if live, then swap old back |
| `Committed` | installed new | finish cleanup; never roll back |
| `RollingBack` | both bundles exist | identify versions and complete restoration of old version |
| any phase | state and paths disagree | preserve all bundles and enter `RecoveryRequired` |

Recovery uses bundle IDs, versions, canonical paths, and transaction metadata together. A filename alone is never proof of identity. Unknown transaction schema versions are not mutated; the app reports manual recovery instructions.

## UI and lifecycle

### Process-wide state

Update state moves out of each `AppShell` into a GPUI process-wide model so multiple windows observe one transaction. Window overlays render that state but do not own it. Concurrent app processes are serialized by the durable lock.

The UI state is:

```text
Idle
Checking
Available
Downloading { received, total }
Preparing { step }
ReadyToRestart
WaitingForHelper
Failed { phase, error_code, message, retry_action }
```

`ReadyToRestart` is durable and survives an ordinary app restart. An active transaction disables duplicate check/download/install actions.

### User flow

The available-update action is labeled **Download Update**, not **Download & Install**. After preparation:

```text
Ready to update
Sleipnir <version> has been downloaded and verified.
Restarting will close running terminal processes.
```

Buttons:

```text
Later
Restart & Update
```

Restart uses the existing terminal close-confirm semantics before creating the install transaction. Once helper readiness is durable, the dialog shows **Restarting…** and the app quits. If readiness fails, the app remains open and offers retry or manual installation.

### Final outcome UI

On a successful next launch, show a dismissible notice:

```text
Sleipnir was updated to <version>.
```

On rollback, show a high-priority dialog:

```text
Update couldn't be completed

Sleipnir <new> did not finish starting within 60 seconds,
so version <old> was restored.

[View Update Log] [Open Releases] [Close]
```

On non-writable installation:

```text
Automatic update isn't available for this installation

Sleipnir can't update <path> without administrator access.
The verified disk image has been kept for manual installation.

[Open Disk Image] [Show in Finder] [Close]
```

On unrecoverable inconsistency:

```text
Sleipnir couldn't safely complete or roll back the update

No retained application bundle was deleted. Backup and diagnostic paths:
<paths>

[Show Backup] [View Update Log] [Open Releases]
```

Acknowledgement archives the result. At most five acknowledged summaries are retained.

## Error model and logging

Stable error codes drive recovery and UI; strings are explanatory only:

```text
ManifestSignatureInvalid
ManifestSchemaUnsupported
ArtifactSizeMismatch
ArtifactHashMismatch
BundleIdentifierMismatch
BundleVersionMismatch
BundleSignatureInvalid
BundleLayoutInvalid
InstallDirectoryNotWritable
SwapRenameUnsupported
CrossVolumeStagingUnsupported
HelperStartFailed
OldProcessWatchFailed
OldProcessExitTimeout
AtomicSwapFailed
CandidateLaunchFailed
CandidateExitedEarly
HealthConfirmationTimeout
HealthConfirmationInvalid
CandidateTerminationFailed
RollbackFailed
RecoveryStateInconsistent
```

Each structured log event includes:

```text
timestamp, transaction_id, phase, event,
old_version, new_version,
old_pid, helper_pid, candidate_pid,
relevant path, error_code, os_error
```

Logs never contain terminal output, command history, environment variables, credentials, or the nonce. Log writes are best-effort after transaction state safety; inability to append a log cannot justify deleting or overwriting bundles.

## Filesystem and process safety

- Control directories use mode `0700`; control files use `0600`; executables retain the minimum executable mode.
- Paths are absolute and canonicalized at trust transitions.
- Transaction and staging paths reject unexpected symlinks and use no-follow/create-new behavior where supported.
- The installed app path must match the running executable's enclosing bundle at transaction creation.
- Transaction IDs are validated UUIDs; paths are derived internally rather than concatenated from arbitrary input.
- State only advances through the transition function.
- A user-level exclusive lock prevents two windows or app processes from installing concurrently.
- Helper readiness is durable before main-process quit.
- The supervisor never swaps while the old PID may still be alive.
- The supervisor never swaps back while the candidate PID may still be alive.
- Unsupported swap semantics trigger manual installation, never a weaker replacement sequence.

## Release packaging and trust

`scripts/make-app.sh` and `.github/workflows/build-and-release.yml` build `sleipnir-update-helper` for the same architecture and copy it to:

```text
Sleipnir.app/Contents/MacOS/sleipnir-update-helper
```

The complete bundle is code signed after both executables are present. Packaging validation checks executable mode, architecture, and bundle signature. Universal application builds, if added later, must include a matching universal helper.

The release workflow:

1. builds and signs the complete app;
2. creates/notarizes/staples the DMG where credentials permit;
3. computes final size and SHA-256 after all DMG mutations;
4. writes the canonical v1 manifest;
5. signs the exact manifest bytes with the Ed25519 release key;
6. verifies the signature with the public key as a CI step;
7. uploads DMG, digest sidecar, manifest, and signature together;
8. fails the release if any required asset is absent.

The manifest private key is the only new credential. It must be documented as a release secret with a rotation procedure. Public-key rotation requires an application release that trusts both old and new keys before manifests are signed only by the new key.

## Bootstrap and compatibility

The first release containing the helper is the bootstrap release, expected to be 0.3.2 unless the project version advances first.

- Updating from 0.3.1 to the bootstrap release still uses the legacy shell updater embedded in 0.3.1.
- The bootstrap release packages the Rust helper and understands transaction schema v1.
- Its startup verifies that the packaged helper exists and records a diagnostic if packaging is incomplete; it does not block normal app use.
- The next release is the qualification release and exercises the new path end to end.
- Legacy shell generation is removed from source only after the bootstrap package is published and verified.

Transaction records carry schema versions. A helper refuses unsupported schemas. A newer app may display a known older final outcome but must not mutate unknown active schemas. Windows and Linux never invoke the transaction supervisor.

## Testing strategy

Implementation follows test-driven development for pure policy, transaction transitions, recovery decisions, and helper behavior.

### Unit tests

- valid, invalid, truncated, and field-mutated manifest signatures;
- canonical manifest parsing and unsupported schemas;
- upgrade comparison, same-version rejection, and downgrade rejection;
- every legal and illegal transaction transition;
- atomic transaction write behavior under injected write/rename failures;
- health marker transaction ID, nonce, PID, path, and version validation;
- filesystem evidence to recovery-action decision table;
- canonical path, symlink, permission, and same-volume policies;
- error-code to user-action mapping;
- process-wide UI state transitions.

### Helper integration tests

Fake adapters inject failures at each boundary:

- old process exits normally;
- old process exceeds 60 seconds;
- helper is killed before swap;
- helper is killed immediately after swap;
- candidate launch callback fails;
- candidate exits before health;
- health marker is absent;
- marker has wrong transaction ID, nonce, PID, path, or version;
- candidate confirms within 60 seconds and survives 5 seconds;
- candidate cannot be terminated before rollback;
- rollback swap fails;
- state write fails before and after swap;
- logging fails or disk becomes full;
- two supervisors contend for one transaction.

Tests assert not only the final phase but which bundle/version occupies each path and which files remain recoverable.

### macOS runtime tests

Using temporary directories on a local APFS volume:

- probe and execute `RENAME_SWAP` in both directions;
- observe a spawned process with `kqueue NOTE_EXIT`;
- launch a fixture `.app` through `NSWorkspace` and obtain its real PID;
- validate correctly and incorrectly signed/structured fixture bundles;
- receive a valid health confirmation;
- launch a fixture candidate that intentionally exits and verify automatic restoration.

CI does not write `/Applications`. A packaged-app smoke test uses a writable temporary application directory; a separate release checklist covers a normal `/Applications` install.

### Release acceptance

Every macOS release after bootstrap must prove:

```text
N-1 → N succeeds and reports Committed
N-1 → intentionally failing N restores and launches N-1
non-writable install retains verified DMG and offers manual install
helper termination at pre-swap and post-swap checkpoints recovers deterministically
double click / two app instances produce one active transaction
```

The release job also verifies that the DMG's embedded helper exists, is executable, has the expected architecture, and is covered by bundle code signing.

## Delivery plan

This design touches more than eight files and introduces one workspace crate. It should ship as one feature branch because partial production wiring would create two competing install paths, but implementation should use reviewable commits:

1. transaction schema, manifest verification, and pure recovery policy;
2. helper with fake-adapter integration tests;
3. macOS adapters and runtime fixtures;
4. updater-library preparation and process-wide UI integration;
5. startup health reporting and outcome UI;
6. packaging, release signing, documentation, and acceptance scripts;
7. legacy shell removal after bootstrap qualification.

Commits 1–3 are inert library/helper additions and keep the app usable. Commit 4 must not switch production installation until commits 5–6 are present and verified; the feature branch is merged only as a complete path. Legacy removal is a later independently mergeable cleanup after field qualification.

## Rollback of this product change

Before the qualification release, reverting the feature branch restores the existing shell updater and does not change user settings or session data. Transaction files are isolated under application support and can be left unread by older versions. After a transaction has swapped bundles, rollback is handled by that transaction's copied helper, not by a future app build. Release engineering must not delete bootstrap artifacts or rotate the manifest key while an update remains supported.

## Acceptance criteria

The redesign is complete when:

- the app bundle contains the Rust helper and packaging validates it;
- signed manifest verification is the only path to automatic installation;
- UI code no longer owns DMG paths or performs bundle replacement;
- the main app quits only after durable helper readiness;
- old-process timeout never proceeds to swap;
- swap and rollback use supported same-volume `RENAME_SWAP` semantics;
- candidate launch returns a monitored PID through LaunchServices;
- a valid first-window health marker is required within 60 seconds;
- the candidate survives a 5-second stabilization period before commit;
- launch failure, early exit, and health timeout restore and relaunch the previous version;
- interrupted states recover according to the documented decision table;
- non-writable and unsupported filesystems retain the verified DMG and offer manual installation without elevation;
- final outcomes and structured logs are visible on the next launch;
- unit, helper integration, macOS runtime, packaging, and release-upgrade checks pass;
- Windows and Linux update behavior is unchanged.

## Fragile assumption

The design assumes the installed bundle's filesystem supports swap renaming. The macOS SDK exposes a capability for this and Sleipnir requires macOS 14, but external or unusual filesystems may not support it. The runtime probe is authoritative. Failure degrades to verified manual installation; it never silently selects a non-atomic algorithm.
