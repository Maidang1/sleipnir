# Sleipnir

A standalone terminal emulator built on GPUI (macOS, Windows, and Linux). This
glossary fixes the language for the window/tab/pane model and its interaction surface.

## Window & layout

**Window**:
One OS window (AppKit `NSWindow` on macOS, a GPUI/Win32 window on Windows).
Sleipnir may open several. Each hosts one chrome band and one content area.
_Avoid_: App, screen.

**Tab**:
A labelled page occupying the whole content area below the chrome band. Only one
Tab is visible per Window at a time. A Tab owns exactly one PaneTree.
_Avoid_: Page.

**Pane**:
A rectangular region inside a Tab that holds exactly one terminal session (one
PTY). A Pane is always a leaf of its Tab's PaneTree.
_Avoid_: Split, terminal, view, cell (a cell is one character in the grid).

**PaneTree**:
The recursive binary tree inside a single Tab. Interior nodes are Splits; leaves
are Panes. A Tab with no splits is a PaneTree of a single Pane.
_Avoid_: Layout, grid (grid is the character grid inside one Pane).

**Split**:
An interior node of a PaneTree that divides its area into two children along one
axis. A Split has an orientation and a ratio.
_Avoid_: Pane, divider (the draggable line is the Divider).

**Divider**:
The draggable line between the two children of a Split; dragging it changes the
Split's ratio.

**Active Pane**:
The single Pane that receives keyboard input and shows the focus treatment.
Exactly one Pane is active per Window (it lives in the active Tab).
_Avoid_: Focused terminal, current pane.

## Chrome & appearance

**Chrome**:
The custom title region at the top of a Window: traffic-light clearance, the tab
strip, and any window controls. It is deferential to terminal content per HIG.
_Avoid_: Titlebar, header, toolbar.

**Theme**:
A named color palette applied to terminal content (e.g. the Catppuccin flavors
plus classic terminal palettes). A Theme colors cells; Chrome derives its
surfaces from the Theme.
_Avoid_: Palette (a Palette is the concrete resolved color set of a Theme),
color scheme, skin.

**Appearance**:
Whether the system (and therefore Sleipnir) is in dark or light mode. A value of
`dark` or `light`.
_Avoid_: Mode, dark mode (as a noun).

**Auto Theme**:
A user-selectable theme choice that binds a dark Theme and a light Theme as a
pair and follows the system Appearance, swapping between the two automatically.
Distinct from selecting a single fixed Theme.
_Avoid_: System theme, adaptive theme.

## Run Ledger

**Run**:
One command execution, from OSC 133 `C` (start) to `D` (end, with an exit
code), belonging to exactly one Pane. A Run carries a redacted command line,
cwd, wall-clock start, monotonic duration, exit status, `RunId`, `PaneKey`,
`LaunchId`, an Anchor, and whether it was inferred. The command line is
redacted at capture, so memory and disk hold the same text.
_Avoid_: Task (already taken in this repo by the Zed-derived `SpawnInTerminal`
/ `TaskState`), Command (a command is the text; a Run is one execution),
Block, Job.

**Ledger**:
The ordered collection of every Run across Window / Tab / Pane. There is one
in-process Ledger, persisted to `runs.json`. It is the only source of truth;
UI reads snapshots of it.
_Avoid_: History (shell history is a text history of commands), Log, Timeline.

**Anchor**:
The Run's position in its Pane's scrollback, reused from `Osc133Marker`
line/column and `command_output_range()`, used to jump back to that output.
Valid only for the process lifetime — after a restart the scrollback is gone
and the Anchor is gone with it. Anchors are not written to `runs.json`.
_Avoid_: Mark (a Mark is a raw OSC 133 marker), Bookmark.

**Attention**:
The set of finished Runs the user has not yet seen. "Seen" means the Pane was
focused, or the Run was clicked in the Ledger panel. Attention drives badges
and notifications. It does not survive a restart (loaded history is marked
seen).
_Avoid_: Unread, Badge (a badge is one rendering of Attention), Alert.
