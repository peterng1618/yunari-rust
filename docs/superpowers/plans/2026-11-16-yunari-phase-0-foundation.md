# Yunari Native Port — Phase 0 Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the cargo workspace, the pure document model with validation, the design-token owner, the golden-image harness, and a gpui-kit application shell that opens a window.

**Architecture:** One workspace, one binary crate. `yunari-model` holds the versioned theme document, parsing and refusal rules and imports no UI, OS or network crate. `yunari-theme` owns the editor's spacing/radius/type scale as data. `yunari-golden` owns pixel comparison and update-on-demand golden files. `yunari-app` is the gpui-kit binary. Later phases add the render, host, editor, player and support crates without changing these interfaces.

**Tech Stack:** Rust stable (MSVC, 1.92+), edition 2024, gpui-kit 0.9.1, serde/serde_json, thiserror, proptest, image.

**Spec:** `docs/superpowers/specs/2026-11-16-yunari-rust-port-design.md`

## Global Constraints

- Windows-first; build with the MSVC toolchain. Rust 1.92 or later.
- Every crate starts with `#![forbid(unsafe_code)]`.
- `yunari-model` depends on no UI, OS, filesystem or network crate: `serde`, `serde_json`, `serde_ignored`, `thiserror` only (plus `proptest` as a dev-dependency).
- Validation refuses invalid input with a JSON pointer-style path; it never clamps, rounds or coerces a value into range.
- Numeric rules use finite checks: NaN and infinities are refused.
- Document IDs match `^[A-Za-z0-9_-]{1,64}$`.
- Artboard maximum is 16384 px per dimension; a circle artboard requires equal width and height.
- Node depth is 32 (roots are depth 1); node count is 5000.
- Metadata name is at most 200 UTF-8 bytes; at most 5 tags; each tag matches `^[a-z0-9-]{1,24}$`.
- Binding precision is an integer from 0 to 6.
- Asset paths match `^assets/[A-Za-z0-9._/-]{1,200}$` and contain no `.` or `..` segment.
- Schema major is 1; a major other than 1 is refused. No migration exists pre-release.
- The theme document is authored state only: no selection, viewport, timeline or derived telemetry.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` must pass before every commit.
- No extension, store, sensor, panel or render code in this phase.

## Review Focus

- **Unknown or duplicate keys in saved JSON.** A theme edited by hand or by a future version must fail loudly, not silently drop fields. Every serialized struct refuses unknown keys; the envelope test pins it.
- **Non-finite numbers.** JSON cannot carry NaN, but a Rust caller can construct one. Validation must refuse NaN/infinite widths, offsets, scales and angles rather than letting them reach layout.
- **Pathological nesting.** A 10,000-level node chain must produce a depth refusal without overflowing the validator's stack; validation walks iteratively.
- **Huge node counts.** 5001 nodes are refused and 5000 are accepted; the count is checked before per-node work so a hostile document cannot cost more than a bounded walk.
- **Circle artboards.** A circle with unequal sides is refused instead of being rounded to a square or silently drawn as a rectangle.

---

### Task 0: Install the Windows Rust toolchain and native prerequisites

**Files:** none (machine setup)

**Interfaces:**
- Produces: `cargo`, `rustc` (MSVC target), `cmake`, and the MSVC C++ build tools available to every later task.

- [ ] **Step 1: Check what is already present**

Run each command in a new PowerShell window:

```powershell
cargo --version
rustc --version
rustup show active-toolchain
cmake --version
& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
```

Expected: `cargo`/`rustc` at 1.92 or later, an `msvc` active toolchain, a cmake version, and a VS installation path. Ask the user before installing anything that is missing.

- [ ] **Step 2: Install missing tools with winget**

Only run the lines whose check failed. The Visual Studio installer is large and needs elevation, so confirm with the user first.

```powershell
winget install --id Rustlang.Rustup -e --scope user --accept-package-agreements --accept-source-agreements
winget install --id Kitware.CMake -e --accept-package-agreements --accept-source-agreements
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-package-agreements --accept-source-agreements --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

- [ ] **Step 3: Select the MSVC toolchain**

Run in a new terminal so the updated PATH is visible:

```powershell
rustup default stable-msvc
rustup show active-toolchain
```

Expected: a line containing `x86_64-pc-windows-msvc`.

- [ ] **Step 4: Verify the gates**

Run:

```powershell
cargo --version
rustc --version
cmake --version
```

Expected: all print versions and `rustc --version` reports 1.92 or later.

No commit: this task changes the machine, not the repository.

---

### Task 1: Create the workspace, model crate and app placeholder

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `crates/model/Cargo.toml`
- Create: `crates/model/src/lib.rs`
- Create: `crates/app/Cargo.toml`
- Create: `crates/app/src/main.rs`

**Interfaces:**
- Produces: workspace members `yunari-model` and `yunari-app`; workspace dependency versions reused by later tasks.
- Produces: `yunari_model` crate root for Task 2's modules.

- [ ] **Step 1: Write the workspace manifest**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/model", "crates/app"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
license = "MIT"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
image = "0.25"
proptest = "1"
yunari-model = { path = "crates/model" }
yunari-theme = { path = "crates/theme" }
yunari-golden = { path = "crates/golden" }
```

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

`.gitignore`:

```text
/target
```

- [ ] **Step 2: Create the model crate placeholder**

`crates/model/Cargo.toml`:

```toml
[package]
name = "yunari-model"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
serde_ignored.workspace = true
thiserror.workspace = true

[dev-dependencies]
proptest.workspace = true
```

`crates/model/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! Yunari theme document model and validation.

pub mod document;
pub mod validate;
```

Create empty `crates/model/src/document.rs` and `crates/model/src/validate.rs` so the crate compiles; Task 2 fills them.

- [ ] **Step 3: Create the app placeholder**

`crates/app/Cargo.toml`:

```toml
[package]
name = "yunari-app"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
yunari-model.workspace = true
```

`crates/app/src/main.rs`:

```rust
#![forbid(unsafe_code)]

fn main() {
    println!("yunari {}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 4: Verify the workspace builds**

Run:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p yunari-app
```

Expected: checks pass; the binary prints `yunari 0.1.0`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore crates/model crates/app
git commit -m "chore: create cargo workspace skeleton"
```

---

### Task 2: Define the theme document types and parsing entry points

**Files:**
- Modify: `crates/model/src/document.rs`
- Modify: `crates/model/src/validate.rs`
- Modify: `crates/model/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/model/src/document.rs`

**Interfaces:**
- Consumes: workspace dependencies from Task 1.
- Produces: `ThemeDocument`, its full type tree, `parse_document(&str) -> Result<ThemeDocument, DocumentError>` and `write_document(&ThemeDocument) -> Result<String, DocumentError>`.
- Produces: `validate_document(&ThemeDocument) -> Result<(), Vec<ValidationIssue>>` as a stub that Task 3 extends.
- Produces: `ValidationIssue { path: String, message: String }` and `DocumentError::{Json, Invalid}`.

- [ ] **Step 1: Write the failing parsing tests**

Append to `crates/model/src/document.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "schemaVersion": "1.0.0",
        "id": "theme_1",
        "artboard": { "width": 800, "height": 480 },
        "nodes": []
    }"#;

    #[test]
    fn parses_minimal_document() {
        let doc = parse_document(MINIMAL).expect("minimal document parses");
        assert_eq!(doc.artboard.width, 800.0);
        assert_eq!(doc.artboard.height, 480.0);
        assert!(doc.nodes.is_empty());
        assert!(doc.metadata.name.is_none());
    }

    #[test]
    fn round_trips_through_json() {
        let doc = parse_document(MINIMAL).expect("parses");
        let json = write_document(&doc).expect("writes");
        assert_eq!(parse_document(&json).expect("reparses"), doc);
    }

    #[test]
    fn refuses_unknown_top_level_field() {
        let json = MINIMAL.replace("\"nodes\": []", "\"nodes\": [], \"extra\": 1");
        let error = parse_document(&json).expect_err("unknown field is refused");
        assert!(matches!(error, DocumentError::Json(_)));
    }

    #[test]
    fn accepts_minor_and_patch_within_supported_major() {
        let json = MINIMAL.replace("\"1.0.0\"", "\"1.4.9\"");
        assert!(parse_document(&json).is_ok());
    }

    #[test]
    fn refuses_newer_major() {
        let json = MINIMAL.replace("\"1.0.0\"", "\"2.0.0\"");
        let error = parse_document(&json).expect_err("major 2 is refused");
        let DocumentError::Invalid(issues) = error else {
            panic!("expected validation issues");
        };
        assert_eq!(issues[0].path, "schemaVersion");
    }

    #[test]
    fn refuses_unparseable_schema_version() {
        let json = MINIMAL.replace("\"1.0.0\"", "\"one\"");
        let error = parse_document(&json).expect_err("non-semver is refused");
        let DocumentError::Invalid(issues) = error else {
            panic!("expected validation issues");
        };
        assert_eq!(issues[0].path, "schemaVersion");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-model`

Expected: compilation fails because `parse_document`, `write_document`, `ThemeDocument` and `DocumentError` do not exist.

- [ ] **Step 3: Implement the document types**

In `crates/model/src/document.rs`, before the tests, define the types below. Every struct uses `#[serde(rename_all = "camelCase", deny_unknown_fields)]`; every struct with optional fields derives `Default` and marks each field `#[serde(default)]`; every field not marked optional is required.

Charts and nodes use internally tagged enums, where serde ignores unknown fields in struct variants. `parse_document` therefore wraps `serde_json::Deserializer` in `serde_ignored` and refuses every ignored path, so a typo in a node is an error rather than a silent drop.

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::validate::{validate_document, ValidationIssue};

pub const SCHEMA_VERSION: &str = "1.0.0";
pub const SUPPORTED_SCHEMA_MAJOR: u64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeDocument {
    pub schema_version: String,
    pub id: String,
    #[serde(default)]
    pub metadata: Metadata,
    pub artboard: Artboard,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub palette: BTreeMap<String, Paint>,
    #[serde(default)]
    pub type_tokens: BTreeMap<String, TypeToken>,
    #[serde(default)]
    pub assets: Vec<AssetReference>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<Binding>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub remix_of: Option<Remix>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Remix {
    pub catalogue_id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artboard {
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub content_fit: ContentFit,
    #[serde(default)]
    pub shape: ArtboardShape,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentFit {
    #[default]
    Contain,
    Cover,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtboardShape {
    #[default]
    Rect,
    Circle,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Background {
    #[serde(default)]
    pub mode: BackgroundMode,
    #[serde(default)]
    pub color: Option<StyleValue>,
    #[serde(default)]
    pub media: Option<BackgroundMedia>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundMode {
    #[default]
    Simple,
    Advanced,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundMedia {
    pub asset_id: String,
    #[serde(default)]
    pub color: Option<StyleValue>,
    #[serde(default)]
    pub bounds: BackgroundBounds,
    #[serde(default)]
    pub offset_x: f64,
    #[serde(default)]
    pub offset_y: f64,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    #[serde(default)]
    pub fallback_frame: Option<u32>,
    #[serde(default)]
    pub composition: Option<BackgroundComposition>,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundBounds {
    #[default]
    Theme,
    Display,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundComposition {
    #[serde(default)]
    pub anchor: BackgroundAnchor,
    #[serde(default)]
    pub extent: Option<f64>,
    #[serde(default)]
    pub fade: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundAnchor {
    #[default]
    Top,
    Right,
    Bottom,
    Left,
    Center,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StyleValue {
    Ref(StyleRef),
    Literal(StyleLiteral),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleRef {
    pub r#ref: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleLiteral {
    pub value: serde_json::Value,
}

pub type StyleMap = BTreeMap<String, StyleValue>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Paint {
    Solid { color: String },
    Gradient { angle: f64, stops: Vec<GradientStop> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradientStop {
    pub offset: f64,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeToken {
    pub family: String,
    #[serde(default)]
    pub weight: Option<String>,
    #[serde(default)]
    pub face: Option<Face>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Face {
    pub asset_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetReference {
    pub id: String,
    pub kind: AssetKind,
    pub path: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub license: Option<AssetLicense>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    Image,
    Svg,
    Gif,
    Video,
    Font,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssetLicense {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub attribution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub kind: NodeKind,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default = "default_true")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub style: StyleMap,
    #[serde(default)]
    pub children: Vec<Node>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transform {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default = "one")]
    pub scale_x: f64,
    #[serde(default = "one")]
    pub scale_y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NodeKind {
    Group,
    Rect {
        #[serde(default)]
        corner_radius: f64,
    },
    Ellipse,
    Line,
    Path {
        data: String,
        #[serde(default)]
        stroke_width: f64,
        #[serde(default)]
        closed: bool,
    },
    Text {
        content: TextContent,
    },
    Chart {
        content: ChartContent,
    },
    Image {
        content: ImageContent,
    },
    Video {
        content: VideoContent,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextContent {
    pub runs: Vec<TextRun>,
    #[serde(default, rename = "box")]
    pub box_: Option<TextBox>,
    #[serde(default)]
    pub overflow: TextOverflow,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub vertical_align: VerticalAlign,
    #[serde(default)]
    pub vertical: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextBox {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextOverflow {
    #[default]
    Visible,
    Clip,
    Ellipsis,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerticalAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextRun {
    Literal {
        text: String,
        type_token: String,
        #[serde(default)]
        style: StyleMap,
    },
    Value {
        binding_id: String,
        #[serde(default)]
        precision: Option<u8>,
        #[serde(default)]
        unit_display: Option<UnitDisplay>,
        type_token: String,
        #[serde(default)]
        style: StyleMap,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub id: String,
    pub semantic_key: String,
    #[serde(default)]
    pub precision: Option<u8>,
    #[serde(default)]
    pub unit_display: Option<UnitDisplay>,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub offset: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub auto_scale: Option<bool>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnitDisplay {
    None,
    Short,
    Long,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartContent {
    pub family: ChartFamily,
    pub settings: ChartSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartFamily {
    Gauge,
    Line,
    Bar,
    Pie,
    Radar,
    PolarBar,
}

// ponytail: settings stay an untyped map until the render plan types them in
// phase 1. The document contract and validation rules live here; typed
// accessors arrive with the chart geometry that needs them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChartSettings(pub serde_json::Map<String, serde_json::Value>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageContent {
    pub asset_id: String,
    #[serde(default)]
    pub fit: ImageFit,
    #[serde(default)]
    pub monochrome: Option<StyleValue>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFit {
    #[default]
    Contain,
    Cover,
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoContent {
    pub asset_id: String,
    #[serde(default, rename = "loop")]
    pub loop_: bool,
    #[serde(default)]
    pub muted: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("invalid theme JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid theme document: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<ValidationIssue>),
}

pub fn parse_document(json: &str) -> Result<ThemeDocument, DocumentError> {
    let document: ThemeDocument = serde_json::from_str(json)?;
    validate_document(&document).map_err(DocumentError::Invalid)?;
    Ok(document)
}

pub fn write_document(document: &ThemeDocument) -> Result<String, DocumentError> {
    validate_document(document).map_err(DocumentError::Invalid)?;
    Ok(serde_json::to_string_pretty(document)?)
}
```

In `crates/model/src/validate.rs`, implement the stub:

```rust
use std::fmt;

use crate::document::ThemeDocument;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

pub fn validate_document(document: &ThemeDocument) -> Result<(), Vec<ValidationIssue>> {
    let _ = document;
    Ok(())
}
```

`ValidationIssue` must remain constructible from tests in other modules; keep fields `pub`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-model`

Expected: all six tests pass.

- [ ] **Step 5: Run the gates**

Run:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/model
git commit -m "feat(model): define theme document types and JSON entry points"
```

---

### Task 3: Validate schema major, artboard, IDs and metadata

**Files:**
- Modify: `crates/model/src/validate.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/model/src/validate.rs`

**Interfaces:**
- Consumes: `ThemeDocument`, `Artboard`, `ArtboardShape`, `Metadata` from Task 2.
- Produces: `is_stable_id(value: &str) -> bool` (public helper used by later phases) and the document-level checks other tasks extend.

- [ ] **Step 1: Write the failing tests**

Append to `crates/model/src/validate.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;

    fn minimal() -> ThemeDocument {
        parse_document(
            r#"{"schemaVersion":"1.0.0","id":"theme_1",
                "artboard":{"width":800,"height":480},"nodes":[]}"#,
        )
        .expect("minimal document")
    }

    fn paths(result: Result<(), Vec<ValidationIssue>>) -> Vec<String> {
        let mut issues = result.expect_err("expected validation issues");
        issues.sort();
        issues.into_iter().map(|issue| issue.path).collect()
    }

    #[test]
    fn accepts_minimal_document() {
        assert!(validate_document(&minimal()).is_ok());
    }

    #[test]
    fn refuses_circle_artboard_that_is_not_square() {
        let mut document = minimal();
        document.artboard.shape = ArtboardShape::Circle;
        assert_eq!(paths(validate_document(&document)), vec!["artboard/shape"]);
    }

    #[test]
    fn accepts_square_circle_artboard() {
        let mut document = minimal();
        document.artboard = Artboard { width: 480.0, height: 480.0, ..document.artboard };
        document.artboard.shape = ArtboardShape::Circle;
        assert!(validate_document(&document).is_ok());
    }

    #[test]
    fn refuses_artboard_above_16384() {
        let mut document = minimal();
        document.artboard.width = 16385.0;
        assert_eq!(paths(validate_document(&document)), vec!["artboard/width"]);
    }

    #[test]
    fn refuses_zero_artboard_dimension() {
        let mut document = minimal();
        document.artboard.height = 0.0;
        assert_eq!(paths(validate_document(&document)), vec!["artboard/height"]);
    }

    #[test]
    fn refuses_non_finite_artboard() {
        let mut document = minimal();
        document.artboard.width = f64::NAN;
        assert_eq!(paths(validate_document(&document)), vec!["artboard/width"]);
    }

    #[test]
    fn refuses_invalid_document_id() {
        let mut document = minimal();
        document.id = "bad id".into();
        assert_eq!(paths(validate_document(&document)), vec!["id"]);
    }

    #[test]
    fn refuses_metadata_name_over_200_utf8_bytes() {
        let mut document = minimal();
        document.metadata.name = Some("é".repeat(101));
        assert_eq!(paths(validate_document(&document)), vec!["metadata/name"]);
    }

    #[test]
    fn refuses_more_than_five_tags() {
        let mut document = minimal();
        document.metadata.tags = (0..6).map(|index| format!("tag{index}")).collect();
        assert_eq!(paths(validate_document(&document)), vec!["metadata/tags"]);
    }

    #[test]
    fn refuses_bad_tag_and_duplicate_tag() {
        let mut document = minimal();
        document.metadata.tags = vec!["Ok_Tag".into()];
        assert_eq!(paths(validate_document(&document)), vec!["metadata/tags/0"]);

        document.metadata.tags = vec!["mono".into(), "mono".into()];
        assert_eq!(paths(validate_document(&document)), vec!["metadata/tags/1"]);
    }

    #[test]
    fn refuses_invalid_node_id() {
        let mut document = minimal();
        document.nodes.push(Node {
            id: "bad id".into(),
            name: None,
            kind: NodeKind::Group,
            transform: Transform::default(),
            visible: true,
            locked: false,
            style: StyleMap::new(),
            children: Vec::new(),
        });
        assert_eq!(paths(validate_document(&document)), vec!["nodes/0/id"]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-model validate`

Expected: the new tests fail because the stub accepts everything.

- [ ] **Step 3: Implement the document-level checks**

In `crates/model/src/validate.rs`, replace the stub body with a collector and checks. Keep this shape:

```rust
const MAX_ARTBOARD_DIMENSION: f64 = 16384.0;
const MAX_METADATA_NAME_BYTES: usize = 200;
const MAX_TAGS: usize = 5;

pub fn is_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub fn validate_document(document: &ThemeDocument) -> Result<(), Vec<ValidationIssue>> {
    let mut issues = Vec::new();

    validate_schema_version(&document.schema_version, &mut issues);
    if !is_stable_id(&document.id) {
        issues.push(issue("id", "An id must match [A-Za-z0-9_-]{1,64}."));
    }
    validate_metadata(&document.metadata, &mut issues);
    validate_artboard(&document.artboard, &mut issues);

    if issues.is_empty() {
        Ok(())
    } else {
        issues.sort();
        Err(issues)
    }
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> ValidationIssue {
    ValidationIssue { path: path.into(), message: message.into() }
}
```

Implement `validate_schema_version`: split on `.`, require three numeric parts; parse the first as `u64`; push `schemaVersion` issue when the shape is wrong or the major differs from `SUPPORTED_SCHEMA_MAJOR`. Message for a wrong major: `"Only schema major 1 is supported."`

Implement `validate_metadata`: name byte length; tags length at most 5; each tag matches `^[a-z0-9-]{1,24}$` with issue path `metadata/tags/{index}`; a duplicate tag is refused at its own index with message `"A tag is listed twice."`

Implement `validate_artboard`: each dimension must be finite, greater than 0 and at most 16384, with paths `artboard/width` and `artboard/height`; when `shape == Circle` and dimensions differ, push `artboard/shape` with message `"A circle artboard must be square."`

Import `SUPPORTED_SCHEMA_MAJOR` from `crate::document`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-model`

Expected: all tests pass, including Task 2's.

- [ ] **Step 5: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add crates/model
git commit -m "feat(model): validate schema version, artboard, ids and metadata"
```

---

### Task 4: Validate the node tree and bindings

**Files:**
- Modify: `crates/model/src/validate.rs`
- Test: inline tests in `crates/model/src/validate.rs`

**Interfaces:**
- Consumes: `Node`, `NodeKind`, `Binding`, `ChartFamily`, `TextRun` from Task 2; `issue` collector from Task 3.
- Produces: `MAX_NODE_DEPTH`, `MAX_NODE_COUNT` constants and iterative node validation that later phases reuse for scene limits.

- [ ] **Step 1: Write the failing tests**

Append to the test module:

```rust
    fn group(id: &str, children: Vec<Node>) -> Node {
        Node {
            id: id.into(),
            name: None,
            kind: NodeKind::Group,
            transform: Transform::default(),
            visible: true,
            locked: false,
            style: StyleMap::new(),
            children,
        }
    }

    fn with_nodes(nodes: Vec<Node>) -> ThemeDocument {
        let mut document = minimal();
        document.nodes = nodes;
        document
    }

    #[test]
    fn accepts_5000_nodes() {
        let nodes = (0..5000).map(|index| group(&format!("n{index}"), vec![])).collect();
        assert!(validate_document(&with_nodes(nodes)).is_ok());
    }

    #[test]
    fn refuses_5001_nodes() {
        let nodes = (0..5001).map(|index| group(&format!("n{index}"), vec![])).collect();
        assert_eq!(paths(validate_document(&with_nodes(nodes))), vec!["nodes"]);
    }

    #[test]
    fn accepts_depth_32_and_refuses_depth_33() {
        let mut node = group("leaf", vec![]);
        for depth in (1..32).rev() {
            node = group(&format!("n{depth}"), vec![node]);
        }
        assert!(validate_document(&with_nodes(vec![node])).is_ok());

        let mut node = group("leaf", vec![]);
        for depth in (1..33).rev() {
            node = group(&format!("n{depth}"), vec![node]);
        }
        let issues = validate_document(&with_nodes(vec![node])).expect_err("depth 33");
        assert!(issues.iter().any(|issue| issue.message.contains("Node depth")));
    }

    #[test]
    fn validates_10000_deep_chain_without_overflow() {
        let mut node = group("leaf", vec![]);
        for depth in (0..10_000).rev() {
            node = group(&format!("n{depth}"), vec![node]);
        }
        let result = validate_document(&with_nodes(vec![node]));
        assert!(result.is_err());
    }

    #[test]
    fn refuses_duplicate_node_ids() {
        let document = with_nodes(vec![group("same", vec![]), group("same", vec![])]);
        assert_eq!(paths(validate_document(&document)), vec!["nodes/1/id"]);
    }

    #[test]
    fn refuses_binding_for_unknown_node() {
        let mut document = minimal();
        document.bindings.insert("ghost".into(), vec![]);
        assert_eq!(paths(validate_document(&document)), vec!["bindings/ghost"]);
    }

    #[test]
    fn refuses_gauge_with_two_bindings() {
        let mut document = with_nodes(vec![Node {
            kind: NodeKind::Chart { content: ChartContent {
                family: ChartFamily::Gauge,
                settings: ChartSettings::default(),
            }},
            ..group("chart", vec![])
        }]);
        document.bindings.insert("chart".into(), vec![
            Binding { id: "a".into(), semantic_key: "cpu.load".into(), precision: None,
                unit_display: None, scale: None, offset: None, unit: None, auto_scale: None,
                format: None, time_zone: None },
            Binding { id: "b".into(), semantic_key: "ram.used".into(), precision: None,
                unit_display: None, scale: None, offset: None, unit: None, auto_scale: None,
                format: None, time_zone: None },
        ]);
        assert_eq!(paths(validate_document(&document)), vec!["bindings/chart/0"]);
    }

    #[test]
    fn refuses_radar_with_thirteen_bindings() {
        let mut document = with_nodes(vec![Node {
            kind: NodeKind::Chart { content: ChartContent {
                family: ChartFamily::Radar,
                settings: ChartSettings::default(),
            }},
            ..group("chart", vec![])
        }]);
        document.bindings.insert("chart".into(), (0..13).map(|index| Binding {
            id: format!("b{index}"),
            semantic_key: "gpu.load".into(),
            precision: None, unit_display: None, scale: None, offset: None, unit: None,
            auto_scale: None, format: None, time_zone: None,
        }).collect());
        assert_eq!(paths(validate_document(&document)), vec!["bindings/chart/0"]);
    }

    #[test]
    fn refuses_precision_outside_zero_to_six() {
        let mut document = minimal();
        document.bindings.insert("cpu".into(), vec![Binding {
            id: "load".into(), semantic_key: "cpu.load".into(), precision: Some(7),
            unit_display: None, scale: None, offset: None, unit: None, auto_scale: None,
            format: None, time_zone: None,
        }]);
        document.nodes.push(group("cpu", vec![]));
        assert_eq!(paths(validate_document(&document)), vec!["bindings/cpu/0/precision"]);
    }

    #[test]
    fn refuses_text_value_run_without_binding() {
        let mut document = with_nodes(vec![Node {
            id: "label".into(),
            kind: NodeKind::Text { content: TextContent {
                runs: vec![TextRun::Value {
                    binding_id: "missing".into(),
                    precision: None,
                    unit_display: None,
                    type_token: "typeTokens.body".into(),
                    style: StyleMap::new(),
                }],
                box_: None,
                overflow: TextOverflow::Visible,
                align: TextAlign::Left,
                vertical_align: VerticalAlign::Top,
                vertical: false,
            }},
            ..group("label", vec![])
        }]);
        document.bindings.insert("label".into(), vec![]);
        assert_eq!(paths(validate_document(&document)), vec!["nodes/0/text/runs/0/bindingId"]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-model`

Expected: new tests fail; several panic because validation still accepts.

- [ ] **Step 3: Implement iterative node and binding validation**

Add to `validate_document` after the artboard check:

```rust
    validate_nodes(document, &mut issues);
    validate_bindings(document, &mut issues);
```

Use an explicit stack of `(&Node, usize /* depth */, String /* path */)`; do not recurse.

Rules, in walk order:
- Before the walk, count nodes into a `usize` with the same stack; if the count exceeds `MAX_NODE_COUNT` (5000), push `"nodes"` with message `"A theme carries at most 5000 nodes."` and skip per-node checks.
- A node whose depth exceeds `MAX_NODE_DEPTH` (32) is refused at its own path `"nodes/0/children/0/..."` with message `"Node depth exceeds 32."`; skip its subtree.
- Refuse an id that fails `is_stable_id` at `"<path>/id"`.
- Refuse a repeated id at the node where it repeats, path `"<path>/id"`.
- For `NodeKind::Text`, each `TextRun::Value` whose `binding_id` does not appear in `document.bindings.get(<node id>)` is refused at `"<path>/text/runs/<index>/bindingId"`.
- For `NodeKind::Chart`, the binding count is checked in `validate_bindings` using the node's family; expose a family lookup helper.

`validate_bindings`:
- The bindings map key must be an existing node id; otherwise issue at `"bindings/<nodeId>"`.
- A node's bindings: ids must satisfy `is_stable_id`; duplicate binding ids are refused at the later index; `semantic_key` must match `^[a-z0-9]+(\.[a-z0-9-]+)+$` (issue at `"bindings/<node>/<index>/semanticKey"`); `precision` must be `Some(0..=6)` or absent; `scale` and `offset` must be finite when present; `unit` must be non-empty when present.
- Chart family caps: `Gauge` refuses more than 1 binding, `Radar` more than 12, every other family has no cap. Report at `"bindings/<node>/0"` with message `"A gauge reads one binding."` or `"A radar chart draws at most 12 bindings."`
- A bindings entry on a non-chart node with more than 0 entries is allowed; only charts have caps.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-model`

Expected: all tests pass. The 10,000-deep test must complete without a stack overflow.

- [ ] **Step 5: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add crates/model
git commit -m "feat(model): validate node limits, ids, bindings and chart caps"
```

---

### Task 5: Validate the background and style references

**Files:**
- Modify: `crates/model/src/validate.rs`
- Test: inline tests in `crates/model/src/validate.rs`

**Interfaces:**
- Consumes: `Background`, `BackgroundMedia`, `BackgroundComposition`, `StyleValue`, `StyleRef` from Task 2.
- Produces: `resolve_style_ref<'a>(document: &'a ThemeDocument, reference: &str) -> Option<StyleRefTarget<'a>>` for the render phase; validation uses the same lookup.

- [ ] **Step 1: Write the failing tests**

Append to the test module:

```rust
    #[test]
    fn refuses_both_background_colors() {
        let mut document = minimal();
        document.background.color = Some(StyleValue::Literal(StyleLiteral {
            value: serde_json::json!("#000000"),
        }));
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(),
            color: Some(StyleValue::Literal(StyleLiteral { value: serde_json::json!("#111111") })),
            bounds: BackgroundBounds::Theme,
            offset_x: 0.0, offset_y: 0.0, scale: 1.0, flip_x: false, flip_y: false,
            fallback_frame: None, composition: None,
        });
        document.assets.push(AssetReference {
            id: "bg".into(), kind: AssetKind::Image, path: "assets/bg.png".into(),
            sha256: None, source_url: None, license: None,
        });
        assert_eq!(paths(validate_document(&document)), vec!["background/color"]);
    }

    #[test]
    fn refuses_composition_on_simple_background() {
        let mut document = minimal();
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(), color: None, bounds: BackgroundBounds::Theme,
            offset_x: 0.0, offset_y: 0.0, scale: 1.0, flip_x: false, flip_y: false,
            fallback_frame: None,
            composition: Some(BackgroundComposition {
                anchor: BackgroundAnchor::Top, extent: Some(0.5), fade: None,
            }),
        });
        document.assets.push(AssetReference {
            id: "bg".into(), kind: AssetKind::Image, path: "assets/bg.png".into(),
            sha256: None, source_url: None, license: None,
        });
        assert_eq!(paths(validate_document(&document)), vec!["background/media/composition"]);
    }

    #[test]
    fn accepts_advanced_composition_with_defaults() {
        let mut document = minimal();
        document.background.mode = BackgroundMode::Advanced;
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(), color: None, bounds: BackgroundBounds::Theme,
            offset_x: 0.0, offset_y: 0.0, scale: 1.0, flip_x: false, flip_y: false,
            fallback_frame: None,
            composition: Some(BackgroundComposition {
                anchor: BackgroundAnchor::Center, extent: None, fade: None,
            }),
        });
        document.assets.push(AssetReference {
            id: "bg".into(), kind: AssetKind::Image, path: "assets/bg.png".into(),
            sha256: None, source_url: None, license: None,
        });
        assert!(validate_document(&document).is_ok());
    }

    #[test]
    fn refuses_composition_ranges() {
        for (composition, expected) in [
            (BackgroundComposition { anchor: BackgroundAnchor::Top, extent: Some(0.09), fade: None }, "background/media/composition/extent"),
            (BackgroundComposition { anchor: BackgroundAnchor::Top, extent: Some(1.01), fade: None }, "background/media/composition/extent"),
            (BackgroundComposition { anchor: BackgroundAnchor::Top, extent: None, fade: Some(0.009) }, "background/media/composition/fade"),
        ] {
            let mut document = minimal();
            document.background.mode = BackgroundMode::Advanced;
            document.background.media = Some(BackgroundMedia {
                asset_id: "bg".into(), color: None, bounds: BackgroundBounds::Theme,
                offset_x: 0.0, offset_y: 0.0, scale: 1.0, flip_x: false, flip_y: false,
                fallback_frame: None, composition: Some(composition),
            });
            document.assets.push(AssetReference {
                id: "bg".into(), kind: AssetKind::Image, path: "assets/bg.png".into(),
                sha256: None, source_url: None, license: None,
            });
            assert_eq!(paths(validate_document(&document)), vec![expected]);
        }
    }

    #[test]
    fn refuses_media_transform_ranges() {
        for (offset_x, offset_y, scale, expected) in [
            (-5.01, 0.0, 1.0, "background/media/offsetX"),
            (5.01, 0.0, 1.0, "background/media/offsetX"),
            (0.0, 0.0, 0.99, "background/media/scale"),
            (0.0, 0.0, 5.01, "background/media/scale"),
            (f64::NAN, 0.0, 1.0, "background/media/offsetX"),
        ] {
            let mut document = minimal();
            document.background.media = Some(BackgroundMedia {
                asset_id: "bg".into(), color: None, bounds: BackgroundBounds::Theme,
                offset_x, offset_y, scale, flip_x: false, flip_y: false,
                fallback_frame: None, composition: None,
            });
            document.assets.push(AssetReference {
                id: "bg".into(), kind: AssetKind::Image, path: "assets/bg.png".into(),
                sha256: None, source_url: None, license: None,
            });
            assert_eq!(paths(validate_document(&document)), vec![expected]);
        }
    }

    #[test]
    fn refuses_unresolved_style_refs() {
        let mut document = minimal();
        document.palette.insert("accent".into(), Paint::Solid { color: "#f00".into() });
        document.nodes.push(Node {
            id: "panel".into(),
            kind: NodeKind::Rect { corner_radius: 4.0 },
            style: StyleMap::from([(
                "fill".into(),
                StyleValue::Ref(StyleRef { r#ref: "palette.missing".into() }),
            )]),
            ..group("panel", vec![])
        });
        assert_eq!(paths(validate_document(&document)), vec!["nodes/0/style/fill/ref"]);

        document.nodes[0].style.insert(
            "fill".into(),
            StyleValue::Ref(StyleRef { r#ref: "typeTokens.body".into() }),
        );
        assert_eq!(paths(validate_document(&document)), vec!["nodes/0/style/fill/ref"]);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-model`

Expected: new tests fail because background and style checks are absent.

- [ ] **Step 3: Implement background and style validation**

Add `validate_background(document, &mut issues)` before the node walk.

Rules:
- `background.color.is_some() && background.media.as_ref().and_then(|media| media.color.as_ref()).is_some()` → issue at `background/color`, message `"A background with media keeps its paint on the media."`
- Media present:
  - `bounds` is an enum, no check.
  - `offset_x` and `offset_y` must be finite and within `-5..=5`; paths `background/media/offsetX`, `background/media/offsetY`.
  - `scale` must be finite and within `1..=5`; path `background/media/scale`.
  - `composition` present while `mode != Advanced` → issue at `background/media/composition`, message `"Only an advanced background may compose its media."`
  - `extent` when present must be finite and within `0.1..=1.0`; path `background/media/composition/extent`.
  - `fade` when present must be finite and within `0.01..=1.0`; path `background/media/composition/fade`.
- Style refs: walk every `StyleMap` (node styles, image monochrome, background colors, media color). A `StyleRef` must match `^(palette|typeTokens)\.([A-Za-z0-9_-]{1,64})$`; `palette.<id>` must exist in `document.palette`, `typeTokens.<id>` in `document.type_tokens`. Report at the style entry's `.../ref` path.

Implement and export:

```rust
pub enum StyleRefTarget<'a> {
    Palette(&'a Paint),
    TypeToken(&'a TypeToken),
}

pub fn resolve_style_ref<'a>(document: &'a ThemeDocument, reference: &str) -> Option<StyleRefTarget<'a>> {
    let (group, id) = reference.split_once('.')?;
    match group {
        "palette" => document.palette.get(id).map(StyleRefTarget::Palette),
        "typeTokens" => document.type_tokens.get(id).map(StyleRefTarget::TypeToken),
        _ => None,
    }
}
```

Have validation call `resolve_style_ref` rather than re-parsing the reference.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-model`

Expected: all tests pass.

- [ ] **Step 5: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add crates/model
git commit -m "feat(model): validate background composition and style references"
```

---

### Task 6: Validate assets and binding values

**Files:**
- Modify: `crates/model/src/validate.rs`
- Test: inline tests in `crates/model/src/validate.rs`

**Interfaces:**
- Consumes: `AssetReference`, `AssetKind`, `Binding` from Task 2.
- Produces: `asset_path_refusal(path: &str, kind: AssetKind) -> Option<&'static str>` reused by the theme package and asset loader in phase 3.

- [ ] **Step 1: Write the failing tests**

Append to the test module:

```rust
    fn asset(id: &str, kind: AssetKind, path: &str) -> AssetReference {
        AssetReference {
            id: id.into(), kind, path: path.into(),
            sha256: None, source_url: None, license: None,
        }
    }

    #[test]
    fn refuses_asset_paths_outside_assets() {
        for path in ["img/a.png", "assets/../a.png", "assets//a.png", "assets/./a.png", "assets/"] {
            let mut document = minimal();
            document.assets.push(asset("a", AssetKind::Image, path));
            assert_eq!(
                paths(validate_document(&document)),
                vec!["assets/0/path"],
                "path {path}"
            );
        }
    }

    #[test]
    fn refuses_asset_kind_extension_mismatch() {
        let mut document = minimal();
        document.assets.push(asset("a", AssetKind::Video, "assets/a.png"));
        assert_eq!(paths(validate_document(&document)), vec!["assets/0/path"]);
    }

    #[test]
    fn accepts_declared_asset_kinds() {
        let mut document = minimal();
        for (index, (kind, path)) in [
            (AssetKind::Image, "assets/a.png"),
            (AssetKind::Svg, "assets/a.svg"),
            (AssetKind::Gif, "assets/a.gif"),
            (AssetKind::Video, "assets/a.mp4"),
            (AssetKind::Font, "assets/a.woff2"),
        ].into_iter().enumerate() {
            document.assets.push(asset(&format!("a{index}"), kind, path));
        }
        assert!(validate_document(&document).is_ok());
    }

    #[test]
    fn refuses_duplicate_and_invalid_asset_ids() {
        let mut document = minimal();
        document.assets.push(asset("same", AssetKind::Image, "assets/a.png"));
        document.assets.push(asset("same", AssetKind::Image, "assets/b.png"));
        assert_eq!(paths(validate_document(&document)), vec!["assets/1/id"]);

        let mut document = minimal();
        document.assets.push(asset("bad id", AssetKind::Image, "assets/a.png"));
        assert_eq!(paths(validate_document(&document)), vec!["assets/0/id"]);
    }

    #[test]
    fn refuses_media_asset_that_is_not_declared() {
        let mut document = minimal();
        document.background.media = Some(BackgroundMedia {
            asset_id: "missing".into(), color: None, bounds: BackgroundBounds::Theme,
            offset_x: 0.0, offset_y: 0.0, scale: 1.0, flip_x: false, flip_y: false,
            fallback_frame: None, composition: None,
        });
        assert_eq!(paths(validate_document(&document)), vec!["background/media/assetId"]);
    }

    #[test]
    fn refuses_fallback_frame_on_non_video_asset() {
        let mut document = minimal();
        document.assets.push(asset("bg", AssetKind::Image, "assets/bg.png"));
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(), color: None, bounds: BackgroundBounds::Theme,
            offset_x: 0.0, offset_y: 0.0, scale: 1.0, flip_x: false, flip_y: false,
            fallback_frame: Some(0), composition: None,
        });
        assert_eq!(paths(validate_document(&document)), vec!["background/media/fallbackFrame"]);
    }

    #[test]
    fn refuses_bad_binding_values() {
        let cases: Vec<(Binding, &str)> = vec![
            (Binding { id: "x".into(), semantic_key: "CPU Load".into(), precision: None,
                unit_display: None, scale: None, offset: None, unit: None, auto_scale: None,
                format: None, time_zone: None }, "bindings/cpu/0/semanticKey"),
            (Binding { id: "x".into(), semantic_key: "cpu.load".into(), precision: None,
                unit_display: None, scale: Some(f64::INFINITY), offset: None, unit: None,
                auto_scale: None, format: None, time_zone: None }, "bindings/cpu/0/scale"),
            (Binding { id: "x".into(), semantic_key: "cpu.load".into(), precision: None,
                unit_display: None, scale: None, offset: None, unit: Some(String::new()),
                auto_scale: None, format: None, time_zone: None }, "bindings/cpu/0/unit"),
        ];
        for (binding, expected) in cases {
            let mut document = minimal();
            document.nodes.push(group("cpu", vec![]));
            document.bindings.insert("cpu".into(), vec![binding]);
            assert_eq!(paths(validate_document(&document)), vec![expected]);
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-model`

Expected: new tests fail.

- [ ] **Step 3: Implement asset and binding value validation**

Add `validate_assets(document, &mut issues)` before background validation so background and node asset references can resolve.

Rules:
- Asset id must pass `is_stable_id`; duplicates refused at the later index (`assets/<index>/id`).
- `asset_path_refusal` returns:
  - `Some("An asset path must start with assets/.")` when the path does not start with `assets/` or has no file name after it.
  - `Some("An asset path may not contain . or .. segments.")` when any `/`-separated segment is `.`, `..` or empty.
  - `Some("An asset kind must match its file extension.")` when the extension does not match the kind: image → `png|jpg|jpeg|webp`, svg → `svg`, gif → `gif`, video → `mp4|webm|mov`, font → `woff2|woff|ttf|otf`.
  - `None` otherwise.
- Background media `asset_id` must resolve; otherwise `background/media/assetId` with `"The media asset is not declared."`
- `fallback_frame` is valid only when the resolved background asset's kind is `Video`; otherwise `background/media/fallbackFrame` with `"Only a video asset has a fallback frame."`
- Node `Image`/`Video` content `asset_id` must resolve to a declared asset; otherwise `<node path>/image/assetId` or `<node path>/video/assetId`.
- Binding `semantic_key` must match `^[a-z0-9]+(\.[a-z0-9-]+)+$`; message `"A semantic key is family.quantity[.qualifier] in lower case."`
- Binding `scale`/`offset` finite when present; `unit` non-empty when present.
- Binding ids must be unique within their node and pass `is_stable_id`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-model`

Expected: all tests pass.

- [ ] **Step 5: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add crates/model
git commit -m "feat(model): validate assets and binding values"
```

---

### Task 7: Pin determinism and round-trips with property tests

**Files:**
- Modify: `crates/model/Cargo.toml` (already has proptest)
- Modify: `crates/model/src/validate.rs`
- Create: `crates/model/tests/properties.rs`

**Interfaces:**
- Consumes: `parse_document`, `write_document`, `validate_document` from Tasks 2–6.
- Produces: assurance that validation is total and error order is deterministic; no new public API.

- [ ] **Step 1: Write the failing property tests**

`crates/model/tests/properties.rs`:

```rust
use proptest::prelude::*;
use yunari_model::document::{parse_document, write_document, Artboard, ThemeDocument};
use yunari_model::validate::validate_document;

fn base() -> ThemeDocument {
    parse_document(
        r#"{"schemaVersion":"1.0.0","id":"theme_1",
            "artboard":{"width":800,"height":480},"nodes":[]}"#,
    )
    .expect("base document")
}

proptest! {
    #[test]
    fn validation_never_panics(width in any::<f64>(), height in any::<f64>()) {
        let mut document = base();
        document.artboard = Artboard { width, height, ..document.artboard };
        let _ = validate_document(&document);
    }

    #[test]
    fn writing_a_valid_document_round_trips(name in "[a-zA-Z0-9 ]{0,120}") {
        let mut document = base();
        document.metadata.name = Some(name);
        if let Ok(json) = write_document(&document) {
            let reparsed = parse_document(&json).expect("written documents parse");
            prop_assert_eq!(reparsed, document);
        }
    }
}
```

Also add a deterministic-order test to the inline test module:

```rust
    #[test]
    fn issues_are_sorted_by_path() {
        let mut document = minimal();
        document.id = "bad id".into();
        document.artboard.width = -1.0;
        let issues = validate_document(&document).expect_err("invalid");
        let mut sorted = issues.clone();
        sorted.sort();
        assert_eq!(issues, sorted);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-model`

Expected: the sort test fails if sorting is not applied; property tests may pass already. If they pass immediately, keep them: they pin behaviour that later tasks could break.

- [ ] **Step 3: Make the minimum change**

Ensure `validate_document` sorts issues before returning on every path. If the sort test already passes, no code change is needed.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-model`

Expected: all tests pass.

- [ ] **Step 5: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add crates/model
git commit -m "test(model): pin validation totality, round-trips and error order"
```

---

### Task 8: Own the editor design tokens

**Files:**
- Create: `crates/theme/Cargo.toml`
- Create: `crates/theme/src/lib.rs`
- Modify: `Cargo.toml` (add workspace member)

**Interfaces:**
- Produces: `yunari_theme::SPACING_STEPS_PX`, `RADIUS_PX`, `TYPE_SCALE`, `FONT_UI`, `FONT_MONO`, `spacing_is_allowed(u16) -> bool`.
- Consumed by: the app shell in Task 10 and the editor in phase 3.

- [ ] **Step 1: Write the failing test**

`crates/theme/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! The editor's design tokens: one owner for spacing, radius and type scale.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadiusScale {
    pub sm: u16,
    pub md: u16,
    pub lg: u16,
    pub xl: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeStep {
    pub name: &'static str,
    pub size_px: u16,
    /// Line height × 10, so 12 means 1.2.
    pub line_height_tenths: u16,
}

pub const SPACING_STEPS_PX: [u16; 10] = [4, 6, 8, 10, 12, 14, 16, 20, 24, 32];

pub const RADIUS_PX: RadiusScale = RadiusScale { sm: 4, md: 6, lg: 8, xl: 10 };

pub const TYPE_SCALE: [TypeStep; 6] = [
    TypeStep { name: "2xs", size_px: 10, line_height_tenths: 12 },
    TypeStep { name: "xs", size_px: 11, line_height_tenths: 13 },
    TypeStep { name: "sm", size_px: 12, line_height_tenths: 14 },
    TypeStep { name: "md", size_px: 13, line_height_tenths: 13 },
    TypeStep { name: "lg", size_px: 15, line_height_tenths: 13 },
    TypeStep { name: "xl", size_px: 19, line_height_tenths: 12 },
];

pub const FONT_UI: &str = "system-ui";
pub const FONT_MONO: &str = "ui-monospace";

pub fn spacing_is_allowed(value: u16) -> bool {
    SPACING_STEPS_PX.contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_scale_is_the_only_scale() {
        assert_eq!(SPACING_STEPS_PX, [4, 6, 8, 10, 12, 14, 16, 20, 24, 32]);
        assert!(spacing_is_allowed(16));
        assert!(!spacing_is_allowed(7));
    }

    #[test]
    fn radius_scale_matches_the_design_document() {
        assert_eq!(RADIUS_PX, RadiusScale { sm: 4, md: 6, lg: 8, xl: 10 });
    }

    #[test]
    fn type_scale_matches_the_design_document() {
        let actual: Vec<(&str, u16, u16)> = TYPE_SCALE
            .iter()
            .map(|step| (step.name, step.size_px, step.line_height_tenths))
            .collect();
        assert_eq!(
            actual,
            vec![
                ("2xs", 10, 12),
                ("xs", 11, 13),
                ("sm", 12, 14),
                ("md", 13, 13),
                ("lg", 15, 13),
                ("xl", 19, 12),
            ]
        );
    }
}
```

`crates/theme/Cargo.toml`:

```toml
[package]
name = "yunari-theme"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
```

Add `"crates/theme"` to `members` in the root `Cargo.toml`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-theme`

Expected: crate does not exist until the manifest and file exist; after creating them, run again to see the tests pass. This task's red state is the missing crate, checked by `cargo test -p yunari-theme` failing with `package ID specification ... did not match`.

- [ ] **Step 3: Confirm the token values against the source design document**

Compare the constants with `D:\git-repos\yunari\DESIGN.md` front matter: spacing 4/6/8/10/12/14/16/20/24/32, radii 4/6/8/10, type 10–19 px with the line heights above. No code change unless they differ.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-theme`

Expected: all three tests pass.

- [ ] **Step 5: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add Cargo.toml crates/theme
git commit -m "feat(theme): own the editor spacing, radius and type tokens"
```

---

### Task 9: Build the golden-image harness

**Files:**
- Create: `crates/golden/Cargo.toml`
- Create: `crates/golden/src/lib.rs`
- Create: `crates/golden/tests/harness.rs`
- Create: `crates/golden/tests/goldens/.gitkeep`
- Modify: `Cargo.toml` (add workspace member)

**Interfaces:**
- Produces: `GoldenConfig`, `GoldenMismatch`, `compare(expected: &[u8], actual: &[u8], config: GoldenConfig) -> Result<(), GoldenMismatch>`, `assert_golden(dir: &Path, name: &str, width: u32, height: u32, rgba: &[u8], config: GoldenConfig)`.
- Consumed by: every render test from phase 1 onward.

- [ ] **Step 1: Write the failing harness tests**

`crates/golden/tests/harness.rs`:

```rust
use std::path::Path;
use yunari_golden::{assert_golden, compare, GoldenConfig};

#[test]
fn identical_pixels_pass() {
    assert!(compare(&[0, 0, 0, 255], &[0, 0, 0, 255], GoldenConfig::default()).is_ok());
}

#[test]
fn one_channel_within_tolerance_passes() {
    let config = GoldenConfig { channel_tolerance: 2, max_diff_ratio: 0.0 };
    assert!(compare(&[10, 10, 10, 255], &[12, 10, 10, 255], config).is_ok());
}

#[test]
fn channel_beyond_tolerance_fails_with_counts() {
    let config = GoldenConfig { channel_tolerance: 2, max_diff_ratio: 1.0 };
    let mismatch = compare(
        &[10, 10, 10, 255, 10, 10, 10, 255],
        &[10, 10, 10, 255, 30, 10, 10, 255],
        config,
    )
    .expect_err("differs");
    assert_eq!(mismatch.differing_pixels, 1);
    assert_eq!(mismatch.total_pixels, 2);
    assert_eq!(mismatch.max_channel_delta, 20);
}

#[test]
fn differing_ratio_beyond_limit_fails() {
    let config = GoldenConfig { channel_tolerance: 0, max_diff_ratio: 0.0 };
    assert!(compare(&[0; 16], &[1; 16], config).is_err());
}

#[test]
fn complains_about_length_mismatch() {
    let mismatch = compare(&[0; 4], &[0; 8], GoldenConfig::default()).expect_err("lengths");
    assert_eq!(mismatch.message(), "expected and actual buffers differ in length");
}

#[test]
fn writes_and_reads_a_golden() {
    let dir = std::env::temp_dir().join("yunari-golden-test");
    let _ = std::fs::remove_dir_all(&dir);
    let rgba = [255u8, 0, 0, 255].repeat(16);
    yunari_golden::write_golden(&dir, "red-4x4", 4, 4, &rgba).expect("write");
    assert_golden(&dir, "red-4x4", 4, 4, &rgba, GoldenConfig::default());
    let path = yunari_golden::golden_path(&dir, "red-4x4");
    assert!(path.is_file());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = Path::new("");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p yunari-golden`

Expected: the crate does not exist; `cargo test -p yunari-golden` fails with a package-match error.

- [ ] **Step 3: Implement the harness**

`crates/golden/Cargo.toml`:

```toml
[package]
name = "yunari-golden"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
image.workspace = true
```

Add `"crates/golden"` to workspace members.

`crates/golden/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! Pixel golden-file comparison for render tests.

use std::path::{Path, PathBuf};

use image::{ImageFormat, RgbaImage};

#[derive(Debug, Clone, Copy)]
pub struct GoldenConfig {
    /// A channel delta up to and including this value is not a difference.
    pub channel_tolerance: u8,
    /// Refuse when the share of differing pixels exceeds this.
    pub max_diff_ratio: f64,
}

impl Default for GoldenConfig {
    fn default() -> Self {
        Self { channel_tolerance: 2, max_diff_ratio: 0.002 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenMismatch {
    pub differing_pixels: usize,
    pub total_pixels: usize,
    pub max_channel_delta: u8,
    message: &'static str,
}

impl GoldenMismatch {
    pub fn message(&self) -> &'static str {
        self.message
    }
}

pub fn golden_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.png"))
}

pub fn compare(expected: &[u8], actual: &[u8], config: GoldenConfig) -> Result<(), GoldenMismatch> {
    // Compare as RGBA pixels. A length mismatch is its own mismatch with a
    // stated message; it never indexes out of bounds.
    // A pixel differs when any channel delta exceeds `channel_tolerance`.
    // Differ when `differing_pixels as f64 / total_pixels as f64 > max_diff_ratio`,
    // including when no pixel differs but the ratio threshold is negative
    // (treat a negative threshold as 0.0).
}

pub fn write_golden(dir: &Path, name: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    // Build an RgbaImage, create `dir`, and save the PNG through `image`.
}

pub fn assert_golden(dir: &Path, name: &str, width: u32, height: u32, rgba: &[u8], config: GoldenConfig) {
    // When YUNARI_UPDATE_GOLDEN=1, write the golden and return.
    // Otherwise load the PNG, compare its raw bytes, and panic with a message
    // naming the golden path, differing pixels, total pixels and max delta.
}
```

Implement the four bodies from the comments. `compare` must accept a negative `max_diff_ratio` as `0.0`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p yunari-golden`

Expected: all six tests pass.

- [ ] **Step 5: Add one committed golden**

Add to `crates/golden/tests/harness.rs`:

```rust
#[test]
fn committed_red_golden_matches() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let rgba = [255u8, 0, 0, 255].repeat(16);
    assert_golden(&dir, "red-4x4", 4, 4, &rgba, GoldenConfig::default());
}
```

Run `YUNARI_UPDATE_GOLDEN=1 cargo test -p yunari-golden`, then `cargo test -p yunari-golden`. Expected: the second run passes using the committed PNG. Keep `tests/goldens/.gitkeep`.

- [ ] **Step 6: Run the gates and commit**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

```bash
git add Cargo.toml crates/golden
git commit -m "test(golden): add pixel golden harness"
```

---

### Task 10: Add the gpui-kit application shell

**Files:**
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/main.rs`

**Interfaces:**
- Consumes: `yunari-theme` tokens from Task 8.
- Produces: `yunari-app` opens a 1280×800 window titled `Yunari` with the product name and a status line. Phase 3 replaces the body with the editor shell; the window boot sequence stays.

- [ ] **Step 1: Add the dependency and write the failing check**

`crates/app/Cargo.toml`:

```toml
[dependencies]
gpui-kit = "0.9.1"
yunari-model.workspace = true
yunari-theme.workspace = true
```

`crates/app/src/main.rs`:

```rust
#![forbid(unsafe_code)]

use gpui_kit::*;
use yunari_theme::SPACING_STEPS_PX;

struct AppShell;

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap(px(SPACING_STEPS_PX[2] as f32))
            .p(px(SPACING_STEPS_PX[5] as f32))
            .child("Yunari")
            .child("native shell — phase 0")
    }
}

fn main() {
    application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            init(cx);
            open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| AppShell)
            })
            .expect("failed to open Yunari window");
        });
}
```

Run `cargo check -p yunari-app`. Expected before the dependency is fetched: a build failure resolving gpui-kit. This is the red check for the task.

- [ ] **Step 2: Build and run**

Run:

```powershell
cargo build -p yunari-app
cargo run -p yunari-app
```

Expected: the first build takes several minutes (GPUI and gpui-kit compile). A window opens showing `Yunari` and `native shell — phase 0`. Close the window to exit.

If the build fails for a missing native tool, return to Task 0. If gpui-kit 0.9.1's public API differs from the snippet, use the crate's own `website/docs/getting-started.md` example as the source of truth and keep the same structure: `application().with_assets(...).run`, `init(cx)`, `open_window(WindowOptions::default(), ...)`.

- [ ] **Step 3: Verify the gates**

Run:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: clean. The manual window check is the UI verification for this phase; no automated UI test exists yet.

- [ ] **Step 4: Commit**

```bash
git add crates/app
git commit -m "feat(app): open a gpui-kit shell window"
```

---

### Task 11: Add the Windows CI gate

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: one required check that runs the same commands as the local gates.

- [ ] **Step 1: Write the workflow**

`.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
  pull_request:

jobs:
  windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - name: Format
        run: cargo fmt --all -- --check

      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Test
        run: cargo test --workspace

      - name: Build app
        run: cargo build --release -p yunari-app
```

- [ ] **Step 2: Verify the file parses**

Run (PowerShell, with Python available) or inspect manually:

```powershell
Get-Content .github/workflows/ci.yml
```

Expected: the file matches the block above; no tabs.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: gate formatting, clippy, tests and the app build on Windows"
```

---

## Phase 0 exit checklist

Run from a clean checkout:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p yunari-app
```

Expected:
- Every command above succeeds.
- The window opens.
- `yunari-model` refuses unknown JSON fields, wrong schema majors, invalid IDs, over-limit artboards and node trees, background rule violations and bad assets, with deterministic paths.
- `yunari-theme` values match the design document.
- `yunari-golden` passes the committed 4×4 fixture.
- CI contains the same four gates.
