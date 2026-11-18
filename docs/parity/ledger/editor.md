# Editor parity ledger

Source: `D:\git-repos\yunari` @ `1241866e53df252307e22ff3c51c842f89e458c8`
Port: this repository, branch `audit/editor`.
Method: source behaviour read from the implementation and its tests first, then
the source host editor (`node packages/host/bin/yunari.js --no-browser`, 1920x1080
Chromium via Playwright) was compared with the port (`cargo build -p yunari-app`,
client area 1920x1080 captured with `scripts/capture-window.ps1`). The port was
also captured at 2560x1440 to separate "absent" from "clipped at 1080p".

| Row | Status | Evidence |
|---|---|---|
| E1 | differs #25, #27 | Inspector and status footer are outside the 1920x1080 client area; there is no header, mode switch or assign-to-display action. Shots: `test-results/parity-shots/editor-source-shell-1920x1080.png`, `editor-port-shell-1920x1080.png`, `editor-port-2560x1440.png`. |
| E2 | differs #28 | No Themes/library surface. Startup is the editor; Open is a flat "Stored themes" modal (`crates/editor/src/ui/library_dialog.rs`); no search, facets, sort, density, thumbnails, store half or empty-state library. Shot: `editor-source-themes-1920x1080.png`. |
| E3 | differs #26 | `Command::ALL` is 48 plain text buttons in one wrapping row (`crates/editor/src/ui.rs:2788`, `ui/menus.rs:57`); labels clip at 1920x1080 ("Shape preset…" → "Shape p…"). Source has grouped labelled icon buttons in the header, inserts in Add, arrange/group in the pane footer. |
| E4 | differs #30 | Port canvas has no zoom/fit/fullscreen control, no zoom readout, no document identity chip, no display switch, no selection dock and no context menu. Selection handles/marquee exist. Shots: `editor-source-shell-1920x1080.png`, `editor-source-selected-1920x1080.png`, `editor-port-shell-1920x1080.png`. |
| E5 | differs #29 | Port rows are `name` + literal `V`/`L` letters; no kind icon/word, no count, no rename, no layer-action footer (`crates/editor/src/ui.rs:2815`). Source rows are icon + name + kind with state icons, count in the pane header and `LayerActions` footer. |
| E6 | differs #25, #31 | Inspector unreachable at 1920x1080 (issue #25). Where it paints (2560x1440) every field is a static text chip that opens a one-line prompt; source uses `ControlNumber/Pair/Slider/Segmented/Select/Swatch/Toggle` inside `InspectorSection`. Shot: `editor-port-2560x1440.png` vs `editor-source-selected-1920x1080.png`. |
| E7 | differs #32 | No Design tokens pane. Palette and type tokens are text rows under the inspector, with hex strings and no swatches, use counts, type previews, token inspector or layer-use links. Shot: `editor-source-tokens-1920x1080.png` vs `editor-port-2560x1440.png`. |
| E8 | matched | Port: `cargo test -p yunari-editor history` → 5 passed (140 total in the editor crate). Source: `cd src/web && npx vitest run packages/editor/src/history-manager --reporter=verbose` → 17 passed. Both suites cover one entry per action, suspended bursts, artboard/global restoration and selection preservation. |
| E9 | differs #33 | Save/reopen logic exists (`crates/editor/src/persistence.rs`), but there is no draft autosave or recovery: `grep -rin "draft\|autosave" crates/editor crates/app` matches only test fixture ids. Source writes a `DraftStore` and starts autosave in `editor-main.ts:131,170`. |
| E10 | differs #34 | Port `on_key` (`crates/editor/src/ui.rs:2586`) has no `?` case and no override/reference surface; `Command::shortcut` strings are never displayed. Source registers `help.shortcuts` and renders `shortcut-reference.tsx`; Settings → Keyboard persists overrides. |
| E11 | differs #25, #28 | Status/error copy cannot be seen at 1920x1080 because the footer is outside the window (#25); there is no library empty state to read (#28), and no loading/boot-failure surface beyond status strings. Source has the footer counts and explicit empty/refused/draft-recovery states. |

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
