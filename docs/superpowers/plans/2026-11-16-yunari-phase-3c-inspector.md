# Phase 3C — The Inspector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every property a theme can carry becomes editable in the editor window — node transforms and styles, per-kind content, palette and type tokens, artboard and background, assets, bindings, charts and the card presets — each edit through `Editor::edit`, so undo and the dirty dot stay in step.

**Architecture:** The inspector is a second 260 px column beside the canvas. It is data-driven: `inspector` describes the selected node as a list of sections and rows, each row carrying a getter and an edit closure, and the window renders the rows and submits the change. Numeric typing coalesces into one history entry per focus session, like a drag. Nothing new renders: every field writes a path the render plan already reads, so an edited document looks different before it is saved.

**Tech Stack:** Rust 2024, gpui-kit, `yunari-model` documents, `yunari-render` plan.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- Every mutation goes through `Editor::edit`; a gesture or a run of keystrokes in one field goes through `begin_burst`/`end_burst`, so one change is one Ctrl+Z.
- Unknown ids, locked nodes and out-of-range numbers are refused, never clamped silently into a save.
- Palette references (`{"ref":"palette.<id>"}`) stay references when a value is edited; a literal becomes `{"value":"<hex>"}`.
- A numeric field accepts only what the model accepts: finite numbers, geometry at least 1 unit on a side, opacity 0–1, rotation 0–360, scale > 0.
- The inspector never invents a document: it only writes fields the model already declares, and a save must still validate.
- Tests live beside the modules they cover and use `crate::fixtures`.

## Review Focus

- Typing a colour, a name or a number leaves exactly one undo entry, and a refused value leaves the document and the field as they were, with the reason on the status line.
- A palette token that a node still references cannot be deleted; deleting an asset still referenced by a node is refused with the referencing node named.
- Changing artboard width/height does not move or clip nodes silently; the canvas refits and the change is one entry.
- A text run's binding survives editing the run's text or token, and removing a binding removes the run reference.
- A chart with an unknown semantic key or a malformed settings number draws the gap/issue the plan already defines, and the inspector reports it rather than hiding it.

---

### Task 1: Inspector scaffold and the transform section

**Files:**
- Create: `crates/editor/src/inspector.rs`
- Modify: `crates/editor/src/lib.rs`, `crates/editor/src/ui.rs` (inspect column renders on the left of the layers panel, or a tab beside it)
- Test: in `inspector.rs`

**Interfaces:**
- Produces: `Section { title: &'static str, rows: Vec<Field> }`;
  `Field { key: String, label: &'static str, kind: FieldKind, value: String }`;
  `FieldKind::{Number { min, max, step }, Text, Colour, Choice(Vec<&'static str>), Toggle}`;
  `pub fn sections(document: &ThemeDocument, id: &str) -> Vec<Section>` — the selected node's fields in the order the inspector shows them (Transform first);
  `pub fn apply(document: &mut ThemeDocument, id: &str, key: &str, value: &str) -> Result<(), String>` — writes one field; `key` is `"transform.x"`, `"style.fill"`, `"kind.cornerRadius"`, `"kind.content.runs.0.text"`, …;
  `pub fn set_style(document: &mut ThemeDocument, id: &str, key: &str, value: Option<StyleValue>)`.
- Transform keys: `transform.x`, `transform.y`, `transform.width`, `transform.height`, `transform.rotation`, `transform.scaleX`, `transform.scaleY` — whole-unit geometry, rotation 0–360, scale > 0 and at most 64.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn the_transform_section_leads_for_a_selected_node() { /* sections()[0].title == "Transform", first key "transform.x" */ }

#[test]
fn a_typed_number_becomes_the_nodes_transform() { /* apply "transform.x" "42" -> 42.0 */ }

#[test]
fn a_non_number_is_refused_and_changes_nothing() { /* apply "transform.x" "wide" -> Err, x unchanged */ }

#[test]
fn geometry_never_goes_below_one_unit() { /* apply "transform.width" "0" -> Err */ }

#[test]
fn the_rotation_wraps_into_zero_to_three_sixty() { /* "400" -> 40.0 */ }
```

- [x] **Step 2:** Run, see them fail.
- [x] **Step 3:** Implement `sections` and `apply` for the transform half. A write is `document.nodes` walk by id then `transform.<field> = parsed`.
- [x] **Step 4:** Render the column: a heading per section and one row per field (label left, value right). Clicking a row opens the existing prompt overlay over the field's value — gpui's text input would be a new widget for the same result — and Enter submits through `editor.edit(apply)`, Esc cancels; the status line carries a refusal. Layers moved to the left and the inspector sits right of the canvas, as the source lays them out.
- [x] **Step 5:** Tests green. By hand: adding a rectangle shows Transform with X 40, Y 40, Width 360, Height 200, Rotation 0, Scale 1. Opening a field prompt needs a click, which the driver cannot send, so the write path is covered by `apply` tests only.
- [x] **Step 6:** Commit. `feat(editor): an inspector that edits the selected node's transform`

### Task 2: Appearance and per-kind fields

- [ ] Filled from the source survey.

### Task 3: Palette tokens

- [ ] Filled from the source survey.

### Task 4: Type tokens and text runs

- [ ] Filled from the source survey.

### Task 5: Artboard, background and theme metadata

- [ ] Filled from the source survey.

### Task 6: Assets and bindings

- [ ] Filled from the source survey.

### Task 7: Charts

- [ ] Filled from the source survey.

### Task 8: Card presets and the 3C exit check

- [ ] Filled from the source survey.
