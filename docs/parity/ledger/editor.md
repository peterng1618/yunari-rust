# Editor UI ledger

Source: `D:\git-repos\yunari` @ `1241866e53df252307e22ff3c51c842f89e458c8`.
Port: `D:\git-repos\yunari-rust`.

The reference capture is the source host driven at 1920x1080
(`node packages/host/bin/yunari.js --no-browser`). The served bundle names the
first mode **Themes**, which is the pinned commit's vocabulary; the checked-out
HEAD renamed it Library, so the served bundle is pinned-era, not HEAD.

## P1 — shell

Evidence run:

- `cargo test -p yunari-editor` — 144 passed (2 suites), including
  `shell_modes_are_themes_editor_settings_in_order`,
  `rail_slots_match_the_source_order`, `toolbar_groups_match_the_source_header`,
  `toolbar_groups_do_not_repeat_a_command`.
- `cargo build -p yunari-app` — finished.
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
  LAN switch beside the source's warning. The pairing address/code and QR are
  not printed: the source refuses to compose them locally, and the port has no
  host answer to read yet. P4 (dialogs/modes).
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
- **Layer rows** print raw ids instead of the source's kind column. P2.
- **Pane scrolling.** Resolved: the pane bodies use the kit's
  `ScrollableElement` scroll area, so the wheel and the scrollbar share one
  offset instead of the old row-count estimate.
- **Mode surfaces.** Themes is live: the header segment shows the stored-theme
  list (open / duplicate / delete) in a scrolling page, reloaded whenever the
  mode is entered or the library changes. Settings names what it holds and
  opens the one settings dialog; its own surface is P4. P4 also owns the
  source's library grid, halves and facets.

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

## P3 — shortcut sheet

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

- `cargo test -p yunari-editor` — 153 passed, including
  `only_the_commands_the_source_binds_print_a_chord`,
  `the_recent_list_keeps_the_last_five_opens_newest_first` and
  `the_reference_lists_the_source_rows_in_the_source_group_order`.
- `cargo test --workspace` — no failures.
- `test-results/parity-shots/editor-port-shortcuts.png` — 1042x747 physical
  (1024x700 logical at 125%), sheet inside the window, every group visible.
- `test-results/parity-shots/editor-port-1080p-p1.png` — the sheet is closed on
  boot, so the default shell is unchanged.
- `test-results/parity-shots/editor-port-recent.png` — 1280x875, the recents
  menu sized to its rows. The seeded list was reverted after the shot.
- `test-results/parity-shots/editor-port-publish.png` — 1280x875, the publish
  surface with four stored displays, the hosting switch and the warning.
- `test-results/parity-shots/editor-port-chrome-final.png` / `-polish.png` / `-spacing.png` —
  1920x1080, the mode toggles with the kit's `spacing_tokens().sm` gap between
  icon and label, a segmented container behind the mode group, a bolder brand,
  small toolbar buttons and the primary accent publish button.


