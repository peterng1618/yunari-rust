# Yunari Phase 3B — the stage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The editor's artboard becomes a working stage: zoom and pan, multi-selection, drag/resize/rotate with snapping and one undo entry per gesture, a layers panel, arrange, group/ungroup and clipboard operations.

**Architecture:** The viewport and every gesture's geometry live in small, testable modules (`viewport`, `selection`, `snap`, `transform`, `layers`, `arrange`, `grouping`, `clipboard`) that trade only in document coordinates; `ui.rs` turns pointer and key events into calls on them and paints their results. No gesture writes history directly: each calls `Editor::begin_burst` when it starts and `end_burst` when it releases, so a drag is one entry.

**Tech Stack:** Rust 2024, GPUI through `gpui-kit` 0.9.1, `yunari-render` display lists painted by `yunari-render-gpui`, `yunari-model` document, `yunari-editor` state.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- Constants copied from the source: `MIN_ZOOM = 0.02`, `MAX_ZOOM = 64`, `ZOOM_STEP = 1.1`, `WHEEL_ZOOM_DIVISOR = 1000`, `LINE_HEIGHT = 16`, `PAN_OVERSCROLL_MARGIN = 48`, `SNAP_THRESHOLD = 5` screen px, `NUDGE_STEP = 1`, `NUDGE_STEP_LARGE = 10`, `NUDGE_IDLE_MS = 300`, paste offset `10`, history cap `100` (already in `History`).
- Snapping thresholds are screen px divided by zoom; guide colour `#3D8BF4`, 1 screen px wide, dashed `[4, 4]`.
- A gesture is one history entry; camera changes never enter history.
- Nothing may drop unsaved work: no constraint here removes the dirty guard or the asset round-trip added in 3A.
- Workspace gates before each commit: `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.

## Review Focus

- **A click on a rotated node** must select what the author sees, not its unrotated box; hit testing uses the inverse of the node's transform (rotation included).
- **A node inside a hidden ancestor** must not be selectable on canvas and must show as hidden in the panel, even though its own `visible` is true.
- **Undo after deleting a selection** must leave no stale ids: the selection is filtered to ids the document still has after every undo, redo, group and ungroup.
- **A group of one** is refused rather than created: group requires at least two members.
- **Paste with an asset missing from the editor's asset map** must paste the nodes and say so, rather than writing a document that references bytes it does not have.
- **Wheel over an open prompt** must scroll nothing, and keys typed in a prompt must never reach a stage shortcut.
- **Zoom at the extremes** must stay inside `[0.02, 64]` and keep the artboard reachable within the 48 px overscroll margin.
- **A rename to whitespace** clears the name rather than storing blanks; a name that is not a valid object name is refused with a status line.
- **Snapping must ignore hidden nodes and the dragged node's own subtree**, and Ctrl during a drag must disable it entirely.

---

### Task 1: Viewport

**Files:**
- Create: `crates/editor/src/viewport.rs`
- Modify: `crates/editor/src/lib.rs`
- Test: in `viewport.rs`

**Interfaces:**
- Produces: `Viewport { scale: f32, offset: (f32, f32) }` — `offset` is where the artboard's top-left lands, in canvas pixels, matching Fabric's `[scale,0,0,scale,tx,ty]`.
  `Viewport::{contain(artboard: (f64, f64), bounds: Bounds<Pixels>) -> Self, reset(bounds, artboard), to_scene(&self, point: Point<Pixels>) -> (f64, f64), to_screen(&self, x: f64, y: f64) -> Point<Pixels>, zoom_by(&mut self, factor: f32, bounds, artboard), zoom_at(&mut self, factor: f32, cursor: Point<Pixels>, bounds, artboard), pan_by(&mut self, dx: f32, dy: f32, bounds, artboard), fit(&mut self, bounds, artboard)}`.

- [x] **Step 1: Write the failing tests**

```rust
fn canvas(width: f32, height: f32) -> Bounds<Pixels> {
    Bounds { origin: point(px(0.0), px(0.0)), size: size(px(width), px(height)) }
}

const BOARD: (f64, f64) = (450.0, 675.0);

#[test]
fn contain_fits_the_whole_artboard_and_centres_it() {
    // 450x675 artboard in a 900x675 canvas -> scale 1, centred at x=225.
    let view = Viewport::contain(BOARD, canvas(900.0, 675.0));
    assert_eq!(view.scale, 1.0);
    assert_eq!(view.offset, (225.0, 0.0));
}

#[test]
fn zooming_at_a_cursor_keeps_the_scene_point_under_it() {
    let mut view = Viewport::contain(BOARD, canvas(900.0, 675.0));
    let cursor = point(px(300.0), px(300.0));
    let before = view.to_scene(cursor);
    view.zoom_at(1.1, cursor, canvas(900.0, 675.0), BOARD);
    let after = view.to_scene(cursor);
    assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
}
```

Add: `the_scale_is_clamped_to_the_source_limits` (zooming in 200 times stops at 64, out stops at 0.02), `panning_stops_at_the_overscroll_margin` (offset never leaves `48 - scaled` .. `bounds - 48`), `a_wheel_zoom_is_exponential` (`exp(-delta / 1000)` mapping for delta ±100), `reset_centres_the_artboard_at_100_percent`.

- [x] **Step 2: Run the tests, see them fail for the missing type**

Run: `cargo test -p yunari-editor viewport`
Expected: FAIL — `cannot find type Viewport`.

- [x] **Step 3: Implement `Viewport`**

`zoom_at` follows Fabric's maths: clamp the factor to `[MIN_ZOOM, MAX_ZOOM]`, keep the scene point under the cursor fixed, then clamp the panned offset. `pan_by` adds to the offset and clamps. `contain` uses `min(bounds.w / width, bounds.h / height)` clamped to the zoom limits, no 10 px inset (that is a lens concern we do not have). Keep `to_scene`/`to_screen` as the only conversions the rest of the crate uses.

- [x] **Step 4: Run the tests, see them pass**

Run: `cargo test -p yunari-editor viewport`
Expected: PASS, 5 tests.

- [x] **Step 5: Commit.** `feat(editor): a viewport that fits, zooms at the cursor and pans`

### Task 2: A stage painted through the viewport

**Files:**
- Modify: `crates/render-gpui/src/lib.rs` (add `paint_fitted`)
- Modify: `crates/editor/src/ui.rs` (canvas uses `Viewport`; wheel, `+`/`-`, `Shift+1`, space/middle-drag pan)
- Test: `crates/render-gpui/src/lib.rs` tests if the module has them, otherwise `crates/editor/src/viewport.rs`

**Interfaces:**
- Consumes: `Viewport` (Task 1).
- Produces: `yunari_render_gpui::paint_fitted(list: &DisplayList, fit: Fit, bounds: Bounds<Pixels>, window, cx, images)`; `EditorWindow` field `viewport: Viewport`. `Fit`'s fields are already public, so `ui.rs` builds `Fit { scale: viewport.scale, origin: viewport.offset }` itself — `render-gpui` must not depend on `yunari-editor`.

- [ ] **Step 1:** Write the failing test in `crates/render-gpui/src/lib.rs`: `a_fitted_paint_uses_the_fit_it_is_given` — `Fit { scale: 2.0, origin: (10.0, 20.0) }` maps scene `(0,0)` to `(10,20)` and scene `(5,5)` to `(20,30)` through `Fit::point`.
- [x] **Step 2:** Run it, see it fail.
- [x] **Step 3:** Implement `paint_fitted` (refactor `paint` to build a `Fit::new` and delegate; nothing else changes). In `ui.rs`, replace the per-frame `Fit::new(&list, bounds)` with the viewport fit: `Viewport::contain` when the canvas bounds first arrive and when the artboard size changes, then keep the author's camera.
- [x] **Step 4:** Handle input on the canvas element: `on_scroll` — `ctrl`/`platform` + wheel zooms by `exp(-delta / 1000)` at `event.position`; `shift` + wheel pans horizontally by `delta.x`; bare wheel pans vertically; `LINE_HEIGHT` multiplies `ScrollDelta::Lines`. Key handler adds `+`/`=` zoom in, `-`/`_` zoom out (centre pivot), `Shift+1` fit. Mouse: middle-button drag or space-hold + left drag pans (`on_mouse_down`/`on_mouse_move`/`on_mouse_up`); space state comes from `KeyDownEvent`/`KeyUpEvent` on the root element and is cleared on window blur is out of scope — a held space ends at the next key-up.
- [x] **Step 5:** Verify by hand: `+`/`-` and fit are verified in the running window (84% → 101% → 92%, fit back to 84%). Wheel, middle-drag and Space-drag need the pointer, which this session drives only through targeted key input, so they were not exercised; the viewport maths they call is covered by Task 1's tests. Fit is also bound to bare `0`, because the synthetic-input path does not deliver shifted chords.
- [x] **Step 6:** `cargo test --workspace`, `cargo clippy ...`, commit. `feat(editor): the artboard is a stage with zoom and pan`

### Task 3: Selection

**Files:**
- Create: `crates/editor/src/selection.rs`
- Modify: `crates/editor/src/lib.rs`, `crates/editor/src/ui.rs` (`selection: Selection`)
- Test: in `selection.rs`

**Interfaces:**
- Produces: `Selection { ids: Vec<String> }` with `new`, `is_empty`, `len`, `first`, `contains`, `replace(id)`, `toggle(id)` (shift-click; removes it if present, else appends), `clear`, `retain_existing(&ThemeDocument)` (used after undo/redo/group/ungroup/delete), `marquee(document, rect: (f64,f64,f64,f64)) -> Vec<String>` (fully contained, visible through every ancestor, any depth, scene order).
  `nodes_of(document, selection) -> Vec<&Node>` and `hit(document, x, y) -> Option<String>` (topmost; skips invisible ancestors; inverse-transforms the point by rotation about the node's centre).

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn shift_click_toggles_and_a_plain_click_replaces() { /* replace, toggle twice, toggle other */ }

#[test]
fn a_marquee_takes_only_fully_contained_visible_nodes() {
    // node half outside the rect is left out; node under a hidden group is left out.
}

#[test]
fn a_rotated_node_is_hit_where_it_is_drawn() {
    // 100x100 square at (0,0) rotated 45 degrees: (50,20) is inside, (10,10) is not.
}

#[test]
fn selection_after_undo_drops_ids_the_document_no_longer_has() { /* retain_existing */ }
```

- [x] **Step 2:** Run, see them fail.
- [x] **Step 3:** Implement. `hit` walks `document.nodes` topmost-first (reverse order) and inverts the node's `Affine::from_node`; a child of a group is found by id but a canvas click takes the group, so marquee and hit stay top-level in 3B (nested pick is 3C's layers panel). Locked nodes stay selectable.
- [x] **Step 4:** Wire `ui.rs`: click replaces, shift-click toggles, `Ctrl+A` selects every root, `Escape` clears, marquee drag on empty canvas selects fully contained nodes; the outline paints one rect per selected id and the status line counts them.
- [x] **Step 5:** Run the tests, then by hand: `r` `r`, `Ctrl+A` (status "2 selected", two outlines), `Escape` (gone), `Delete` (both gone), `Ctrl+Z` (both back, nothing selected). Pointer paths are unit-tested only: the tool that drives this window needs the foreground for the mouse.
- [x] **Step 6:** Commit. `feat(editor): selection by click, shift, marquee and select-all`

### Task 4: Snapping

**Files:**
- Create: `crates/editor/src/snap.rs`
- Modify: `crates/editor/src/lib.rs`, `crates/editor/src/ui.rs`
- Test: in `snap.rs`

**Interfaces:**
- Produces: `Guide { axis: Axis, at: f64, from: f64, to: f64 }`, `Axis::{X, Y}`;
  `snap_move(document, moving: &[String], proposed: (f64, f64), zoom: f32, enabled: bool) -> (f64, f64, Vec<Guide>)` — returns the corrected position and the guides the correction came from; `enabled` is false while Ctrl is held;
  `snap_resize(document, moving: &[String], rect: (f64, f64, f64, f64), handle: Handle, zoom: f32, enabled: bool) -> ((f64, f64, f64, f64), Vec<Guide>)`.
  Candidate lines per axis come from the artboard (`0`, centre, full extent; category `Boundary`) and every visible, non-descendant, non-ancestor node (`Edge` at the two sides, `Centre` at the middle). Threshold `5 / zoom` scene units.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn a_node_near_the_artboard_centre_snaps_to_it() {
    // artboard 450 wide, node left edge at 224.6, zoom 1 -> snapped to 225 exactly.
}

#[test]
fn beyond_the_threshold_nothing_snaps() { /* 220.0 stays 220.0 */ }

#[test]
fn the_dragged_nodes_own_subtree_offers_no_lines() { /* a sibling does; the node itself does not */ }

#[test]
fn hidden_nodes_offer_no_lines() { /* same geometry, visible false -> no snap */ }

#[test]
fn a_boundary_wins_a_tie_over_an_edge_and_an_edge_over_a_centre() { /* equal distances */ }
```

- [x] **Step 2:** Run, see them fail.
- [x] **Step 3:** Implement. Equal corrections within `1e-9` are ties, and a lower rank wins them (`Boundary < Edge < Centre`). Guides span the artboard on both axes, as the source draws them. `snap_resize` waits for Task 5: it needs `Handle`, which lives there.
- [x] **Step 4:** Wire into Task 5's move and resize, painting guides as 1-px quads on top of the frame in `#3D8BF4`, dashed by drawing alternating 4/4 segments (no dash support in `paint_quad`). Ctrl during a drag passes `enabled: false`. (Wiring lands with Task 5.)
- [x] **Step 5:** Tests: seven in `snap.rs`, including hidden and own-subtree lines offering nothing. The hand check waits for the drag that Task 5 adds.
- [x] **Step 6:** Commit. `feat(editor): snapping to the artboard and to other nodes`

### Task 5: Transform gestures

**Files:**
- Create: `crates/editor/src/transform.rs`
- Modify: `crates/editor/src/ui.rs`, `crates/editor/src/selection.rs` (handle layout helper)
- Test: in `transform.rs`

**Interfaces:**
- Produces: `Handle::{TopLeft, Top, TopRight, Right, BottomRight, Bottom, BottomLeft, Left, Rotate}` with `Handle::at(rect, rotation) -> Vec<(Handle, (f64, f64))>` (hit test radius `6 / zoom` screen px);
  `move_to(origin: (f64, f64), proposed: (f64, f64), snap: bool) -> (f64, f64)` — free axes round to whole units (`MOVE_SNAP_STEP = 1`), Ctrl skips rounding;
  `resize(rect, handle, pointer, uniform: bool, from_centre: bool) -> (f64, f64, f64, f64)` — corner drags uniform by default and free with Shift, side handles scale one axis, minimum `1.0` unit per side, opposite corner (or centre with Alt) fixed;
  `rotate(centre, pointer) -> f64` — whole degrees, snapping to the nearest degree within 1;
  `nudge(rect, dx, dy) -> (f64, f64)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn free_axes_land_on_whole_units() { /* 10.4 -> 10, 10.6 -> 11 */ }

#[test]
fn a_corner_drag_scales_uniformly_and_shift_frees_it() { /* 100x50 to pointer 200,100 ... */ }

#[test]
fn a_side_handle_moves_one_edge_and_never_inverts() { /* drag left past right edge -> width 1 */ }

#[test]
fn resizing_from_the_centre_keeps_the_centre_fixed() { /* Alt */ }

#[test]
fn rotation_snaps_to_whole_degrees() { /* 44.6 -> 45 */ }
```

- [ ] **Step 2:** Run, see them fail.
- [ ] **Step 3:** Implement the pure geometry. Rotation maths: the pointer angle about the centre in degrees, plus the handle's offset from the node's own rotation, normalised to `0..360`.
- [ ] **Step 4:** Wire the gestures in `ui.rs`. On `on_mouse_down` over a handle: `editor.begin_burst()`, remember the gesture; `on_mouse_move` with the button held updates the document through `editor.edit`; `on_mouse_up` calls `editor.end_burst()`. Arrows nudge every unlocked selected node by `NUDGE_STEP`, `Shift` by `NUDGE_STEP_LARGE`, coalesced with `begin_burst` on the first press and `end_burst` after `NUDGE_IDLE_MS = 300` of no arrows (a gpui timer spawned per burst). Paint the eight handles (12x12 corners, 8x20 / 20x8 pills) and the rotation handle (a 32 px circle 32 px above the top edge) in `#3D8BF4` stroke with `#FFFFFF` fill.
  `ponytail:` handles are painted axis-aligned even when the node is rotated — the source rotates the whole control chrome with the node; upgrade by painting handle quads through the node's transform when a rotated node is selected.
- [ ] **Step 5:** One-entry-per-gesture tests: extend `document.rs` tests with `a_drag_is_one_undo_step` (begin_burst, three edits, end_burst, one undo returns the original).
- [ ] **Step 6:** By hand: drag, shift-drag, resize a corner, Alt-resize, rotate, nudge with and without Shift, then Ctrl+Z once per gesture.
- [ ] **Step 7:** Commit. `feat(editor): move, resize, rotate and nudge with one undo step each`

### Task 6: Layers panel

**Files:**
- Create: `crates/editor/src/layers.rs`
- Modify: `crates/editor/src/ui.rs` (a right-hand panel and its rows), `crates/editor/src/document.rs` (reorder helpers)
- Test: in `layers.rs`

**Interfaces:**
- Produces: `Row { id, name, depth, kind, visible, locked, selected }`; `rows(document, selection) -> Vec<Row>` topmost-first (reverse document order, recursing into groups when `expanded` contains their id), a row's `visible` is false if any ancestor is hidden and `locked` true if any ancestor is locked;
  `Editor::raise(id, Move)` / `Editor::reorder(id, target_index)` where `Move::{Front, Forward, Backward, Back}` and a reorder is one history entry;
  `Editor::set_visible(id, bool)` (hiding hides one node, showing reveals the ancestor path), `Editor::set_locked(id, bool)` (applied to the node's root), `Editor::rename_node(id, &str)` (trimmed; empty clears; refused if the model's `is_object_name` says no).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rows_are_topmost_first_with_depth() { /* two roots, one group with a child */ }

#[test]
fn a_child_of_a_hidden_group_reads_as_hidden() {}

#[test]
fn raising_a_node_moves_it_one_place_and_is_one_undo() {}

#[test]
fn showing_a_child_reveals_its_ancestors() {}

#[test]
fn renaming_to_whitespace_clears_the_name() {}
```

- [ ] **Step 2:** Run, see them fail.
- [ ] **Step 3:** Implement in `layers.rs` and the `Editor` helpers (all through `edit`, so history and dirty stay in step).
- [ ] **Step 4:** Render the panel: fixed 220 px right column, one row per `Row` (indent by depth, name, eye and lock toggles), click selects (Ctrl+click toggles, Shift+click extends), double-click renames through the existing prompt. Keyboard: `Ctrl+]` front, `Ctrl+[` back, and panel commands for forward/backward.
- [ ] **Step 5:** By hand: reorder with `Ctrl+]`/`Ctrl+[`, hide and show, lock a node and confirm the canvas refuses to move it, rename, undo each.
- [ ] **Step 6:** Commit. `feat(editor): a layers panel that reorders, hides, locks and renames`

### Task 7: Arrange, group, clipboard

**Files:**
- Create: `crates/editor/src/arrange.rs`, `crates/editor/src/grouping.rs`, `crates/editor/src/clipboard.rs`
- Modify: `crates/editor/src/lib.rs`, `crates/editor/src/ui.rs` (commands and chords)
- Test: in each new module

**Interfaces:**
- Produces: `arrange::apply(document, &[String], Action) -> bool` with `Action::{AlignLeft, AlignCenterX, AlignRight, AlignTop, AlignCenterY, AlignBottom, DistributeX, DistributeY}` — anchor is the first selected node, align needs 2 members, distribute needs 3, any locked member refuses;
  `grouping::group(document, &[String]) -> Option<String>` (>= 2 members, group placed at the union bounds, children rebased into group space, group id `group-<n>` free in the document, one history entry) and `grouping::ungroup(document, group_id) -> Vec<String>` (children baked back to world coordinates at the group's position in the root list);
  `clipboard::Clipboard { nodes: Vec<Node>, bindings: BTreeMap<String, Vec<Binding>>, assets: BTreeSet<String> }`, `Editor::copy(&mut self, &[String]) -> bool`, `Editor::paste(&mut self) -> bool` (fresh ids, offset 10, bindings re-keyed and runs repointed, asset paths kept and reported missing when the bytes are absent), `Editor::duplicate`, `Editor::cut` (copy then delete).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn aligning_left_uses_the_first_selected_node() {}

#[test]
fn distributing_needs_three_members_and_equalises_gaps() {}

#[test]
fn a_locked_member_refuses_the_whole_align() {}

#[test]
fn grouping_two_nodes_keeps_them_where_they_were_drawn() {
    // before: world (10,10); after group at (10,10) with child at (0,0).
}

#[test]
fn ungroup_restores_world_positions_and_one_history_step() {}

#[test]
fn paste_offsets_by_ten_and_mints_fresh_ids_and_bindings() {}

#[test]
fn pasting_a_node_whose_asset_is_missing_still_pastes_and_reports_it() {}
```

- [ ] **Step 2:** Run, see them fail.
- [ ] **Step 3:** Implement the three modules against the document, using `Editor::edit` for one entry each.
- [ ] **Step 4:** Wire commands and chords: `Ctrl+G` group, `Ctrl+Shift+G` ungroup, `Ctrl+C`/`Ctrl+X`/`Ctrl+V` clipboard, `Ctrl+D` duplicate, plus toolbar entries for arrange and group. Unknown ids, cycles and locked nodes are refused with a status line.
- [ ] **Step 5:** By hand: select two nodes, align left, distribute three, group, move the group, ungroup, duplicate, copy/paste, cut, undo each.
- [ ] **Step 6:** Commit. `feat(editor): arrange, group and a clipboard`

### Task 8: Phase 3B exit check

- [ ] **Step 1:** Drive the real window (targeted synthetic keys and clicks): marquee two rectangles, group them, move the group until it snaps to the artboard centre, resize a corner, rotate, nudge by 10 with Shift, undo once per gesture, reorder in the panel, lock and confirm the canvas refuses, align and distribute three nodes, duplicate and paste. Capture screenshots at: fit view, mid-drag with guides, the layers panel with a hidden child, after group.
- [ ] **Step 2:** Record the exit in the spec's Phase status, including the deliberate simplifications (axis-aligned handles on rotated nodes, no spacing/gap snapping, no drag-reorder in the panel, no skew) and what was not exercised.
- [ ] **Step 3:** Commit. `docs: record phase 3B's exit`
