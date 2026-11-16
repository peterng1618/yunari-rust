# Phase 7: Asset import, measurements and the release build

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The editor can take a picture, SVG or video from this PC and show it in a theme; the port's cost is measured against the Node baseline; and the app has a release build someone can install.

**Architecture:** Import is one editor command with one validator (`crates/editor/src/assets.rs`): a typed path is read once, sniffed by content rather than extension, capped, sanitised if it is SVG, and stored as both a document `AssetReference` and in-memory bytes. Rendering already reaches assets through `yunari_raster::Images`, so the store gains an in-memory `put` and the editor seeds it from the bytes it holds. Measurements are taken from the release binary; the installer is the release binary plus the files a user needs.

**Tech Stack:** Rust 2024, `image` (already a raster dependency), `sha2`, GPUI typed-path prompts (the idiom the package import already uses).

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- A file is untrusted data: cap its size, sniff its kind, never trust its name or extension, and never write outside the theme's `assets/` folder.
- Import refuses with one owner-actionable sentence; a partial import leaves the document unchanged.
- SVG is sanitised, not trusted: no script, no event handlers, no `javascript:` URLs, no `<foreignObject>`, no external references.
- Measurements are reported with the method that took them; nothing is estimated and presented as measured.
- Workspace gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.

## Review Focus

- A 300 MB file, a file that is not a picture, and a `.png` that is really a ZIP: each is refused with its own sentence and the document does not change.
- An SVG carrying `<script>` or an external `href`: refused before the bytes are stored.
- Two files with the same name: both land, neither overwrites the other, and both draw.
- A theme with an imported picture, saved and reopened: the picture still draws after the restart.

---

### Task 1: Import a file as a theme asset

**Files:**
- Create: `crates/editor/src/assets.rs`
- Modify: `crates/editor/src/lib.rs`, `crates/editor/src/document.rs`, `crates/editor/src/ui.rs`, `crates/editor/src/ui/menus.rs`
- Modify: `crates/raster/src/images.rs`
- Test: same files

**Interfaces:**
- Produces: `assets::{MAX_IMAGE_BYTES, MAX_SVG_BYTES, sniff(name, &[u8]) -> Result<AssetKind, String>, sanitize_svg(&str) -> Result<(), String>}`; `Editor::import_asset(&mut self, name: &str, bytes: Vec<u8>) -> Result<String, String>` returning the new asset id; `Images::put(&mut self, path: &str, bytes: &[u8])`; `Command::{ImportAsset, AddImage}`.

- [x] Test: sniffing accepts PNG/JPEG/WebP/GIF by magic bytes and SVG by its own grammar; a ZIP named `.png` and a plain text file are refused with `assets::NOT_A_FILE_MESSAGE`.
- [x] Test: `sanitize_svg` refuses `<script`, `onload=`, `javascript:` and `<foreignObject`, and accepts a plain `<svg>`.
- [x] Test: `import_asset` mints a free asset id, names the path `assets/<file>`, records the SHA-256, and a second import of the same file name gets a distinct path.
- [x] Implement the module, the editor method, `Images::put`, and the two commands (asset prompt and image placement).
- [x] Test: placing an image node with no image asset is refused with `assets::NO_PICTURE_MESSAGE`.
- [x] Run the workspace gates.
- [x] Commit.

### Task 2: Measure the port against the Node baseline

**Files:**
- Modify: `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

- [ ] Build `cargo build --release`.
- [ ] Measure idle private working set and idle CPU of `yunari-app` for five minutes with the editor open on the blank theme, and again with a player open.
- [ ] Measure editor frame timings with `cargo run --release` and the app's own status line where it reports, or a tracing span if the status line does not.
- [ ] Measure panel cadence on hardware if a panel is attached; otherwise record "not measured — no panel attached".
- [ ] Record every figure with its method in a "Performance" section of the spec.
- [ ] Commit.

### Task 3: The release artifact

**Files:**
- Create: `docs/install.md`
- Modify: `README.md`, `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

- [ ] `cargo build --release` produces `target/release/yunari-app.exe`; confirm it starts, opens the editor and serves `/api/health`.
- [ ] Write `docs/install.md`: what the binary needs (Windows 10/11, WebView-free, no install step), where state lives (`%USERPROFILE%\.yunari`), the CA-root step for a phone, and how to uninstall (delete the folder and the startup entry).
- [ ] Copy the binary, `README.md`, `LICENSE` and the notices into a `yunari-<version>-windows-x64/` folder and zip it; record the artifact name and size in the spec.
- [ ] Record in the spec that an MSI/MSIX-winget manifest is not built: signing and a published channel are open follow-ups, and an unsigned installer is worse than a zip.
- [ ] Commit.
