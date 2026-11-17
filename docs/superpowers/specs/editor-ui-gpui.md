# Editor UI in gpui-kit — element map and design

**Status:** approved 2026-11-17. This reverses the desktop-editor part of
`2026-11-17-nsh-ui-migration.md`. NSH remains the stack for the separate mobile
player app; the editor and desktop player stay on gpui-kit.
Source of truth for the UI is the pinned tree
`1241866e53df252307e22ff3c51c842f89e458c8` in `D:\git-repos\yunari`,
`src/web/packages/editor/src`.

**Source correction (2026-11-17):** the source header has no menu bar.
`shell-menu-bar.tsx` is the editor toolbar; the mode switch is a `ToggleGroup`
with Themes, Editor and Settings. The shell section below reflects that.

**App theme (2026-11-17):** the shell ships on the source's **Graphite** palette
(`[data-shell-palette="graphite"]` in `editor-shell.css`) — a dark shell. Other
palettes come later. The values are transcribed into `apply_graphite` in
`crates/app/src/main.rs`, not re-tuned.

This is not a line-by-line React translation. Where the source has a bespoke
control, the table names the gpui-kit component that owns the same behaviour,
and the composition we will ship. gpui-kit supplies the styled control layer;
the app supplies placement, data and the canvas.

## Decision summary

- **Desktop editor:** gpui-kit (`gpui` + `gpui_kit::component`), rebuilt against
  the source reference and the screenshot gate.
- **Desktop player:** existing gpui-kit player, kept.
- **Mobile player:** separate NSH app that displays a theme; it does not carry
  the editor, inspector, store or settings surfaces.
- **No second design system.** Use `gpui_kit::component` and `ActiveTheme`
  directly; `crates/ui` holds only repeated compositions (property row, theme
  card, gradient control, crop overlay), not a wrapper around every kit control.
- **No Dock for the editor shell.** The editor is a fixed-column workspace,
  matching the source's 46px rail + 246px / 1fr / 276px grid; `h_flex` with
  `flex_shrink_0` side columns is the honest primitive. Dock is reconsidered
  only if the product gains user-arranged panels.

The workspace already depends on gpui-kit 0.9.1 and opens windows through
`gpui_kit::open_window`. The defect is the surface: `crates/editor/src/ui.rs`
builds raw `div()` chrome and hard-coded constants instead of kit components.
This map replaces that surface, not the dependency.

## Why this reverses the NSH editor decision

NSH was selected when the mobile roadmap looked like it required one UI stack
for the whole product. The scope is now narrower: a mobile **player**, not the
editor. The editor remains desktop-only, where GPUI is the stronger tool.

The lasting costs of an NSH editor are real and independent of effort:

- `nsh/base` is unstyled; the app owns the entire component library forever.
- NSH Canvas excludes pixel access, filters and SVG images; the editor's
  canvas and effects would be more constrained.
- NSH desktop backends are younger and spread across more platforms; the
  desktop editor would be a secondary use case.
- NSH is one month old at v0.5.1; gpui-kit is older and already carries the
  styled components this editor needs.

Those costs buy mobile reach the editor does not need. The mobile player still
gets NSH, but as a small, separate app that shares the portable crates.

The editor UI failure was not caused by GPUI. It was never built to the source
reference: raw `div()` chrome, hard-coded colours, command rows as labels, no
reachable inspector. Switching frameworks does not fix that. The reference map
and the screenshot gate remain the work.

## Scope and boundaries

Keep, unchanged:

- `crates/model`, `crates/render`, `crates/raster`, `crates/render-gpui`,
  `crates/library`, `crates/store`, `crates/packs`, `crates/theme`,
  `crates/locale`, `crates/panels`, `crates/golden`, `crates/host`,
  `crates/providers`, `crates/nvml`, `crates/rtss`;
- editor behaviour: `document`, `selection`, `transform`, `arrange`,
  `grouping`, `history`, `layers`, `inspector`, `palette`, `tokens`,
  `bindings`, `persistence`, `clipboard`, `snap`, `assets`, `fixtures`;
- `crates/player` for the desktop player.

Replace:

- `crates/editor/src/ui.rs` and `crates/editor/src/ui/*` with kit-composed
  screens;
- `crates/app` shell chrome with `TitleBar`, `Toolbar`, `h_flex` and
  `StatusBar`;
- ad-hoc colour/radius/spacing constants with `ActiveTheme` tokens.

Add:

- `crates/ui`: repeated app compositions only;
- `crates/player-core`: portable player state and rendering boundary;
- `apps/player-mobile`: NSH player shell, a separate binary target that does
  not link `crates/editor` or `crates/app`.

Do not migrate the desktop player to NSH. The plugin/webview surfaces are
mapped for source parity but remain out of v1 unless the extension platform
returns.

```
crates/model, render, raster, library, store, theme, locale, player-core
        |                                   |
  crates/app (gpui-kit)               apps/player-mobile (NSH)
  editor + desktop player             player shell only
```

## Rules

1. **Kit component first.** A styled control is a `gpui_kit::component`
   component. A hand-rolled interactive `div()` is a defect unless it is canvas
   geometry or a drag surface.
2. **Theme tokens only.** Colours, radii, spacing, text sizes and states come
   from `ActiveTheme`/`Theme`, never literals such as `0x1d1d24`.
3. **One action source.** `Command::ALL` remains the registry; menus, toolbar,
   context menus, the command palette and keybindings all render from it.
4. **Source IA unless marked "change".** The reference map and acceptance
   checklist decide placement.
5. **The stage is raster + custom paint.** `DisplayList` renders through the
   existing rasteriser into an image; selection, guides, crop and rulers use
   GPUI painting. Kit `Chart`/`Plot` never draws a theme chart.
6. **Every control has states and a keyboard path.** Rest, hover, focus,
   pressed, disabled, selected, empty, loading and error must be visible.
7. **No Dock for the fixed shell.** Use `h_flex` with three fixed columns
   (246px / 1fr / 276px), matching the source grid. Dock is a workspace product
   decision, not a styling one.
8. **No business logic in UI modules.** UI reads state and dispatches commands.

## What gpui-kit 0.9.1 provides

`gpui-kit` 0.9.1 is already in the workspace and re-exports GPUI, the base
primitives, the styled component library, the default icons and assets. The
styled layer is not an example we copy: it is a dependency we theme.

Relevant components, all documented under `https://gpui-kit.com/component/`
(the page name is the component slug):

- shell: `TitleBar`, `Toolbar`, `StatusBar`, `Command`, `Menu`, `PopupMenu`,
  `ContextMenu`, `Tabs`, `TabBar`, `Sidebar`, `Resizable`;
- content: `Tree`, `List`, `VirtualList`, `DataTable`, `Table`, `Accordion`,
  `Collapsible`, `DescriptionList`, `GroupBox`, `Settings`;
- inputs: `Input`, `Textarea`, `NumberInput`, `Select`, `Combobox`,
  `Checkbox`, `Switch`, `Radio`, `Slider`, `ColorPicker`, `DatePicker`,
  `TimeField`, `Form`;
- feedback and overlays: `Dialog`, `AlertDialog`, `Sheet`, `Popover`,
  `Tooltip`, `Notification`, `Alert`, `Progress`, `Spinner`, `Skeleton`,
  `Empty`, `Badge`, `Tag`, `Kbd`;
- data: `Chart`, `Plot`, `Diff`, `Editor`, `TextView`;
- layout: `h_flex`, `GridLayout`, `Scrollable`;
- large lists: `v_virtual_list`/`h_virtual_list` (documented as VirtualList).

Use them directly. Do not add `crates/ui` wrappers that only forward props:
that hides the kit, duplicates its API and creates a second theme layer.

`gpui_kit::init(cx)` initializes the enabled layers. `gpui_kit::open_window`
mounts `Root`, which supplies dialogs, sheets, notifications, tooltips and
menus. Call both once at app boot.

## Shell

```
custom title bar: brand · mode switch · file actions · document actions · publish
┌─────────────────────────────────────────────────────────────────────┐
│ Yunari [Themes|Editor|Settings] [open] [save] [find]      Publish   │ TitleBar
├────┬───────────────┬──────────────────────────────┬─────────────────┤
│rail│ left pane     │ stage                        │ inspector       │ h_flex
│    │ Layers        │  raster artboard             │  sections       │
│    │ Add           │  selection/guides overlay    │  fields         │
│    │ Widgets       │  zoom/display cluster        │                 │
│    │ Assets        │  canvas context menu         │                 │
│    │ Tokens        │                              │                 │
│    │ Document      │                              │                 │
├────┴───────────────┴──────────────────────────────┴─────────────────┤
│ counts · message · diagnostics · save · publish · live              │ StatusBar
└─────────────────────────────────────────────────────────────────────┘
```

- The source header is **one row**: brand, mode switch, the editor's toolbar
  and its publish control. `TitleBar` carries all of it as child content;
  build the window from `TitleBar::window_options()`.
- There is no source menu bar. `shell-menu-bar.tsx` is the editor toolbar, not
  File/Edit/View menus; do not add an `AppMenuBar` that the source does not
  have. Commands without a visible toolbar control live in the command
  palette and their shortcuts (P3).
- Mode switch is a `ToggleGroup` (segmented well) with the source's three
  modes, in order Themes, Editor, Settings, not a `TabBar`.
- Fixed columns match the source grid: 46px rail, 246px left pane, 1fr stage,
  276px inspector. Pane collapse changes the column set; it never resizes the
  stage under a drag. The widths are editor view state, never document state.
- `StatusBar` left/right/center regions: counts, message, diagnostics, save
  state, publish indicator, live/offline.
- Left pane tabs are a compact `Sidebar` rail (icon + tooltip); `TabBar` is
  horizontal and is used for store/section tabs, not the rail.

```rust
use gpui_kit::component::*;

h_flex()
    .size_full()
    .child(div().w(px(46.0)).flex_shrink_0().h_full().child(self.rail(cx)))
    .when(!left_collapsed, |this| {
        this.child(div().w(px(246.0)).flex_shrink_0().h_full().child(self.left_pane(cx)))
    })
    .child(div().flex_1().min_w(px(0.0)).h_full().child(self.stage(cx)))
    .when(!right_collapsed, |this| {
        this.child(div().w(px(276.0)).flex_shrink_0().h_full().child(self.inspector(cx)))
    })
```

Why not Dock: Dock is a tabbed, draggable workspace. The editor has fixed
semantic regions and no user-arranged panels. Why not `h_resizable`: the source
does not resize its panes, and a resizable group fights the fixed 246/276
columns when the window changes size. Revisit only for a multi-panel workspace.

## Shell and navigation

| Yunari element (source) | Job | gpui-kit proposal | Docs | Notes |
|---|---|---|---|---|
| Brand (`<strong>`) `shell-layout.tsx` | identity | `TitleBar` child content + `svg` logo | [TitleBar](https://gpui-kit.com/component/title-bar) | `TitleBar::window_options()` |
| Mode switch (`ModeSwitch`) | Themes/Editor/Settings | `ToggleGroup` + `Toggle` | [ToggleGroup](https://gpui-kit.com/component/toggle-group) | source order; segmented well in header |
| Store halves (`StoreHalves`) | Local/Online tabs | `TabBar` pill inside Store header | [Tabs](https://gpui-kit.com/component/tabs) | not global chrome |
| Header toolbar (`shell-menu-bar.tsx`) | open/save/export/find | `Toolbar` + `Button` from `Command::ALL` | [Toolbar](https://gpui-kit.com/component/toolbar) | no menu bar in source |
| Toolbar open actions | New, template, library, recent, import | `Toolbar` + `Button`; recent = `PopupMenu`/`Menu` | [Toolbar](https://gpui-kit.com/component/toolbar) | `Kbd` in tooltip |
| Toolbar document actions | Save, Save as, Activate, Export | `Toolbar`; Save primary; Export `Button` | [Toolbar](https://gpui-kit.com/component/toolbar) | disabled from session |
| Find assets | open finder | `Button` + `Kbd` tooltip; action binding | [Button](https://gpui-kit.com/component/button) | also command palette |
| Publish control + indicator | publish live / assign | `Button` primary + `Dialog`; status `Badge` | [Dialog](https://gpui-kit.com/component/dialog) | dialog hosts assign + QR |
| Pane switch (`rail.tsx`) | choose left pane | 46px rail of ghost `Button`s, icon + tooltip, `toggled` for the active slot | [Button](https://gpui-kit.com/component/button) | six panes; settings gear at the rail's foot |
| Pane chrome (`pane.tsx`) | icon, title, count, body, footer | `v_flex`; tab label; footer `Toolbar` | [Toolbar](https://gpui-kit.com/component/toolbar) | counts from same store |
| Pane footer actions (`LayerActions`) | object actions | `Toolbar` + ghost/danger `Button`s | [Toolbar](https://gpui-kit.com/component/toolbar) | filter ineligible actions |
| Document identity | name, dirty, rename | `Input` inline rename; dirty `Badge`/`Marker` | [Input](https://gpui-kit.com/component/input) | one owner |
| Display/zoom cluster (`display-switch.tsx`) | playback, zoom, lens | `Toolbar` + `ButtonGroup` + `Button`; lens `PopupMenu` | [Toolbar](https://gpui-kit.com/component/toolbar) | zoom % is a button trigger |
| Background playback | play/pause video | icon `Button` in zoom toolbar | [Button](https://gpui-kit.com/component/button) | state from player core |
| Canvas dock (`canvas-dock.tsx`) | contextual object actions | floating `Toolbar` over stage | [Toolbar](https://gpui-kit.com/component/toolbar) | hide when no document |
| Canvas context menu | right-click actions | `ContextMenu` from `Command::ALL` | [Menu](https://gpui-kit.com/component/menu) | grouped like source |
| Inspector toggle | collapse right pane | icon `Button` + fixed column state | [Button](https://gpui-kit.com/component/button) | persisted view state |
| Shortcut reference (`?`) | list all shortcuts | `Command` palette, or `Sheet` + `Kbd` rows | [Command](https://gpui-kit.com/component/command) | prefer palette |
| Status readout | object/selection counts | `StatusBar` left region | [StatusBar](https://gpui-kit.com/component/status-bar) | no bridge = empty |
| Status message / diagnostics | transient + persistent | `Notification`; persistent `StatusBar` item + `Alert` | [Notification](https://gpui-kit.com/component/notification) | one status owner per mode |
| Save state | saved/dirty/saving | `StatusBar` item + `Badge`/`Spinner` | [StatusBar](https://gpui-kit.com/component/status-bar) | live region |
| Publish indicator | live/offline | `StatusBar` item + `Badge` | [Badge](https://gpui-kit.com/component/badge) | live region |
| Cursor indicators | angle/size while dragging | custom paint overlay + `Label` | — | pointer-transparent |
| Held update notice | banner + actions | `Alert` + `Button`, or `Notification` with action | [Alert](https://gpui-kit.com/component/alert) | non-blocking |
| Window / tray / global shortcut | desktop integration | GPUI window + existing `crates/app/tray.rs` | — | keep native |

## Left panes

### Composition / Layers

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Layer tree (`role="tree"`) | `Tree` + `TreeItem`, custom row renderer | [Tree](https://gpui-kit.com/component/tree) | virtualized; expansion owned by app |
| Row expand/collapse | Tree's native folder press + arrow keys | [Tree](https://gpui-kit.com/component/tree) | no custom twisty |
| Kind mark / icon | `Icon` or `svg` in row | [Icon](https://gpui-kit.com/component/icon) | theme tokens |
| Inline rename | `Input` in the row on F2/double press | [Input](https://gpui-kit.com/component/input) | one editing row |
| Visibility / lock toggles | `Checkbox` or icon `Button` in row | [Checkbox](https://gpui-kit.com/component/checkbox) | label includes layer name |
| Enter / leave group | `MenuItem`s in row `ContextMenu`; keys | [Menu](https://gpui-kit.com/component/menu) | via command registry |
| Multi-select (shift/cmd) | app-managed selection set; Tree primary selection remains anchor | [Tree](https://gpui-kit.com/component/tree) | kit Tree is single-select |
| Drag reorder / reparent | custom GPUI drag + insertion marker | — | no kit tree reorder |
| Keyboard navigation | Tree arrows, Home/End, typeahead; app handles modifiers | [Tree](https://gpui-kit.com/component/tree) | one selection owner |
| Selection lead (`selection-lead.tsx`) | `Toolbar` above tree sections | [Toolbar](https://gpui-kit.com/component/toolbar) | arrange actions, lock marker |
| Footer actions | `Toolbar` pinned to pane footer | [Toolbar](https://gpui-kit.com/component/toolbar) | filtered by eligibility |

### Add

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Insert grid (`yunari-add-grid`) | `v_virtual_list` rows of tile buttons, or `h_flex` wrap | [VirtualList](https://gpui-kit.com/component/virtual-list) | tiles are `Button`s |
| Text tile | `Button` + `Icon`/glyph + `Label` | [Button](https://gpui-kit.com/component/button) | inserts text object |
| Shape tiles | `Button` + GPUI `svg`/path preview | [Button](https://gpui-kit.com/component/button) | path preview from model |
| Chart family tiles | `Button` + glyph | [Button](https://gpui-kit.com/component/button) | inserts chart |
| Tooltip | `Tooltip` | [Tooltip](https://gpui-kit.com/component/tooltip) | label only |

### Widgets

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Installed widget grid | `v_virtual_list` rows of card buttons | [VirtualList](https://gpui-kit.com/component/virtual-list) | card = `Button` |
| Widget thumbnail | `Image` or GPUI `svg` | [Image](https://gpui-kit.com/component/image) | lazy |
| Empty state | `Empty` | [Empty](https://gpui-kit.com/component/empty) | primary action |
| Count | pane tab label `Badge` | [Badge](https://gpui-kit.com/component/badge) | no separate control |

### Assets

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Asset card | `Button` card + `Image` thumb + `PopupMenu` actions | [Button](https://gpui-kit.com/component/button) | preview/place/replace/remove/type-token |
| Asset mark | `Icon`/`Tag` by kind | [Tag](https://gpui-kit.com/component/tag) | |
| Import file | GPUI native path prompt | — | no hidden web input |
| Find assets | opens finder `Dialog` | [Dialog](https://gpui-kit.com/component/dialog) | |
| Size meter (`AssetSizeMeter`) | `Progress` + `Label` | [Progress](https://gpui-kit.com/component/progress) | pass/total bytes |
| Font metadata form | `Form`/`field` + `Input`/`NumberInput`/`Select` | [Form](https://gpui-kit.com/component/form) | family/weight/style/licence/source/role |
| Place image modal | `Dialog` + size/placement fields | [Dialog](https://gpui-kit.com/component/dialog) | |
| Import video modal | `Dialog` + `Progress` + status `Label` | [Dialog](https://gpui-kit.com/component/dialog) | prepared/importing/done |
| Video trim modal | `Dialog` + custom video preview + `Slider` | [Slider](https://gpui-kit.com/component/slider) | duration/start/end |
| Asset preview modal | `Sheet` + `Image`/video + `DescriptionList` + actions | [Sheet](https://gpui-kit.com/component/sheet) | long detail |
| Replace/remove confirm | `AlertDialog` destructive | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | |
| Import errors | `Alert` inline + `Notification` | [Alert](https://gpui-kit.com/component/alert) | |

### Tokens

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Paints list | `v_virtual_list` rows: swatch + name + kind + uses + menu | [VirtualList](https://gpui-kit.com/component/virtual-list) | |
| Add colour | `Button` in section header | [Button](https://gpui-kit.com/component/button) | |
| Paint swatch | `ColorPicker` swatch trigger or `Marker` | [ColorPicker](https://gpui-kit.com/component/color-picker) | |
| Paint kind | `Select` Solid/Gradient | [Select](https://gpui-kit.com/component/select) | |
| Solid colour | `ColorPicker` + presets | [ColorPicker](https://gpui-kit.com/component/color-picker) | eyedropper availability per kit |
| Gradient editor | custom `GradientControl`: track, stops, angle | [ColorPicker](https://gpui-kit.com/component/color-picker) | no kit gradient widget |
| Gradient presets | `PopupMenu`/`ContextMenu` | [Menu](https://gpui-kit.com/component/menu) | |
| Delete/reassign token | `Select` reassignment + `AlertDialog` | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | |
| Type tokens list | `v_virtual_list` rows + add `Button` | [VirtualList](https://gpui-kit.com/component/virtual-list) | |
| Type face fields | `Form`/`field` + `Input`/`Select`/`NumberInput`/`ColorPicker` | [Form](https://gpui-kit.com/component/form) | family/weight/style/size/line/letter/colour |
| Font actions | `PopupMenu` import/adopt/remove | [Menu](https://gpui-kit.com/component/menu) | |
| Layer uses list | `v_virtual_list` + `Button`/`Link` to reveal | [VirtualList](https://gpui-kit.com/component/virtual-list) | |
| Token inspector | right inspector or `Sheet` | [Sheet](https://gpui-kit.com/component/sheet) | one editing surface |
| Preset context menu | `ContextMenu` | [Menu](https://gpui-kit.com/component/menu) | |

### Document

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Section frame | `Accordion` multiple, open by default | [Accordion](https://gpui-kit.com/component/accordion) | source keeps sections open |
| Theme metadata | `Form`/`field` + `Input`; tags `Tag` + add/remove `Button` | [Form](https://gpui-kit.com/component/form) | |
| Tags | `Input` + `Tag` chips + `Button`s | [Tag](https://gpui-kit.com/component/tag) | no tag input widget |
| Language | `Select` + sample `Label` | [Select](https://gpui-kit.com/component/select) | |
| Release version | `Input` or read-only `Label` | [Input](https://gpui-kit.com/component/input) | |
| Artboard shape | `Select` + `TabBar::segmented` where binary | [Tabs](https://gpui-kit.com/component/tabs) | |
| Ratio / orientation / resolution | `Select`; disabled where shape decides | [Select](https://gpui-kit.com/component/select) | |
| Size | `NumberInput` ×2 + unit `Label` | [NumberInput](https://gpui-kit.com/component/number-input) | inline validation |
| Artboard preview ratio | `Switch` + `NumberInput`/`Input` | [Switch](https://gpui-kit.com/component/switch) | |
| Credits | `v_virtual_list` or `DescriptionList` | [DescriptionList](https://gpui-kit.com/component/description-list) | link actions |

## Selection inspector

One right pane. Sections are `Accordion` with counts; closed sections keep
content mounted so drafts survive.

| Section / element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Inspector header | `Icon` + name `Input` + lock `Badge` | [Input](https://gpui-kit.com/component/input) | |
| Selection lead (`selection-lead.tsx`) | `Toolbar` of arrange actions | [Toolbar](https://gpui-kit.com/component/toolbar) | destructive styling |
| Section header | `Accordion` header + count `Badge` | [Accordion](https://gpui-kit.com/component/accordion) | all open by default |
| Content | `field` rows + extras | [Form](https://gpui-kit.com/component/form) | |
| Position | `NumberInput` pairs; shape geometry fields | [NumberInput](https://gpui-kit.com/component/number-input) | point text size read-only |
| Crop | custom GPUI paint overlay + handles + `Slider`/`NumberInput` | — | no kit crop |
| Layer | rotation `Slider` + `NumberInput`, opacity `Slider` | [Slider](https://gpui-kit.com/component/slider) | |
| Paint | panel material fields, glass fields, chart paint | [Form](https://gpui-kit.com/component/form) | |
| Advanced | `Collapsible` with chart advanced fields | [Collapsible](https://gpui-kit.com/component/collapsible) | starts closed |
| Text runs | `Accordion` per run; `Input`, `Select`, `NumberInput`, `ColorPicker`, remove `Button`; add `Button` | [Accordion](https://gpui-kit.com/component/accordion) | complex sub-surface stays app-owned |
| Run layout | `ToggleGroup` for align; `Switch` vertical text | [Toggle](https://gpui-kit.com/component/toggle) | |
| Chart content/paint | `Form`/`field` controls; chart preview from raster | [Form](https://gpui-kit.com/component/form) | renderer owns chart |
| Design token edit | `Popover` + `ColorPicker` | [Popover](https://gpui-kit.com/component/popover) | |
| Field controls | `Input`, `NumberInput`, `Select`, `Switch`, `Checkbox`, `Slider`, `ColorPicker` | [Form](https://gpui-kit.com/component/form) | one property-row renderer |
| Error/refusal | `Alert` + optional `Notification` | [Alert](https://gpui-kit.com/component/alert) | no silent refusal |
| Locked selection | hide writing fields; show lock `Badge` | [Badge](https://gpui-kit.com/component/badge) | as source |

## Canvas and stage

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Artboard render | raster `Image` from `DisplayList`; redraw on revision | [Image](https://gpui-kit.com/component/image) | preserves text shaping, letter spacing, vertical text, SVG, glass |
| Live video nodes | custom GPUI video element over artboard bounds, clipped | — | spike item; otherwise raster frame |
| Selection box/handles/marquee | custom GPUI paint + drag/resize | — | no DOM handles |
| Snap guides/rulers | custom GPUI paint | — | |
| Crop UI | custom paint + handles; Escape/Return actions | — | |
| Hit testing | Rust scene model, not Fabric | — | port from `scene-fabric` |
| Canvas context menu | `ContextMenu` on stage | [Menu](https://gpui-kit.com/component/menu) | commands by selection |
| Zoom/fit/actual/fullscreen | `Toolbar` + `ButtonGroup` + `PopupMenu` | [Toolbar](https://gpui-kit.com/component/toolbar) | fullscreen via GPUI window |
| Display lenses | `PopupMenu` radio group | [Menu](https://gpui-kit.com/component/menu) | Landscape/Portrait/Square |
| Document identity | top-left `Label` + inline rename | [Input](https://gpui-kit.com/component/input) | |
| Cursor readouts | absolute `Label`, pointer-transparent | — | never intercept pointer |
| Drag and drop files | GPUI external path drop → import asset | — | native |
| Canvas keyboard | GPUI actions/keybindings | [Action](https://gpui-kit.com/docs/action) | one keymap |

## Themes / Library (Store mode)

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Local/online halves | `TabBar` | [Tabs](https://gpui-kit.com/component/tabs) | |
| Search | `Input` with search prefix and clear | [Input](https://gpui-kit.com/component/input) | debounced |
| Facet row (`FacetRow`) | `Popover` per facet + search `Input` + `Checkbox` rows + count `Badge` | [Popover](https://gpui-kit.com/component/popover) | six facets, source order |
| Sort | `Select` | [Select](https://gpui-kit.com/component/select) | |
| Density | `Slider` with end labels | [Slider](https://gpui-kit.com/component/slider) | Comfortable ↔ Dense |
| Gallery | `v_virtual_list` rows of cards, keyed by theme id | [VirtualList](https://gpui-kit.com/component/virtual-list) | card = `Button` + `Image` + `Badge` + `PopupMenu` |
| Card actions | `PopupMenu`/`ContextMenu` | [Menu](https://gpui-kit.com/component/menu) | install/update/remove/export/assign |
| Install/update action | `Button` state machine + `Progress` | [Button](https://gpui-kit.com/component/button) | |
| Submit button | `Button` primary | [Button](https://gpui-kit.com/component/button) | |
| Status badge | `Badge` | [Badge](https://gpui-kit.com/component/badge) | local/remote/update/error |
| Item detail | `Sheet` (desktop), `Dialog` fallback | [Sheet](https://gpui-kit.com/component/sheet) | preview/facts/tags/actions |
| Assign display | `Dialog` + `Select` + confirm | [Dialog](https://gpui-kit.com/component/dialog) | |
| Delete confirm | `AlertDialog` destructive | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | |
| Empty/loading/error | `Empty`, `Skeleton`, `Alert` + retry `Button` | [Empty](https://gpui-kit.com/component/empty) | replaces silent blank |
| Held update notice | `Alert` + `Button` | [Alert](https://gpui-kit.com/component/alert) | |

## Settings

Use the kit `Settings` component: it already owns the sidebar, page stack,
groups and items.

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Page navigation | `Settings` sidebar; fallback `Sidebar` + page state | [Settings](https://gpui-kit.com/component/settings) | source page order |
| Page frame | `SettingPage` + heading `Label` + intro | [Settings](https://gpui-kit.com/component/settings) | |
| Group / section | `SettingGroup` / `GroupBox` + hint | [GroupBox](https://gpui-kit.com/component/group-box) | |
| Field rows | `SettingItem` + `field` + kit control | [Form](https://gpui-kit.com/component/form) | |
| Displays page: display list | `Radio`/`RadioGroup` + card rows; edit/remove `Button` | [Radio](https://gpui-kit.com/component/radio) | |
| Display presentation | `TabBar::segmented`, `Slider`, `Switch` | [Tabs](https://gpui-kit.com/component/tabs) | shape/rotation/always-on-top/render mode |
| Panel controls | `v_virtual_list` + `Button`s + `Dialog` with `Slider`/`NumberInput`/`Switch` | [VirtualList](https://gpui-kit.com/component/virtual-list) | |
| Device assignment | `Select` per group | [Select](https://gpui-kit.com/component/select) | |
| Add display | `Select` monitor + `Input` name + `Button` | [Select](https://gpui-kit.com/component/select) | |
| Panels section | rows + `DescriptionList` + add `Button` | [DescriptionList](https://gpui-kit.com/component/description-list) | |
| Autostart | `Switch` | [Switch](https://gpui-kit.com/component/switch) | |
| Hardware | device `Select`s + `Input`s | [Select](https://gpui-kit.com/component/select) | |
| SignalRGB | port `NumberInput`, device rows, add `Button` | [NumberInput](https://gpui-kit.com/component/number-input) | |
| Units | time zone/measurement/per-unit `Select`s | [Select](https://gpui-kit.com/component/select) | |
| Readings sources | `v_virtual_list` + edit/test/delete icon `Button`s; form `Dialog` + `Form` | [Dialog](https://gpui-kit.com/component/dialog) | password `Input` content type |
| Weather | provider `Select`, label `Input`, lat/long `NumberInput`, location `Switch`, poll `NumberInput`, search `Input` + `List` | [Input](https://gpui-kit.com/component/input) | |
| Appearance | palette `Radio` + follow-system `Switch` | [Radio](https://gpui-kit.com/component/radio) | |
| Previews | ratio rows, add/remove `Button`, name `Input`, reason `Label` | [Button](https://gpui-kit.com/component/button) | |
| Path presets | built-in/custom rows, remove icon `Button`, name/data `Input`, add `Button` | [Input](https://gpui-kit.com/component/input) | |
| Keyboard | rows with chord `Kbd`, rebind `Button`, listening state, reset, conflict `Alert` | [Kbd](https://gpui-kit.com/component/kbd) | capture via actions |
| Asset sources | credential rows + `Input` + save/test/clear `Button` + `Badge` | [Form](https://gpui-kit.com/component/form) | masked password |
| Extensions | plugin manager `v_virtual_list`, install path prompt, enable `Switch`, move/remove `Button` | [VirtualList](https://gpui-kit.com/component/virtual-list) | |
| About | `DescriptionList` + version `Label` | [DescriptionList](https://gpui-kit.com/component/description-list) | |
| Plugin settings | dynamic `Form`/`field` from schema; plugin page custom webview | [Form](https://gpui-kit.com/component/form) | webview is app-owned |

## Asset finder

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Modal frame | `Dialog` full-size + `TabBar` categories | [Dialog](https://gpui-kit.com/component/dialog) | |
| Search | `Input` | [Input](https://gpui-kit.com/component/input) | |
| Filters | `ToggleGroup` or `Select` | [Toggle](https://gpui-kit.com/component/toggle) | |
| Source chips | `ToggleGroup` of `Toggle` chips | [Toggle](https://gpui-kit.com/component/toggle) | |
| Connections | icon `Button` → settings page | [Button](https://gpui-kit.com/component/button) | |
| Grid | `v_virtual_list` rows of cards; `Image`/`svg` preview | [VirtualList](https://gpui-kit.com/component/virtual-list) | icons/images/fonts/palettes/gradients |
| Card state | `Badge`/`Progress`/`Icon` overlay | [Badge](https://gpui-kit.com/component/badge) | import/busy/failed |
| Font preview | `Label` with font family/size | [Label](https://gpui-kit.com/component/label) | |
| Preview modal | `Sheet`/`Dialog` + `Image`/video + `DescriptionList` | [Sheet](https://gpui-kit.com/component/sheet) | |
| Footer actions | `Toolbar` with primary import/place | [Toolbar](https://gpui-kit.com/component/toolbar) | |
| Empty/loading/error | `Empty`, `Skeleton`, retry `Button` | [Empty](https://gpui-kit.com/component/empty) | |
| Wallpaper tools | `Button`s + native path prompt | [Button](https://gpui-kit.com/component/button) | refresh/import/guide |
| Wallpaper Engine guide | `Dialog` + `TextView` markdown | [TextView](https://gpui-kit.com/component/text-view) | |
| Plugin body | custom webview inside finder | — | capability and allow-list |

## Dialogs, sheets and overlays

| Yunari dialog | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| New document chooser | `Dialog` + `Select` + `NumberInput` + primary/cancel | [Dialog](https://gpui-kit.com/component/dialog) | |
| Theme library chooser | `Dialog` + `Input` + `v_virtual_list` + actions | [Dialog](https://gpui-kit.com/component/dialog) | |
| Save as | `Dialog` + `Input` | [Dialog](https://gpui-kit.com/component/dialog) | |
| Theme conflict | `Dialog` with three `Button`s | [Dialog](https://gpui-kit.com/component/dialog) | |
| Delete confirmation | `AlertDialog` destructive | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | |
| Draft recovery | `AlertDialog` or `Dialog` with recover/discard | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | discard destructive |
| Replace document | `AlertDialog` | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | |
| Save cleanup | `Dialog` + `Checkbox` list + select-all + confirm | [Checkbox](https://gpui-kit.com/component/checkbox) | |
| Font import | `Dialog` + font metadata `Form` | [Form](https://gpui-kit.com/component/form) | |
| Output bake | `Dialog` + `Progress` + status | [Progress](https://gpui-kit.com/component/progress) | |
| Video import / trim | `Dialog` + `Progress` + custom video + `Slider` | [Slider](https://gpui-kit.com/component/slider) | |
| Place image | `Dialog` + placement controls | [Dialog](https://gpui-kit.com/component/dialog) | |
| Asset preview | `Sheet` | [Sheet](https://gpui-kit.com/component/sheet) | |
| Publish/assign + QR | `Dialog` + `Select` + custom QR paint | [Dialog](https://gpui-kit.com/component/dialog) | QR via `qrcode` crate |
| Shortcut reference | `Sheet` + `Kbd` rows, or `Command` | [Command](https://gpui-kit.com/component/command) | |
| Wallpaper guide | `Dialog` + `TextView` | [TextView](https://gpui-kit.com/component/text-view) | |
| Plugin frame | `Dialog`/pane + custom webview | — | behind feature and allow-list |
| Toasts | `Notification` | [Notification](https://gpui-kit.com/component/notification) | success/error/info |
| Tooltips | `Tooltip` | [Tooltip](https://gpui-kit.com/component/tooltip) | all icon buttons |

## Host admin chooser (`packages/host/src/admin`)

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Chooser page | `v_flex` + header `Label` + grouped sections | [Label](https://gpui-kit.com/component/label) | separate surface, same product UI |
| Display/template/theme group labels | `Label` section headings | [Label](https://gpui-kit.com/component/label) | |
| Theme row (`theme-row.tsx`) | `Button` card + `Image`/placeholder + name/author + current `Badge` | [Button](https://gpui-kit.com/component/button) | |
| Edit action | icon `Button` opening editor window | [Button](https://gpui-kit.com/component/button) | |
| Delete action | icon `Button` + `AlertDialog` destructive | [AlertDialog](https://gpui-kit.com/component/alert-dialog) | recycle-bin copy |
| Cost warning | `Alert` + `Badge` with live-region semantics | [Alert](https://gpui-kit.com/component/alert) | |
| Status line | `Label` in a live region | [Label](https://gpui-kit.com/component/label) | |
| Empty state | `Empty` | [Empty](https://gpui-kit.com/component/empty) | |

## Desktop player surface

| Element | gpui-kit proposal | Docs | Notes |
|---|---|---|---|
| Theme canvas | existing GPUI player element; video themes use player core | — | full-bleed |
| Theme chooser strip | `Select` or `Sheet` list | [Select](https://gpui-kit.com/component/select) | paired/own theme |
| Top/bottom text strips | `Label` absolute overlay | [Label](https://gpui-kit.com/component/label) | safe areas on mobile |
| Display controls | `Toolbar`: fullscreen `Button`, keep-awake `Toggle`, dim `Slider`, status `Label` | [Toolbar](https://gpui-kit.com/component/toolbar) | auto-hide on idle |
| Load failure | `v_flex` + heading/lede/reason + retry `Button` + host `Button`/link | — | app-owned failure view |
| Availability notice | `Alert` + `Button` | [Alert](https://gpui-kit.com/component/alert) | |

## Mobile player (separate NSH app)

The mobile app is a player only. It never links the editor, inspector, store,
settings or asset finder UI. It reuses the portable model/render/raster/player
core and packages with NSH for iOS and Android.

| Element | NSH proposal | Docs | Notes |
|---|---|---|---|
| Theme canvas | `video()` for video themes; `canvas()`/`image()` for rendered themes | [Video](https://nsh.dev/elements/video) | full-bleed, safe areas |
| Theme chooser | `select()` or `sheet()` list | [Select](https://nsh.dev/elements/select) | paired/own theme |
| Top/bottom text strips | `view` + `text` | [Text](https://nsh.dev/elements/text) | safe-area insets |
| Display controls | `toolbar()`: fullscreen `button`, keep-awake `toggle`, dim `slider`, status `text` | [Toolbar](https://nsh.dev/elements/toolbar) | auto-hide |
| Load failure | `view` + retry `button` | [Button](https://nsh.dev/elements/button) | |
| Availability notice | banner `view` + `button` | [View](https://nsh.dev/elements/view) | |
| Android live wallpaper / iOS wallpaper | not an NSH element | — | native host/service work, not this app |

The full NSH player mapping remains in `editor-ui-nsh.md`; that document is no
longer the editor map.

## Source atoms → gpui-kit

| Source atom | gpui-kit |
|---|---|
| `ControlText`, `ControlWell` inputs | `Input` / `Textarea` |
| `ControlNumber`, `ControlPair` | `NumberInput` × 1/2 |
| `ControlSelect`, `ControlSwatch` | `Select` / `ColorPicker` |
| `ControlSegmented` | `ToggleGroup` / `TabBar::segmented` |
| `ControlToggle`, `ControlTogglePair` | `Switch` |
| `ControlChoiceRow` | `Radio` + `RadioGroup` |
| `ControlSlider` | `Slider` + `NumberInput` |
| `ControlHint` | `Tooltip` |
| `ControlIconButton` | icon `Button` + `Tooltip` |
| `ControlTooltip` | `Tooltip` |
| `ColourPicker`, `ColourPickerPanel` | `ColorPicker` |
| `GradientEditor` | custom `GradientControl` |
| `Dialog`, `openModal` | `Dialog`, `AlertDialog`, `Sheet` |
| `InspectorSection` | `Accordion` / `Collapsible` |
| `PresetContextMenu` | `ContextMenu` |
| `SensorKeyDot` | `Badge` / `Marker` |
| Base UI Toggle/ToggleGroup | `Toggle` / `ToggleGroup` |
| Base UI Select | `Select` |
| Base UI Switch | `Switch` |
| Base UI Checkbox | `Checkbox` |
| Base UI RadioGroup | `Radio` |
| Base UI Slider | `Slider` |
| Base UI Popover/Menu/Dialog | `Popover` / `Menu` / `Dialog` |
| Base UI Accordion | `Accordion` |
| Base UI Tabs | `Tabs` / `TabBar` |
| Fabric canvas | raster `Image` + custom GPUI paint |

## Gaps and custom work (no kit component)

- **Gradient authoring:** stop track, stop dragging, presets, angle. Compose
  `ColorPicker`, `Slider` and custom paint; do not wait for a kit widget.
- **Crop / transform handles / snap guides:** custom GPUI paint and hit
  testing; the kit has no scene editor.
- **Tree multi-select and drag-reorder:** kit Tree is single-select and has no
  reorder; app owns selection and drag.
- **QR code:** render with a Rust QR crate into a GPUI image/canvas.
- **Live video inside the artboard:** custom GPUI video element over the raster,
  or rasterised frames; decide in the spike.
- **Glass / backdrop effects:** use the existing `2026-11-16-glass-backdrop-probe.md`
  result; kit surfaces are styled, but arbitrary backdrop blur remains custom.
- **Plugin frame / webview:** custom platform embedding behind a feature and an
  allow-list; not a kit component.
- **Mobile live wallpaper / background playback:** native platform service, not
  NSH and not the desktop editor.

## What changes from the source (greenfield choices)

1. **Fixed three-region shell** with `h_flex`, matching the source's 246px /
   1fr / 276px grid. Pane collapse changes the columns; no drag-resize.
2. **Kit chrome and theme** instead of raw `div()` and hard-coded colours;
   `ActiveTheme` owns every visual token.
3. **One property-row renderer** built from kit `field` + controls; no bespoke
   field framework.
4. **Kit `ColorPicker`** replaces the bespoke React colour panel; gradient and
   crop stay custom.
5. **`Sheet` for detail/edit panels** (store item, asset preview, token edit)
   instead of nested modals.
6. **`AlertDialog` for destructive confirms**, `Notification` for transient
   status; the status bar keeps persistent facts.
7. **Virtual lists everywhere lists can grow**; `Tree` only where hierarchy is
   semantic.
8. **Native path prompts, clipboard and file drops** replace web file inputs.

## Phases and acceptance gate

**P0 — reference (current).** Screen map, source captures, component map,
divergence list, per-screen checklists. Exit: reference doc accepted.

**P1 — shell.** Modes, title bar, toolbar, fixed three-region editor, status
bar, theme applied. Exit: editor opens at 1080p with reference layout; no
clipped control row; screenshot pair accepted.

**P2 — panels.** Layers tree, inspector sections per kind, palette, assets,
typography. Exit: each section's checklist passes; selection shows the right
inspector.

**P3 — commands and canvas.** Command surface, context menus, shortcuts,
selection/transform overlays, snap guides, zoom/fit. Exit: every `Command::ALL`
entry is reachable and keyboard-driven; canvas feedback matches source.

**P4 — dialogs and modes.** Settings, store, asset finder, widget/path/
typography pickers, library, MCP, save/error states. Exit: each dialog checklist
passes.

**P5 — parity resume.** Probes and review loop run on the new surface; editor
rows E1–E11 enter the matrix.

**M0 — mobile player (parallel, separate lane).** NSH shell, theme load, video/
canvas display, chooser, failure states, iOS/Android packaging. Starts only
after P1; shares portable crates. Does not block the editor.

Each screen passes only when all four hold, with evidence in the ledger:

1. **Function** — every source control exists, is reachable and matches source
   behaviour, refusals and defaults.
2. **Information architecture** — same grouping and persistence; primary action
   in the same place.
3. **State** — selected, hover, focus, disabled, empty, loading, error and
   keyboard focus are visible and distinguishable.
4. **Craft** — at 1920x1080 and minimum window size: no clipped label, no
   overflowing row, readable contrast, hit targets at least 24px, consistent
   spacing and alignment.

Evidence: a 1920x1080 port capture beside the source reference, plus a keyboard
walkthrough note. The verifier regenerates both; the fixer's screenshots are not
enough.

## Workflow

Same loop mechanics as the parity review:

- one Orca workspace per lane, launched with `orca agent launch`;
- lanes are screen families with disjoint files (shell, panels, dialogs,
  canvas), each with reference shots and a checklist;
- fixer opens a PR per screen family; verifier regenerates the screenshot pair
  and walks the checklist; orchestrator merges one PR at a time, retires the
  workspace, and records the screen in `docs/parity/ledger/editor.md`;
- a screen that fails twice is parked with `needs:human` and the failure
  reports; the lane moves on.

No new parity-bug lanes start until P1 exits.

## Risks

- **Kit components used as decoration** — the gate checks interaction and
  states, not presence in a diff.
- **Raw `div()` returning** — review rejects interactive non-kit controls
  except canvas geometry and drag surfaces.
- **Theme drift** — literals fail review; only `ActiveTheme` tokens.
- **Dock temptation** — fixed shell uses `h_flex`, matching the source grid;
  Dock is a separate product decision.
- **Canvas scope creep** — the stage is raster; overlays stay small and tested.
- **Two UI stacks** — the mobile player is deliberately separate; share
  portable crates, not UI components.
- **Screenshot theatre** — evidence is a same-size, same-state pair judged
  against the checklist.

## Approval checklist

- [ ] Desktop editor and desktop player stay on gpui-kit; NSH is mobile-player
      only.
- [ ] Fixed three-region shell with `h_flex` (246 / 1fr / 276); no Dock for the
      editor.
- [ ] `gpui_kit::component` + `ActiveTheme` are the only widget and theme
      layers; `crates/ui` holds compositions only.
- [ ] Left panes: Tree + virtual lists + Toolbar footers.
- [ ] Inspector: Accordion sections + kit form controls; runs and crop custom.
- [ ] Stage: raster artboard + custom GPUI overlays + native drag/drop.
- [ ] Store, Settings, Asset finder, dialogs and host admin mappings accepted.
- [ ] Desktop player mapping accepted; mobile player is a separate NSH app.
- [ ] Custom gaps (gradient, crop, QR, tree selection/drag, video, webview,
      glass) accepted as app work.
