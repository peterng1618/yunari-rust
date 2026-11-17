# Editor UI in NSH — element map and design

**Status:** proposed 2026-11-17, for approval. Supersedes the widget layer of
`editor-ui-components.md` (gpui-component). Source of truth for Yunari UI is
the pinned tree `1241866e53df252307e22ff3c51c842f89e458c8` in
`D:\git-repos\yunari`, `src/web/packages/editor/src`.

This is not a line-by-line React translation. Where the source has a bespoke
control, the table names the NSH base component that owns the same behaviour,
and the composition we will ship. The app owns every look; NSH owns press,
focus, keyboard, popup placement, accessibility and text editing.

## Rules

1. **Every interactive element is an NSH base control.** A raw
   `view().on_press(..)` is only allowed for canvas geometry and drag surfaces.
2. **One wrapper per control.** `crates/ui` wraps `nsh_base` once (`AppButton`,
   `IconButton`, `FieldRow`, `Section`, `TokenRow`, …); screens never style a
   base control directly.
3. **Theme tokens only.** Colours, radii, spacing, type and states come from
   the app theme, never literals.
4. **Source IA unless marked "change".** The reference map and acceptance
   checklist still decide placement.
5. **The stage is raster + Canvas.** `DisplayList` renders through the
   existing rasteriser into an `Image`; selection, guides, crop and rulers use
   `Canvas`. NSH Canvas cannot shape vertical text, letter-spaced runs or SVG
   images, and the raster path already matches the goldens.
6. **NSH base is unstyled.** No screen ships until its controls have a visible
   rest/hover/focus/pressed/disabled state and a keyboard path.

## Pre-styled components: what NSH actually ships

There is **no published pre-styled component library**. `nsh/base` is the
unstyled behaviour layer, and the official position (`docs/goals.md`) is that
applications and third-party libraries own the styled layer, the role
`gpui-component` plays for GPUI.

The Gallery's look is real code, but it is an example application, not an API:

- `examples/nsh-gallery-ui/` — the TSX Gallery's private styled layer
  (`package.json` says `"private": true`; README: "maintained with these
  examples, rather than published as an application API"). It holds the
  canonical light/dark tokens and `look/` modules for button, field, menu,
  tabs, overlay, surface, choice, listbox, color, workspace (Dock/Resizable),
  tree, command, data table and scrollbar.
- `examples/gallery-rs/native/src/ui/` — the Rust Gallery's smaller equivalent:
  `theme.rs` (AdaptiveColor roles, finishes, radii, motion, focus ring),
  `parts.rs` (`Card`, `Section`, `Row`, `Badge`, `Button`), `controls.rs`
  (toggle/segmented), `page.rs`, `icons.rs`, `sheet.rs`.
- `examples/gallery-rs/native/src/demos/` — usage of every component.

Decision: **port the architecture, not the skin.** `crates/ui` will follow the
Gallery's pattern — a `theme` module of roles and finishes, `look/` functions
that return style props spread on `nsh_base` components, and thin wrappers —
but with Yunari's own dark studio tokens, not the Gallery's graphite/citrus
palette. The Gallery is the quality bar and the worked reference for every
state; it is not a dependency and gets no compatibility guarantee.

`nsh-ui-mobile` exists on crates.io at 0.1.1 but is not in the current NSH
tree (last published 2026-10-18); do not build on it.

## Recommended shell

```
native window title bar + application menu          nsh/desktop
┌──────────────────────────────────────────────────────────────┐
│ brand | mode toggle | file/edit toolbar | publish            │  header: toolbar
├───────────────┬──────────────────────────────┬───────────────┤
│ left dock     │ stage                        │ right dock    │
│ Layers        │  raster artboard             │ Inspector     │
│ Add           │  Canvas overlays             │  sections     │
│ Widgets       │  zoom/display cluster        │  fields       │
│ Assets        │  canvas context menu         │               │
│ Tokens        │                              │               │
│ Document      │                              │               │
├───────────────┴──────────────────────────────┴───────────────┤
│ status strip: counts · message · diagnostics · save · live   │  footer: view
└──────────────────────────────────────────────────────────────┘
```

- `dock_area()` with `DockAreaLayout`, `DockLayout::tabs()`,
  `DockRegion`, `DockPlacement::{Left,Right}`. Left dock holds the six editor
  panes as tabs; right dock holds the inspector. `.locked(true)` keeps the
  arrangement fixed for v1 (tabs, resizing and dock open/close still work).
- Why Dock rather than the source's rail plus one hidden pane: it gives native
  tab semantics, keyboard switching, resizable docks, persisted layout and a
  compact drawer presentation for free. The source rail becomes the left tab
  strip with icon + tooltip.
- The header is a `toolbar()`: `toggle_group()` for modes, `toolbar_group()`s
  for document lifecycle / document actions / tools, `button()` for publish.
- The application menu is the OS menu (`set_application_menu`), not a drawn
  menu bar. `command()` provides the keyboard palette.
- The status strip is a plain `view` with `text`, because NSH has no app-level
  status bar component. Each mode publishes its own facts, as the source does.

```rust
use nsh_base::dock::{
    DockAreaLayout, DockLayout, DockPanel, DockPlacement, DockRegion, DockTab,
};
use nsh_base::{dock_area, theme};

let left = DockRegion::new(
    DockLayout::tabs()
        .panel("layers")
        .panel("add")
        .panel("widgets")
        .panel("assets")
        .panel("tokens")
        .panel("document"),
)
.size(px(288.0));
let right = DockRegion::new(DockLayout::tabs().panel("inspector")).size(px(300.0));
let layout = DockAreaLayout::new(DockLayout::tabs().panel("stage"), None)
    .with_dock(DockPlacement::Left, left)
    .with_dock(DockPlacement::Right, right);

dock_area()
    .layout(move || layout.clone())
    .locked(true)
    .panels(vec![
        // closable(false) for every editor panel; the inspector may close.
        DockPanel::new("stage").closable(false).zoomable(false),
    ])
    .render_tab(|tab: &DockTab| self.tab(tab))
    .render_panel(|panel: &DockPanel| self.panel(panel))
    .flex_1()
```

(`DockPanel`/`panels` field names are the documented `DockPanelOptions`; the
spike settles the exact builder names against the installed crate.)

If the source rail must be preserved exactly, the fallback is
`h_resizable` + a vertical `toolbar()` of `toggle()`s + one `resizable_panel`
for the pane and one for the inspector. Recommendation remains Dock.

## Reusable UI layer (`crates/ui`)

| Wrapper | Built from | Replaces |
|---|---|---|
| `AppButton` | `button()` + theme + `.disabled_style` + `.focus_visible` | all raw `<button>` |
| `IconButton` | `button()` + `tooltip()` + `.accessibility_label` | `ControlIconButton`, rail slots |
| `LinkButton` | `link()` | host/help links |
| `FieldRow` | `view` + `text` + control + error/hint | `ControlRow`, `ControlWell`, `SettingField` |
| `TextControl` | `input()` (single/multiline) | `ControlText`, controlled wells |
| `NumberControl` | `number_input()` | `ControlNumber` |
| `PairControl` | two `NumberControl`s in a row | `ControlPair`, `ControlTogglePair` |
| `ChoiceControl` | `radio_group()` + `radio()` | `ControlChoiceRow` |
| `SegmentedControl` | `toggle_group()` + `toggle()` | `ControlSegmented` |
| `SelectControl` | `select()` + `SelectItem` | `ControlSelect`, `ControlSwatch` |
| `ToggleControl` | `switch()` | `ControlToggle` |
| `SliderControl` | `slider()` + `number_input()` | `ControlSlider` |
| `ColorControl` | `color_picker()` + area/slider/field/swatches | bespoke colour picker |
| `GradientControl` | custom: stops list + `color_picker` + `slider` + `canvas` preview | `GradientEditor` |
| `PropertySection` | `accordion()` / `collapsible()` | `InspectorSection` |
| `DialogSurface` | `dialog()` + header/footer + `dialog()` close | `Dialog` |
| `Confirm` | `alert_dialog()` | destructive custom dialogs |
| `Toast` | `ToastManager` + `toast_stack()` | status messages |
| `Badge` / `Tag` / `Kbd` | `view` + `text` + tokens | pills, status badges, shortcut chips |
| `EmptyState` | `view` + icon + `text` + action | `FinderState`, empty store states |
| `SkeletonRow` | `view` + token fill + animation | `SkeletonCard` |

Every wrapper takes an accessibility label; icon-only controls are not allowed
without one.

## Shell and navigation

| Yunari element (source) | Job | NSH proposal | Change / notes |
|---|---|---|---|
| Brand (`<strong>`) `shell-layout.tsx` | identity | `text` + `svg` logo in header | no behaviour |
| Mode switch (`ModeSwitch`) | one of Themes/Editor/Settings | `toggle_group()` + three `toggle()`s, one pressed | keep order Themes, Editor, Settings; gear still opens Settings |
| Store halves (`StoreHalves`) | Local / Online tabs | `tabs()` or `toggle_group()` in header | model as `tabs()` with a panel below |
| Shell menu actions (`shell-menu-bar.tsx`) | File/Edit/View/Theme/Help commands | native `set_application_menu`, `MenuItem` + roles + accelerators | no drawn menu bar |
| Toolbar open actions | New, template, open library, open recent, import | `toolbar_group()` of `button()`s; open recent = `menu()` | tooltips carry `Kbd` label |
| Toolbar document actions | Save, Save as, Activate, Export theme/widget | `toolbar_group()`; primary weight for Save | disabled states from session |
| Find assets | open finder | `button()` + `on_key("mod+shift+f")` | command palette entry too |
| Publish control + indicator | publish live / assign | `button()` primary + `dialog()`; indicator = `Badge` in status | dialog hosts assign + QR |
| Rail (`rail.tsx`) | choose left pane, settings gear | left `DockArea` tab strip; gear = `button()` in header | recommendation: Dock tabs; fallback vertical `toolbar` + `toggle` |
| Pane chrome (`pane.tsx`) | icon, title, count, body, footer | `render_tab` for icon/title; panel content = `view`; footer = `toolbar()` | counts from same store as before |
| Pane footer actions (`LayerActions`) | object actions for selection | `toolbar()` + `IconButton`s, destructive style | filter ineligible actions, as source |
| Document identity | name, dirty mark, rename | `text` + inline `input()` on rename | one owner |
| Display/zoom cluster (`display-switch.tsx`) | playback, zoom −/%, +, lens menu | `toolbar()` + buttons + `popover()`/`menu()`; zoom % is a `button` trigger | lens groups = `menu` groups; fit/actual/fullscreen = menu items |
| Background playback | play/pause video | `IconButton` in zoom toolbar | state from renderer |
| Canvas dock (`canvas-dock.tsx`) | contextual object actions | floating `toolbar()` over stage | same registry; hide when no document |
| Canvas context menu | right-click actions | `context_menu()` on stage | grouped by command registry |
| Inspector toggle | collapse right dock | `IconButton`; `area.toggle_dock("right")` | dock state persisted |
| Shortcut reference (`?`) | list all shortcuts | `command()` palette with shortcuts, or `sheet()` + `text` rows | prefer command palette; keep `?` as sheet |
| Status readout | object/selection counts | `text` in status strip | no bridge = render nothing |
| Status message / diagnostics | transient + persistent reports | `toast()` for transient; `text` + severity icon in strip for persistent | one status owner per mode |
| Save state | saved/dirty/saving | `Badge` in status strip | live region |
| Publish indicator | live/offline | `Badge` | live region |
| Cursor indicators | angle/size while dragging | `Canvas` overlay or absolute `view` + `text` | must not intercept pointer |
| Held update notice | banner + actions | `view` banner + `button`s, or `toast` with actions | non-blocking |
| Window / tray / global shortcut | desktop integration | `nsh/desktop`: `window()`, `set_tray`, `register_global_shortcut`, `displays` | native, no HTML chrome |

## Left panes

### Composition / Layers

| Element | NSH proposal | Notes |
|---|---|---|
| Layer tree (`role="tree"`) | `tree()` + `TreeItem { id, label, children }`, `render_item` | virtualized; expanded/selected owned by app |
| Row expand/collapse | tree's native folder press + arrow keys | no custom twisty button |
| Kind mark / icon | `svg` or `text` glyph in row | theme tokens |
| Inline rename | `input()` shown in the row on F2/double press | keep one editing row |
| Visibility / lock toggles | `IconButton` or `toggle()` in row | label includes layer name |
| Enter / leave group | `menu_item()`s in row context menu; keys | via command registry |
| Multi-select (shift/cmd) | app-managed selected `Vec<String>` in row press; tree's primary selection remains anchor | NSH Tree is single-select; render multi-selected rows with selected style |
| Drag reorder / reparent | custom `PanResponder` on row + drop target row, with insertion marker | NSH Tree has no drag reorder |
| Keyboard navigation | tree arrows, Home/End, typeahead; app handles multi-select modifiers | |
| Selection lead (`selection-lead.tsx`) | `toolbar()` above tree sections | arrange actions, lock marker |
| Footer actions | `toolbar()` pinned to pane footer | filtered by eligibility |

### Add

| Element | NSH proposal | Notes |
|---|---|---|
| Insert grid (`yunari-add-grid`) | `virtual_list()` with `columns(n)` or `view` wrap rows | tiles are buttons |
| Text tile | `button()` + glyph + label | inserts text object |
| Shape tiles | `button()` + `PathGlyph` via `svg`/`canvas` | path preview from model |
| Chart family tiles | `button()` + glyph | inserts chart |
| Tooltip | `tooltip()` | label only |

### Widgets

| Element | NSH proposal | Notes |
|---|---|---|
| Installed widget grid | `virtual_list()` columns + card views | card = `button()` |
| Widget thumbnail | `image()`/`svg()`/`canvas()` | lazy |
| Empty state | `EmptyState` wrapper | |
| Count | pane tab title count | |

### Assets

| Element | NSH proposal | Notes |
|---|---|---|
| Asset card | `button()` card + `image`/`video` thumb + `menu()` for actions | preview/place/replace/remove/type-token |
| Asset mark | `svg`/`text` by kind | |
| Import file | `pick_files()` native | no hidden `<input type=file>` |
| Find assets | opens finder `dialog()` | |
| Size meter (`AssetSizeMeter`) | `progress()` + `text` | pass/total bytes |
| Font metadata form | `FieldRow` + `input`/`number_input`/`select` | family/weight/style/licence/source/role |
| Place image modal | `dialog()` + size/placement controls | |
| Import video modal | `dialog()` with `progress()` + status text | steps prepared/importing/done |
| Video trim modal | `dialog()` + `video()` preview + `slider()` range | duration/start/end |
| Asset preview modal | `sheet()` (desktop) with `image`/`video`/`text` facts + actions | long detail benefits from sheet |
| Replace/remove confirm | `alert_dialog()` destructive | |
| Import errors | inline `text` + `toast()` | |

### Tokens

| Element | NSH proposal | Notes |
|---|---|---|
| Paints list | `virtual_list()` rows; each row = swatch + name + kind + uses + menu | |
| Add colour | `button()` in section header | |
| Paint swatch | `color_swatch()` or painted `view` | |
| Paint kind | `select()` Solid/Gradient | |
| Solid colour | `color_picker()` with `color_area`, `color_slider`, `color_field`, presets | NSH eyedropper is macOS/web only; Windows fallback |
| Gradient editor | custom `GradientControl`: stop track (`canvas`), stop `slider`s, per-stop `color_picker`, angle `number_input`, add/remove `button`s, preset menu | NSH has no gradient element |
| Gradient presets | `menu()` / `context_menu()` from preset store | |
| Delete/reassign token | `select()` reassignment + `alert_dialog()` confirm | |
| Type tokens list | `virtual_list()` rows; add button | |
| Type face fields | `FieldRow` + `input`/`select`/`number_input`/`color_picker` | family/weight/style/size/line/letter/colour |
| Font actions | `menu()` (import, adopt, remove) | |
| Layer uses list | `virtual_list()` + `link()`/`button()` to reveal layer | |
| Token inspector | right inspector or `sheet()` | keep one editing surface; sheet on narrow |
| Preset context menu | `context_menu()` | |

### Document

| Element | NSH proposal | Notes |
|---|---|---|
| Section frame | `accordion()` multiple, each open by default | source keeps sections open |
| Theme metadata | `FieldRow` + `input` (name, author, description), `TagsField` | |
| Tags | `input()` + `Badge` chips + add/remove `button`s | no tag input widget |
| Language | `select()` with language names | sample text preview via `text` |
| Release version | `input()` / read-only `text` | |
| Artboard shape | `select()` + `segmented` where binary | |
| Ratio / orientation / resolution | `select()`; disabled where shape decides | |
| Size | `PairControl` (`number_input` × 2), unit label | validation inline |
| Artboard preview ratio | `toggle()` + `number_input`/`input` | |
| Credits | `virtual_list()` or plain rows | link actions |

## Selection inspector

The inspector is one right dock panel. Sections use `accordion()` with counts;
closed sections keep content mounted (`.keep_mounted`) so drafts survive.

| Section / element | NSH proposal | Notes |
|---|---|---|
| Inspector header | kind `svg` + name `input` (rename) + lock marker | |
| Selection lead (`selection-lead.tsx`) | `toolbar()` of arrange actions | destructive styling |
| Section header | `accordion_header` + `accordion_trigger` + count `Badge` | all open by default |
| Content | fields + extras | |
| Position | `PairControl` x/y, width/height; shape geometry fields | point text size read-only |
| Crop | custom `canvas()` overlay + drag handles, `slider`/`number_input`s; no NSH crop widget | |
| Layer | rotation `slider()` + `number_input`, opacity `slider()` | |
| Paint | panel material fields, glass fields, chart paint | |
| Advanced | collapsed `accordion`/`collapsible` with chart advanced fields | starts closed |
| Text runs | `accordion` per run; fields: text `input`, token `select`, size/line/letter `number_input`, colour `color_picker`, source/unit/zone `select`, remove `IconButton`; add run `button` | complex sub-surface stays app-owned |
| Run layout | `segmented` for align/vertical align/overflow, `switch` for vertical text | |
| Chart content/paint | `chart-manager` fields composed from `FieldRow` + controls; chart preview from raster | |
| Design token edit | `popover()` with `TokenFaceFields` / `PaintKindField` / `ColorPickerPanel` | |
| Field controls | `TextControl`, `NumberControl`, `PairControl`, `ToggleControl`, `TogglePair`, `SelectControl`, `SegmentedControl`, `SwatchControl`, `SliderControl`, `ColorControl` | one `PropertyRow` renderer |
| Error/refusal | `text` with error colour + `toast` optional | no silent refusal |
| Locked selection | hide writing fields, show lock `Badge` | as source |

## Canvas and stage

| Element | NSH proposal | Notes |
|---|---|---|
| Artboard render | `image()` from `render-raster` (`DisplayList` → image); redraw on document revision | preserves text shaping, letter spacing, vertical text, SVG, glass |
| Live video nodes | `video()` layered into the artboard bounds, clipped | spike item; otherwise raster frame |
| Selection box/handles/marquee | `canvas()` overlay; `PanResponder` for drag/resize | no DOM handles |
| Snap guides/rulers | `canvas()` overlay | |
| Crop UI | `canvas()` + handles; `Escape`/Return via `on_key` | |
| Hit testing | Rust scene model, not Fabric | port from `scene-fabric` |
| Canvas context menu | `context_menu()` on stage | commands by selection |
| Zoom/fit/actual/fullscreen | `toolbar` + `menu`/`popover`; fullscreen via desktop window API | |
| Display lenses | `menu` radio group (`menu_item` checked) | groups Landscape/Portrait/Square |
| Document identity | top-left `text` + rename inline | |
| Cursor readouts | absolute `view` + `text`, pointer-transparent | |
| Drag and drop files | `View` desktop file drops (`on_file_drop`) → import asset | native |
| Canvas keyboard | `on_key` for arrows, nudge, delete, escape | one keymap |

## Themes / Library (Store mode)

| Element | NSH proposal | Notes |
|---|---|---|
| Local/online halves | `tabs()` with header halves | |
| Search | `input()` with clear button | debounced |
| Facet row (`FacetRow`) | `popover()` per facet + search `input` + `checkbox()` rows; active count `Badge` | six facets, source order |
| Sort | `select()` | |
| Density | `slider()` with end labels | Comfortable ↔ Dense |
| Gallery | `virtual_list()` with `columns(n)`, keyed by theme id | card = `button()` + `image` + title + status `Badge` + `menu` |
| Card actions | `menu()` / `context_menu()` | install, update, remove, export, assign |
| Install/update action | `button()` state machine; `progress()` for download | |
| Submit button | `button()` primary | |
| Status badge | `Badge` composition | local/remote/update/error |
| Item detail | `sheet()` on desktop, `dialog()` fallback | preview, facts, tags, actions |
| Assign display | `dialog()` + `select()` + confirm | |
| Delete confirm | `alert_dialog()` destructive | |
| Empty/loading/error | `EmptyState`, `SkeletonRow`, `text` + retry `button` | |
| Held update notice | banner `view` + buttons | |

## Settings

| Element | NSH proposal | Notes |
|---|---|---|
| Page navigation | vertical `tabs()` (styled `flex_col`, indicator on left) with grouped `tab()`s | or `virtual_list` nav; Tabs keeps a11y |
| Page frame | `view` + heading `text` + intro `text` | |
| Group / section | `view` with heading + hint; optional `accordion`/`collapsible` | |
| Field rows | `FieldRow` + controls | |
| Displays page: display list | `radio_group()` + card rows; edit/remove `button`s | |
| Display presentation | shape `segmented`, rotation `slider`, always-on-top `switch`, render mode `segmented` | |
| Panel controls | `virtual_list` of panels; identify/edit `button`s; settings `dialog()` with `slider`/`number_input`/`switch` | |
| Device assignment | `select()` per group | |
| Add display | `select()` monitor + name `input` + `button` | |
| Panels section | rows + facts + add `button` | |
| Autostart | `switch()` | |
| Hardware | device `select()`s + name `input`s | |
| SignalRGB | port `number_input`, device rows, add `button` | |
| Units | time zone `select`, measurement `select`, per-unit `select`s | |
| Readings sources | `virtual_list` + edit/test/delete `IconButton`s; form `dialog()` with `input`/`number_input`; credentials | password `input().masked(true)` |
| Weather | provider `select`, label `input`, lat/long `number_input`, Windows location `switch`, poll `number_input`, search `input` + results list | |
| Appearance | palette `radio_group` + follow-system `switch` | |
| Previews | ratio rows, add/remove `button`s, name `input`, reason `text` | |
| Path presets | built-in + custom rows, remove `IconButton`, name/data `input`s, add `button` | |
| Keyboard | rows with chord `text`/`Kbd`, rebind `IconButton`, listening state, reset, conflict `text` | capture keys via `on_key` |
| Asset sources | credential rows + `input` + save/test/clear `button`s + status | masked |
| Extensions | plugin manager list, install `pick_files()`, enable `switch`, move up/down, remove | |
| About | facts list + version `text` | |
| Plugin settings | dynamic `FieldRow`s from schema; plugin page `web_view()` | |

## Asset finder

| Element | NSH proposal | Notes |
|---|---|---|
| Modal frame | `dialog()` full-size, `Tabs` categories | |
| Search | `input()` | |
| Filters | `toggle_group()` / `select()` | |
| Source chips | `toggle_group()` of `toggle()` chips | |
| Connections | `IconButton` → settings page | |
| Grid | `virtual_list()` with columns; card = `button()` + `image`/`svg`/`canvas` preview | icons/images/fonts/palettes/gradients |
| Card state | overlay `Badge`/`progress()`/check | import/busy/failed |
| Font preview | `text()` with `.font_family()`/size | |
| Preview modal | `sheet()`/`dialog()` + `image`/`video` + facts | |
| Footer actions | `toolbar()` with primary import/place | |
| Empty/loading/error | `EmptyState`, `SkeletonRow`, retry `button` | |
| Wallpaper tools | `button`s + `pick_files()` | refresh/import/guide |
| Wallpaper Engine guide | `dialog()` + `text_view()` markdown | |
| Plugin body | `web_view()` inside the finder | needs `webview` capability |

## Dialogs, sheets and overlays

| Yunari dialog | NSH proposal | Notes |
|---|---|---|
| New document chooser | `dialog()` + selects/`PairControl` + primary/cancel | |
| Theme library chooser | `dialog()` + search `input` + `virtual_list` + actions | |
| Save as | `dialog()` + `input()` | |
| Theme conflict | `dialog()` with three actions | |
| Delete confirmation | `alert_dialog()` destructive | |
| Draft recovery | `alert_dialog()` or `dialog()` with recover/discard | destructive discard |
| Replace document | `alert_dialog()` | |
| Save cleanup | `dialog()` + `checkbox()` list + select-all + confirm | |
| Font import | `dialog()` + font metadata form | |
| Output bake | `dialog()` + `progress()` + status | |
| Video import / trim | `dialog()` + `progress`/`video`/`slider` | |
| Place image | `dialog()` + placement controls | |
| Asset preview | `sheet()` | |
| Publish/assign + QR | `dialog()` + `select` + `canvas()` QR | QR via `qrcode` crate |
| Shortcut reference | `sheet()` + `text` rows, or `command()` | |
| Wallpaper guide | `dialog()` + `text_view()` | |
| Plugin frame | `dialog()`/pane + `web_view()` | |
| Toasts | `toast_stack()` + `toast()` | success/error/info |
| Tooltips | `tooltip()` | all icon buttons |

## Host admin chooser (`packages/host/src/admin`)

The host's small browser chooser is a separate surface from the editor shell,
but it is the same product UI and maps the same way.

| Element | NSH proposal | Notes |
|---|---|---|
| Chooser page | `view` + header `text` + grouped sections | |
| Display / template / theme group labels | `text` section headings | |
| Theme row (`theme-row.tsx`) | `button()` card + `image`/placeholder + name/author + current `Badge` | |
| Edit action | `IconButton` opening the editor window | |
| Delete action | `IconButton` + `alert_dialog()` destructive confirm | recycle-bin copy |
| Cost warning | `text` + `Badge`, `role=status` equivalent (`aria-live` via NSH accessibility) | |
| Status line | `text` in a live region | |
| Empty state | `EmptyState` | |

## Player surface (desktop now, mobile roadmap)

| Element | NSH proposal | Notes |
|---|---|---|
| Theme canvas | `video()` for video themes; `canvas()`/`image()` for rendered themes | full-bleed |
| Theme chooser strip | `select()` or `sheet()` list | paired/own theme |
| Top/bottom text strips | `view` + `text`, absolute overlay | safe areas on mobile |
| Display controls | `toolbar()`: fullscreen `button`, keep-awake `toggle`, dim `slider`, status `text` | auto-hide on idle |
| Load failure | `view` + heading/lede/reason + retry `button` + host `link()` | |
| Availability notice | banner `view` + `button` | |
| Mobile player | `view` + `video`/`canvas` + `overlay` controls, `sheet` chooser, `alert_dialog` failures | iOS/Android packaging |
| Android live wallpaper / iOS wallpaper | not an NSH element | native host/service work |

## Source atoms → NSH

| Source atom | NSH |
|---|---|
| `ControlText`, `ControlWell` inputs | `input()` |
| `ControlNumber`, `ControlPair` | `number_input()` × 1/2 |
| `ControlSelect`, `ControlSwatch` | `select()` |
| `ControlSegmented` | `toggle_group()` + `toggle()` |
| `ControlToggle`, `ControlTogglePair` | `switch()` |
| `ControlChoiceRow` | `radio_group()` + `radio()` |
| `ControlSlider` | `slider()` + `number_input()` |
| `ControlHint` | `tooltip()` + `IconButton` |
| `ControlIconButton` | `button()` + `tooltip()` + label |
| `ControlTooltip` | `tooltip()` |
| `ColourPicker`, `ColourPickerPanel` | `color_picker()` + parts |
| `GradientEditor` | custom `GradientControl` |
| `Dialog`, `openModal` | `dialog()`, `alert_dialog()`, `sheet()` |
| `InspectorSection` | `accordion`/`collapsible` |
| `PresetContextMenu` | `context_menu()` |
| `SensorKeyDot` | `Badge`/`view` dot |
| Base UI Toggle/ToggleGroup | `toggle`/`toggle_group` |
| Base UI Select | `select` |
| Base UI Switch | `switch` |
| Base UI Checkbox | `checkbox` |
| Base UI RadioGroup | `radio_group` |
| Base UI Slider | `slider` |
| Base UI Popover/Menu/Dialog | `popover`/`menu`/`dialog` |
| Base UI Accordion | `accordion` |
| Base UI Tabs | `tabs` |
| Fabric canvas | raster `image()` + `canvas()` overlays |

## Gaps and custom work (no NSH element)

- **Gradient authoring**: stop track, stop dragging, presets, angular control. Build
  on `slider`, `color_picker`, `canvas`; do not wait for a kit widget.
- **Crop / transform handles / snap guides**: `Canvas` + `PanResponder`; NSH has no
  scene editor.
- **QR code**: render with a Rust QR crate into `canvas`/`image`.
- **Tree multi-select and drag-reorder**: NSH Tree is single-select and has no
  reorder; keep app-managed selection and `PanResponder` drag.
- **Eyedropper on Windows**: `color_eye_dropper()` is documented on macOS/web only;
  hide the control on Windows or implement a screen sampler natively.
- **Live video inside the artboard**: layer `Video` over the raster, or rasterise
  frames; decide in the spike.
- **Plugin frame**: `web_view()` requires the webview capability and platform
  runtime; keep it behind a feature and an allow-list.
- **Live wallpaper/background playback on mobile**: native platform service, not
  NSH.
- **Native title bar branding**: native title bar + app header; custom title bar
  only if the brand demands it.

## What changes from the source (greenfield choices)

1. **Dock workspace** instead of rail + one hidden pane: tabs, resizing, persisted
   layout, compact drawers.
2. **Native application menu + command palette** instead of drawn menu/toolbar
   sprawl; every command still has a control.
3. **One `FieldRow` renderer** for every inspector/settings control; no bespoke
   field frameworks.
4. **NSH ColorPicker (OKLCH, fields, swatches, eyedropper)** replaces the bespoke
   React colour panel; gradient stays custom.
5. **`Sheet` for detail/edit panels** (store item, asset preview, token edit) instead
   of nested modals.
6. **`AlertDialog` for destructive confirms**, `Toast` for transient status; the
   status strip keeps only persistent facts.
7. **Virtual lists everywhere** lists can grow; `Tree` only where hierarchy is
   semantic.
8. **Native pickers, clipboard and file drops** replace web file inputs.

## Approval checklist

- [ ] Shell: DockArea workspace, native menu, header toolbar, status strip.
- [ ] `crates/ui` wrapper set is the only styling layer over NSH base.
- [ ] Left panes: Tree + virtual lists + Toolbar footers; no bespoke controls.
- [ ] Inspector: Accordion sections + `FieldRow` controls; runs and crop custom.
- [ ] Stage: raster artboard + Canvas overlays + native drag/drop.
- [ ] Themes/Store, Settings, Asset finder mappings accepted.
- [ ] Dialogs: Dialog/Sheet/AlertDialog/Toast split accepted.
- [ ] Player mapping accepted; mobile native gaps acknowledged.
- [ ] Custom gaps (gradient, crop, QR, tree multi-select, Windows eyedropper)
      accepted as app work.
