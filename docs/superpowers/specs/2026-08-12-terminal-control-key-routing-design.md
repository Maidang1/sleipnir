# Terminal Control-Key Routing Fix

## Problem

`TermView::on_key_down` currently treats every Control/Command + C/V keystroke as a clipboard shortcut and returns before terminal input mapping. As a result, plain `Ctrl+C` never reaches the PTY as ETX (`0x03`), so foreground programs do not receive `SIGINT`. Plain `Ctrl+V` is similarly swallowed instead of being sent as SYN (`0x16`).

## Design

Extract the clipboard-routing predicate into a small pure function in `sleipnir_ui.rs`. It will intercept only shortcuts that have clipboard actions:

- `Cmd+C` and `Cmd+V`
- `Ctrl+Shift+C` and `Ctrl+Shift+V`
- `Ctrl+Cmd+V`

Plain `Ctrl+C` and `Ctrl+V` must not be intercepted and will continue through `Terminal::try_keystroke` to the existing terminal key mapper.

No PTY, signal, shell, or keymap architecture changes are required.

## Tests

Use test-driven development:

1. Add focused unit tests for the routing predicate that fail under the current broad interception rule.
2. Cover the complete relevant matrix: plain Control, Control+Shift, Command, and Control+Command combinations for C/V, plus an unrelated key.
3. Keep or add terminal mapping assertions proving `Ctrl+C` maps to `\x03` and `Ctrl+V` maps to `\x16`.
4. Run targeted crate tests, formatting checks, and the project test suite.

## Success Criteria

- Plain `Ctrl+C` reaches the PTY and can interrupt foreground programs.
- Plain `Ctrl+V` reaches the PTY.
- Documented copy/paste shortcuts remain intercepted for their GPUI actions.
- Regression tests fail with the old predicate and pass with the corrected predicate.
