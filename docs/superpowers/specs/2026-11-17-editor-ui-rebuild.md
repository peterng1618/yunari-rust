# Editor UI rebuild

**Status:** approved 2026-11-17. This replaces the parity loop as the primary
workstream until the editor shell exists.

## Why

The port's behaviour layer is real and tested, but its editor surface was built
as a command list: every command rendered as a text label in wrapping rows,
panels assembled from bare `div()`, no kit chrome, no reachable inspector in the
default layout, controls clipped off the right edge. The first real launch was
the first visual check. The parity sweep had marked editor capabilities "Met"
from tests and code reading; the review loop's matrix had no editor rows, so its
coverage metric could not see the surface at all.

Two rules follow from that, and they bind this rebuild:

1. **No surface is done without a both-side screenshot judged on function,
   information architecture, state communication and craft.** A test proves a
   callback; it does not prove a user can find or understand the control.
2. **`gpui-kit` means `gpui-component`, not just the re-exported GPUI.**
   `gpui-component` 0.9.1 ships dock, sidebar, toolbar, title bar, status bar,
   tabs, menu, command palette, table, tree, input, dialog, resizable, theme,
   colour picker and inspector. A hand-rolled interactive `div()` is a defect
   unless it is canvas geometry (handles, guides, overlays).

## Scope

Keep, do not rewrite:

- `crates/model`, `crates/render*`, `crates/host`, `crates/player`,
  `crates/library`, `crates/providers`, `crates/panels`;
- the editor's behaviour modules: `document`, `selection`, `transform`,
  `arrange`, `grouping`, `history`, `layers`, `inspector`, `palette`, `tokens`,
  `bindings`, `persistence`, `clipboard`, `snap`, `assets`, `fixtures`.

Rebuild:

- `crates/editor/src/ui.rs` and `crates/editor/src/ui/*`: shell, navigation,
  editor frame, panel docking, inspector rendering, command surface, dialogs,
  status bar, canvas chrome;
- the theme layer the editor uses for colours, spacing, type and control
  states — from `gpui-component`'s theme, not ad-hoc constants.

Out of scope: changing product behaviour, pixel-matching the web UI, the
extension/plugin platform (dropped for v1), and the display-only player surface
(already under its own verified fixes).

## Reference

`docs/superpowers/specs/editor-ui-reference.md` (in progress) holds the source
screen map: screen → regions → controls → source file → kit widget, plus the
per-screen acceptance checklist. Screenshots live under
`test-results/editor-reference/` and are regenerated, never committed.

The source app is run through its real host
(`node packages/host/bin/yunari.js --no-browser`), driven at 1920x1080. The
vite-preview page is not evidence.

## Shape of the result

- Library / Themes mode, Editor mode, Settings mode and Store mode are distinct
  surfaces with persistent navigation, as in the source — not rows of buttons.
- Editor mode: persistent left pane (layers/assets), canvas centre, right
  inspector, top command bar, status bar. Panes are resizable and collapsible
  through the kit's dock/sidebar, not manual pixel layout.
- Every command in the current `Command::ALL` maps to a labelled control in the
  same group the source uses (menu, toolbar, context menu, shortcut), and the
  keyboard path exists for each.
- Inspector sections render as forms with the kit's input, select, slider,
  switch, colour picker and token rows; empty, disabled, focus and error states
  are distinguishable.
- Dialogs use the kit dialog, not bespoke overlays.

## Acceptance gate (per screen)

A screen passes only when all four hold, with evidence linked in the ledger:

1. **Function** — every source control on that screen exists, is reachable and
   does what the source does; refusals and defaults match.
2. **Information architecture** — same grouping and persistence; primary action
   in the same place; nothing important moved behind an undiscoverable path.
3. **State** — selected, hover, focus, disabled, empty, loading, error are all
   visible and distinguishable; keyboard focus is visible.
4. **Craft** — at 1920x1080 and at the minimum supported window size: no clipped
   label, no overflowing control row, no unreadable contrast, hit targets at
   least 24px, consistent spacing scale and alignment.

Evidence: a 1920x1080 port capture beside the source reference capture, plus a
keyboard walkthrough note for the screen's commands. The verifier regenerates
both; the fixer's screenshots are not enough.

## Phases and exit criteria

**P0 — reference (current).** Screen map, IA summary, component mapping,
divergence list, acceptance checklists. Exit: reference doc PR merged.

**P1 — shell.** Navigation between library/editor/settings/store, editor frame
with docked left canvas right panes, top bar, status bar, theme applied. Exit:
editor opens with the reference layout at 1080p; no command row overflows;
screenshot pair accepted.

**P2 — panels.** Layers tree, inspector sections per node kind, palette, assets,
typography. Exit: each section's acceptance checklist passes; selecting each
kind shows the right inspector.

**P3 — commands and canvas.** Command surface, context menus, shortcuts,
selection/transform overlays, snap guides, zoom/fit. Exit: every current
`Command::ALL` is reachable and keyboard-driven; canvas interactions match the
source's visible feedback.

**P4 — dialogs and modes.** Settings, store, asset finder, widget/path/typography
pickers, library, MCP, save/error states. Exit: each dialog's checklist passes.

**P5 — parity loop resumes.** Probes and the review loop run on top of the new
surface. Editor rows E1–E11 enter the matrix; the loop's first pass is a
regression sweep, not a discovery pass.

## Workflow

Same loop mechanics as the parity review:

- one Orca workspace per lane, launched with `orca agent launch`;
- lanes are screen families with disjoint files (shell, panels, dialogs,
  canvas), each with its reference shots and checklist;
- fixer opens a PR per screen family; verifier regenerates the screenshot pair
  and walks the checklist; orchestrator merges one PR at a time, retires the
  workspace, and records the screen as accepted in
  `docs/parity/ledger/editor.md`;
- a screen that fails the gate twice is parked with `needs:human` and the two
  failure reports, and the lane moves on.

No new parity-bug lanes start until P1 exits. In-flight renderer correctness
fixes (text layout, assets) finish because they are independent of the shell.

## Risks

- **Building chrome that still does not fit the source IA** — the reference map
  is the acceptance target, not the port's current layout.
- **Kit components used as decoration** — the gate checks interaction and
  states, not presence in a diff.
- **Scope creep into behaviour** — the behaviour layer already has tests; a
  behaviour change needs its own issue.
- **Screenshot theatre** — evidence is a pair at the same size and state, judged
  against the checklist; a single flattering shot is rejected.
