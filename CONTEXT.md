# Harbor

A standalone macOS terminal emulator built on GPUI. This glossary fixes the
language for the window/tab/pane model and its interaction surface.

## Window & layout

**Window**:
One macOS `NSWindow`. Harbor may open several. Each hosts one chrome band and
one content area.
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
Whether the system (and therefore Harbor) is in dark or light mode. A value of
`dark` or `light`.
_Avoid_: Mode, dark mode (as a noun).

**Auto Theme**:
A user-selectable theme choice that binds a dark Theme and a light Theme as a
pair and follows the system Appearance, swapping between the two automatically.
Distinct from selecting a single fixed Theme.
_Avoid_: System theme, adaptive theme.
