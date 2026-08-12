# Terminal Control-Key Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore plain `Ctrl+C` and `Ctrl+V` delivery to the PTY while preserving all documented clipboard shortcuts and adding regression coverage.

**Architecture:** Keep keyboard routing in `sleipnir_ui.rs`, but isolate the decision to reserve a keystroke for a clipboard action in a pure predicate. Test that UI-bound clipboard combinations are reserved while terminal control combinations are not; separately assert the terminal mapper emits the expected control bytes.

**Tech Stack:** Rust 2024, GPUI `Keystroke`, Cargo unit tests, rustfmt.

---

## File Structure

- Modify `crates/sleipnir_ui/src/sleipnir_ui.rs`: add the pure clipboard-routing predicate, use it from `TermView::on_key_down`, and add focused unit tests.
- Modify `crates/terminal/src/mappings/keys.rs`: add explicit control-byte regression assertions to the existing mapper tests if equivalent assertions are absent.

### Task 1: Add failing UI routing tests

**Files:**
- Modify/Test: `crates/sleipnir_ui/src/sleipnir_ui.rs`

- [ ] **Step 1: Add a test-only expectation matrix for the desired predicate**

Add tests equivalent to:

```rust
#[test]
fn plain_control_c_and_v_are_sent_to_the_terminal() {
    assert!(!is_clipboard_shortcut(&Keystroke::parse("ctrl-c").unwrap()));
    assert!(!is_clipboard_shortcut(&Keystroke::parse("ctrl-v").unwrap()));
}

#[test]
fn configured_clipboard_shortcuts_are_reserved() {
    for shortcut in ["cmd-c", "cmd-v", "ctrl-shift-c", "ctrl-shift-v", "ctrl-cmd-v"] {
        assert!(is_clipboard_shortcut(&Keystroke::parse(shortcut).unwrap()), "{shortcut}");
    }
}
```

Also assert unrelated and unbound modified combinations are not reserved.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test -p sleipnir_ui clipboard_shortcut -- --nocapture`

Expected: compilation failure because `is_clipboard_shortcut` does not exist. This proves the tests precede production implementation.

### Task 2: Implement minimal routing correction

**Files:**
- Modify: `crates/sleipnir_ui/src/sleipnir_ui.rs:232-250`

- [ ] **Step 1: Add the minimal pure predicate**

Implement behavior equivalent to:

```rust
fn is_clipboard_shortcut(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    let is_c = keystroke.key.eq_ignore_ascii_case("c");
    let is_v = keystroke.key.eq_ignore_ascii_case("v");

    (modifiers.platform && (is_c || is_v))
        || (modifiers.control && modifiers.shift && !modifiers.platform && (is_c || is_v))
}
```

`Ctrl+Cmd+V` is covered by the Command branch. Plain Control combinations are deliberately excluded.

- [ ] **Step 2: Replace the broad inline condition**

Use `is_clipboard_shortcut(&event.keystroke)` as the early-return condition in `TermView::on_key_down`.

- [ ] **Step 3: Run focused tests and verify GREEN**

Run: `cargo test -p sleipnir_ui clipboard_shortcut -- --nocapture`

Expected: all routing tests pass.

### Task 3: Add explicit terminal byte regression tests

**Files:**
- Modify/Test: `crates/terminal/src/mappings/keys.rs`

- [ ] **Step 1: Add explicit assertions**

Extend `test_ctrl_codes` or add a focused test:

```rust
#[test]
fn plain_control_c_and_v_emit_terminal_control_bytes() {
    assert_eq!(
        to_esc_str(&Keystroke::parse("ctrl-c").unwrap(), Modes::NONE, false),
        Some("\x03".into())
    );
    assert_eq!(
        to_esc_str(&Keystroke::parse("ctrl-v").unwrap(), Modes::NONE, false),
        Some("\x16".into())
    );
}
```

- [ ] **Step 2: Run focused mapper tests**

Run: `cargo test -p terminal plain_control_c_and_v_emit_terminal_control_bytes -- --nocapture`

Expected: pass, documenting the already-correct PTY byte mapping.

### Task 4: Verify and review

**Files:**
- Verify all modified Rust files; do not modify the user's existing `CHANGELOG.md` work.

- [ ] **Step 1: Format check**

Run: `cargo fmt --all -- --check`

Expected: exit 0.

- [ ] **Step 2: Run affected crate suites**

Run: `cargo test -p sleipnir_ui -p terminal`

Expected: exit 0 with zero failed tests.

- [ ] **Step 3: Run workspace tests**

Run: `cargo test --workspace`

Expected: exit 0 with zero failed tests.

- [ ] **Step 4: Confirm the diff is scoped**

Run: `git diff --check && git diff -- crates/sleipnir_ui/src/sleipnir_ui.rs crates/terminal/src/mappings/keys.rs`

Expected: no whitespace errors; only the routing fix and regression tests appear.

- [ ] **Step 5: Report results**

Report the root cause, exact fix, RED/GREEN evidence, generated cases, verification commands, and any validation limitations. Do not commit implementation unless requested.
