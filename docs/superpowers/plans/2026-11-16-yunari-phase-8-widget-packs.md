# Phase 8: Widget packs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An installed `yunari-widget-package` gives the editor insertable card units, with their own subtree, bindings and asset bytes, and the host owns the installed packs.

**Architecture:** A new `yunari-packs` crate owns the package boundary — the ZIP gate, the strict manifest, and the derivation of one unit per top-level object — and lifts each unit's scene through `yunari_model::source`, so there is one Fabric reader. The host lists packs from its settings folder and installs a package by path; the editor asks the host for that listing (or reads the folder when it runs in the same process) and inserts a unit by deep-copying its nodes with fresh ids.

**Tech Stack:** Rust 2024, `zip` 9 (read + write for tests), `sha2`, `serde_json`.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- A package is untrusted: cap the archive at 95 MiB, each entry at 80 MiB, the expansion at 180 MiB and the entry count at 128; admit only `manifest.json`, `widget.json` and `assets/` paths; refuse a duplicate or climbing name.
- Every declared asset must carry a SHA-256 the reader verifies before the unit is offered.
- Refusal sentences match the source: `Unsupported widget package manifest.`, `A widget package needs manifest.json and widget.json.`, `A widget document must not carry an artboard.`, `Package assets must exactly match the widget declaration.`, `Widget unit "x" names no top-level object "y".`
- Insertion never mutates the pack: fresh node ids, fresh binding ids, remapped runs, and asset bytes held beside the document.
- Workspace gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.

## Review Focus

- A ZIP whose entries climb (`../x`, absolute paths) or repeat a name: refused, nothing written.
- A package declaring an asset with no digest, or bytes that do not match the one declared: refused.
- A unit whose root object the model cannot draw: the other units still list; that one is refused by name.
- Inserting the same unit twice: no id collision, both draw, one undo.

---

### Task 1: The package reader

**Files:**
- Create: `crates/packs/Cargo.toml`, `crates/packs/src/lib.rs`, `crates/packs/src/package.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Produces: `packs::{MAX_ARCHIVE_BYTES, MAX_ENTRY_BYTES, MAX_EXPANDED_BYTES, MAX_ENTRIES}`; `package::{Pack, Unit, read(bytes: &[u8]) -> Result<Pack, String>}`; `Pack { name: Option<String>, version: Option<String>, units: Vec<Unit> }`; `Unit { id: String, label: String, document: ThemeDocument, assets: BTreeMap<String, Vec<u8>> }`.

- [x] Test: a package written by the test helper lists one unit with its nodes, its binding and its asset bytes.
- [x] Test: a manifest naming an unknown key, a missing `widget.json`, a widget carrying `artboard`, and an asset whose bytes miss its digest are each refused with the source's sentence.
- [x] Test: provenance-derived units get unique ids (`card`, `card-2`) and a declared unit naming no object is refused by name.
- [x] Test: a climbing entry name and a duplicate entry name are refused before any unit is built.
- [x] Implement the crate.
- [x] Run the workspace gates.
- [x] Commit.

### Task 2: The host owns installed packs

**Files:**
- Create: `crates/host/src/packs_routes.rs`
- Modify: `crates/host/src/{lib.rs,http.rs,store.rs}`, `crates/host/assets/` (starter pack if one ships)

**Interfaces:**
- Produces: `GET /api/widget-packs` → `{packs:[{id,name,version,units:[{id,label}]}]}`; `POST /api/widget-packs` `{path}` copies a package into `<settings>/widget-packs/` after the reader accepts it; both loopback-only.

- [x] Test: a package placed in the settings folder is listed with its units; a phone gets 403; a package the reader refuses is not installed and the response carries its sentence.
- [x] Implement the routes and the folder reading (one directory scan per request; a file the reader refuses is skipped, not fatal).
- [x] Run the workspace gates.
- [x] Commit.

### Task 3: The editor inserts a unit

**Files:**
- Create: `crates/editor/src/widgets.rs`, `crates/editor/src/ui/widget_dialog.rs`
- Modify: `crates/editor/src/{lib.rs,document.rs,ui.rs,ui/menus.rs}`, `crates/editor/Cargo.toml`

**Interfaces:**
- Produces: `widgets::{insert(editor: &mut Editor, unit: &packs::Unit, at: (f64, f64)) -> Result<String, String>}`; `Command::Widgets`; a dialog listing `packs::Pack` units.

- [x] Test: inserting a unit twice yields distinct node ids and binding owners, both roots at the requested offset, and one undo removes the second.
- [x] Test: an asset the unit declares lands in `document.assets` and in `editor.assets()` under its package path, without overwriting a different path's bytes.
- [x] Implement insertion, the dialog and the command (`Ctrl+Shift+W`).
- [x] Run the workspace gates.
- [x] Commit.

### Task 4: One committed pack so a fresh install has something to insert

**Files:**
- Create: `crates/packs/assets/starter-card.yunari-widget-package`
- Modify: `crates/packs/src/lib.rs`, `crates/host/src/server.rs`, `crates/host/src/http.rs`

**Interfaces:**
- Produces: `packs::STARTER: &[u8]`; the host installs it on start when `<settings>/widget-packs` is empty.

- [x] Test: the committed package reads as one pack named "Starter cards" with a "CPU card" unit, one root of four children, and a value run bound to `reading`.
- [x] Test: a host on a fresh settings folder lists the committed pack without an install; a phone still gets 403.
- [x] Implement the install-on-empty-folder and the committed package.
- [x] Run the workspace gates.
- [x] Commit.

This covers product item 10: the pack ships committed, and `New` (and a fresh window) now opens a starter document with a title and a clock reading live, on the blank theme's tokens. The theme is built in Rust beside `blank()` rather than read from a committed JSON file; the commands, undo and `Editor` paths are the same either way.
