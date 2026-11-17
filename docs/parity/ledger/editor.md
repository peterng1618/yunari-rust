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
  display" chip plus its indicator. The port has no publish/assign command yet;
  the header keeps the pane toggles there instead. P4 (dialogs/modes).
- **Toolbar coverage.** Source rows with no port command yet: new from
  template, open recent (menu), activate theme, preview/live source toggle.
  The port toolbar carries New, Open, Import / Save, Save as, Export, Export
  widget, Open display / Find assets. P3 (commands).
- **Rail pane bodies.** All six slots have bodies: Layers, Add, Widgets,
  Assets, Design tokens and Theme settings. Pane-level drag/drop, rename and
  keyboard tree navigation remain P2/P3.
- **Stage chrome.** Artboard label chip, zoom/fit cluster, the inspector toggle
  and the bottom asset button are in place. Canvas context menu, snap guides
  against the source and drag feedback remain P3.
- **Layer rows** print raw ids instead of the source's kind column. P2.
- **Inspector scrolling.** The panes keep a wheel offset but their rows have no
  scrollbar and the estimate is row-count based. Replace with the kit scroll
  container in P2.
- **Mode surfaces.** Themes and Settings render a named empty state, not their
  source surfaces. P4.

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
- Still absent: `Ctrl+Shift+P` (open publishing), because the publish surface is
  P4 and a sheet row that opened nothing would be a promise the port cannot
  keep.

Evidence:

- `cargo test -p yunari-editor` — 148 passed, including
  `only_the_commands_the_source_binds_print_a_chord` and
  `the_reference_lists_the_source_rows_in_the_source_group_order`.
- `cargo test --workspace` — no failures.
- `test-results/parity-shots/editor-port-shortcuts.png` — 1042x747 physical
  (1024x700 logical at 125%), sheet inside the window, every group visible.
- `test-results/parity-shots/editor-port-1080p-p1.png` — the sheet is closed on
  boot, so the default shell is unchanged.


