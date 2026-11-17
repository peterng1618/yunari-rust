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

The **Docs** column links each proposed NSH component to its reference page on
nsh.dev.

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

| Wrapper | Built from | Replaces | Docs |
|---|---|---|---|
| `AppButton` | `button()` + theme + `.disabled_style` + `.focus_visible` | all raw `<button>` | [Button](https://nsh.dev/elements/button) |
| `IconButton` | `button()` + `tooltip()` + `.accessibility_label` | `ControlIconButton`, rail slots | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip), [Accessibility](https://nsh.dev/docs/accessibility) |
| `LinkButton` | `link()` | host/help links | [Link](https://nsh.dev/elements/link) |
| `FieldRow` | `view` + `text` + control + error/hint | `ControlRow`, `ControlWell`, `SettingField` | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| `TextControl` | `input()` (single/multiline) | `ControlText`, controlled wells | [Input](https://nsh.dev/elements/input) |
| `NumberControl` | `number_input()` | `ControlNumber` | [NumberInput](https://nsh.dev/elements/number-input) |
| `PairControl` | two `NumberControl`s in a row | `ControlPair`, `ControlTogglePair` | [NumberInput](https://nsh.dev/elements/number-input) |
| `ChoiceControl` | `radio_group()` + `radio()` | `ControlChoiceRow` | [Radio](https://nsh.dev/elements/radio) |
| `SegmentedControl` | `toggle_group()` + `toggle()` | `ControlSegmented` | [Toggle](https://nsh.dev/elements/toggle) |
| `SelectControl` | `select()` + `SelectItem` | `ControlSelect`, `ControlSwatch` | [Select](https://nsh.dev/elements/select) |
| `ToggleControl` | `switch()` | `ControlToggle` | [Switch](https://nsh.dev/elements/switch) |
| `SliderControl` | `slider()` + `number_input()` | `ControlSlider` | [Slider](https://nsh.dev/elements/slider), [NumberInput](https://nsh.dev/elements/number-input) |
| `ColorControl` | `color_picker()` + area/slider/field/swatches | bespoke colour picker | [ColorPicker](https://nsh.dev/elements/color-picker), [Slider](https://nsh.dev/elements/slider) |
| `GradientControl` | custom: stops list + `color_picker` + `slider` + `canvas` preview | `GradientEditor` | [ColorPicker](https://nsh.dev/elements/color-picker), [Slider](https://nsh.dev/elements/slider), [Canvas](https://nsh.dev/elements/canvas) |
| `PropertySection` | `accordion()` / `collapsible()` | `InspectorSection` | [Accordion](https://nsh.dev/elements/accordion), [Collapsible](https://nsh.dev/elements/collapsible) |
| `DialogSurface` | `dialog()` + header/footer + `dialog()` close | `Dialog` | [Dialog](https://nsh.dev/elements/dialog) |
| `Confirm` | `alert_dialog()` | destructive custom dialogs | [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| `Toast` | `ToastManager` + `toast_stack()` | status messages | [Toast](https://nsh.dev/elements/toast) |
| `Badge` / `Tag` / `Kbd` | `view` + `text` + tokens | pills, status badges, shortcut chips | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| `EmptyState` | `view` + icon + `text` + action | `FinderState`, empty store states | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| `SkeletonRow` | `view` + token fill + animation | `SkeletonCard` | [View](https://nsh.dev/elements/view) |

Every wrapper takes an accessibility label; icon-only controls are not allowed
without one.

## Shell and navigation

| Yunari element (source) | Job | NSH proposal | Change / notes | Docs |
|---|---|---|---|---|
| Brand (`<strong>`) `shell-layout.tsx` | identity | `text` + `svg` logo in header | no behaviour | [Text](https://nsh.dev/elements/text), [SVG image](https://nsh.dev/docs/images-and-assets) |
| Mode switch (`ModeSwitch`) | one of Themes/Editor/Settings | `toggle_group()` + three `toggle()`s, one pressed | keep order Themes, Editor, Settings; gear still opens Settings | [Toggle](https://nsh.dev/elements/toggle) |
| Store halves (`StoreHalves`) | Local / Online tabs | `tabs()` or `toggle_group()` in header | model as `tabs()` with a panel below | [Tabs](https://nsh.dev/elements/tabs), [Toggle](https://nsh.dev/elements/toggle) |
| Shell menu actions (`shell-menu-bar.tsx`) | File/Edit/View/Theme/Help commands | native `set_application_menu`, `MenuItem` + roles + accelerators | no drawn menu bar | [nsh/desktop](https://nsh.dev/modules/nsh-desktop) |
| Toolbar open actions | New, template, open library, open recent, import | `toolbar_group()` of `button()`s; open recent = `menu()` | tooltips carry `Kbd` label | [Toolbar](https://nsh.dev/elements/toolbar), [Button](https://nsh.dev/elements/button), [Menu](https://nsh.dev/elements/menu) |
| Toolbar document actions | Save, Save as, Activate, Export theme/widget | `toolbar_group()`; primary weight for Save | disabled states from session | [Toolbar](https://nsh.dev/elements/toolbar) |
| Find assets | open finder | `button()` + `on_key("mod+shift+f")` | command palette entry too | [Button](https://nsh.dev/elements/button), [on_key](https://nsh.dev/modules/nsh) |
| Publish control + indicator | publish live / assign | `button()` primary + `dialog()`; indicator = `Badge` in status | dialog hosts assign + QR | [Button](https://nsh.dev/elements/button), [Dialog](https://nsh.dev/elements/dialog), [View](https://nsh.dev/elements/view) |
| Rail (`rail.tsx`) | choose left pane, settings gear | left `DockArea` tab strip; gear = `button()` in header | recommendation: Dock tabs; fallback vertical `toolbar` + `toggle` | [Dock](https://nsh.dev/elements/dock), [Button](https://nsh.dev/elements/button) |
| Pane chrome (`pane.tsx`) | icon, title, count, body, footer | `render_tab` for icon/title; panel content = `view`; footer = `toolbar()` | counts from same store as before | [View](https://nsh.dev/elements/view), [Toolbar](https://nsh.dev/elements/toolbar) |
| Pane footer actions (`LayerActions`) | object actions for selection | `toolbar()` + `IconButton`s, destructive style | filter ineligible actions, as source | [Toolbar](https://nsh.dev/elements/toolbar), [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| Document identity | name, dirty mark, rename | `text` + inline `input()` on rename | one owner | [Text](https://nsh.dev/elements/text), [Input](https://nsh.dev/elements/input) |
| Display/zoom cluster (`display-switch.tsx`) | playback, zoom −/%, +, lens menu | `toolbar()` + buttons + `popover()`/`menu()`; zoom % is a `button` trigger | lens groups = `menu` groups; fit/actual/fullscreen = menu items | [Toolbar](https://nsh.dev/elements/toolbar), [Popover](https://nsh.dev/elements/popover), [Menu](https://nsh.dev/elements/menu), [Button](https://nsh.dev/elements/button) |
| Background playback | play/pause video | `IconButton` in zoom toolbar | state from renderer | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip), [Toolbar](https://nsh.dev/elements/toolbar) |
| Canvas dock (`canvas-dock.tsx`) | contextual object actions | floating `toolbar()` over stage | same registry; hide when no document | [Toolbar](https://nsh.dev/elements/toolbar) |
| Canvas context menu | right-click actions | `context_menu()` on stage | grouped by command registry | [Menu](https://nsh.dev/elements/menu) |
| Inspector toggle | collapse right dock | `IconButton`; `area.toggle_dock("right")` | dock state persisted | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| Shortcut reference (`?`) | list all shortcuts | `command()` palette with shortcuts, or `sheet()` + `text` rows | prefer command palette; keep `?` as sheet | [Command](https://nsh.dev/elements/command), [Sheet](https://nsh.dev/elements/sheet), [Text](https://nsh.dev/elements/text) |
| Status readout | object/selection counts | `text` in status strip | no bridge = render nothing | [Text](https://nsh.dev/elements/text) |
| Status message / diagnostics | transient + persistent reports | `toast()` for transient; `text` + severity icon in strip for persistent | one status owner per mode | [Toast](https://nsh.dev/elements/toast), [Text](https://nsh.dev/elements/text) |
| Save state | saved/dirty/saving | `Badge` in status strip | live region | [View](https://nsh.dev/elements/view) |
| Publish indicator | live/offline | `Badge` | live region | [View](https://nsh.dev/elements/view) |
| Cursor indicators | angle/size while dragging | `Canvas` overlay or absolute `view` + `text` | must not intercept pointer | [Canvas](https://nsh.dev/elements/canvas), [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| Held update notice | banner + actions | `view` banner + `button`s, or `toast` with actions | non-blocking | [View](https://nsh.dev/elements/view), [Button](https://nsh.dev/elements/button), [Toast](https://nsh.dev/elements/toast) |
| Window / tray / global shortcut | desktop integration | `nsh/desktop`: `window()`, `set_tray`, `register_global_shortcut`, `displays` | native, no HTML chrome | [nsh/desktop](https://nsh.dev/modules/nsh-desktop) |

## Left panes

### Composition / Layers

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Layer tree (`role="tree"`) | `tree()` + `TreeItem { id, label, children }`, `render_item` | virtualized; expanded/selected owned by app | [Tree](https://nsh.dev/elements/tree) |
| Row expand/collapse | tree's native folder press + arrow keys | no custom twisty button | [Tree](https://nsh.dev/elements/tree) |
| Kind mark / icon | `svg` or `text` glyph in row | theme tokens | [SVG image](https://nsh.dev/docs/images-and-assets), [Text](https://nsh.dev/elements/text) |
| Inline rename | `input()` shown in the row on F2/double press | keep one editing row | [Input](https://nsh.dev/elements/input) |
| Visibility / lock toggles | `IconButton` or `toggle()` in row | label includes layer name | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip), [Toggle](https://nsh.dev/elements/toggle) |
| Enter / leave group | `menu_item()`s in row context menu; keys | via command registry | [Menu](https://nsh.dev/elements/menu) |
| Multi-select (shift/cmd) | app-managed selected `Vec<String>` in row press; tree's primary selection remains anchor | NSH Tree is single-select; render multi-selected rows with selected style | [Tree](https://nsh.dev/elements/tree) |
| Drag reorder / reparent | custom `PanResponder` on row + drop target row, with insertion marker | NSH Tree has no drag reorder | [Gestures](https://nsh.dev/docs/touch-and-gestures) |
| Keyboard navigation | tree arrows, Home/End, typeahead; app handles multi-select modifiers | | [Tree](https://nsh.dev/elements/tree) |
| Selection lead (`selection-lead.tsx`) | `toolbar()` above tree sections | arrange actions, lock marker | [Toolbar](https://nsh.dev/elements/toolbar), [Tree](https://nsh.dev/elements/tree) |
| Footer actions | `toolbar()` pinned to pane footer | filtered by eligibility | [Toolbar](https://nsh.dev/elements/toolbar) |

### Add

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Insert grid (`yunari-add-grid`) | `virtual_list()` with `columns(n)` or `view` wrap rows | tiles are buttons | [List](https://nsh.dev/elements/list), [View](https://nsh.dev/elements/view) |
| Text tile | `button()` + glyph + label | inserts text object | [Button](https://nsh.dev/elements/button) |
| Shape tiles | `button()` + `PathGlyph` via `svg`/`canvas` | path preview from model | [Button](https://nsh.dev/elements/button), [SVG image](https://nsh.dev/docs/images-and-assets), [Canvas](https://nsh.dev/elements/canvas) |
| Chart family tiles | `button()` + glyph | inserts chart | [Button](https://nsh.dev/elements/button) |
| Tooltip | `tooltip()` | label only | [Tooltip](https://nsh.dev/elements/tooltip) |

### Widgets

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Installed widget grid | `virtual_list()` columns + card views | card = `button()` | [List](https://nsh.dev/elements/list) |
| Widget thumbnail | `image()`/`svg()`/`canvas()` | lazy | [Image](https://nsh.dev/elements/image), [SVG image](https://nsh.dev/docs/images-and-assets), [Canvas](https://nsh.dev/elements/canvas) |
| Empty state | `EmptyState` wrapper | | [View](https://nsh.dev/elements/view) |
| Count | pane tab title count | | — |

### Assets

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Asset card | `button()` card + `image`/`video` thumb + `menu()` for actions | preview/place/replace/remove/type-token | [Button](https://nsh.dev/elements/button), [Image](https://nsh.dev/elements/image), [Video](https://nsh.dev/elements/video), [Menu](https://nsh.dev/elements/menu) |
| Asset mark | `svg`/`text` by kind | | [SVG image](https://nsh.dev/docs/images-and-assets), [Text](https://nsh.dev/elements/text) |
| Import file | `pick_files()` native | no hidden `<input type=file>` | [Picker](https://nsh.dev/modules/nsh) |
| Find assets | opens finder `dialog()` | | [Dialog](https://nsh.dev/elements/dialog) |
| Size meter (`AssetSizeMeter`) | `progress()` + `text` | pass/total bytes | [Progress](https://nsh.dev/elements/progress), [Text](https://nsh.dev/elements/text) |
| Font metadata form | `FieldRow` + `input`/`number_input`/`select` | family/weight/style/licence/source/role | [View](https://nsh.dev/elements/view), [Input](https://nsh.dev/elements/input), [NumberInput](https://nsh.dev/elements/number-input), [Select](https://nsh.dev/elements/select) |
| Place image modal | `dialog()` + size/placement controls | | [Dialog](https://nsh.dev/elements/dialog) |
| Import video modal | `dialog()` with `progress()` + status text | steps prepared/importing/done | [Dialog](https://nsh.dev/elements/dialog), [Progress](https://nsh.dev/elements/progress), [Text](https://nsh.dev/elements/text) |
| Video trim modal | `dialog()` + `video()` preview + `slider()` range | duration/start/end | [Dialog](https://nsh.dev/elements/dialog), [Video](https://nsh.dev/elements/video), [Slider](https://nsh.dev/elements/slider) |
| Asset preview modal | `sheet()` (desktop) with `image`/`video`/`text` facts + actions | long detail benefits from sheet | [Sheet](https://nsh.dev/elements/sheet), [Image](https://nsh.dev/elements/image), [Video](https://nsh.dev/elements/video), [Text](https://nsh.dev/elements/text) |
| Replace/remove confirm | `alert_dialog()` destructive | | [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Import errors | inline `text` + `toast()` | | [Text](https://nsh.dev/elements/text), [Toast](https://nsh.dev/elements/toast) |

### Tokens

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Paints list | `virtual_list()` rows; each row = swatch + name + kind + uses + menu | | [List](https://nsh.dev/elements/list), [Menu](https://nsh.dev/elements/menu) |
| Add colour | `button()` in section header | | [Button](https://nsh.dev/elements/button) |
| Paint swatch | `color_swatch()` or painted `view` | | [ColorPicker](https://nsh.dev/elements/color-picker), [View](https://nsh.dev/elements/view) |
| Paint kind | `select()` Solid/Gradient | | [Select](https://nsh.dev/elements/select) |
| Solid colour | `color_picker()` with `color_area`, `color_slider`, `color_field`, presets | NSH eyedropper is macOS/web only; Windows fallback | [ColorPicker](https://nsh.dev/elements/color-picker) |
| Gradient editor | custom `GradientControl`: stop track (`canvas`), stop `slider`s, per-stop `color_picker`, angle `number_input`, add/remove `button`s, preset menu | NSH has no gradient element | [ColorPicker](https://nsh.dev/elements/color-picker), [Slider](https://nsh.dev/elements/slider), [Canvas](https://nsh.dev/elements/canvas), [NumberInput](https://nsh.dev/elements/number-input), [Button](https://nsh.dev/elements/button), [Menu](https://nsh.dev/elements/menu) |
| Gradient presets | `menu()` / `context_menu()` from preset store | | [Menu](https://nsh.dev/elements/menu) |
| Delete/reassign token | `select()` reassignment + `alert_dialog()` confirm | | [Select](https://nsh.dev/elements/select), [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Type tokens list | `virtual_list()` rows; add button | | [List](https://nsh.dev/elements/list), [Button](https://nsh.dev/elements/button) |
| Type face fields | `FieldRow` + `input`/`select`/`number_input`/`color_picker` | family/weight/style/size/line/letter/colour | [View](https://nsh.dev/elements/view), [Input](https://nsh.dev/elements/input), [Select](https://nsh.dev/elements/select), [NumberInput](https://nsh.dev/elements/number-input), [ColorPicker](https://nsh.dev/elements/color-picker) |
| Font actions | `menu()` (import, adopt, remove) | | [Menu](https://nsh.dev/elements/menu) |
| Layer uses list | `virtual_list()` + `link()`/`button()` to reveal layer | | [List](https://nsh.dev/elements/list), [Link](https://nsh.dev/elements/link), [Button](https://nsh.dev/elements/button) |
| Token inspector | right inspector or `sheet()` | keep one editing surface; sheet on narrow | [Sheet](https://nsh.dev/elements/sheet) |
| Preset context menu | `context_menu()` | | [Menu](https://nsh.dev/elements/menu) |

### Document

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Section frame | `accordion()` multiple, each open by default | source keeps sections open | [Accordion](https://nsh.dev/elements/accordion) |
| Theme metadata | `FieldRow` + `input` (name, author, description), `TagsField` | | [View](https://nsh.dev/elements/view), [Input](https://nsh.dev/elements/input) |
| Tags | `input()` + `Badge` chips + add/remove `button`s | no tag input widget | [Input](https://nsh.dev/elements/input), [View](https://nsh.dev/elements/view), [Button](https://nsh.dev/elements/button) |
| Language | `select()` with language names | sample text preview via `text` | [Select](https://nsh.dev/elements/select) |
| Release version | `input()` / read-only `text` | | [Input](https://nsh.dev/elements/input), [Text](https://nsh.dev/elements/text) |
| Artboard shape | `select()` + `segmented` where binary | | [Select](https://nsh.dev/elements/select), [Toggle](https://nsh.dev/elements/toggle) |
| Ratio / orientation / resolution | `select()`; disabled where shape decides | | [Select](https://nsh.dev/elements/select) |
| Size | `PairControl` (`number_input` × 2), unit label | validation inline | [NumberInput](https://nsh.dev/elements/number-input) |
| Artboard preview ratio | `toggle()` + `number_input`/`input` | | [Toggle](https://nsh.dev/elements/toggle), [NumberInput](https://nsh.dev/elements/number-input), [Input](https://nsh.dev/elements/input) |
| Credits | `virtual_list()` or plain rows | link actions | [List](https://nsh.dev/elements/list) |

## Selection inspector

The inspector is one right dock panel. Sections use `accordion()` with counts;
closed sections keep content mounted (`.keep_mounted`) so drafts survive.

| Section / element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Inspector header | kind `svg` + name `input` (rename) + lock marker | | [SVG image](https://nsh.dev/docs/images-and-assets), [Input](https://nsh.dev/elements/input) |
| Selection lead (`selection-lead.tsx`) | `toolbar()` of arrange actions | destructive styling | [Toolbar](https://nsh.dev/elements/toolbar) |
| Section header | `accordion_header` + `accordion_trigger` + count `Badge` | all open by default | [Accordion](https://nsh.dev/elements/accordion), [View](https://nsh.dev/elements/view) |
| Content | fields + extras | | — |
| Position | `PairControl` x/y, width/height; shape geometry fields | point text size read-only | [NumberInput](https://nsh.dev/elements/number-input) |
| Crop | custom `canvas()` overlay + drag handles, `slider`/`number_input`s; no NSH crop widget | | [Canvas](https://nsh.dev/elements/canvas), [Slider](https://nsh.dev/elements/slider), [NumberInput](https://nsh.dev/elements/number-input) |
| Layer | rotation `slider()` + `number_input`, opacity `slider()` | | [Slider](https://nsh.dev/elements/slider), [NumberInput](https://nsh.dev/elements/number-input) |
| Paint | panel material fields, glass fields, chart paint | | — |
| Advanced | collapsed `accordion`/`collapsible` with chart advanced fields | starts closed | [Accordion](https://nsh.dev/elements/accordion), [Collapsible](https://nsh.dev/elements/collapsible) |
| Text runs | `accordion` per run; fields: text `input`, token `select`, size/line/letter `number_input`, colour `color_picker`, source/unit/zone `select`, remove `IconButton`; add run `button` | complex sub-surface stays app-owned | [Accordion](https://nsh.dev/elements/accordion), [Text](https://nsh.dev/elements/text), [Input](https://nsh.dev/elements/input), [Select](https://nsh.dev/elements/select), [NumberInput](https://nsh.dev/elements/number-input), [ColorPicker](https://nsh.dev/elements/color-picker), [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| Run layout | `segmented` for align/vertical align/overflow, `switch` for vertical text | | [Toggle](https://nsh.dev/elements/toggle), [Switch](https://nsh.dev/elements/switch), [Text](https://nsh.dev/elements/text) |
| Chart content/paint | `chart-manager` fields composed from `FieldRow` + controls; chart preview from raster | | [View](https://nsh.dev/elements/view) |
| Design token edit | `popover()` with `TokenFaceFields` / `PaintKindField` / `ColorPickerPanel` | | [Popover](https://nsh.dev/elements/popover) |
| Field controls | `TextControl`, `NumberControl`, `PairControl`, `ToggleControl`, `TogglePair`, `SelectControl`, `SegmentedControl`, `SwatchControl`, `SliderControl`, `ColorControl` | one `PropertyRow` renderer | [Input](https://nsh.dev/elements/input), [NumberInput](https://nsh.dev/elements/number-input), [Switch](https://nsh.dev/elements/switch), [Select](https://nsh.dev/elements/select), [Toggle](https://nsh.dev/elements/toggle), [Slider](https://nsh.dev/elements/slider), [ColorPicker](https://nsh.dev/elements/color-picker) |
| Error/refusal | `text` with error colour + `toast` optional | no silent refusal | [Text](https://nsh.dev/elements/text), [Toast](https://nsh.dev/elements/toast) |
| Locked selection | hide writing fields, show lock `Badge` | as source | [View](https://nsh.dev/elements/view) |

## Canvas and stage

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Artboard render | `image()` from `render-raster` (`DisplayList` → image); redraw on document revision | preserves text shaping, letter spacing, vertical text, SVG, glass | [Image](https://nsh.dev/elements/image) |
| Live video nodes | `video()` layered into the artboard bounds, clipped | spike item; otherwise raster frame | [Video](https://nsh.dev/elements/video) |
| Selection box/handles/marquee | `canvas()` overlay; `PanResponder` for drag/resize | no DOM handles | [Canvas](https://nsh.dev/elements/canvas), [Gestures](https://nsh.dev/docs/touch-and-gestures) |
| Snap guides/rulers | `canvas()` overlay | | [Canvas](https://nsh.dev/elements/canvas) |
| Crop UI | `canvas()` + handles; `Escape`/Return via `on_key` | | [Canvas](https://nsh.dev/elements/canvas), [on_key](https://nsh.dev/modules/nsh) |
| Hit testing | Rust scene model, not Fabric | port from `scene-fabric` | — |
| Canvas context menu | `context_menu()` on stage | commands by selection | [Menu](https://nsh.dev/elements/menu) |
| Zoom/fit/actual/fullscreen | `toolbar` + `menu`/`popover`; fullscreen via desktop window API | | [Toolbar](https://nsh.dev/elements/toolbar), [Menu](https://nsh.dev/elements/menu), [Popover](https://nsh.dev/elements/popover) |
| Display lenses | `menu` radio group (`menu_item` checked) | groups Landscape/Portrait/Square | [Menu](https://nsh.dev/elements/menu), [Radio](https://nsh.dev/elements/radio) |
| Document identity | top-left `text` + rename inline | | [Text](https://nsh.dev/elements/text) |
| Cursor readouts | absolute `view` + `text`, pointer-transparent | | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| Drag and drop files | `View` desktop file drops (`on_file_drop`) → import asset | native | [View file drop](https://nsh.dev/elements/view) |
| Canvas keyboard | `on_key` for arrows, nudge, delete, escape | one keymap | [on_key](https://nsh.dev/modules/nsh) |

## Themes / Library (Store mode)

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Local/online halves | `tabs()` with header halves | | [Tabs](https://nsh.dev/elements/tabs) |
| Search | `input()` with clear button | debounced | [Input](https://nsh.dev/elements/input) |
| Facet row (`FacetRow`) | `popover()` per facet + search `input` + `checkbox()` rows; active count `Badge` | six facets, source order | [Popover](https://nsh.dev/elements/popover), [Input](https://nsh.dev/elements/input), [Checkbox](https://nsh.dev/elements/checkbox), [View](https://nsh.dev/elements/view) |
| Sort | `select()` | | [Select](https://nsh.dev/elements/select) |
| Density | `slider()` with end labels | Comfortable ↔ Dense | [Slider](https://nsh.dev/elements/slider) |
| Gallery | `virtual_list()` with `columns(n)`, keyed by theme id | card = `button()` + `image` + title + status `Badge` + `menu` | [List](https://nsh.dev/elements/list) |
| Card actions | `menu()` / `context_menu()` | install, update, remove, export, assign | [Menu](https://nsh.dev/elements/menu) |
| Install/update action | `button()` state machine; `progress()` for download | | [Button](https://nsh.dev/elements/button), [Progress](https://nsh.dev/elements/progress) |
| Submit button | `button()` primary | | [Button](https://nsh.dev/elements/button) |
| Status badge | `Badge` composition | local/remote/update/error | [View](https://nsh.dev/elements/view) |
| Item detail | `sheet()` on desktop, `dialog()` fallback | preview, facts, tags, actions | [Sheet](https://nsh.dev/elements/sheet), [Dialog](https://nsh.dev/elements/dialog) |
| Assign display | `dialog()` + `select()` + confirm | | [Dialog](https://nsh.dev/elements/dialog), [Select](https://nsh.dev/elements/select) |
| Delete confirm | `alert_dialog()` destructive | | [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Empty/loading/error | `EmptyState`, `SkeletonRow`, `text` + retry `button` | | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text), [Button](https://nsh.dev/elements/button) |
| Held update notice | banner `view` + buttons | | [View](https://nsh.dev/elements/view) |

## Settings

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Page navigation | vertical `tabs()` (styled `flex_col`, indicator on left) with grouped `tab()`s | or `virtual_list` nav; Tabs keeps a11y | [Tabs](https://nsh.dev/elements/tabs) |
| Page frame | `view` + heading `text` + intro `text` | | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| Group / section | `view` with heading + hint; optional `accordion`/`collapsible` | | [View](https://nsh.dev/elements/view), [Accordion](https://nsh.dev/elements/accordion), [Collapsible](https://nsh.dev/elements/collapsible) |
| Field rows | `FieldRow` + controls | | [View](https://nsh.dev/elements/view) |
| Displays page: display list | `radio_group()` + card rows; edit/remove `button`s | | [Radio](https://nsh.dev/elements/radio), [Button](https://nsh.dev/elements/button) |
| Display presentation | shape `segmented`, rotation `slider`, always-on-top `switch`, render mode `segmented` | | [Toggle](https://nsh.dev/elements/toggle), [Slider](https://nsh.dev/elements/slider), [Switch](https://nsh.dev/elements/switch) |
| Panel controls | `virtual_list` of panels; identify/edit `button`s; settings `dialog()` with `slider`/`number_input`/`switch` | | [List](https://nsh.dev/elements/list), [Button](https://nsh.dev/elements/button), [Dialog](https://nsh.dev/elements/dialog), [Slider](https://nsh.dev/elements/slider), [NumberInput](https://nsh.dev/elements/number-input), [Switch](https://nsh.dev/elements/switch) |
| Device assignment | `select()` per group | | [Select](https://nsh.dev/elements/select) |
| Add display | `select()` monitor + name `input` + `button` | | [Select](https://nsh.dev/elements/select), [Input](https://nsh.dev/elements/input), [Button](https://nsh.dev/elements/button) |
| Panels section | rows + facts + add `button` | | [Button](https://nsh.dev/elements/button) |
| Autostart | `switch()` | | [Switch](https://nsh.dev/elements/switch) |
| Hardware | device `select()`s + name `input`s | | [Select](https://nsh.dev/elements/select), [Input](https://nsh.dev/elements/input) |
| SignalRGB | port `number_input`, device rows, add `button` | | [NumberInput](https://nsh.dev/elements/number-input), [Button](https://nsh.dev/elements/button) |
| Units | time zone `select`, measurement `select`, per-unit `select`s | | [Select](https://nsh.dev/elements/select) |
| Readings sources | `virtual_list` + edit/test/delete `IconButton`s; form `dialog()` with `input`/`number_input`; credentials | password `input().masked(true)` | [List](https://nsh.dev/elements/list), [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip), [Dialog](https://nsh.dev/elements/dialog), [Input](https://nsh.dev/elements/input), [NumberInput](https://nsh.dev/elements/number-input) |
| Weather | provider `select`, label `input`, lat/long `number_input`, Windows location `switch`, poll `number_input`, search `input` + results list | | [Select](https://nsh.dev/elements/select), [Input](https://nsh.dev/elements/input), [NumberInput](https://nsh.dev/elements/number-input), [Switch](https://nsh.dev/elements/switch) |
| Appearance | palette `radio_group` + follow-system `switch` | | [Radio](https://nsh.dev/elements/radio), [Switch](https://nsh.dev/elements/switch) |
| Previews | ratio rows, add/remove `button`s, name `input`, reason `text` | | [Button](https://nsh.dev/elements/button), [Input](https://nsh.dev/elements/input), [Text](https://nsh.dev/elements/text) |
| Path presets | built-in + custom rows, remove `IconButton`, name/data `input`s, add `button` | | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip), [Input](https://nsh.dev/elements/input) |
| Keyboard | rows with chord `text`/`Kbd`, rebind `IconButton`, listening state, reset, conflict `text` | capture keys via `on_key` | [Text](https://nsh.dev/elements/text), [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| Asset sources | credential rows + `input` + save/test/clear `button`s + status | masked | [Input](https://nsh.dev/elements/input), [Button](https://nsh.dev/elements/button) |
| Extensions | plugin manager list, install `pick_files()`, enable `switch`, move up/down, remove | | [Picker](https://nsh.dev/modules/nsh), [Switch](https://nsh.dev/elements/switch) |
| About | facts list + version `text` | | [Text](https://nsh.dev/elements/text) |
| Plugin settings | dynamic `FieldRow`s from schema; plugin page `web_view()` | | [View](https://nsh.dev/elements/view), [WebView](https://nsh.dev/elements/web-view) |

## Asset finder

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Modal frame | `dialog()` full-size, `Tabs` categories | | [Dialog](https://nsh.dev/elements/dialog), [Tabs](https://nsh.dev/elements/tabs) |
| Search | `input()` | | [Input](https://nsh.dev/elements/input) |
| Filters | `toggle_group()` / `select()` | | [Toggle](https://nsh.dev/elements/toggle), [Select](https://nsh.dev/elements/select) |
| Source chips | `toggle_group()` of `toggle()` chips | | [Toggle](https://nsh.dev/elements/toggle) |
| Connections | `IconButton` → settings page | | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| Grid | `virtual_list()` with columns; card = `button()` + `image`/`svg`/`canvas` preview | icons/images/fonts/palettes/gradients | [List](https://nsh.dev/elements/list), [Button](https://nsh.dev/elements/button), [Image](https://nsh.dev/elements/image), [SVG image](https://nsh.dev/docs/images-and-assets), [Canvas](https://nsh.dev/elements/canvas) |
| Card state | overlay `Badge`/`progress()`/check | import/busy/failed | [View](https://nsh.dev/elements/view), [Progress](https://nsh.dev/elements/progress) |
| Font preview | `text()` with `.font_family()`/size | | [Text](https://nsh.dev/elements/text) |
| Preview modal | `sheet()`/`dialog()` + `image`/`video` + facts | | [Sheet](https://nsh.dev/elements/sheet), [Dialog](https://nsh.dev/elements/dialog), [Image](https://nsh.dev/elements/image), [Video](https://nsh.dev/elements/video) |
| Footer actions | `toolbar()` with primary import/place | | [Toolbar](https://nsh.dev/elements/toolbar) |
| Empty/loading/error | `EmptyState`, `SkeletonRow`, retry `button` | | [View](https://nsh.dev/elements/view), [Button](https://nsh.dev/elements/button) |
| Wallpaper tools | `button`s + `pick_files()` | refresh/import/guide | [Button](https://nsh.dev/elements/button), [Picker](https://nsh.dev/modules/nsh) |
| Wallpaper Engine guide | `dialog()` + `text_view()` markdown | | [Dialog](https://nsh.dev/elements/dialog), [TextView](https://nsh.dev/elements/text-view) |
| Plugin body | `web_view()` inside the finder | needs `webview` capability | [WebView](https://nsh.dev/elements/web-view) |

## Dialogs, sheets and overlays

| Yunari dialog | NSH proposal | Notes | Docs |
|---|---|---|---|
| New document chooser | `dialog()` + selects/`PairControl` + primary/cancel | | [Dialog](https://nsh.dev/elements/dialog), [NumberInput](https://nsh.dev/elements/number-input) |
| Theme library chooser | `dialog()` + search `input` + `virtual_list` + actions | | [Dialog](https://nsh.dev/elements/dialog), [Input](https://nsh.dev/elements/input), [List](https://nsh.dev/elements/list) |
| Save as | `dialog()` + `input()` | | [Dialog](https://nsh.dev/elements/dialog), [Input](https://nsh.dev/elements/input) |
| Theme conflict | `dialog()` with three actions | | [Dialog](https://nsh.dev/elements/dialog) |
| Delete confirmation | `alert_dialog()` destructive | | [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Draft recovery | `alert_dialog()` or `dialog()` with recover/discard | destructive discard | [AlertDialog](https://nsh.dev/elements/alert-dialog), [Dialog](https://nsh.dev/elements/dialog) |
| Replace document | `alert_dialog()` | | [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Save cleanup | `dialog()` + `checkbox()` list + select-all + confirm | | [Dialog](https://nsh.dev/elements/dialog), [Checkbox](https://nsh.dev/elements/checkbox) |
| Font import | `dialog()` + font metadata form | | [Dialog](https://nsh.dev/elements/dialog) |
| Output bake | `dialog()` + `progress()` + status | | [Dialog](https://nsh.dev/elements/dialog), [Progress](https://nsh.dev/elements/progress) |
| Video import / trim | `dialog()` + `progress`/`video`/`slider` | | [Dialog](https://nsh.dev/elements/dialog), [Progress](https://nsh.dev/elements/progress), [Video](https://nsh.dev/elements/video), [Slider](https://nsh.dev/elements/slider) |
| Place image | `dialog()` + placement controls | | [Dialog](https://nsh.dev/elements/dialog) |
| Asset preview | `sheet()` | | [Sheet](https://nsh.dev/elements/sheet) |
| Publish/assign + QR | `dialog()` + `select` + `canvas()` QR | QR via `qrcode` crate | [Dialog](https://nsh.dev/elements/dialog), [Select](https://nsh.dev/elements/select), [Canvas](https://nsh.dev/elements/canvas) |
| Shortcut reference | `sheet()` + `text` rows, or `command()` | | [Sheet](https://nsh.dev/elements/sheet), [Text](https://nsh.dev/elements/text), [Command](https://nsh.dev/elements/command) |
| Wallpaper guide | `dialog()` + `text_view()` | | [Dialog](https://nsh.dev/elements/dialog), [TextView](https://nsh.dev/elements/text-view) |
| Plugin frame | `dialog()`/pane + `web_view()` | | [Dialog](https://nsh.dev/elements/dialog), [WebView](https://nsh.dev/elements/web-view) |
| Toasts | `toast_stack()` + `toast()` | success/error/info | [Toast](https://nsh.dev/elements/toast) |
| Tooltips | `tooltip()` | all icon buttons | [Tooltip](https://nsh.dev/elements/tooltip) |

## Host admin chooser (`packages/host/src/admin`)

The host's small browser chooser is a separate surface from the editor shell,
but it is the same product UI and maps the same way.

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Chooser page | `view` + header `text` + grouped sections | | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| Display / template / theme group labels | `text` section headings | | [Text](https://nsh.dev/elements/text) |
| Theme row (`theme-row.tsx`) | `button()` card + `image`/placeholder + name/author + current `Badge` | | [Button](https://nsh.dev/elements/button), [Image](https://nsh.dev/elements/image), [View](https://nsh.dev/elements/view) |
| Edit action | `IconButton` opening the editor window | | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| Delete action | `IconButton` + `alert_dialog()` destructive confirm | recycle-bin copy | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip), [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Cost warning | `text` + `Badge`, `role=status` equivalent (`aria-live` via NSH accessibility) | | [Text](https://nsh.dev/elements/text), [View](https://nsh.dev/elements/view) |
| Status line | `text` in a live region | | [Text](https://nsh.dev/elements/text) |
| Empty state | `EmptyState` | | [View](https://nsh.dev/elements/view) |

## Player surface (desktop now, mobile roadmap)

| Element | NSH proposal | Notes | Docs |
|---|---|---|---|
| Theme canvas | `video()` for video themes; `canvas()`/`image()` for rendered themes | full-bleed | [Video](https://nsh.dev/elements/video), [Canvas](https://nsh.dev/elements/canvas), [Image](https://nsh.dev/elements/image) |
| Theme chooser strip | `select()` or `sheet()` list | paired/own theme | [Select](https://nsh.dev/elements/select), [Sheet](https://nsh.dev/elements/sheet) |
| Top/bottom text strips | `view` + `text`, absolute overlay | safe areas on mobile | [View](https://nsh.dev/elements/view), [Text](https://nsh.dev/elements/text) |
| Display controls | `toolbar()`: fullscreen `button`, keep-awake `toggle`, dim `slider`, status `text` | auto-hide on idle | [Toolbar](https://nsh.dev/elements/toolbar), [Button](https://nsh.dev/elements/button), [Toggle](https://nsh.dev/elements/toggle), [Slider](https://nsh.dev/elements/slider), [Text](https://nsh.dev/elements/text) |
| Load failure | `view` + heading/lede/reason + retry `button` + host `link()` | | [View](https://nsh.dev/elements/view), [Button](https://nsh.dev/elements/button), [Link](https://nsh.dev/elements/link) |
| Availability notice | banner `view` + `button` | | [View](https://nsh.dev/elements/view), [Button](https://nsh.dev/elements/button) |
| Mobile player | `view` + `video`/`canvas` + `overlay` controls, `sheet` chooser, `alert_dialog` failures | iOS/Android packaging | [View](https://nsh.dev/elements/view), [Video](https://nsh.dev/elements/video), [Canvas](https://nsh.dev/elements/canvas), [Overlay](https://nsh.dev/elements/overlay), [Sheet](https://nsh.dev/elements/sheet), [AlertDialog](https://nsh.dev/elements/alert-dialog) |
| Android live wallpaper / iOS wallpaper | not an NSH element | native host/service work | — |

## Source atoms → NSH

| Source atom | NSH | Docs |
|---|---|---|
| `ControlText`, `ControlWell` inputs | `input()` | [Input](https://nsh.dev/elements/input) |
| `ControlNumber`, `ControlPair` | `number_input()` × 1/2 | [NumberInput](https://nsh.dev/elements/number-input) |
| `ControlSelect`, `ControlSwatch` | `select()` | [Select](https://nsh.dev/elements/select) |
| `ControlSegmented` | `toggle_group()` + `toggle()` | [Toggle](https://nsh.dev/elements/toggle) |
| `ControlToggle`, `ControlTogglePair` | `switch()` | [Switch](https://nsh.dev/elements/switch) |
| `ControlChoiceRow` | `radio_group()` + `radio()` | [Radio](https://nsh.dev/elements/radio) |
| `ControlSlider` | `slider()` + `number_input()` | [Slider](https://nsh.dev/elements/slider), [NumberInput](https://nsh.dev/elements/number-input) |
| `ControlHint` | `tooltip()` + `IconButton` | [Tooltip](https://nsh.dev/elements/tooltip), [Button](https://nsh.dev/elements/button) |
| `ControlIconButton` | `button()` + `tooltip()` + label | [Button](https://nsh.dev/elements/button), [Tooltip](https://nsh.dev/elements/tooltip) |
| `ControlTooltip` | `tooltip()` | [Tooltip](https://nsh.dev/elements/tooltip) |
| `ColourPicker`, `ColourPickerPanel` | `color_picker()` + parts | [ColorPicker](https://nsh.dev/elements/color-picker) |
| `GradientEditor` | custom `GradientControl` | [ColorPicker](https://nsh.dev/elements/color-picker), [Slider](https://nsh.dev/elements/slider), [Canvas](https://nsh.dev/elements/canvas) |
| `Dialog`, `openModal` | `dialog()`, `alert_dialog()`, `sheet()` | [Dialog](https://nsh.dev/elements/dialog), [AlertDialog](https://nsh.dev/elements/alert-dialog), [Sheet](https://nsh.dev/elements/sheet) |
| `InspectorSection` | `accordion`/`collapsible` | [Accordion](https://nsh.dev/elements/accordion), [Collapsible](https://nsh.dev/elements/collapsible) |
| `PresetContextMenu` | `context_menu()` | [Menu](https://nsh.dev/elements/menu) |
| `SensorKeyDot` | `Badge`/`view` dot | [View](https://nsh.dev/elements/view) |
| Base UI Toggle/ToggleGroup | `toggle`/`toggle_group` | [Toggle](https://nsh.dev/elements/toggle) |
| Base UI Select | `select` | [Select](https://nsh.dev/elements/select) |
| Base UI Switch | `switch` | [Switch](https://nsh.dev/elements/switch) |
| Base UI Checkbox | `checkbox` | [Checkbox](https://nsh.dev/elements/checkbox) |
| Base UI RadioGroup | `radio_group` | [Radio](https://nsh.dev/elements/radio) |
| Base UI Slider | `slider` | [Slider](https://nsh.dev/elements/slider) |
| Base UI Popover/Menu/Dialog | `popover`/`menu`/`dialog` | [Popover](https://nsh.dev/elements/popover), [Menu](https://nsh.dev/elements/menu), [Dialog](https://nsh.dev/elements/dialog) |
| Base UI Accordion | `accordion` | [Accordion](https://nsh.dev/elements/accordion) |
| Base UI Tabs | `tabs` | [Tabs](https://nsh.dev/elements/tabs) |
| Fabric canvas | raster `image()` + `canvas()` overlays | [Image](https://nsh.dev/elements/image), [Canvas](https://nsh.dev/elements/canvas) |

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
