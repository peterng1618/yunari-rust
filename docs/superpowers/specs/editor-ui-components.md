# Editor UI components — gpui-kit mapping

Decision record for the editor UI rebuild. Every Yunari surface element is
mapped to a `gpui-component` 0.9.1 component, or to a composition of kit
primitives when the kit has no exact widget. Evidence paths are in the crate as
installed (`gpui-component-0.9.1/src/...`); `gpui-kit` re-exports it as
`gpui_kit::component`.

## Decision rules

1. **Kit component first.** A styled control is used unless the element is
   canvas geometry (selection handles, guides, artboard chrome) or a direct
   drawing of theme content (the renderer's own output).
2. **Compose, don't re-implement.** A card, property row or token swatch is a
   non-interactive container built from kit containers plus kit controls. A
   hand-rolled *interactive* element is a defect.
3. **Theme tokens, not constants.** Colours, radii, spacing, text sizes come
   from `ActiveTheme`/`Theme`, never `rgb(0x2e2e38)` literals.
4. **Source IA decides placement.** The kit decides how a control looks and
   behaves; the reference screen map decides where it lives and what it is next
   to.
5. **One action source.** `Command::ALL` remains the single registry; menus,
   toolbar and the command palette all render from it.

## App shell: Resizable, not Dock

**Recommendation: `h_resizable` + `resizable_panel`, with `TabBar` inside panes.
Do not use `DockArea` for the main editor layout.**

Why Dock is the wrong fit here:

- `DockArea` (`gpui-base/src/dock/dock_area.rs`, skinned by
  `gpui-component/src/dock/dock.rs`) is a tabbed docking workspace: panels are
  tab groups, draggable, closable, zoomable, and the layout tree is serialized.
  Every pane becomes a `Panel` impl with `panel_name`, `panel_id`, `dump`,
  `visible`, `closable`, `zoomable`, activation events.
- Yunari's editor is a **single-document surface with fixed semantic regions**
  (left assets/layers, centre canvas, right inspector). The source has no panel
  drag, no per-pane tabs of user-chosen panels, no floating panels.
- Using Dock means disabling most of Dock: `closable=false`, `zoomable=false`,
  fixed sizes, drag constraints. That is `Resizable` with extra state,
  persistence and migration surface.
- Dock's strengths become relevant only if the product gains a multi-panel
  workspace (several themes tiled, a console/debug dock). Revisit after P5.

Use instead:

```rust
use gpui_kit::component::*;

h_resizable("editor-split")
    .child(
        resizable_panel()
            .size(px(260.))
            .size_range(px(200.)..px(380.))
            .child(self.left_pane(cx)),
    )
    .child(resizable_panel().child(self.canvas(cx)))
    .child(
        resizable_panel()
            .size(px(300.))
            .size_range(px(260.)..px(460.))
            .child(self.inspector(cx)),
    )
```

(`h_resizable` in `gpui-component/src/resizable/mod.rs` adds the kit divider;
`resizable_panel`/`size_range` in `gpui-base/src/resizable/panel.rs`.)

## Shell and navigation

| Yunari element | Component | Notes |
|---|---|---|
| Window title bar / drag region | `title_bar::TitleBar` | `gpui-component/src/title_bar.rs`; use `WindowOptions` with custom titlebar on Windows |
| Menu bar (File/Edit/View/Theme/Help) | `menu::AppMenuBar` | `src/menu/app_menu_bar.rs`; builds `OwnedMenu` from `Command` groups; Windows/Linux placement |
| Mode navigation (Themes / Editor / Settings / Store) | `tab::TabBar` with `TabVariant::Underline` | `src/tab/`; one Tab per mode, same order as source |
| Global actions (Save, Export, Undo/Redo) | `button::Button` (+`ButtonVariants`) | primary/ghost/danger variants; label + icon, `Kbd` hint in tooltip |
| Command palette | `command::Command` + `CommandState` | `src/command/`; entries generated from `Command::ALL`, grouped as source menus |
| Status bar | `status_bar::StatusBar` | `src/status_bar.rs`; left = document state/zoom, right = provider/save status |
| Resizable pane dividers | `resizable::{h_resizable, resize_handle}` | kit hairline + drag pill; no manual drag code |
| Collapse/expand a pane | `button::Button` icon + `resizable_panel().visible(false)` or a kit `Collapsible` in the pane header | state persists in editor view state, never the document |

## Editor regions

### Left pane

| Element | Component | Notes |
|---|---|---|
| Pane switch (Layers / Assets) | `tab::TabBar` (Pill or Outline) | matches source's pane tabs; state in editor view state |
| Layers tree with groups | `tree::tree(&TreeState, render_item)` | `TreeItem{id,label,children}` in `gpui-base/src/tree.rs`; row is a `ListItem` so eye/lock/rename controls fit; context menu via `ContextMenuExt` |
| Layer drag reorder | Tree row + kit drag plumbing (`on_drag`/drop target from GPUI) | tree gives hierarchy and selection; reorder stays our behaviour layer |
| Asset grid/list with thumbnails | `list::List` or `list::VirtualList` with a custom row | thumbnails come from `crates/render-raster`; row uses kit `Skeleton` while loading |
| Asset search | `input::Input` with search content type (`input::search`) | clear button and focus ring come from the kit |
| Asset category filter | `select::Select` or `combobox::Combobox` | searchable when the list is long (stock sources, Wallpaper Engine) |

### Canvas

| Element | Component | Notes |
|---|---|---|
| Artboard rendering | not a kit component | `crates/render-gpui` draws the document's display list; this is content, like a chart canvas |
| Zoom / fit controls | `button::Button` group + `slider::Slider` for continuous zoom | source exposes the same actions; `Kbd` hints in tooltips |
| Selection handles, marquee, snap guides | custom GPUI painting | canvas geometry; kit has no scene editor. Use kit `Tooltip`/`PopupMenu` for associated affordances |
| Canvas context menu | `menu::PopupMenu` / `ContextMenuExt` | right-click actions from `Command::ALL`, grouped like the source |
| Hover/selected outlines | theme tokens from `ActiveTheme` | never literal colours |

### Right inspector

| Element | Component | Notes |
|---|---|---|
| Inspector sections | `accordion::Accordion` or `collapsible::Collapsible` | section open/closed state is view state; source persists it per session |
| Property row (label + control) | `h_flex` + `label::Label` + control | compose one row helper; do not build a bespoke "field" framework |
| Text / number fields | `input::{Input, InputState}`; `input::number_input` for numeric | validation/error state via the kit's error styling |
| Enum choice (align, overflow, blend) | `select::Select` | icon-only where the source uses icons |
| Toggle (visible, lock, autoplay) | `switch::Switch` or `checkbox::Checkbox` | source uses checkboxes in inspector rows; match |
| Colour paint | `color_picker::ColorPicker` (+ presets) | `src/color_picker/`; gradient authoring stays a custom control composed of colour pickers and stops |
| Opacity / angle / threshold | `slider::Slider` with `Input` beside it | source pairs slider + numeric field |
| Palette tokens | `table::Table` or list of rows | token id, colour swatch, delete; table only if columns earn it |
| Type tokens | list of rows with `Select` + `Input` + font picker | family/weight/size per role |
| Binding key picker | `select::Select`/`combobox` with unavailable rows disabled | host catalogue feeds it; `Badge` marks unavailable |
| Chart requirement rows | list rows with `Badge`, `Select`, `ColorPicker` | one row per binding/series, matching source order |

Note: `gpui-component/src/inspector/` is a **developer inspector** (debug UI for
GPUI state), not a property panel. Do not use it for the theme inspector.

### Library / Themes mode

| Element | Component | Notes |
|---|---|---|
| Theme grid | `list::VirtualList` + custom card container | card = kit container + thumbnail + `Button` menu; no kit card widget |
| Search box | `input::Input` (search) | same control as assets |
| Filter dropdowns (Author, Colour mode, Sensors, Shape, Style, Sort) | `select::Select` | source has six compact filters; keep their order |
| Density control | `slider::Slider` (Comfortable ↔ Dense) | kit slider with end labels |
| Empty / unreadable library state | `empty::Empty` + `Button` retry | replaces the current silent blank |
| Import / New theme actions | `button::Button` in the page toolbar | primary action placed as source |

### Settings / Store modes

| Element | Component | Notes |
|---|---|---|
| Settings page layout | `setting::{SettingPage, SettingGroup, SettingItem}` | `src/setting/`; built for exactly this |
| Credentials / custom source form | `form::Form` + `Input`/`Select`/`Switch` | validation messages from kit form state |
| Store listings | `list::VirtualList` + card rows; `Table` only for tabular catalogue data | keep thumbnail, publisher, version visible |
| Install / publish progress | `progress::Progress` or `spinner::Spinner` | disabled action until done |
| Account sign-in dialog | `dialog::Dialog` + `Input` | no bespoke overlay |

## Overlays and feedback

| Element | Component |
|---|---|
| Modal dialogs (Settings, Store, widgets, path presets, MCP, save cleanup, font import) | `dialog::Dialog` / `AlertDialog`, `DialogHeader`, `DialogFooter` (`src/dialog/`) |
| Side sheet (long forms, asset details) | `sheet::Sheet` |
| Tooltips with shortcut hint | `tooltip::Tooltip` + `kbd::Kbd` |
| Toasts (saved, export written, install finished) | `notification::Notification` (`Notification::success\|error\|info`) |
| Inline validation / refusals | kit `Alert` inside the form, plus field error styling |
| Loading placeholders | `skeleton::Skeleton` for cards/rows, `spinner::Spinner` for blocking work |
| Empty states | `empty::Empty` with the source's copy and the primary action |
| Keyboard shortcut hints | `kbd::Kbd`; one tabled mapping from `Command::ALL` |
| Badges/tags (version, store kind, unavailable) | `badge::Badge`, `tag::Tag` |
| Breadcrumb (Store category path) | `breadcrumb::Breadcrumb` |

## Custom compositions (no exact kit widget)

These are allowed because they are containers or canvas geometry, not
re-implemented controls:

- Theme card: kit container + `Skeleton`/thumbnail + `Button` overflow menu.
- Property row: `h_flex` + `Label` + kit control, with a shared helper.
- Gradient editor: stops list + two `ColorPicker`s + `Slider` for the angle;
  the preview strip is painted.
- Colour/token swatch strip: painted strip + popover with `ColorPicker`.
- Canvas overlays: selection box, handles, snap guides, ruler ticks.
- Chart preview inside the inspector: the renderer's own output
  (`crates/render-raster`), not `plot`/`chart` kit widgets.
- Status indicators that are pure labels: kit `Label`/`Badge`, not custom text.

Rejected: `DockArea` for the shell (above); `inspector` (developer tool);
`plot`/`chart` for theme charts (the renderer owns them); `CodeEditor` unless a
raw JSON/theme editor is deliberately exposed (not in the source editor).

## Implementation notes

- `gpui_kit::component::init(cx)` must run before any kit component; call it
  beside `gpui_kit::init(cx)` in app boot.
- Apply the kit theme once (`Theme`/`ThemeMode`, `src/theme/`); don't set
  per-widget colours. The render canvas still uses the document's palette.
- Every interactive control needs an accessible label. The kit components carry
  AccessKit roles; a custom canvas overlay needs an explicit label and state.
- Toolbar/menu items are generated from `Command::ALL` so the keyboard path and
  the visible control cannot drift apart.
- Panes: `TabBar` for pane switching, not `Sidebar`. Sidebar is for app-level
  navigation with icons and collapsible groups; Yunari's mode switch is a
  four-item tab bar, matching the source.
- Persisted editor view state (pane widths, open sections, selected pane tab)
  lives in the editor shell, never in the document — same rule as the source.

## Acceptance per component

Each screen's checklist (reference doc) names which of these must be visible and
interactive. The proof is a 1920x1080 pair (source reference, port capture) plus
an interaction note for the screen's controls. A component in the tree that is
not visible, not focused, or not usable fails the screen regardless of which kit
widget it is.

## Open questions for P1

1. Whether the left pane's Layers/Assets switch is a `TabBar` (source shows
   tabs) or a segmented control — resolved against the reference screenshots,
   not by preference.
2. Whether the layers tree rows need inline rename (`Input` popped over the row)
   or a rename dialog; source behaviour decides.
3. Whether store catalogue needs `Table` (sortable columns) or cards; depends on
   the reference screen.
4. Multi-select semantics in lists (shift/cmd click) — check kit `List`
   selection support before writing a custom one.
