# Split panes via a recursive PaneTree

**Status:** accepted (supersedes the "no split panes" Non-Goal in
`docs/ui-chrome-hig-redesign.md`)

To match modern terminals (iTerm2, Warp), a Tab may now contain multiple terminal
sessions arranged by splitting. We model each Tab's contents as a recursive
binary **PaneTree**: interior nodes are **Splits** (an axis + a ratio) and leaves
are **Panes** (one PTY each). This replaces the previous flat `Vec<Tab>` model
where a Tab *was* a single `TermView`.

Nesting is arbitrary (no depth cap). Closing a Pane collapses its parent Split and
returns the freed space to the sibling subtree (standard iTerm2/tmux behavior);
closing the last Pane in a Tab closes the Tab.

## Consequences

- Focus, keyboard routing, and layout must operate on the tree, not on a single
  active `TermView` — exactly one **Active Pane** per Window receives input.
- The `docs/ui-chrome-hig-redesign.md` design doc listed split panes as a
  Non-Goal; this decision deliberately overrides that. That doc's chrome work
  (unified title band, connected active tab) still stands — only the "no splits"
  boundary is reversed.
