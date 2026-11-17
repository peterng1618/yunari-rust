# Editor UI reference — Yunari source at `1241866e`

Captured 2026-11-17 from `D:/git-repos/yunari @ 1241866e53df252307e22ff3c51c842f89e458c8`.

This is the acceptance reference for rebuilding the Rust shell. It maps what the source editor actually draws, where each control comes from, and which gpui-component 0.9.1 widget is the closest candidate. It is not a feature list: two features can share one surface, and one surface can carry several modes.

**Coverage: 47 captured states — 8 Themes states, 8 editor selection states, 8 pane states, 6 dialog/save states, 3 Settings pages, 8 transient states and 6 canvas-chrome states.**

Component-choice rationale (kit widget or composition, theming, resizable-vs-dock) lives in `docs/superpowers/specs/editor-ui-components.md`. This document is the source-side placement reference that the component choices must fit.

Screenshots live in `test-results/editor-reference/` and are **not committed** — `/test-results/` is ignored by `.gitignore:3`. This document is the committed artefact; the PNGs are local acceptance evidence.

## 0. Capture method

- Source snapshot extracted with `git archive` to `.source-yunari/` at the pinned commit, so the source checkout was not modified.
- Built with `npm run build` from `.source-yunari/src/web`.
- Ran the real host, not `vite preview`:
  `node .source-yunari/src/web/packages/host/bin/yunari.js --no-browser --app-dir <dir> --port 4317`
- Two isolated app dirs:
  - `.reference-app-empty/` — empty library (`01`).
  - `.reference-app/` — populated library. Themes created/copied for capture: `Mono slate`, `Aurora dashboard`, `System dashboard`.
- Playwright headless, Chromium 1248, viewport `1920×1080`, device scale 1, full-viewport screenshots.
- No window was foregrounded; no user pointer or keyboard was taken over.
- Deterministic selection used `window.yunariEditorBridge.selectLayers([...])` / `selectLayer(...)`.
- Loading, failure and refusal states were induced with Playwright routing:
  - `35` delayed `/api/assets**`;
  - `33` aborted `PUT /api/themes/**`;
  - `34` imported a non-font text file.
- `45` was recaptured after moving a rectangle to artboard coordinates `(-90, -60)` so the clip is visible.

### Capture limits

- **Online store is not populated in this environment.** The host has no remote catalogue, so `05` and `06a` capture the online half's failure body (`The store could not be read. Try again.`). A populated online gallery was not available. The local half is fully captured.
- **No rulers, grid overlay, or persistent hover outline exists at this commit.** `42` is retained as evidence of the absence; the pointer is over `wordmark` and no outline is drawn. Fabric only supplies a hover cursor.
- **No widget, path-preset, typography, or MCP/agent-tools dialog exists at this commit.** The nearest real surfaces are captured: Widgets is a pane (`17`), path presets are a Settings page (`29`), typography is the type-token inspector plus the Font finder (`21`, `26`), and MCP is a host CLI with no editor UI.
- `15-pane-layers.png` is the **collapsed pane-column state** (Layers slot still pressed, pane hidden). The expanded Layers pane is `07`.
- The publish/assign surface and the output-bake warning were not captured; they are source surfaces (`publish-control.tsx`, `components/output-bake-dialog.tsx`) but were outside the requested screen set.

## 1. Shell frame

The shell is three peers: a persistent header, a mode surface, and a persistent status strip. Only the editor owns the canvas.

### 1.1 Measured geometry at 1920×1080

| Region | Size / position | Source |
|---|---|---|
| Header | 42px high, full width, 14px side padding, 12px gap, bottom hairline | `editor-shell.css` `--header-height: 42px`; `shell-layout.tsx` header |
| Body (editor) | 1920×1012 between header and status | CSS grid |
| Rail | 46px wide | `editor-shell.css` `grid-template-columns: 46px 246px minmax(0,1fr) 276px` |
| Pane column | 246px wide | same |
| Stage | remaining width: 1352px in the default state | same |
| Inspector | 276px wide | same |
| Inspector collapsed | 38px (`26px + 2×--space-6`, `--space-6: 6px`) | `editor-shell.css` collapsed variant |
| Pane collapsed | column removed; rail + stage + inspector remain | `data-collapsed="true"` variant |
| Status | min-height 26px, grows for wrapped diagnostics | `editor-shell.css` |
| Settings nav | 176px | `--settings-nav-width` |
| Settings page measure | max 1200px | `--settings-page-measure` |
| Standard dialog | min 340px, max 560px, max-height 720px | `--dialog-min-width`, `--dialog-max-width`, `--dialog-max-height` |
| Asset finder | 1416×880, centred | `--finder-width`, `--finder-height` |
| Finder search | 510px | `--finder-search-width` |
| Store search | 360px | `--store-search-width` |

The 246px pane is one column, not a sidebar plus a second panel. The inspector is always present; it is not opened by selection.

### 1.2 Header anatomy

Left to right:

1. Wordmark `Yunari`.
2. Mode switch, order **Themes, Editor, Settings** (`MODE_ORDER`), each with glyph and label. Editor is the boot mode.
3. Mode-level control: in Themes only, the **Local library / Online store** halves toggle.
4. Editor-only toolbar: 12 icon buttons in three groups divided by rules, then the primary button.
   - Document lifecycle: New theme, New from template, Open, Open recent (menu), Import theme.
   - Document actions: Save, Save as, Activate, Export theme, Export selection as widget.
   - Tools: Find assets…, Readings from (preview/live toggle).
   - Primary: **Assign to a display** (filled accent).
5. No document name, dirty mark, zoom, or save state in the header.

Every icon button's accessible name is its label; its tooltip prints the label and the live shortcut chord. The toolbar is `role="toolbar"` named **Theme document actions**.

Source: `editor-shell/shell-layout.tsx`, `shell-menu-bar.tsx`, `publish-control.tsx`, `editor-shell.css`.

### 1.3 Body anatomy

- **Rail** — 46px, six 36px slots in order: Layers (`composition`), Add, Widgets, Assets, Design tokens, Theme settings. One active slot at a time (`aria-pressed`). Clicking the active slot collapses the pane column; `aria-pressed` stays true so reopening restores it. Settings gear is pinned to the rail foot and switches the shell to the Settings mode; it is not a pane slot.
- **Pane column** — 246px; every pane has the same chrome: glyph, title, optional count, body, optional footer toolbar. Panes are `hidden`, never unmounted, because their content hosts persist. One pane is visible.
- **Stage** — artboard and its chrome. Contains the identity chip, zoom/display cluster, canvas dock, context-menu binding, selection indicators, snap guides, background playback control, and the global finder/shortcut-sheet mounts.
- **Inspector** — 276px, always mounted; collapse control at its top-left. Content is selection-driven (see §3.2).

Source: `editor-shell/rail.tsx`, `pane.tsx`, `shell-layout.tsx`, `editor-shell.css`.

### 1.4 Status strip

Readings only, no commands. Editor mode, left to right:

1. `N objects`.
2. `N selected`.
3. Status message (transient; e.g. `Saved to library`).
4. Diagnostic message with severity word (`Error:` / `Warning:`) and optional detail.
5. Save state — empty when saved, `Unsaved changes` when dirty (`role="status"`).
6. `live` mark only while LAN publishing is active.

Themes mode replaces the left block with `N themes`. Settings mode replaces it with display count and the host status sentence. The strip is the same element in all modes; each mode publishes its own facts.

Source: `status-readout.tsx`, `status-message.tsx`, `diagnostic-message.tsx`, `save-state.tsx`, `publish-indicator.tsx`, `shell-layout.tsx` footer.

## 2. Screen map (a)

Surface abbreviations: **T** = Themes mode, **E** = Editor mode, **S** = Settings mode. All paths are under `src/web/packages/editor/src/`.

### 2.1 Themes mode

Themes mode removes the rail, pane and inspector entirely. Header + region + status only. The header carries brand, mode switch, and the local/online halves. The region toolbar carries search, facets, sort, and density. The body is the gallery; each half scrolls independently and batches 24 cells.

| # | Screen | Regions and controls | Source | Candidate kit |
|---|---|---|---|---|
| 01 | Empty library | Header: brand, modes (Themes active), halves (Local active). Toolbar: search (360px), five facet triggers (Author, Color mode, Sensors, Shape, Style), `Sort themes`, density far right. Body: centred `No themes are saved on the Host yet.` / `Build one in the editor, or import a package.` Status: `0 themes`. | `store-mode/region.tsx`, `halves.tsx`, `facet-row.tsx`, `density-slider.tsx` | `title_bar` + `tab` + `toolbar` + `input` + `select` + `slider` + `list` |
| 02 | Populated library | Same frame. Justified card rows; each card = thumbnail, name, modified date. Cell actions appear on hover/focus: `Open in editor`, `Assign to display`, `Export theme package`, `Delete`. Clicking the card opens its item surface. Status: `3 themes`. | `store-mode/gallery.tsx`, `local-source.ts`, `item-modal.tsx` | `list` + `menu` + `tooltip` + `dialog` |
| 03 | Author facet open | Popover under the `Author` trigger: `Search values` field, checked option `Yunari` with count `3`. Toggling filters the gallery in place and returns to the first batch. Popover is dismissed by click-away/Escape. | `store-mode/facet-row.tsx` | `menu`/popover + `input` + `list` |
| 04 | Sort menu open | Small menu under `Sort themes`: `Name`, `Updated`. Choice is per half and persists in storage. | `store-mode/sort-orders.ts`, `view-preferences.ts` | `menu` + `command` |
| 05 | Online store (failure) | Halves: `Online store` active. The same toolbar and gallery frame remain. Body: `The store could not be read.` + `Try again` link. No cards. This is the only online state obtainable here. | `store-mode/region.tsx` (`Failure`), `mine-listing.ts` | `list` empty/failure state + `button` |
| 06 | Local search `mono` | Search well contains `mono`, clear `×`; gallery narrows to the matching card. Status count follows the filtered set. | `store-mode/local-source.ts` (`matchesQuery`) | `input` + `list` |
| 06a | Online search `dashboard` | Query is present; online body still shows the read failure. Shows that the toolbar is shared by both halves. | `store-mode/region.tsx` | `input` + `list` |
| 06b | Local search `zzzz` | Body: `No themes match these filters.` Search well focused, clear button visible. Status `0 themes`. | `store-mode/region.tsx` (`empty-filter`) | `input` + empty `list` |

### 2.2 Editor: base and selection states

Common frame for all editor screens: header (Editor active, full toolbar, Assign primary); rail 46px; pane 246px; stage 1352px until pane collapse; inspector 276px; status 26px.

| # | Screen | Regions and controls | Source | Candidate kit |
|---|---|---|---|---|
| 07 | Nothing selected | Rail: Layers active. Pane: `Layers`, count `10 objects`, tree of groups/text/background, no footer. Stage: identity chip `Mono slate` with dirty dot; zoom cluster `− 81% + ⤢` and fullscreen; artboard on checkerboard. No dock. Inspector: collapse chevron + `Select an object on the canvas to inspect it.` Status: `10 objects`, `0 selected`, `Opened Mono slate`. | `shell-layout.tsx`, `document-identity.tsx`, `display-switch.tsx`, `selection-inspector/inspector.tsx` | `title_bar` + `sidebar` + `tree` + `inspector` + `status_bar` |
| 08 | Text selected (`strapline`) | Layers: row selected; canvas: bounds, 8 handles, rotation handle. Pane footer: object actions (copy/cut/duplicate/lock/arrange/delete). Canvas dock appears bottom-centre. Inspector: subject `T strapline`; `CONTENT 1` (Runs align, Vertical text switch, Vertical align, Overflow, run text, Type token, Size, Line height, Letter spacing, Color, Reads); `POSITION 2` (X/Y/W/H); `LAYER 2` (Rotation, Opacity); `PAINT` below fold. Status `1 selected`. | `panes/composition/composition-pane.tsx`, `editor-shell/layer-panel.tsx`, `canvas-dock.tsx`, `selection-inspector/per-kind-column.ts` | `sidebar` + `tree` + `inspector` + `input` + `select` + `switch` + `slider` + `color_picker` + `toolbar` |
| 09 | Group selected (`group-cpu-card`) | Layer row selected with enter-group control. Canvas group bounds. Inspector: subject `group-cpu-card`; `POSITION 2` and `LAYER 2` only — no Content or Paint. Arrange row absent (single subject). Dock and footer present. | `per-kind-column.ts` (group kind), `selection-inspector/column.tsx` | `inspector` + `input` + `slider` + `toolbar` |
| 10 | Multiple selection (2 text objects) | Two layer rows selected; canvas shows both bounds. Inspector subject `unidentified`; eight arrange icon buttons at the top, `Distribute horizontally`/`Distribute vertically` disabled at two; `POSITION 2`, `LAYER 2`. No Content/Paint. Status `2 selected`. | `selection-inspector/selection-lead.tsx`, `arrange.ts`, `object-actions.ts` | `inspector` + `toolbar` + `tooltip` |
| 11 | Shape selected (`Rectangle`) | Add pane open. Canvas rectangle bounds/handles. Inspector subject `Rectangle`; `POSITION 2`, `LAYER 2`, `PAINT 6` (Fill `Panel`, Stroke `not set`, Border width `1`, Corner radius `10`, Shadow `not set`, Frosted glass switch). No Content. | `selection-inspector/panel.ts`, `per-kind-column.ts` (shape) | `inspector` + `select` + `input` + `slider` + `switch` + `color_picker` |
| 12 | Chart selected (`Bar`) | Add pane open. Chart bounds. Inspector subject `Bar`; `CONTENT 1` (Add a series, Orientation, Minimum, Maximum, Bar width, Category gap, Corner radius, Track corner radius, Segments, Segment spacing, Show axes, Transition duration, Transition easing, Appearance duration, Appearance easing); `POSITION 2`; `LAYER 2`; `PAINT 1` (Fill paint, Track paint). | `chart-manager/chart-fields.ts`, `per-kind-column.ts` (chart) | `inspector` + `select` + `input` + `slider` + `switch` + `color_picker` |
| 13 | Image selected | Assets pane open; image fills the artboard. Inspector subject `Image`; `CONTENT 1` (Sensor `not set`); `POSITION 3` (X/Y/W/H + `Crop`); `LAYER 2`; no Paint. | `per-kind-column.ts` (image), `selection-inspector/crop.tsx` | `inspector` + `input` + `select` + `button` |
| 14 | Background selected | Assets pane open. Inspector subject `Background`; `CONTENT 3` (Mode Simple/Advanced, Background color `None` + picker, Background media `starter-backdrop`); `POSITION 4` (Bounds `Fill theme`, Scale `100`, Offset X/Y, Flip horizontal/vertical); no Layer/Paint. Selecting the Background layer is a first-class selection, not a node. | `selection-inspector/background-column.ts` | `inspector` + `switch` + `select` + `slider` + `color_picker` |

### 2.3 Panes

| # | Screen | Regions and controls | Source | Candidate kit |
|---|---|---|---|---|
| 15 | Pane collapsed | Rail only, 46px; Layers slot pressed; pane column hidden; stage widens; inspector unchanged. This state is restored by clicking the same slot. | `rail.tsx`, `shell-layout.tsx` (`choosePane`) | `sidebar` + `tooltip` |
| 16 | Add | Title `Add`, no count, no footer. Full-width `Text` and `Vertical text` rows. `BASIC SHAPE`: Rectangle, Ellipse, Polygon, Path. `CUSTOM SHAPE`: Horizontal line, Arc, Wedge (plus user presets). `CHART`: Gauge, Line, Bar, Pie, Radar, Polar bar. Tiles are insert buttons; shape/chart tiles disclose their name by tooltip. | `panes/add/add-pane.tsx`, `new-object-panel.ts` | `sidebar` + `list`/grid + `tooltip` |
| 17 | Widgets | Title `Widgets`, count `8 units`. Group `CARD`; 8 tiles in a 4×2 grid: Clock, CPU, GPU, RAM, VRAM, Performance, Storage, Network. Tile = glyph + label; inserts a prebuilt card unit. | `new-object-panel.ts` (`widgetGroups`), `widget-packs.ts` | `sidebar` + `list` + `tooltip` |
| 18 | Assets | Title `Assets`, count `1 file`. Header actions: `Import file`, `Asset Finder`. Size meter `469.9 kB of 90 MB · Unused 0 B`. `IMAGES` section. Card: thumbnail, filename, `Image · 428.3 kB`, `In use` pill, actions (preview, place, replace, remove). Videos/fonts/SVGs get equivalent cards and sections. | `panes/assets/assets-pane.tsx`, `asset-card.tsx`, `asset-size-meter.tsx` | `sidebar` + `list` + `button` + `toolbar` |
| 19 | Design tokens, paint list | Title `Design tokens`. `PAINT TOKENS` heading + `17 tokens` count. 17 rows: swatch, name, usage count; selected row outlined. `+` add control at the end. `TYPE TOKENS` heading + `3 tokens`: `[Heading] Segoe UI… 7`, `[Body] Segoe UI… 21`, `[Mono] Consolas… 0`, each with `Aa` specimen and `+` add. No pane footer. | `panes/tokens/tokens-pane.tsx`, `paint-list.tsx`, `type-preset-manager/panel.ts` | `sidebar` + `list` + `color_picker` + `input` |
| 20 | Paint token selected (`Text`) | Tokens pane keeps its list; inspector swaps to the token inspector: subject `TEXT`; `Name`; `Paint` (`Solid`); `Color` `#ecf5ff` + picker + delete; `Layers` list of every bound layer id; unbind action. No canvas selection. | `panes/tokens/token-inspector.tsx`, `palette-manager` | `inspector` + `input` + `select` + `color_picker` + `list` |
| 21 | Type token selected (`[Body]`) | Tokens pane keeps its list; inspector: subject `[BODY] SEGOE UI, SANS-SERIF 400`; `Name`; `Aa` specimen; `Family` select; `Weight` `400`; `Layers` list of bound ids; unbind; delete. Family menu includes `Font finder…` and `Import local font…`. | `panes/tokens/token-inspector.tsx`, `type-token-manager/font-actions.ts` | `inspector` + `input` + `select` + `list` |
| 22 | Theme settings / Artboard | Title `Theme settings`, no count. `THEME METADATA`: Name, Author, Description, Style tags (chips + propose/add), Theme language select, Sample, Release version. `ARTBOARD`: Shape, Ratio, Orientation, Resolution, Size W/H, derived note `Objects are not moved or resized. Anything outside the artboard is not shown on a display.` Credits section only when credits exist. Inspector retains the previously selected type token. | `panes/document/document-pane.tsx`, `artboard-fields.tsx`, `metadata-fields.tsx` | `sidebar` + `inspector` + `input` + `select` + `color_picker` |

### 2.4 Dialogs and modal surfaces

All standard dialogs are centred over a dimmed shell. Width 340–560px, max-height 720px. The finder is the one large surface.

| # | Screen | Regions and controls | Source | Candidate kit |
|---|---|---|---|---|
| 23 | Open theme | ~388px wide. Title `Open a theme`; lead repeats the choice; `Theme` select (`Starter – System dashboard`); actions `Remix`, `Duplicate`, `Delete`, `Cancel`, `Open`. No search or gallery. Delete asks for confirmation. | `theme-library-dialog.tsx` | `dialog` + `select` + `button` |
| 24 | New theme | ~480px wide. Title `Choose an artboard size`; Shape, Ratio, Orientation, Resolution select rows; Size W/H pair; derived `1920 × 1080`; `Preview ratio: 16:9`; `Cancel` / `Create`. Creating is blocked until a valid size exists. | `new-document-chooser.tsx` | `dialog` + `select` + `input` + `button` |
| 25 | Asset finder (Icons) | 1416×880. Title bar: `Asset Finder`, tabs `Colors`, `Fonts`, `Icons`, `Images`, `Wallpaper Engine`, close `×`. Search row: 510px well + source chips (`Lucide`, `LineIcons`, `Font Awesome Free`). Body: 7-column card grid at 168px min. Footer: `150 results · 3 sources`, primary `Place on canvas`. Cards show glyph, name, licence/source mark. | `asset-finder/finder-frame.tsx`, `modal.tsx` | `dialog` + `tab` + `input` + `list` + `button` |
| 26 | Font finder (Typography path) | Same finder, `Fonts` tab open. Filters `All / Trios / Fonts`, `A–Z / Provider`, `Curated fonts / Google Fonts`; disclosure of the upstream font API. Cards: specimen, family, classification · faces · licence. Footer `577 results · 2 sources`. Reached from a type token's `Family` menu → `Font finder…`; there is no standalone Typography dialog. | `asset-finder/*`, `type-token-manager/font-actions.ts` | `dialog` + `tab` + `input` + `list` |
| 27 | Import local font | ~340px wide. Title `Import local font…`; filename; `FONT DETAILS`; Family (`consola`), Weight (`400`), Style (`Normal`), License (`unknown`), Source URL; note `A font is declared in the package; it is not placed on the canvas.`; `Create type token` / `Cancel`. | `components/font-import-dialog.tsx`, `panes/assets/font-metadata-form.tsx` | `dialog` + `input` + `select` + `button` |
| 31 | Save cleanup | ~530px wide. Title `Save`; subtitle `Mono slate · Save`; `UNUSED FILES 1 file · 7.0 B`; checkbox row for the unused file; note `Nothing on the canvas, the background, or a type token refers to these files. Their attribution goes with them.`; `Remove all unused assets`; warning `Moves them to the Recycle Bin. The theme itself is saved either way.`; size summary; `Cancel` / `Save`. Blocks save until answered. | `components/save-cleanup-dialog.tsx` | `dialog` + `table`/`list` + `checkbox` + `button` |

### 2.5 Settings mode

Settings is a **mode**, not a dialog. It replaces the body region; the editor body stays mounted behind `hidden`. Header shows only brand + mode switch. The nav is 176px with two groups; the page is 1200px max measure and scrolls independently. Status shows display count and host status.

| # | Screen | Regions and controls | Source | Candidate kit |
|---|---|---|---|---|
| 28 | Displays | Nav: `THIS PC` — Displays, Hardware, SignalRGB, Units & time, Readings; `EDITOR` — Appearance, Preview ratios, Custom shape presets, Keyboard, Asset sources, About, Extensions. Page: heading + page hint; three columns: `ALL DISPLAYS` list (`Unassigned displays`, `No theme`), `DISPLAY SETTINGS` detail, `ADD A LOCAL DISPLAY` (Monitor select, Name, Add display), `USB PANELS` cards with Add, `STARTUP` switch. Status `0 displays` / `Changes apply to every display immediately.` | `settings-mode/region.tsx`, `pages/displays-page.tsx`, `nav.ts` | `sidebar` + `list` + `select` + `input` + `switch` + `status_bar` |
| 29 | Custom shape presets | Page heading + hint; `CUSTOM SHAPES` list of built-ins with remove actions; add row: `Name` (`Star`), `SVG path data`, `Add shape`, `Restore defaults`. These entries appear in the Add pane's Custom Shape group. No dialog. | `settings-mode/pages/path-presets-page.tsx` | `list` + `input` + `button` |
| 30 | Extensions | `INSTALL` file picker (`Extension package`, `Choose File`); `INSTALLED EXTENSIONS` empty `No extensions are installed.`; `PROVIDER ORDER` hint + `No extension provides sensor readings.` No MCP/agent-tools page. | `settings-mode/pages/extensions-page.tsx`, `ui-copy.ts` extensions | `list` + `button` + `dialog` for file picker |

### 2.6 Transient states and controls

| # | Screen | What is shown | Source | Candidate kit |
|---|---|---|---|---|
| 32 | Save success | Cleanup dismissed; status `Saved to library`; identity dirty dot cleared; save-state empty. | `save-state.tsx`, save flow | `status_bar` |
| 33 | Save failure | Status line `Could not save to library: Failed to fetch` in error colour; document remains dirty; no dialog. | `persistence-manager`, `diagnostic-message.tsx` | `status_bar` |
| 34 | Validation refusal | Assets pane inline `That file could not be imported.`; status `Error: That file could not be imported.`; no asset card added. | `panes/assets/asset-rows.ts`, `editor-shell/error-manager` | `status_bar` + inline `label` |
| 35 | Finder loading | Finder open with title/tabs/search; skeleton card grid, no counts or footer actions yet. | `asset-finder/finder-frame.tsx` loading body | `dialog` + `skeleton`/`list` |
| 36 | Finder empty search | Query `zzzzzz`; body `No results. Try another word.` + `Clear query`; footer retains the placement action. | `asset-finder/finder-frame.tsx` empty body | `dialog` + `input` + empty `list` |
| 37 | Hover tooltip | Pointer over Save icon; tooltip `Save` + `ctrl+s`; icon button hover state. | `components/ui/control-icon-button.tsx`, `editor-shell/controls/tooltip.tsx` | `tooltip` + `toolbar` |
| 38 | Keyboard focus | Artboard `W` field focused: 2px accent outline, visible value; focus ring not obscured by pane. | `components/ui/control-well.tsx`, `artboard-fields.tsx` | `input` + `focus_ring` |
| 39 | Disabled arrange | Two-object selection; `Distribute horizontally` and `Distribute vertically` disabled (faint, present), six align actions enabled. No tooltip-only substitute. | `selection-inspector/selection-lead.tsx`, `object-actions.ts` | `toolbar` + `tooltip` |

### 2.7 Canvas chrome

| # | Screen | What is shown | Source | Candidate kit |
|---|---|---|---|---|
| 40 | Zoomed in | Zoom readout `98%`; artboard larger than stage; surrounding content cropped at stage edges; checkerboard remains. Zoom cluster is in the stage top-right, not the status bar. | `display-switch.tsx`, `viewport-manager` | `toolbar` + native canvas |
| 41 | Zoom to fit | Readout `81%`; whole artboard visible with pasteboard margin; fit button is pressed/enabled in the cluster. | `display-switch.tsx` | `toolbar` |
| 42 | Hover over object | Pointer over `wordmark`; **no outline is drawn**. This is the source behaviour: Fabric supplies `hoverCursor` only. The requested "hover outline" has no implementation at this commit. | canvas interaction (`bridge.ts`, Fabric defaults) | — |
| 43 | Object context menu | Menu anchored at the pointer; object is selected by the right-click. Entries: `Duplicate` (Ctrl+D), `Copy` (Ctrl+C), `Cut` (Ctrl+X), `Paste` (Ctrl+V), `Bring to front` (Ctrl+]), `Bring forward`, `Send backward`, `Send to back` (Ctrl+[), `Lock`, `Convert to paths`, `Delete` (Delete · Backspace, destructive). Empty-canvas variant shows Add creation groups instead. | `editor-shell/canvas-context-menu.tsx`, `object-actions.ts` | `menu` + `command` + `tooltip` |
| 44 | Snap guides | Mid-drag of `wordmark`: dashed vertical and horizontal guides where it aligns with the strapline/artboard; dragged object translucent. Guides are drawn over the canvas, not a grid. | `snap-manager`, `canvas` guide painting | native canvas overlay |
| 45 | Artboard clip | Rectangle at artboard coords `(-90,-60)`, 340×240. The part outside the artboard is clipped exactly at the top/left boundary; pasteboard shows no spill. | `artboard-clip.ts`, `editor-shell.css` stage clip | native canvas clip |

## 3. Information architecture (b)

### 3.1 Persistent vs contextual

**Persistent in editor mode**

- Header: brand, mode switch, editor toolbar, Assign primary.
- Rail: six slots + Settings gear.
- Pane column: exactly one pane; its chrome is persistent, its body changes.
- Stage: artboard, identity chip, zoom/display cluster.
- Inspector: always mounted, collapsible to 38px; its content is contextual but its column is not.
- Status strip: readings, diagnostic, save state, live mark.

**Contextual**

- Pane contents: the chosen slot; clicking the active slot collapses the column.
- Inspector contents: nothing selected → empty sentence; object selection → per-kind sections; paint/type token selection → token inspector; Background layer → background column.
- Canvas dock: appears only with an object selection; entries filtered by the selection's eligibility.
- Layer footer: appears only with a layer selection; actions filtered by eligibility.
- Stage identity: appears only when a document is open; dirty dot reads the same `isDirty()` as the status.
- Context menu: object actions on an object; creation groups on empty canvas.
- Tags, credits, release version, background playback: conditional on document content.

### 3.2 Modes vs dialogs

| Kind | Members | Behaviour |
|---|---|---|
| Mode | Themes, Editor, Settings | Exclusive body surface. Header persists; editor toolbar only in Editor. Editor body is hidden, never unmounted, while Themes/Settings show, so the document survives. Mode is chrome, never persisted. |
| Dialog / modal | Open theme, New theme, Asset finder, Font import, Save cleanup, output-bake warning, publish/assign surface | Over the current surface; blocks pointer; dismissible; does not change mode. |
| Pane | Layers, Add, Widgets, Assets, Design tokens, Theme settings | One at a time in the pane column; persistent host; collapse to rail. |
| Popover/menu | Facets, sort, Open recent, display lens, context menu, tooltips | Anchored, transient, dismissible. |

The dirty-document guard fires **only on the way to Themes**, because Themes puts the document away. Settings is chrome over the same workspace, so the gear stays usable mid-edit. This is a structural rule, not a convenience.

### 3.3 Reachability

| Action | Routes |
|---|---|
| Open a theme | Header `Open`; Themes card `Open in editor`; Open recent menu |
| New theme | Header `New theme`; New from template |
| Save | Header `Save`; Ctrl+S; save cleanup intercept when unused assets exist |
| Export | Header `Export theme`; Themes card `Export theme package` |
| Assign | Header `Assign to a display`; Themes card `Assign to display` |
| Insert anything | Add pane; empty-canvas context menu; Widgets pane for card units |
| Find assets | Header `Find assets…`; Assets pane `Asset Finder`; Assets → finder → `Connections` deep-links to Settings |
| Edit fonts | Type token inspector `Family` → `Font finder…` / `Import local font…`; Assets font card `Create type token` |
| Manage custom shapes | Settings → Custom shape presets; Add pane's Custom Shape group reflects them |
| Edit object | Canvas selection; Layers row; dock; layer footer; context menu; inspector |
| Rename theme | Double-click the identity chip; Enter commits; Escape cancels; empty name cancels |
| Keyboard reference | `?` opens the shortcut sheet (not captured) |

Every insertion and edit is reachable from at least two surfaces except destructive file removal (Assets card / save cleanup), which is deliberately single-purpose.

## 4. Top 10 structural divergences of the Rust shell (c)

Evidence is from `crates/editor/src/ui.rs` and its dialog modules at the branch base. These are structure-level mismatches, not styling gaps.

| # | Source structure | Current Rust shell | Consequence |
|---|---|---|---|
| 1 | **Three peer modes** — Themes, Editor, Settings — with a header mode switch. Themes is a full-window library with local/online halves, facets, sort, density and per-card actions. Settings is a full-page mode. | No modes. `Overlay` opens `StoreDialog` and `SettingsDialog` as modals; there is no Themes surface and no mode switch. | The library and settings read as interruptions, not places. The store cannot own a gallery, facets, sort, density, status or per-card actions at modal scale. |
| 2 | **Rail + pane + stage + inspector** body grid: 46 / 246 / rest / 276, with pane collapse and inspector collapse. | `row(layers_view, canvas_view, inspector_view)` with fixed 220px layers and 260px inspector; no rail, no pane column, no collapse model. | The six panes have nowhere to live. The shell cannot express "one pane at a time", pane counts, pane footers, or reclaimed stage width. |
| 3 | **Six named panes** — Layers, Add, Widgets, Assets, Design tokens, Theme settings — with shared chrome and conditional counts/footers. Layers is a dense tree with twisty, role, bound key, eye, lock, drag/drop and a selection-only footer. | Only a permanent Layers view, built from text rows (`>`/`v`, `V`/`-`, `L`/`.`). Add, Widgets, Assets, Tokens and Document are absent, fused into the inspector, or represented by dialogs. | Layer manipulation is visually and behaviourally below the source. There is no place for insertion, widgets, assets, tokens or document settings. |
| 4 | **Selection-driven inspector**: subject block; arrange row only for multi-selection; ordered `Content`, `Position`, `Layer`, `Paint` sections per kind; background swaps the whole column; token selection swaps the whole column; empty sentence when nothing is selected; collapsed `Advanced` reserved. | Inspector always renders generic key/value rows, then the palette list, then the type-token list. With no selection it renders document sections. Background is not a selectable subject. | Selection editing is diluted by theme editing. Arrangement, per-kind fields, token inspection, background and the empty state cannot be expressed. |
| 5 | **Design tokens are a pane plus a dedicated token inspector**: 17 paint rows with usage counts and swatches; 3 type rows with specimens; selecting either swaps the inspector to name/paint/color/bindings or family/weight/specimen/bindings, with unbind/delete. | Palette and type lists live inside the inspector; edits happen through `begin_field` text prompts. There is no token subject, no usage count, no bound-layer list, no specimen, no unbind/delete surface. | Token editing is a string-prompt workflow and occupies the inspector permanently, even when an object is selected. |
| 6 | **Commands live where they act**: header only for document lifecycle/view; insertion in Add pane and empty-canvas menu; object actions in dock, layer footer, context menu, inspector arrange; shortcuts in a `?` sheet. All header actions are icon-only with label + chord tooltips. | A single wrapping toolbar renders all 48 `Command::ALL` as text buttons above the body. | Command discovery is inverted: every command is always visible and equally weighted, while the contextual surfaces that should own them are missing. The header becomes a command palette without search. |
| 7 | **Stage chrome**: identity chip with dirty dot and double-click rename; zoom/display cluster with readout, fit, selection, 100%, fullscreen, lens menu; canvas dock; context menu; snap guides; selection handles; background playback; artboard clip. No rulers, grid, or hover outline. | Canvas paints the display list, selection rectangles, guides, handles and marquee. Identity, zoom cluster, display lens, dock, context menu and playback are absent; zoom is a status-bar text reading. | The stage is a viewer, not an editing surface. Document identity and view control have no home, and object actions are only in the layer/inspector columns. |
| 8 | **Small, purpose-built dialog set**: Open theme, New theme, Asset finder (1416×880), Font import, Save cleanup, output-bake warning, publish/assign. Widgets/path presets/typography/MCP are not dialogs. | Ten overlay dialogs including `WidgetDialog`, `PathPresetDialog`, `TypographyDialog`, `McpDialog`, `SettingsDialog`, `StoreDialog`, plus `AssetFinder`, `LibraryDialog`, `SaveAs`, `Import`. | The port invents modal surfaces the source does not have and omits the large finder/library surfaces the source does have. MCP becomes product UI instead of a host CLI. |
| 9 | **Settings is a mode with 12 pages** in two groups, including host pages (Displays, Hardware, SignalRGB, Units & time, Readings) and editor pages (Appearance, Preview ratios, Custom shape presets, Keyboard, Asset sources, About, Extensions). | `SettingsDialog` is one monolithic modal of switches, monitor rows, panels, weather, sources, devices and providers. No page nav, no editor appearance/keyboard/path presets/extensions pages. | Settings has no scalable page model and cannot host the editor preferences that the source treats as first-class pages. |
| 10 | **Status strip is readings-only** and mode-specific; document identity lives on the stage. Failure severity is a diagnostic line; save state is a separate live region. | `status_view` prints `name · saved/unsaved · source id`, zoom, selected count and transient status in one row. Identity and zoom are in the footer; no diagnostic severity or live mark. | The footer mixes authored identity, view state and messages. Dirty/save state is a word rather than a persistent state element, and errors have no severity structure. |

Cross-cutting visual divergence: the source is token-driven with light/dark shell palettes, glass pane surfaces, a 26px control rhythm, 340–560px dialogs, 1416×880 finder, and responsive collapsed states. The port hardcodes dark colours (`0x15151a`, `0x1d1d24`, `0x101014`, accent `0x4f9cf9`) and fixed widths. Rebuilding structure without the palette/control tokens will still read as a different product.

## 5. Acceptance checklist (d)

### 5.1 Global invariants

- [ ] At 1920×1080 the editor body is `46 | 246 | rest | 276`; header 42px; status at least 26px; inspector collapsed 38px; pane collapsed removes the 246px column.
- [ ] Header order: brand, Themes/Editor/Settings switch, editor toolbar (three groups), Assign primary. Toolbar absent in Themes and Settings.
- [ ] Editor is the default mode. Themes and Settings render full-width; the editor body is hidden but not unmounted; returning restores the document, selection and scroll.
- [ ] Leaving a dirty editor for Themes asks first; Settings does not ask.
- [ ] Rail: six slots in order, 36px controls, active `aria-pressed`, clicking active collapses, Settings gear at foot.
- [ ] Pane: one visible, shared glyph/title/count/body/footer chrome; counts only where a real list exists (Layers, Widgets, Assets); no fabricated `0`.
- [ ] Inspector: always mounted; collapse control; empty state; object/token/background subjects swap content; no theme token lists inside an object selection.
- [ ] Status: readings only; separate save-state live region; diagnostic severity word; Themes/Settings publish their own facts.
- [ ] No rulers, grid overlay, or persistent hover outline; snap guides only.
- [ ] Light and dark shell palettes; glass surfaces; 26px control rhythm; dialog 340–560px; finder 1416×880.
- [ ] Every icon-only control has label + tooltip + live shortcut chord; focus ring visible; disabled controls remain present.

### 5.2 Themes mode

- [ ] `01` Empty library: header/halves/search/facets/sort/density present; centred empty lead + hint; status `0 themes`.
- [ ] `02` Populated: justified rows; thumbnail/name/date; hover/focus actions; status count matches cards.
- [ ] `03` Facet popover: search values field; checked option with count; toggling filters; click-away/Escape dismisses; first batch reset.
- [ ] `04` Sort menu: Name/Updated; choice persists per half; gallery reorders.
- [ ] `05` Online failure: halves toggle works; failure body names the failure and offers retry; toolbar remains.
- [ ] `06` Local search: query filters; clear control; count follows.
- [ ] `06a` Online search keeps the online read state; no local leakage.
- [ ] `06b` Empty filter: `No themes match these filters.`; zero count.

### 5.3 Editor base and selections

- [ ] `07` Nothing selected: Layers tree; no pane footer; no dock; inspector empty sentence; identity chip and zoom cluster; status `0 selected`.
- [ ] `08` Text: subject name; Content group includes runs, type token, size, line height, spacing, colour, reads; Position W/H; Layer rotation/opacity; Paint available; dock + footer.
- [ ] `09` Group: only Position/Layer; no Content/Paint; enter-group affordance; arrange row absent.
- [ ] `10` Multi: subject `unidentified`; eight arrange actions; distribute disabled at two; Position/Layer; no Content/Paint.
- [ ] `11` Shape: Paint group with fill, stroke, border width, corner radius, shadow, frosted glass; no Content.
- [ ] `12` Chart: Content group with series, orientation, min/max, bar/category geometry, segments, axes, timings; Paint has fill + track.
- [ ] `13` Image: Content sensor; Position with Crop; Layer; no Paint.
- [ ] `14` Background: Content mode/colour/media; Position bounds/scale/offset/flip; no Layer/Paint.

### 5.4 Panes

- [ ] `15` Collapse: rail only; pressed slot retained; stage widens; reopening restores the same pane.
- [ ] `16` Add: Text/Vertical text; Basic Shape 4; Custom Shape 3+; Chart 6; insertion works from every tile.
- [ ] `17` Widgets: title count; CARD group; 8 units; insertion works.
- [ ] `18` Assets: import + finder; size meter; sectioned cards; in-use/unused; card actions.
- [ ] `19` Tokens: Paint count/list/swatches/usage; Type count/list/specimens; add controls.
- [ ] `20` Paint token: inspector name/paint/color/usage/bound layers/delete.
- [ ] `21` Type token: inspector name/specimen/family/weight/bound layers/unbind/delete; family menu reaches finder/import.
- [ ] `22` Theme settings: metadata fields; artboard shape/ratio/orientation/resolution/size/note; credits only when present.

### 5.5 Dialogs

- [ ] `23` Open: theme select; Remix/Duplicate/Delete/Cancel/Open; refusal/confirmation states.
- [ ] `24` New: shape/ratio/orientation/resolution/size; derived size and preview ratio; Create disabled on invalid.
- [ ] `25` Finder: 1416×880; tabs; search; sources; card grid; footer count + action; Escape/close.
- [ ] `26` Font finder: same finder on Fonts; filters; disclosure; results count; font cards.
- [ ] `27` Font import: filename; family/weight/style/license/source URL; create/cancel; declared-not-placed note.
- [ ] `31` Save cleanup: unused list with sizes; remove-all; warning; cancel/save; save blocked until answered.

### 5.6 Settings

- [ ] `28` Displays: 176px nav with both groups and all 12 pages; three-column page; status count + host sentence.
- [ ] `29` Custom shape presets: built-in list + remove; name/path data/add; restore defaults.
- [ ] `30` Extensions: install picker; installed list/empty; provider order; no MCP page.

### 5.7 Transient and canvas

- [ ] `32` Save success: status sentence; dirty mark cleared; save state empty.
- [ ] `33` Save failure: error-coloured message with host reason; dirty retained.
- [ ] `34` Refusal: inline reason and status severity; no mutation.
- [ ] `35` Loading: skeleton grid; no false counts/actions.
- [ ] `36` Empty search: no-results sentence + clear query; footer action retained.
- [ ] `37` Tooltip: label + chord; dismiss on leave/blur.
- [ ] `38` Focus: 2px accent ring on the focused field.
- [ ] `39` Disabled: distribute disabled at two; present and faint, label still naming it.
- [ ] `40` Zoom in: readout/scale change; stage crops without reflow.
- [ ] `41` Fit: whole artboard framed; readout changes.
- [ ] `42` Hover: no outline; cursor only.
- [ ] `43` Context menu: object entries with chords; destructive delete marked; empty-canvas creation groups.
- [ ] `44` Snap guides: dashed guides during drag; release commits.
- [ ] `45` Artboard clip: outside content clipped at the artboard boundary.

## 6. Screenshot index (e)

All under `test-results/editor-reference/`, **uncommitted** (`/test-results/` is gitignored).

| # | File | State |
|---|---|---|
| 01 | `01-themes-empty.png` | Themes, empty library |
| 02 | `02-themes-library-populated.png` | Themes, 3 local themes |
| 03 | `03-themes-filter-open.png` | Author facet open |
| 04 | `04-themes-sort-open.png` | Sort menu open |
| 05 | `05-themes-online-store.png` | Online half, read failure |
| 06 | `06-themes-search-local.png` | Local search `mono` |
| 06a | `06a-themes-search-online.png` | Online search `dashboard`, failure |
| 06b | `06b-themes-search-empty.png` | Local empty filter `zzzz` |
| 07 | `07-editor-nothing-selected.png` | Editor, nothing selected |
| 08 | `08-editor-text-selected.png` | Text selection, full inspector |
| 09 | `09-editor-group-selected.png` | Group selection |
| 10 | `10-editor-multi-selection.png` | Two-object selection, arrange row |
| 11 | `11-editor-shape-selected.png` | Rectangle, Paint group |
| 12 | `12-editor-chart-selected.png` | Bar chart, Content/Paint |
| 13 | `13-editor-image-selected.png` | Image, Crop |
| 14 | `14-editor-background-selected.png` | Background subject |
| 15 | `15-pane-layers.png` | Pane column collapsed |
| 16 | `16-pane-add.png` | Add pane |
| 17 | `17-pane-widgets.png` | Widgets pane |
| 18 | `18-pane-assets.png` | Assets pane |
| 19 | `19-pane-tokens-palette.png` | Paint + type tokens |
| 20 | `20-pane-tokens-palette-selected.png` | Paint token inspector |
| 21 | `21-pane-tokens-type-selected.png` | Type token inspector |
| 22 | `22-pane-document-artboard.png` | Theme settings / Artboard |
| 23 | `23-dialog-library-open.png` | Open theme dialog |
| 24 | `24-dialog-new-theme.png` | New theme chooser |
| 25 | `25-dialog-asset-finder.png` | Asset finder, Icons |
| 26 | `26-dialog-font-finder-typography.png` | Font finder, Fonts |
| 27 | `27-dialog-font-import.png` | Import local font |
| 28 | `28-settings-displays.png` | Settings, Displays |
| 29 | `29-settings-path-presets.png` | Settings, Custom shape presets |
| 30 | `30-settings-extensions-agent-tools.png` | Settings, Extensions |
| 31 | `31-dialog-save-cleanup.png` | Save cleanup |
| 32 | `32-transient-save-success.png` | Save success |
| 33 | `33-transient-save-failure.png` | Save failure |
| 34 | `34-transient-validation-refusal.png` | Import refusal |
| 35 | `35-transient-loading-finder.png` | Finder loading |
| 36 | `36-transient-empty-search.png` | Finder empty search |
| 37 | `37-control-hover-tooltip.png` | Save tooltip + chord |
| 38 | `38-control-focus.png` | Artboard W focus ring |
| 39 | `39-control-disabled-distribute.png` | Disabled distribute |
| 40 | `40-canvas-zoom.png` | Zoomed in, 98% |
| 41 | `41-canvas-fit.png` | Zoom to fit, 81% |
| 42 | `42-canvas-hover-outline.png` | Hover, no outline |
| 43 | `43-canvas-context-menu.png` | Object context menu |
| 44 | `44-canvas-snap-guides.png` | Snap guides mid-drag |
| 45 | `45-canvas-artboard-clip.png` | Artboard clip |

## 7. Source map

Quick file index for rebuild work:

- Shell frame: `editor-shell/shell-layout.tsx`, `editor-shell.css`
- Header: `editor-shell/shell-menu-bar.tsx`, `editor-shell/publish-control.tsx`
- Rail / pane: `editor-shell/rail.tsx`, `editor-shell/pane.tsx`
- Layers: `panes/composition/composition-pane.tsx`, `editor-shell/layer-panel.tsx`, `editor-shell/layer-tree.ts`
- Add / widgets: `panes/add/add-pane.tsx`, `new-object-panel.ts`, `widget-packs.ts`
- Assets: `panes/assets/*`, `asset-manager/*`, `asset-finder/*`
- Tokens: `panes/tokens/*`, `palette-manager/*`, `type-token-manager/*`, `type-preset-manager/panel.ts`
- Document: `panes/document/*`
- Selection inspector: `selection-inspector/*`
- Stage chrome: `editor-shell/document-identity.tsx`, `display-switch.tsx`, `canvas-dock.tsx`, `canvas-context-menu.tsx`, `background-playback.tsx`
- Status: `editor-shell/status-readout.tsx`, `status-message.tsx`, `diagnostic-message.tsx`, `save-state.tsx`, `publish-indicator.tsx`
- Themes: `store-mode/*`
- Settings: `settings-mode/*`
- Dialogs: `theme-library-dialog.tsx`, `new-document-chooser.tsx`, `asset-finder/*`, `components/font-import-dialog.tsx`, `components/save-cleanup-dialog.tsx`, `components/output-bake-dialog.tsx`

## 8. Known gaps in this reference

- Online catalogue populated state was unavailable; only the online failure state exists in these captures.
- Publish/assign and output-bake warning were not captured.
- `42` records the absence of a hover outline; do not treat the filename as evidence that one exists.
- `15` records the collapsed pane column, not the expanded Layers pane (`07` does).
- No source UI exists for MCP/agent tools, a widget dialog, a path-preset dialog, or a standalone typography-preset dialog; any Rust shell surface with those names is an invention to be reconsidered against §4.
