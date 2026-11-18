# Editor UI ledger

Source: `D:\git-repos\yunari` @ `1241866e53df252307e22ff3c51c842f89e458c8`.
Port: `D:\git-repos\yunari-rust`.

The reference capture is the source host driven at 1920x1080
(`node packages/host/bin/yunari.js --no-browser`). The served bundle names the
first mode **Themes**, which is the pinned commit's vocabulary; the checked-out
HEAD renamed it Library, so the served bundle is pinned-era, not HEAD.

## P1 â€” shell

Evidence run:

- `cargo test -p yunari-editor` â€” 144 passed (2 suites), including
  `shell_modes_are_themes_editor_settings_in_order`,
  `rail_slots_match_the_source_order`, `toolbar_groups_match_the_source_header`,
  `toolbar_groups_do_not_repeat_a_command`.
- `cargo build -p yunari-app` â€” finished.
- `scripts/capture-window.ps1` (DPI-aware) at 1920x1080 and at 1280x875
  physical = 1024x700 logical (the window minimum at 125% scaling).

Captures, beside the source reference:

| State | Source | Port |
|---|---|---|
| 1920x1080, Editor mode | `test-results/parity-shots/editor-source-1080p.png` | `test-results/parity-shots/editor-port-1080p-p1.png` |
| 1024x700 logical (minimum) | not captured | `test-results/parity-shots/editor-port-min-1024x700-logical.png` |

Both captures are regenerated, never committed (`.gitignore: /test-results/`).

### Gate

| Check | Verdict | Note |
|---|---|---|
| Header is one row | pass | brand, `[Themes\|Editor\|Settings]`, toolbar, pane toggles; no wrapping at either size |
| Brand, three modes, toolbar visible | pass | mode segments carry the source's icons; toolbar shows in Editor mode only |
| Rail, left pane, stage, inspector all have width | pass | 46 / 246 / 1fr / 276 fixed columns with visible dividers |
| Status bar | pass | object/selection/opened counts and save state left, message and live/offline right |
| No clipped control row | pass | toolbar and header fit at 1024x700 logical |
| Theme | pass (intentional divergence) | Graphite palette, directed by the user as the prioritized shell theme; source default capture is Editorial/light |

Collapse: the header's pane toggles and the rail both hide/show the left pane;
the rail's active-slot press closes it, matching the source's rail press.

### Divergences that are deferred, with the phase that owns them

- **Publish control.** The source header's right end is the accent "Assign to a
  display" chip plus its indicator. The port has the accent button, opening a
  surface that assigns the open theme to a stored display and flips the host's
  LAN switch beside the source's warning. When a host is attached the surface
  prints the host's own bound port, LAN address from the outbound interface,
  host name and current pairing code. The QR symbol is not drawn: the kit has
  no QR element and the code is printed beside the address as the source does
  for devices with no camera. P4 (dialogs/modes).
- **Toolbar coverage.** The rows now follow the source: New theme, New from
  template, Open library, Import theme / Save, Save as, Open display, Export
  theme, Export widget / Find assets. **Open recent** now sits after Open: the
  toolbar's history button opens the editor's own list of opens, newest first,
  persisted in `~/.yunari/recents.json` (capped at five, deduplicated, each by
  the name it had when opened), with the source's "No recent themes yet." empty
  copy. It is a menu, not a command: the library listing is a different thing.
  The source's **Activate** command is covered here by Assign to a display in
  the publish surface.
- **Rail pane bodies.** All six slots have bodies: Layers, Add, Widgets,
  Assets, Design tokens and Theme settings. Pane-level drag/drop, rename and
  keyboard tree navigation remain P2/P3.
- **Stage chrome.** Artboard label chip, zoom/fit cluster, the inspector toggle,
  the bottom asset button, the canvas context menu and the selection's canvas
  dock are in place. The dock floats the registry's object commands (without
  the eight arrange actions, which stay in the inspector as in the source) and
  only while something is selected. A right-press selects the object under the
  pointer and offers the same commands; a press on empty canvas offers the Add
  pane's own insert groups, from the one `INSERT_GROUPS` table the pane reads
  too. The menu flips above the press when it would run off the foot, and
  bounds itself to 520px so a short window scrolls it instead of clipping it.
  Snap guides against the source and drag feedback remain P3.
- **Arrange.** The eight arrange actions sit at the top of the selection
  inspector, which is where the source keeps them: the dock deliberately does
  not repeat them. Each button routes through `Command::apply`, so an
  ineligible selection reports the count it needs in the status bar rather than
  the button going quiet. Evidence:
  `test-results/parity-shots/editor-port-arrange.png` (1280x875, seeded
  two-object selection, reverted after the shot) and
  `the_inspector_keeps_the_eight_arrange_actions_the_dock_leaves_out`, which
  also pins that no arrange command appears in the dock.

The app now embeds gpui-kit's full Lucide catalog (`AllAssets`) rather than the
default bundle, because the source's dock marks (scissors, clipboard, group,
arrow-to-line) are outside the default subset and drew empty.
- **Layer rows** carry the kind column, visibility and lock controls, and now
  have the source's keyboard path: a clicked row owns arrow/Home/End navigation
  over the flattened tree, Right opens or enters a group, Left closes or steps
  out, and Enter/F2 renames. Canvas pointer down hands focus back to the stage. Rows are draggable onto
  another row's slot, including into a group, through one history entry; a node
  cannot be dropped into its own subtree.
- **Pane scrolling.** Resolved: the pane bodies use the kit's
  `ScrollableElement` scroll area, so the wheel and the scrollbar share one
  offset instead of the old row-count estimate.
- **Mode surfaces.** Themes is live: the header segment shows stored themes
  as a card grid (open / duplicate / delete on each card), sorted by Name or
  Updated, with the author and node count on the tile. The page scrolls and
  reloads whenever the mode is entered or the library changes. The source's
  real previews come from each theme folder's `thumbnail.png`, the same file
  the source writes and reads. Rows are justified: cards keep the thumbnail's
  aspect, a row is scaled flush to the scroller, and the last row stays short.
  The packing is a port of `store-mode/justified-rows.ts`. The `Open` dialog
  keeps a compact list, as the source's chooser does. Settings is a page with
  the source's two navigation groups; Displays shows the existing settings
  content when a host has opened it, and the other pages say they are not
  ported rather than drawing pretend controls. The catalogue halves and facets
  remain P4.

### Not yet judged (added after the interactive checks)

- **Pointer-only canvas menu.** A right-press needs the real mouse, which the
  capture tooling will not take over, so the menu was photographed from a
  seeded overlay rather than from a live right-click. `Shift+F10` and the
  context-menu key open the same menu for the selection; that path needs a
  focused window, which the same tooling will not take.
- **Mode switching** was exercised live: Themes and Settings both render, and
  the editor body stays mounted behind them (status bar still reads the open
  theme).

### Not yet judged

Keyboard walkthrough of the header controls, focus visibility after a toggle
press, and hover/pressed states on the rail and toolbar. P3 owns the keyboard
path; the P1 gate above is layout only.

## P3 â€” shortcut sheet

The source has no command palette. `?` opens the shortcut reference, and the
port follows it: `SHORTCUT_REFERENCE` carries the source's own rows, labels and
group order (File, Edit, Canvas, Help), one column per group inside the 460px
dialog. The grid is capped at 420px and scrolls, so the sheet fits the
1024x700 logical minimum instead of running past the viewport. Escape closes it.

The sheet also forced the chords into line with the source's `PRODUCT_SHORTCUTS`:

- Bound: New `Ctrl+N`, Open `Ctrl+O`, Save `Ctrl+S`, Find assets
  `Ctrl+Shift+F`, Undo `Ctrl+Z`, Redo `Ctrl+Shift+Z`/`Ctrl+Y`, Copy, Cut, Paste,
  Duplicate, Group `Ctrl+G`, Ungroup `Ctrl+Shift+G`, Delete/Backspace, Select
  all `Ctrl+A`, front `Ctrl+]`/`Ctrl+}`, back `Ctrl+[`/`Ctrl+{`, nudges with
  the arrows, `?`.
- Dropped as invented: `Ctrl+E`, `Ctrl+Shift+E`, `Ctrl+I`, `Ctrl+Shift+I`,
  `Ctrl+Shift+W`, `Ctrl+Shift+A`, `Ctrl+,`, `Ctrl+Shift+M`, `Ctrl+Shift+S` and
  the single-letter add keys. The source reaches those commands from a pane or
  the toolbar, not from a chord.
- `Ctrl+Shift+P` opens publishing; the sheet's Publish group names the act,
  not the switch inside the surface.

Evidence:

- `cargo test -p yunari-editor` â€” 153 passed, including
  `only_the_commands_the_source_binds_print_a_chord`,
  `the_recent_list_keeps_the_last_five_opens_newest_first` and
  `the_reference_lists_the_source_rows_in_the_source_group_order`.
- `cargo test --workspace` â€” no failures.
- `test-results/parity-shots/editor-port-shortcuts.png` â€” 1042x747 physical
  (1024x700 logical at 125%), sheet inside the window, every group visible.
- `test-results/parity-shots/editor-port-1080p-p1.png` â€” the sheet is closed on
  boot, so the default shell is unchanged.
- `test-results/parity-shots/editor-port-recent.png` â€” 1280x875, the recents
  menu sized to its rows. The seeded list was reverted after the shot.
- `test-results/parity-shots/editor-port-publish.png` â€” 1280x875, the publish
  surface with four stored displays, the hosting switch and the warning.
- `test-results/parity-shots/editor-port-library-state.png` — 1920x1080, the
  installed-state filter beside facets and the compact toolbar.
- `test-results/parity-shots/editor-port-library-nav2.png` — 1920x1080, the
  Library nav column and a single-row toolbar.
- `test-results/parity-shots/editor-port-library-facets.png` — 1920x1080,
  the five facet triggers in source order beside search, sort and density.
- `test-results/parity-shots/editor-port-asset-finder-chips.png` — 1280x875,
  the finder rail with per-source chips.
- `test-results/parity-shots/editor-port-asset-finder-tabs2.png` — 1280x875,
  the six-tab rail with Icons selected and per-tab empty copy.
- `test-results/parity-shots/editor-port-library-toolbar.png` — 1920x1080,
  search, five sort keys and four densities; Comfortable gives seven columns.
- `test-results/parity-shots/editor-port-themes-current-footer.png` â€” 1920x1080,
  the current-source card footer: name/day at rest, actions on hover.
- `test-results/parity-shots/editor-port-themes-contain2.png` â€” 1920x1080, the
  justified gallery reading the box's own `thumbnail.png` files; each picture
  is shown whole, not cropped. Seeded mode reverted after the shot.
- `test-results/parity-shots/editor-port-settings-pages.png` — 1280x875,
  the source's two nav groups with a per-page hint.
- `test-results/parity-shots/editor-port-settings-mode.png` â€” 1920x1080, the
  Settings nav with its two groups. Seeded mode reverted.
- `test-results/parity-shots/editor-port-chrome-final.png` / `-polish.png` / `-spacing.png` â€”
  1920x1080, the mode toggles with the kit's `spacing_tokens().sm` gap between
  icon and label, a segmented container behind the mode group, a bolder brand,
  small toolbar buttons and the primary accent publish button.



## P4 â€” source research

A read-only pass over the source checkout (HEAD `c40dd16d`, newer than the
pinned `1241866e5`) found:

- The Publish surface reads its address, port, host name and pairing code from
  the host (`/api/hosting`, `/api/pairing/code`); it never composes them in the
  editor. The port has no equivalent answer yet, so the port prints no address,
  code or QR, and says so above. Its `http://{port}` line was a defect and was
  removed.
- Settings is a two-group navigation (This PC, Editor) with one title and hint
  per page; the port's Settings mode still points at one dialog.
- Library is a two-half catalogue with nav groups, search, five facets, sort
  and a density control; the port's Themes mode still lists stored themes.
- Asset Finder has six tabs (Colors, Fonts, Icons, Images, Videos, Wallpaper
  Engine), defaulting to Icons, with per-tab search and empty/key-required/
  error states. The port now draws that rail and separates its stock icons,
  its local pictures and its Wallpaper Engine rows by tab. Source chips now switch the stock and local rows on and off, and every row
  carries its source. Colors, Fonts and Videos still say they are not ported;
  per-tab search, previews and the key-required/error states remain P4.
- The source has no Widget picker, Typography, MCP or Path preset dialog. The
  port's versions of those are extra surfaces; widget insert lives in the
  source's library detail, type authoring in the Tokens pane, path presets in
  Settings, and MCP in a separate server.
- Store is a half of Library, not its own mode.

These are P4 gaps, not claims of parity.

### Folding in the newer source

The user asked that the library/store changes made after the port's pinned
commit be folded in. The source `main` HEAD is now `c40dd16d`; between the pin
and HEAD there are 73 changed files in `library-mode`, `store-mode`,
`settings-mode` and `ui-copy.ts` (16,162 insertions / 7,865 deletions), from
"Library a mode with type navigation" through "collection consent". The live
host on this box was started from `main` with `--app-dir ~/.yunari` and showed
132 themes. Capture: `source-library-current.png`.

P4 surfaces now follow HEAD rather than the pin: the Library/Themes gallery,
Settings mode navigation and publish copy are compared against `c40dd16d`.
The rest of the port stays on the pinned commitment. Concrete folding so far:

- The gallery reads each theme's `thumbnail.png` and uses the source's
  justified rows, not a uniform wrap.
- The card's footer follows the current source: name and day share one line;
  the day is the resting reading and Duplicate/Delete fade in over it when the
  card is attended. The author is a hover chip above the picture. A `time`
  dependency formats the ISO day from `ThemeEntry::updated_at`.
- The listing toolbar has the source's inline search and five sort keys
  (Name, Updated, Created, Downloads, Favorites), defaulting to Updated, plus
  four density levels (Compact 150, Comfortable 204, Roomy 270, Spacious 360)
  whose column count is derived from the measured width. Local rows carry no
  catalogue counts, so Created/Downloads/Favorites leave their order alone.

The Library/Themes mode now has the source's navigation column: Browse (nine
pages in source order) and Publish (My submissions). Dashboard themes draws the
local gallery; the other pages name themselves instead of pretending to hold
controls. Search, facets, sort and density share one toolbar row.

The five facets (Author, Color mode, Sensors, Shape, Style) are read off the
stored documents and filter the listing. Counts come from the whole listing,
as in the source. The port derives sensors from semantic-key families and
shape from the artboard; a catalogue row would carry richer readings.

The Installed state filter (All / On this PC / Not installed) is present. Not
installed has no local rows to show and says the catalogue is not loaded; the
current source removed the old Local library / Online store halves, so they
are not ported.

Still to fold: the publish QR symbol, collection cards
and consent; the remaining Settings pages; and the host-reported publish
address. These are P4, not claims of parity.

## Source-reading gate

A `PreToolUse` hook runs `.claude/scripts/check_source_evidence.py` before
Write/Edit/NotebookEdit on editor UI files. It denies the edit unless a record
in `.claude/source-evidence/<file>.json` exists, is under 30 minutes old, names
source files, and every quoted anchor is still present in the source checkout.
The records are intentionally short-lived: a new surface needs a fresh read of
the code it claims to port. The gate was pipe-tested with and without a
record; the settings watcher may need `/hooks` or a restart to load it.

---

# Editor surface audit (PR #35)

## Issue index (this pass)

| Issue | Row(s) | Class | Severity |
|---|---|---|---|
| #25 | E1, E6, E11 | layout | S1 |
| #26 | E3 | layout | S2 |
| #27 | E1 | missing | S2 |
| #28 | E2, E11 | missing | S2 |
| #29 | E5 | missing | S2 |
| #30 | E4 | missing | S2 |
| #31 | E6 | interaction | S3 |
| #32 | E7 | missing | S3 |
| #33 | E9 | missing | S3 |
| #34 | E10 | missing | S3 |

## Evidence commands

Matched row:
```
cargo test -p yunari-editor history
cd D:\git-repos\yunari\src\web
npx vitest run packages/editor/src/history-manager --reporter=verbose
```

Full editor suites run during this pass:
```
cargo test -p yunari-editor
# 140 passed
cd src/web
npx vitest run packages/editor/src/history-manager packages/editor/src/draft-store.test.ts \
  packages/editor/src/shortcut-manager packages/editor/src/layer-manager \
  packages/editor/src/selection-inspector/view.test.ts
# 10 files, 135 tests passed
```
