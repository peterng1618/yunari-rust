# Phase 3A — Editor shell, document store, library, packages

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A blank theme can be authored, saved, listed, reloaded, exported and re-imported, with undo/redo over every document mutation.

**Architecture:** A `yunari-library` crate owns the theme directory (`<library>/<theme-id>/theme.json` + `assets/`) and the portable zip package. A `yunari-editor` crate owns an in-memory document, its history of snapshots and its saved baseline; every mutation goes through one commit point so undo and dirty cannot disagree. A `crates/player` crate keeps the phase-2 live player; `yunari-app` dispatches to editor or player.

**Tech Stack:** Rust 2024, `serde_json` for the document, `zip` 9 for packages, `sha2` for the save base, `gpui-kit` (through `gpui-component`) for the shell.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md` (crate layout, document model, phase 3 row).

## Global Constraints

- Validation refuses rather than coerces: artboard ≤ 16384 px, node depth ≤ 32, ≤ 5000 nodes, ids `[A-Za-z0-9_-]{1,64}`, assets under `assets/`, schema major 1 only.
- Only authored state goes in `theme.json`; selection and view state never persist.
- A save writes the document as the author holds it: no compression round-trip on the working path.
- A save never writes a document validation refuses; the refusal is reported and the stored file is left as it was.
- The save base is `sha256(theme.json bytes)`. A save onto an existing id with no base, or a stale base, is a conflict unless `overwrite` is explicit.
- Package reading bounds the archive before inflating: ≤ 95 MiB archive, ≤ 128 entries, ≤ 8 MiB central directory, and only `manifest.json`, `theme.json`, `credits.json`, `thumbnail.png`, `assets/...` may be read.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` stay green.

## Review Focus

- Unsaved work is never discarded without a confirmation: opening another theme, creating one, importing one, or closing the window.
- Undo after a save must not corrupt the dirty flag: the flag is derived by comparing the current document to the saved baseline, never from an edit counter.
- A drag must produce one history entry, and an unchanged document must produce none.
- An export/import round-trip preserves node ids, bindings, palette and assets exactly.
- An import must never overwrite a stored theme; a colliding id is suffixed.

---

### Task 1: `yunari-library` — the theme directory

**Files:**
- Create: `crates/library/Cargo.toml`, `crates/library/src/lib.rs`, `crates/library/src/store.rs`
- Modify: `Cargo.toml` (member + workspace dependency)

**Interfaces:**
- Produces: `Library::{new(root), default_root, list, read, save, remove}`, `ThemeEntry { id, name, author, version, updated_at, node_count }`, `StoredTheme { entry, document: ThemeDocument, assets: BTreeMap<String, Vec<u8>> }`, `SaveRequest<'a> { id, document, assets, base, overwrite }`, `Saved { entry, base }`, `LibraryError { Conflict, Invalid(Vec<ValidationIssue>), TooLarge(String), Unreadable(String), Io(std::io::Error) }`.

- [x] **Step 1: Write the failing tests** in `crates/library/src/store.rs` (`#[cfg(test)]`; each test makes its own temp root with `std::env::temp_dir()` + unique suffix, and removes it):

```rust
#[test]
fn a_saved_theme_lists_and_reads_back() {
    // save {"id":"alpha"} with no base; list() has one entry named from metadata;
    // read("alpha").document == what was saved.
}

#[test]
fn a_second_first_save_is_a_conflict_until_overwrite() {
    // save alpha; save alpha again with base None -> Err(Conflict);
    // with overwrite: true -> Ok and read() returns the second document.
}

#[test]
fn a_stale_base_is_a_conflict() {
    // save once (base b1), save with base Some("nope") -> Err(Conflict).
}

#[test]
fn a_document_validation_refuses_is_never_written() {
    // artboard width 20000 -> Err(Invalid) and the theme is absent from list().
}

#[test]
fn removing_a_theme_takes_it_out_of_the_list_and_keeps_a_recoverable_copy() {
    // remove("alpha") -> Ok; list() empty; <root>/.trash contains a directory.
}
```

- [x] **Step 2: Run tests to verify they fail.** `cargo test -p yunari-library` — expected: `error[E0425]: cannot find function ...`.

- [x] **Step 3: Implement `store.rs`.** `default_root()` is `%USERPROFILE%\.yunari\themes` (`HOME` fallback). `save` validates with `yunari_model::validate::validate_document`, serializes with `serde_json::to_string_pretty`, writes `<root>/.staging-<nonce>.<id>/theme.json`. Then rename the old `<root>/<id>` to `.retired-<nonce>.<id>`, rename staging into place, remove the retired copy; retry each rename on `PermissionDenied`/`WouldBlock` with `[10, 20, 40, 80, 160]` ms sleeps. Assets are written under `assets/<declared path>`, and the sum above 90 MiB or a single asset over 32 MiB is `TooLarge`. `list()` skips names that are not `[A-Za-z0-9_-]{1,64}`, ignores dot-directories, and removes `.staging-*` leftovers while restoring `.retired-*` when the target is missing. `remove` moves the directory to `<root>/.trash/<id>-<millis>`. `base` is `sha256` of the exact bytes written.

- [x] **Step 4: Run tests to verify they pass.** `cargo test -p yunari-library`, then `cargo clippy -p yunari-library --all-targets -- -D warnings`.

- [x] **Step 5: Commit.** `git add Cargo.toml Cargo.lock crates/library && git commit -m "feat(library): a theme is a directory, written atomically"`

### Task 2: `yunari-library` — the portable package

**Files:**
- Create: `crates/library/src/package.rs`
- Test: same file

**Interfaces:**
- Produces: `write_package(&ThemeDocument, &BTreeMap<String, Vec<u8>>, Option<&[u8]>) -> Result<Vec<u8>, String>`; `read_package(&[u8]) -> Result<Package { document, assets, thumbnail }, String>`; `import_into(library, package) -> Result<Saved, LibraryError>` (choose the first free id from `<id>`, `<id>-2`, … `-100`, else `<id>-<6 hex>`).

- [x] **Step 1: Write the failing tests:**
  - `a_written_package_reads_back_with_its_assets` — document with one asset `assets/a.png`, bytes `[1,2,3]`; round-trip yields the same document JSON and assets.
  - `a_package_without_a_manifest_is_refused` — zip containing only `theme.json`.
  - `a_package_naming_an_undeclared_asset_is_refused` — manifest + theme.json + `assets/b.png` not declared.
  - `a_package_over_the_entry_cap_is_refused` — 129 entries.
  - `import_never_overwrites` — library already has `alpha`; import returns id `alpha-2`.

- [x] **Step 2: Run tests to verify they fail.** `cargo test -p yunari-library package`.

- [x] **Step 3: Implement `package.rs`.** Manifest exactly `{"format":"yunari-theme-package","kind":"theme","version":1,"theme":"theme.json"}`, optional `"thumbnail":"thumbnail.png"` and `"credits":"credits.json"`. Zip written at compression level 6. Reader refuses: file not a zip; any entry name outside the allowed set; more than 128 entries; central directory over 8 MiB; archive over 95 MiB; expanded content over 180 MiB; entry over 80 MiB; entry names containing `..` or starting with `/`; a thumbnail over 2 MiB or without the PNG signature; undeclared assets or declared assets with no file. Asset names must match `/^assets\/[A-Za-z0-9._\/-]{1,200}$/`.

- [x] **Step 4: Run tests to verify they pass**; clippy.

- [x] **Step 5: Commit.** `feat(library): export and import a theme package`

### Task 3: `yunari-editor` — document, history, baseline

**Files:**
- Create: `crates/editor/Cargo.toml`, `crates/editor/src/lib.rs`, `crates/editor/src/history.rs`, `crates/editor/src/document.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `Editor::{new(document), document, document_mut_through(cmd), add_node, remove_nodes, rename_theme, set_metadata_name, undo, redo, can_undo, can_redo, is_dirty, saved_baseline, mark_saved(id, base), begin_burst, end_burst, source}`; `Source { Unsaved, Library { id: String, base: Option<String> } }`; `History::{new, commit, undo, redo, can_undo, can_redo, len}`.
- The one mutation door: `Editor::edit(|document: &mut ThemeDocument| ...)` — snapshots before, mutates, commits after; a no-op edit commits nothing.

- [x] **Step 1: Write the failing tests** in `crates/editor/src/history.rs` and `document.rs`:
  - `an_edit_is_one_entry_and_undo_restores_the_document`
  - `an_unchanged_document_records_no_entry`
  - `redo_is_dropped_by_a_new_edit`
  - `a_burst_is_one_entry` — `begin_burst`, two edits, `end_burst`; one undo returns to the original.
  - `history_is_capped_at_one_hundred_entries`
  - `dirty_is_derived_from_the_saved_document` — save, edit, undo → not dirty; edit differently → dirty.
  - `save_as_adopts_the_new_identity` — `mark_saved("beta", "hash")` sets `Source::Library { id: "beta", base: Some("hash") }`.

- [x] **Step 2: Run tests to verify they fail.** `cargo test -p yunari-editor`.

- [x] **Step 3: Implement.** History holds `Vec<ThemeDocument>` (full snapshots — parity plus background/bindings, unlike the source's scene-only history) with index and cap 100. `Editor::edit` truncates redo, clones, compares to the current entry, pushes only when changed. `is_dirty` compares `serde_json::to_string(document)` to the baseline string captured by `mark_saved`. A burst sets a flag: the first edit inside it commits nothing and `end_burst` commits once.

- [x] **Step 4: Run tests to verify they pass**; clippy.

- [x] **Step 5: Commit.** `feat(editor): one commit point for edits, history and dirty`

### Task 4: `yunari-editor` — save, open, export, import flows

**Files:**
- Create: `crates/editor/src/persistence.rs`
- Test: same file (temp library root)

**Interfaces:**
- Produces: `Editor::{save(&mut self, &Library, overwrite: bool) -> Result<Saved, LibraryError>, save_as(&mut self, &Library, id: &str) -> Result<Saved, LibraryError>, open(&mut self, StoredTheme), export(&self, &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String>}`.
- `save` uses `Source::Unsaved` → `Conflict` unless `save_as` is used; a library source sends its base. `save_as` writes with `base: None` and then adopts the identity (before the baseline is marked, so the document is clean at birth).
- A package import goes through `library::package::import_into` and is opened as an unsaved-for-this-id document whose `Source` is the new library entry.

- [x] **Step 1: Write the failing tests:**
  - `a_first_save_as_adopts_the_identity_and_marks_clean`
  - `a_save_after_a_conflict_reports_it_and_stays_dirty`
  - `an_export_reads_back_as_the_same_document`
  - `an_imported_theme_gets_a_free_id`

- [x] **Step 2: Run to fail**, **Step 3: implement**, **Step 4: run to pass + clippy**.

- [x] **Step 5: Commit.** `feat(editor): save, save-as, export and import flows`

### Task 5: `crates/player` — the live window finds its crate

**Files:**
- Create: `crates/player/Cargo.toml`, `crates/player/src/lib.rs` (move `Stage` and the poll loop out of `crates/app/src/main.rs`)
- Modify: `crates/app/src/main.rs` (dispatch), `Cargo.toml`

**Interfaces:**
- Produces: `yunari_player::open(path: Option<&Path>, cx: &App)` — opens the live window for a theme file, or the embedded fixture.

- [ ] **Step 1:** Move the phase-2 `Stage` verbatim into `crates/player`, keeping the fixture and the 250 ms poll loop. `yunari-app <file>` keeps playing that file; the editor opens when no file is named.
- [ ] **Step 2:** `cargo test --workspace` and `cargo run -p yunari-app -- target/tmp/live.json` still paint the live theme (screenshot check).
- [ ] **Step 3: Commit.** `refactor(player): the live window is a crate`

### Task 6: Editor shell

**Files:**
- Create: `crates/editor/src/ui.rs`, `crates/editor/src/ui/menus.rs`, `crates/editor/src/ui/library_dialog.rs`, `crates/editor/src/ui/prompt.rs`
- Modify: `crates/app/src/main.rs`, `crates/editor/Cargo.toml` (gpui-kit, gpui-component, render-gpui, render-raster)

**Interfaces:**
- Produces: `EditorWindow` GPUI entity holding `Editor`, `Library`, `Fonts`, `Images`, selection `Option<String>`; menu actions New, Open…, Save (Ctrl+S), Save As…, Export…, Import…, Undo (Ctrl+Z), Redo (Ctrl+Shift+Z), Add Rectangle, Add Text, Delete.
- Canvas paints the document through `yunari_render_gpui::paint` with readings polled exactly as the player does.

- [ ] **Step 1:** Wire the window: menu bar, canvas, status line (theme name · dirty dot), and the keyboard shortcuts. Save writes to the library; Save As and Open use a list dialog of `library.list()`; Import reads a file path typed into the prompt and runs `import_into`; Export writes a `.yunari-theme` file beside the library root with a name derived from the theme name.
- [ ] **Step 2:** Unsaved-changes guard: New, Open and Import show a confirm dialog (Discard / Cancel) when `is_dirty()`; closing the window asks the same.
- [ ] **Step 3:** Run the app by hand: New → Add Rectangle → Save As `first` → quit → relaunch → Open `first` shows the rectangle; Export then Import creates `first-2`.
- [ ] **Step 4:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
- [ ] **Step 5: Commit.** `feat(editor): a shell that authors, saves and reopens a theme`

### Task 7: Phase 3A exit check

- [ ] **Step 1:** Run the manual flow above against the real library root and capture: the editor window before and after save, the library listing, and the exported package's file list (`unzip -l`).
- [ ] **Step 2:** Record the exit in the spec's Phase status: what was authored, saved, reloaded, exported, re-imported, plus the undo/redo verification (Ctrl+Z after a move returns the rectangle; Ctrl+Shift+Z restores it) and the deviation (full-document history versus the source's scene-only history).
- [ ] **Step 3: Commit.** `docs: record phase 3A's exit`
