# Crate topology: standard layout, dependency direction, single-sourced ids

**Status:** accepted (follows up [ADR-0015](0015-out-of-process-plugin-rpc.md)
and [ADR-0016](0016-plugin-protocol-v2-and-trust.md); records the findings of
the 2026-09 workspace-wide crate audit)

## Context

The workspace grew to 23 crates without an explicit rule for how a crate is
laid out or which way dependencies may point. An audit found four concrete
drifts, all small, all pointing the same way:

1. **Non-standard lib paths.** 21 of 23 crates declared
   `[lib] path = "src/<crate_name>.rs"` instead of the cargo-default
   `src/lib.rs`. The habit is inherited from the Zed monorepo we vendor GPUI
   from. It buys nothing for us: it surprises every Rust tool and contributor
   that assumes the default, and it already produced real breakage — tests
   that `include_str!` or `read_to_string` their own source file by name are
   landmines on any rename (two such guardrail tests in `sleipnir_ui` read
   `src/sleipnir_ui.rs` and `terminal/src/terminal.rs`; `gpui_platform`'s
   self-check included its own file name). `updater` carried the same drift
   in its worst form: an empty `src/lib.rs` that re-exported
   `#[path = "updater.rs"] mod legacy` — the real crate root hiding behind a
   module literally named `legacy`.
2. **Infrastructure depended on a domain crate.** `plugin_host` (generic
   plugin supervision) depended on `run_ledger` (the "what ran here" domain)
   for exactly one function: `redact_command`. Meanwhile the wire-safety
   invariant that function implements — "plugins never see the raw command
   line" ([ADR-0016](0016-plugin-protocol-v2-and-trust.md) §2) — is a property
   of the *protocol*, and the protocol crate is the one place both callers
   (capture site and wire choke point) can share without a sideways edge.
3. **A declared-but-unused dependency.** `terminal` listed `run_ledger` in
   its manifest but referenced it only in a doc comment; the run ids it
   handles come from `row_geometry`.
4. **Duplicated id aliases.** `RunId` and `PaneKey` were declared as
   `Uuid` aliases independently in `run_ledger` and `plugin_protocol`.
   Harmless while both stay `Uuid` — and a silent fork the day one of them
   becomes a newtype.

None of these broke anything. All of them make the next structural mistake
easier to make.

## Decision

### 1. Standard cargo layout

Every library crate uses the cargo default `src/lib.rs` and carries no
`[lib] path` override. A `[lib]` table is kept only where it still configures
something (today: `doctest = false` in crates whose doc examples are
illustrative rather than runnable). Source-reading guardrail tests must
locate files through `CARGO_MANIFEST_DIR` and the *current* file name — and
are exactly why this convention matters.

### 2. Dependency direction

The workspace layers are:

```
L3  entry      sleipnir (thin binary)
L2  compose    sleipnir_ui
L1  domain     terminal · plugin_host · plugin_grants · run_ledger
               sleipnir_settings · updater · agent_coordination · …
L0  pure leaf  plugin_protocol · atomic_write · diff_core · syntax
               release_channel · row_geometry · sleipnir_ctl · …
```

Dependencies point downward only. An edge between two L1 crates, or from
infrastructure to a specific product domain, is a smell that needs either a
move (the shared code goes down a layer) or an explicit note in the ADR that
covers the crate. `redact_command` is the enforcing example: it moved from
`run_ledger` to `plugin_protocol::redact`, so both the capture site
(`run_ledger`, now `L1 → L0`) and the wire choke point (`plugin_host`, now
depending on `plugin_protocol` only) share one implementation with no
sideways edge. `run_ledger` re-exports it so existing references keep
working.

Declared-but-unused dependencies are removed on sight; a dep whose only
footprint is a doc comment is documentation, not a dependency.

### 3. Ids are single-sourced at the protocol layer

`RunId` / `PaneKey` are defined once, in `plugin_protocol::v2`, and
re-exported by `run_ledger` and `row_geometry`. If an id ever stops being a
plain `Uuid`, exactly one definition changes.

### 4. Manifest hygiene

`license.workspace = true` everywhere; the one deliberate exception is
`gpui_platform` (Apache-2.0), which mirrors upstream GPUI's licensing posture
because it is mostly an OS-branched re-export of that API surface.

### 5. Naming: accepted as-is, with a rule for the future

The mixed prefixes (`sleipnir_ui` / `sleipnir_settings` vs `terminal` /
`run_ledger` / `plugin_host`) are **not** renamed. Rename churn touches
release scripts, installer manifests, and every open branch, and buys only
aesthetics. The rule going forward: user-facing binaries and product-named
surfaces take the `sleipnir_` prefix (or a published crate name like
`sleipnir-plugin`); generic infrastructure does not.

## Consequences

- The audit's findings are fixed: 21 crates moved to `src/lib.rs`
  (`updater.rs` was inlined into its `lib.rs`, retiring the `legacy` shim),
  `plugin_host`'s manifest now names only `plugin_protocol`, `terminal` no
  longer declares `run_ledger`, the id aliases have one definition, and the
  license is workspace-centralized. `cargo check`, `clippy --all-targets`,
  and the full test suite (1044 tests) are green before and after.
- The layering rule is enforced by convention and review, not by tooling.
  A cheap tripwire exists: `cargo metadata` plus a grep over each manifest's
  `workspace = true` deps reproduces the audit in seconds.
- The two stale self-include tests found here are a reminder that
  source-grep guardrails must tolerate renames or pin the layout — they now
  pin the layout, which ADR-0020 §1 makes stable.
