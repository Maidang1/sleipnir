# Sleipnir

A standalone terminal emulator built on GPUI (macOS only). This
glossary fixes the language for the window/tab/pane model and its interaction surface.

## Window & layout

**Window**:
One OS window (AppKit `NSWindow`).
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
The custom chrome of a Window: traffic-light clearance, the Tab rail or top tab
strip, and any window controls. It is deferential to terminal content per HIG.
_Avoid_: Titlebar, header, toolbar.

**Tab rail**:
The left-side list of Tabs, clustered under Workspace headers. Default chrome
(`tab_placement: side`). The historical top strip remains as `tab_placement: top`.
_Avoid_: Sidebar (the rail is the tab list, not a second panel), activity bar.

**Workspace**:
A grouping key for Tabs, derived from the git work tree of the Tab's active Pane
cwd (or the cwd itself if there is no `.git`, or `~` if cwd is unknown). Not a
stored object; `cd` into another repo moves the Tab. Not an OS Window. A rail
row is two lines: the tab title (ellipsis) and, in a work tree, a branch
subtitle with a `+N` / `−M` count of lines inserted / deleted vs `HEAD`
(`git diff --numstat`). Long titles and branch names truncate with `…`.
Untracked files are omitted. A non-repo pane shows no subtitle. The
workspace header includes the tab count.
_Avoid_: Project, folder, space.

**Agent identity**:
A known coding-agent process detected from the Pane's foreground command name.
Rendered as a letter monogram Sleipnir owns. Distinct from a Run badge (who vs
what happened).
_Avoid_: Agent (the process is workload; the user is the person), logo.

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
and notifications. Tab chrome only renders a Failed badge; Running and
Succeeded stay in the Ledger. It does not survive a restart (loaded history
is marked seen).
_Avoid_: Unread, Badge (a badge is one rendering of Attention), Alert.
