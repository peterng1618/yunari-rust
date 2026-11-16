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

- [x] Appearance rows: fill, stroke, stroke width, opacity; rectangle corner radius; image asset + fit; video asset + loop/muted.
- [x] Text layout rows: align, overflow, vertical align, vertical, node line height; per-run size and colour.
- [x] Tests: `a_fill_takes_a_hex_or_a_palette_reference`, `opacity_stays_between_zero_and_one`, `a_rectangle_has_a_radius_and_an_ellipse_does_not`, `run_text_and_a_known_token_are_written`, `alignment_and_overflow_take_their_named_values`.
- [ ] Deferred: shadow, border width and frosted-glass fields (the model carries `glass`, the renderer plans it, but the source's four-row paint band is not ported yet); polygon/triangle kinds do not exist in the Rust model. Both wait for the parity sweep (phase 7), since adding shapes changes the model and every renderer.
- [x] Deliberate difference: the source inspector orders Content/Position/Layer/Paint; this panel keeps Transform first, then Appearance, then the kind's section. Same fields, different grouping.

### Task 3: Palette tokens

- [x] `palette.rs`: add (free `slug` id), rename, recolour, remove refused while any style or chart paint names `palette.<id>`.
- [x] Reserved `none` token: cannot be added, renamed, recoloured or removed.
- [x] Inspector rows: name and colour per token, `+ Add colour`; blank theme seeds the source's eight tokens in `blank()`.
- [ ] Deferred: gradient editing (a solid or gradient token reads as its stop count; only solid colours are writable), delete-with-reassign, and the source's colour picker/gradient editors. Add when the token pane becomes its own pane in the parity sweep.
- [ ] Reference rewrite on delete is refused instead: a token in use cannot be deleted, so no reference can dangle. Reassignment is a phase-7 nicety.

### Task 4: Type tokens and text runs

- [x] `tokens.rs`: add (free id, source default family/weight), family and weight edits (numeric or named), removal refused for role ids `heading`/`body`/`mono` and while any run names the token.
- [x] Inspector rows: family and weight per token, `+ Add type token`; text section lists each run's text, token, size and colour.
- [x] Token field validates against declared tokens and accepts `body` or `typeTokens.body`, storing the bare id the renderer reads.
- [x] Blank theme seeds `heading`, `body`, `mono` with the source's families and weights.
- [ ] Deferred: font faces and the family/weight pickers built from declared faces, the `Import local font…`/`Font finder…` actions, and computed token names. The plain family/weight fields write the same document, so a face picker is an upgrade of the row, not of the model.
- [ ] Deferred: per-run line height/letter spacing (the source stores tracking once per text object; the Rust renderer reads only node `lineHeight`).

### Task 5: Artboard, background and theme metadata

- [x] Document sections shown with nothing selected: Theme (name, author, description, version, language, tags), Artboard (width, height, content fit, shape), Background (mode, colour, media).
- [x] Artboard dimensions are whole numbers from 1 to 16384, refused rather than clamped.
- [x] Background media: pick a declared image/SVG/video asset, clear it, bounds, scale 1–5, offsets -5..5, both flips, video fallback frame, and advanced composition anchor/size/fade with the model's ranges. Media colour is the palette reference once media exists, matching the source's colour slot rule.
- [x] Background colour accepts palette references only, as the source's picker does; an empty field clears the active slot.
- [ ] Deferred: ratio/resolution/orientation presets and the outside-artboard warning; language picker; tag pattern (max 5, `[a-z0-9-]`) — tags are split on commas and stored trimmed; credit list. All read the same fields.

### Task 6: Assets and bindings

- [x] Asset pickers on image and video nodes validate the id against declared assets and refuse an asset of the wrong kind; a background media row does the same.
- [x] `bindings.rs`: add under a free `binding-<n>` id (semantic key validated by the model's own predicate), edit key/unit/precision/unit display/format, remove refused while any value run names the binding.
- [x] Inspector bindings section per node, `+ Add binding`.
- [ ] Deferred: asset import (file picker, kind by extension, 80 MiB/128-asset limits, image downscale, video trim, font metadata form, SVG sanitising) and the semantic-key picker with measured/unavailable states. Import is the phase-6 host surface; the picker needs the live sampler in the window (phase 4).
- [ ] Deliberate difference: bindings are edited as rows with a typed key rather than a dropdown of every vocabulary key, because the vocabulary is ~140 entries and a prompt cannot scroll.

### Task 7: Charts

- [x] `inspector.rs` chart rows: family choice (`gauge|line|bar|pie|radar|polarBar`) and the scalar/flag/paint settings each family's renderer reads, written into the untyped settings map with the shapes `chart_fill` accepts (`{"kind":"solid","color":…}`, `{"ref":"palette.…"}`, numbers, flags).
- [x] Empty scalar clears the key so the renderer's default applies.
- [ ] Deferred: per-family advanced settings (segments, interpolation, dash, sampling, totals, animation durations/easings), paint arrays (`palette.0`…), series aspect ratio, and the series precision/scale/offset controls (the generic binding rows already cover key/precision/unit display/format). Settings the renderer does not read are not offered yet.

### Task 8: Add menu, card library and the 3C exit check

- [x] Add commands and builders: text, vertical text, rectangle, ellipse, polygon-shaped path, horizontal line, arc, wedge, and the six chart families at the source's defaults (240x160, palette-derived paints when the token exists).
- [x] Open path presets paint ink as a stroke at width 2; closed ones take the surface fill, as the source does.
- [ ] Deferred: the eight-card bundled library (`default.widgets.zip`). Cards reference packaged icons and assets, which arrive with the pack/store phase; inserting a card without its bytes would produce broken references. Cascade placement already matches (inset/step).
- [ ] Deferred: custom shape presets and the author/pack preset list.

**Exit check:** every field the Rust model declares for a node or document has a row and a tested write path except the deferred ones above; `cargo test`, `cargo clippy -D warnings` and the workspace run are the gate. Pointer-driven panel clicks remain covered by tests only, because the driver cannot send pointer input to an unfocused window; keyboard and toolbar commands are exercised by hand.
