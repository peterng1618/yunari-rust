use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::validate::{ValidationIssue, validate_document};

pub const SCHEMA_VERSION: &str = "3.0.0";
pub const SUPPORTED_SCHEMA_MAJOR: u64 = 3;

/// The authoritative token bags. `palette` and `type_tokens` reuse the shapes
/// the editor already authors; the ids are the map keys, so a rename never
/// breaks a reference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Globals {
    #[serde(default)]
    pub palette: BTreeMap<String, PaletteEntry>,
    #[serde(default)]
    pub type_tokens: BTreeMap<String, TypeToken>,
}

/// Fabric JSON, retained as scene data. `version` is the Fabric version axis,
/// preserved and never used for dispatch. Extra scene keys are kept as data —
/// the schema's `fabricScene` allows `additionalProperties`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricScene {
    /// The schema requires a `scene` with a version. Legacy source envelopes
    /// and packages may omit it (or write `null`); absence is kept as `None`
    /// rather than dropping the rest of the scene. `None` is omitted on write:
    /// the schema's `version` property is a string, never null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub objects: Vec<FabricObject>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One Fabric scene object. Only the fields Yunari interprets are typed; the
/// rest of the Fabric record stays in `data` so nothing is dropped. A group's
/// children are Fabric objects too, never major-1 `Node`s.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricObject {
    #[serde(default)]
    pub id: String,
    /// Every optional Fabric field is omitted, not nulled, on write: the
    /// schema types these properties as string/object and refuses null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    #[serde(
        default,
        rename = "yunariGlass",
        skip_serializing_if = "Option::is_none"
    )]
    pub yunari_glass: Option<GlassTreatment>,
    #[serde(default, rename = "objects")]
    pub children: Vec<FabricObject>,
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeDocument {
    pub schema_version: String,
    /// The Fabric version axis. Preserved as data; never used for dispatch.
    #[serde(default)]
    pub fabric_version: String,
    pub id: String,
    #[serde(default)]
    pub metadata: Metadata,
    pub artboard: Artboard,
    #[serde(default)]
    pub background: Background,
    #[serde(default)]
    pub globals: Globals,
    #[serde(default)]
    pub scene: FabricScene,
    /// Major-1 in-memory view, kept for the source-lift/editor path. A real
    /// schema-3 document leaves these empty, so they are omitted from output:
    /// the schema forbids them as top-level fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub palette: BTreeMap<String, PaletteEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_tokens: BTreeMap<String, TypeToken>,
    #[serde(default)]
    pub assets: Vec<AssetReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub bindings: BTreeMap<String, Vec<Binding>>,
    /// Editor-owned scratch data travels with a document and is never read by
    /// a renderer.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub editor_metadata: BTreeMap<String, serde_json::Value>,
}

/// A palette token: the display name an author sees and the paint it resolves
/// to. References use the map key, so renaming never breaks a reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaletteEntry {
    pub name: String,
    pub value: Paint,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The language this theme's text is written in, as a BCP 47 tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_language: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remix_of: Option<Remix>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<StyleValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<StyleValue>,
    /// The box the media is clipped to. Required: a picture with no box has no
    /// placement, and the format refuses it rather than guessing one.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_frame: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Paint {
    Solid {
        color: String,
    },
    Gradient {
        angle: f64,
        stops: Vec<GradientStop>,
    },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<TypeWeight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<Face>,
}

/// A weight is written as a named weight or as a numeric one, so both are the
/// same field rather than two the author has to choose between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TypeWeight {
    Name(String),
    Number(f64),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<AssetLicense>,
    // A font asset declares the face it registers, so a run reaches it by
    // family and weight without trusting the file name. Refused on any other
    // kind, and required on a font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<TypeWeight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<FontStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<FontFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subset: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontStyle {
    Normal,
    Italic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontFormat {
    Woff2,
    Woff,
    Ttf,
    Otf,
}

impl FontFormat {
    /// The file extension a face of this format is written in, so the format
    /// and the path's tail cannot disagree.
    pub fn extension(self) -> &'static str {
        match self {
            FontFormat::Woff2 => "woff2",
            FontFormat::Woff => "woff",
            FontFormat::Ttf => "ttf",
            FontFormat::Otf => "otf",
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    /// A frosted backdrop: the panel samples what is painted beneath it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<GlassTreatment>,
    #[serde(default)]
    pub children: Vec<Node>,
}

/// The authored glass treatment: a blur radius in artboard units, and whether
/// it follows a moving picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlassTreatment {
    pub blur_radius: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<GlassSampling>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlassSampling {
    /// The background's declared still, which is the picture an image
    /// background or a paint gives it.
    Static,
    /// The live picture under the panel.
    Dynamic,
}

/// Measured, not a preference: a radius sweep held 2.5–3.7 ms per frame in the
/// source renderer across 0–64 px, and the cost rose at 128. 48 bounds the
/// worst case inside the measured-flat band.
pub const MAX_GLASS_BLUR_RADIUS: f64 = 48.0;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            rotation: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NodeKind {
    Group,
    Rectangle {
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
    #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
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
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TextRun {
    Literal {
        text: String,
        type_token: String,
        #[serde(default)]
        style: StyleMap,
    },
    Value {
        binding_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        precision: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_display: Option<UnitDisplay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_scale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// The keys each internally tagged payload may carry, by tag value.
///
/// `serde` buffers the content of an internally tagged enum, so neither
/// `deny_unknown_fields` nor `serde_ignored` can see a typo inside one. The
/// kinds and text runs are exactly where an author types field names, so the
/// audit lives here rather than being left to a reader's silence.
static PAYLOAD_KEYS: std::sync::LazyLock<
    std::collections::BTreeMap<&'static str, &'static [&'static str]>,
> = std::sync::LazyLock::new(|| {
    std::collections::BTreeMap::from([
        ("group", &["type"][..]),
        ("rectangle", &["type", "cornerRadius"][..]),
        ("ellipse", &["type"][..]),
        ("line", &["type"][..]),
        ("path", &["type", "data", "strokeWidth", "closed"][..]),
        ("text", &["type", "content"][..]),
        ("chart", &["type", "content"][..]),
        ("image", &["type", "content"][..]),
        ("video", &["type", "content"][..]),
        ("literal", &["kind", "text", "typeToken", "style"][..]),
        (
            "value",
            &[
                "kind",
                "bindingId",
                "precision",
                "unitDisplay",
                "typeToken",
                "style",
            ][..],
        ),
    ])
});

fn refuse_unknown_payload_keys(
    payload: &serde_json::Value,
    tag_key: &str,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(payload) = payload.as_object() else {
        return;
    };
    let Some(known) = payload
        .get(tag_key)
        .and_then(|tag| tag.as_str())
        .and_then(|tag| PAYLOAD_KEYS.get(tag))
    else {
        return;
    };
    for key in payload.keys() {
        if !known.contains(&key.as_str()) {
            issues.push(ValidationIssue {
                path: format!("{path}/{key}"),
                message: "Unknown field is refused.".into(),
            });
        }
    }
}

fn refuse_unknown_payload_keys_in_nodes(
    path: &str,
    node: &serde_json::Value,
    issues: &mut Vec<ValidationIssue>,
) {
    let kind = node.get("kind");
    if let Some(kind) = kind {
        refuse_unknown_payload_keys(kind, "type", &format!("{path}/kind"), issues);
    }
    if kind
        .and_then(|kind| kind.get("type"))
        .and_then(|tag| tag.as_str())
        == Some("text")
    {
        let runs = kind
            .and_then(|kind| kind.get("content"))
            .and_then(|content| content.get("runs"))
            .and_then(|runs| runs.as_array());
        for (index, run) in runs.into_iter().flatten().enumerate() {
            refuse_unknown_payload_keys(
                run,
                "kind",
                &format!("{path}/kind/content/runs/{index}"),
                issues,
            );
        }
    }
    if let Some(children) = node
        .get("children")
        .and_then(|children| children.as_array())
    {
        for (index, child) in children.iter().enumerate() {
            refuse_unknown_payload_keys_in_nodes(
                &format!("{path}/children/{index}"),
                child,
                issues,
            );
        }
    }
}

pub fn parse_document(json: &str) -> Result<ThemeDocument, DocumentError> {
    let mut ignored = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let document: ThemeDocument = serde_ignored::deserialize(&mut deserializer, |path| {
        ignored.push(path.to_string());
    })?;
    deserializer.end()?;

    let mut issues = Vec::new();
    for path in ignored {
        issues.push(ValidationIssue {
            path: path.replace('.', "/"),
            message: "Unknown field is refused.".into(),
        });
    }
    let raw: serde_json::Value = serde_json::from_str(json)?;
    // `$defs.fabricScene` requires `objects`. The typed field defaults to an
    // empty array so legacy envelopes still parse, but a document that
    // declares a scene must declare its object list too.
    if let Some(scene) = raw.get("scene")
        && scene.is_object()
        && scene.get("objects").is_none()
    {
        issues.push(ValidationIssue {
            path: "scene/objects".into(),
            message: "A scene declares its objects, even when the list is empty.".into(),
        });
    }
    for (index, node) in raw
        .get("nodes")
        .and_then(|nodes| nodes.as_array())
        .into_iter()
        .flatten()
        .enumerate()
    {
        refuse_unknown_payload_keys_in_nodes(&format!("nodes/{index}"), node, &mut issues);
    }
    if !issues.is_empty() {
        issues.sort();
        return Err(DocumentError::Invalid(issues));
    }

    validate_document(&document).map_err(DocumentError::Invalid)?;
    Ok(document)
}

pub fn write_document(document: &ThemeDocument) -> Result<String, DocumentError> {
    validate_document(document).map_err(DocumentError::Invalid)?;
    Ok(serde_json::to_string_pretty(document)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "schemaVersion": "3.0.0",
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
        assert!(matches!(
            error,
            DocumentError::Json(_) | DocumentError::Invalid(_)
        ));
    }

    #[test]
    fn refuses_unknown_node_field() {
        let json = MINIMAL.replace(
            "\"nodes\": []",
            r#""nodes": [{"id":"n1","kind":{"type":"group"},"typo":true}]"#,
        );
        let error = parse_document(&json).expect_err("unknown node field is refused");
        assert!(matches!(error, DocumentError::Json(_)));
    }

    /// The node's own fields refuse themselves through `deny_unknown_fields`;
    /// a kind's fields are behind an internally tagged enum, so this is the
    /// case that proves the parser refuses a typo rather than dropping it.
    #[test]
    fn refuses_unknown_field_inside_a_node_kind() {
        let json = MINIMAL.replace(
            "\"nodes\": []",
            r#""nodes": [{"id":"n1","kind":{"type":"group","typo":true}}]"#,
        );
        parse_document(&json).expect_err("unknown kind field is refused");
    }

    #[test]
    fn accepts_minor_and_patch_within_supported_major() {
        let json = MINIMAL.replace("\"3.0.0\"", "\"3.4.9\"");
        assert!(parse_document(&json).is_ok());
    }

    #[test]
    fn refuses_newer_major() {
        let json = MINIMAL.replace("\"3.0.0\"", "\"4.0.0\"");
        let error = parse_document(&json).expect_err("major 4 is refused");
        let DocumentError::Invalid(issues) = error else {
            panic!("expected validation issues");
        };
        assert!(issues.iter().any(|issue| issue.path == "schemaVersion"));
    }

    #[test]
    fn refuses_unparseable_schema_version() {
        let json = MINIMAL.replace("\"3.0.0\"", "\"one\"");
        let error = parse_document(&json).expect_err("non-semver is refused");
        let DocumentError::Invalid(issues) = error else {
            panic!("expected validation issues");
        };
        assert!(issues.iter().any(|issue| issue.path == "schemaVersion"));
    }
}
