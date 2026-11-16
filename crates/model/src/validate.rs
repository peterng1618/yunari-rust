use std::collections::BTreeSet;
use std::fmt;

use crate::document::{
    Artboard, ArtboardShape, AssetKind, AssetReference, Background, BackgroundMode, Binding,
    ChartFamily, Metadata, Node, NodeKind, Paint, SUPPORTED_SCHEMA_MAJOR, StyleMap, StyleValue,
    ThemeDocument, TypeToken,
};

pub const MAX_ARTBOARD_DIMENSION: f64 = 16384.0;
pub const MAX_NODE_DEPTH: usize = 32;
pub const MAX_NODE_COUNT: usize = 5000;
pub const MAX_METADATA_NAME_BYTES: usize = 200;
pub const MAX_TAGS: usize = 5;

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

pub enum StyleRefTarget<'a> {
    Palette(&'a Paint),
    TypeToken(&'a TypeToken),
}

pub fn resolve_style_ref<'a>(
    document: &'a ThemeDocument,
    reference: &str,
) -> Option<StyleRefTarget<'a>> {
    let (group, id) = reference.split_once('.')?;
    if !is_stable_id(id) {
        return None;
    }
    match group {
        "palette" => document
            .palette
            .get(id)
            .map(|entry| StyleRefTarget::Palette(&entry.value)),
        "typeTokens" => document.type_tokens.get(id).map(StyleRefTarget::TypeToken),
        _ => None,
    }
}

pub fn is_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Why an asset path is not this format's, or `None` when it is.
pub fn asset_path_refusal(path: &str, kind: AssetKind) -> Option<&'static str> {
    if path.len() > 200 || !path.starts_with("assets/") {
        return Some("An asset path must start with assets/.");
    }
    let rest = &path["assets/".len()..];
    if rest.is_empty()
        || !rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
    {
        return Some("An asset path may only contain A-Za-z0-9._/- after assets/.");
    }
    if rest
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Some("An asset path may not contain . or .. segments.");
    }
    let extension = rest.rsplit_once('.').map(|(_, extension)| extension);
    let matches_kind = match kind {
        AssetKind::Image => matches!(extension, Some("png" | "jpg" | "jpeg" | "webp")),
        AssetKind::Svg => extension == Some("svg"),
        AssetKind::Gif => extension == Some("gif"),
        AssetKind::Video => matches!(extension, Some("mp4" | "webm" | "mov")),
        AssetKind::Font => matches!(extension, Some("woff2" | "woff" | "ttf" | "otf")),
    };
    if !matches_kind {
        return Some("An asset kind must match its file extension.");
    }
    None
}

pub fn validate_document(document: &ThemeDocument) -> Result<(), Vec<ValidationIssue>> {
    let mut issues = Vec::new();

    validate_schema_version(&document.schema_version, &mut issues);
    if !is_stable_id(&document.id) {
        issues.push(issue("id", "An id must match [A-Za-z0-9_-]{1,64}."));
    }
    validate_metadata(&document.metadata, &mut issues);
    validate_artboard(&document.artboard, &mut issues);
    validate_tokens(document, &mut issues);
    validate_assets(document, &mut issues);
    validate_background(document, &mut issues);
    validate_nodes(document, &mut issues);
    validate_bindings(document, &mut issues);

    if issues.is_empty() {
        Ok(())
    } else {
        issues.sort();
        Err(issues)
    }
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> ValidationIssue {
    ValidationIssue {
        path: path.into(),
        message: message.into(),
    }
}

fn validate_schema_version(version: &str, issues: &mut Vec<ValidationIssue>) {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        issues.push(issue(
            "schemaVersion",
            "A schema version is major.minor.patch.",
        ));
        return;
    }
    let major = parts[0].parse::<u64>().ok();
    if major != Some(SUPPORTED_SCHEMA_MAJOR) {
        issues.push(issue(
            "schemaVersion",
            format!("Only schema major {SUPPORTED_SCHEMA_MAJOR} is supported."),
        ));
    }
}

fn validate_metadata(metadata: &Metadata, issues: &mut Vec<ValidationIssue>) {
    if metadata
        .name
        .as_ref()
        .is_some_and(|name| name.len() > MAX_METADATA_NAME_BYTES)
    {
        issues.push(issue(
            "metadata/name",
            format!("A theme name is at most {MAX_METADATA_NAME_BYTES} bytes."),
        ));
    }
    if metadata.tags.len() > MAX_TAGS {
        issues.push(issue(
            "metadata/tags",
            format!("A theme carries at most {MAX_TAGS} tags."),
        ));
    }
    let mut seen = BTreeSet::new();
    for (index, tag) in metadata.tags.iter().enumerate() {
        let valid = !tag.is_empty()
            && tag.len() <= 24
            && tag
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            issues.push(issue(
                format!("metadata/tags/{index}"),
                "A tag matches [a-z0-9-]{1,24}.",
            ));
        }
        if !seen.insert(tag.as_str()) {
            issues.push(issue(
                format!("metadata/tags/{index}"),
                "A tag is listed twice.",
            ));
        }
    }
}

fn validate_artboard(artboard: &Artboard, issues: &mut Vec<ValidationIssue>) {
    for (name, value) in [
        ("artboard/width", artboard.width),
        ("artboard/height", artboard.height),
    ] {
        if !value.is_finite() || value <= 0.0 || value > MAX_ARTBOARD_DIMENSION {
            issues.push(issue(
                name,
                format!(
                    "An artboard dimension is greater than 0 and at most {MAX_ARTBOARD_DIMENSION}."
                ),
            ));
        }
    }
    if artboard.shape == ArtboardShape::Circle && artboard.width != artboard.height {
        issues.push(issue("artboard/shape", "A circle artboard must be square."));
    }
}

fn validate_tokens(document: &ThemeDocument, issues: &mut Vec<ValidationIssue>) {
    for id in document.palette.keys() {
        if !is_stable_id(id) {
            issues.push(issue(
                format!("palette/{id}"),
                "A palette token id must match [A-Za-z0-9_-]{1,64}.",
            ));
        }
    }
    for id in document.type_tokens.keys() {
        if !is_stable_id(id) {
            issues.push(issue(
                format!("typeTokens/{id}"),
                "A type token id must match [A-Za-z0-9_-]{1,64}.",
            ));
        }
    }
    for (id, entry) in &document.palette {
        validate_paint(&entry.value, &format!("palette/{id}"), issues);
    }
}

fn validate_paint(paint: &Paint, path: &str, issues: &mut Vec<ValidationIssue>) {
    if let Paint::Gradient { angle, stops } = paint {
        if !angle.is_finite() {
            issues.push(issue(format!("{path}/angle"), "An angle must be finite."));
        }
        if stops.len() < 2 {
            issues.push(issue(
                format!("{path}/stops"),
                "A gradient has at least two stops.",
            ));
        }
        for (index, stop) in stops.iter().enumerate() {
            if !stop.offset.is_finite() || !(0.0..=1.0).contains(&stop.offset) {
                issues.push(issue(
                    format!("{path}/stops/{index}/offset"),
                    "A gradient stop offset is from 0 to 1.",
                ));
            }
        }
    }
}

fn validate_assets(document: &ThemeDocument, issues: &mut Vec<ValidationIssue>) {
    let mut seen = BTreeSet::new();
    for (index, asset) in document.assets.iter().enumerate() {
        if !is_stable_id(&asset.id) {
            issues.push(issue(
                format!("assets/{index}/id"),
                "An asset id must match [A-Za-z0-9_-]{1,64}.",
            ));
        }
        if !seen.insert(asset.id.as_str()) {
            issues.push(issue(
                format!("assets/{index}/id"),
                "An asset id is declared twice.",
            ));
        }
        if let Some(refusal) = asset_path_refusal(&asset.path, asset.kind) {
            issues.push(issue(format!("assets/{index}/path"), refusal));
        }
        validate_font_face(asset, index, issues);
    }
}

/// A font asset is the only kind that declares a face, and it declares all of
/// it: a run reaches a font by family and weight, never by file name.
fn validate_font_face(asset: &AssetReference, index: usize, issues: &mut Vec<ValidationIssue>) {
    let declares_face = asset.family.is_some()
        || asset.weight.is_some()
        || asset.style.is_some()
        || asset.format.is_some()
        || asset.subset.is_some();
    if asset.kind != AssetKind::Font {
        if declares_face {
            issues.push(issue(
                format!("assets/{index}"),
                "Only a font asset declares a face.",
            ));
        }
        return;
    }
    if asset.family.as_ref().is_none_or(String::is_empty) {
        issues.push(issue(
            format!("assets/{index}/family"),
            "A font asset declares a family.",
        ));
    }
    if asset.weight.is_none() {
        issues.push(issue(
            format!("assets/{index}/weight"),
            "A font asset declares a weight.",
        ));
    }
    if asset.style.is_none() {
        issues.push(issue(
            format!("assets/{index}/style"),
            "A font asset declares a style.",
        ));
    }
    let Some(format) = asset.format else {
        issues.push(issue(
            format!("assets/{index}/format"),
            "A font asset declares its format.",
        ));
        return;
    };
    let extension = asset.path.rsplit_once('.').map(|(_, extension)| extension);
    if extension != Some(format.extension()) {
        issues.push(issue(
            format!("assets/{index}/format"),
            "A font asset's format is its file extension.",
        ));
    }
}

fn asset_by_id<'a>(document: &'a ThemeDocument, id: &str) -> Option<&'a AssetReference> {
    document.assets.iter().find(|asset| asset.id == id)
}

fn validate_background(document: &ThemeDocument, issues: &mut Vec<ValidationIssue>) {
    let background: &Background = &document.background;
    let media_color = background
        .media
        .as_ref()
        .and_then(|media| media.color.as_ref());
    if background.color.is_some() && media_color.is_some() {
        issues.push(issue(
            "background/color",
            "A background with media keeps its paint on the media.",
        ));
    }
    if let Some(color) = &background.color {
        validate_style_value(document, color, "background/color", issues);
    }
    let Some(media) = &background.media else {
        return;
    };
    if let Some(color) = &media.color {
        validate_style_value(document, color, "background/media/color", issues);
    }
    match asset_by_id(document, &media.asset_id) {
        None => issues.push(issue(
            "background/media/assetId",
            "The media asset is not declared.",
        )),
        Some(asset) => {
            if media.fallback_frame.is_some() && asset.kind != AssetKind::Video {
                issues.push(issue(
                    "background/media/fallbackFrame",
                    "Only a video asset has a fallback frame.",
                ));
            }
        }
    }
    for (name, value) in [
        ("background/media/offsetX", media.offset_x),
        ("background/media/offsetY", media.offset_y),
    ] {
        if !value.is_finite() || !(-5.0..=5.0).contains(&value) {
            issues.push(issue(name, "A media offset is from -5 to 5."));
        }
    }
    if !media.scale.is_finite() || !(1.0..=5.0).contains(&media.scale) {
        issues.push(issue(
            "background/media/scale",
            "A media scale is from 1 to 5.",
        ));
    }
    let Some(composition) = &media.composition else {
        return;
    };
    if background.mode != BackgroundMode::Advanced {
        issues.push(issue(
            "background/media/composition",
            "Only an advanced background may compose its media.",
        ));
    }
    if let Some(extent) = composition.extent
        && (!extent.is_finite() || !(0.1..=1.0).contains(&extent))
    {
        issues.push(issue(
            "background/media/composition/extent",
            "An extent is from 0.1 to 1.",
        ));
    }
    if let Some(fade) = composition.fade
        && (!fade.is_finite() || !(0.01..=1.0).contains(&fade))
    {
        issues.push(issue(
            "background/media/composition/fade",
            "A fade is from 0.01 to 1.",
        ));
    }
    // The anchor enum is exhaustive; the composition rule does not depend on
    // which anchor the author chose.
}

struct NodeEntry<'a> {
    node: &'a Node,
    depth: usize,
    path: String,
}

fn validate_nodes(document: &ThemeDocument, issues: &mut Vec<ValidationIssue>) {
    let mut count = 0usize;
    let mut count_stack: Vec<&Node> = document.nodes.iter().collect();
    while let Some(node) = count_stack.pop() {
        count += 1;
        count_stack.extend(node.children.iter());
    }
    if count > MAX_NODE_COUNT {
        issues.push(issue(
            "nodes",
            format!("A theme carries at most {MAX_NODE_COUNT} nodes."),
        ));
        return;
    }

    let mut seen = BTreeSet::new();
    let mut stack: Vec<NodeEntry<'_>> = Vec::with_capacity(document.nodes.len());
    for (index, node) in document.nodes.iter().enumerate().rev() {
        stack.push(NodeEntry {
            node,
            depth: 1,
            path: format!("nodes/{index}"),
        });
    }

    while let Some(entry) = stack.pop() {
        let NodeEntry { node, depth, path } = entry;
        if depth > MAX_NODE_DEPTH {
            issues.push(issue(path, format!("Node depth exceeds {MAX_NODE_DEPTH}.")));
            continue;
        }
        if !is_stable_id(&node.id) {
            issues.push(issue(
                format!("{path}/id"),
                "An id must match [A-Za-z0-9_-]{1,64}.",
            ));
        }
        if !seen.insert(node.id.as_str()) {
            issues.push(issue(format!("{path}/id"), "An id is used twice."));
        }
        validate_transform(node, &path, issues);
        validate_style_map(document, &node.style, &format!("{path}/style"), issues);
        validate_node_kind(document, node, &path, issues);
        validate_glass(node, &path, issues);

        for (index, child) in node.children.iter().enumerate().rev() {
            stack.push(NodeEntry {
                node: child,
                depth: depth + 1,
                path: format!("{path}/children/{index}"),
            });
        }
    }
}

fn validate_transform(node: &Node, path: &str, issues: &mut Vec<ValidationIssue>) {
    let transform = &node.transform;
    for (name, value) in [
        ("x", transform.x),
        ("y", transform.y),
        ("width", transform.width),
        ("height", transform.height),
        ("rotation", transform.rotation),
        ("scaleX", transform.scale_x),
        ("scaleY", transform.scale_y),
    ] {
        if !value.is_finite() {
            issues.push(issue(
                format!("{path}/transform/{name}"),
                "A transform value must be finite.",
            ));
        }
    }
    if transform.width < 0.0 || transform.height < 0.0 {
        issues.push(issue(
            format!("{path}/transform"),
            "A node width and height are non-negative.",
        ));
    }
}

/// Glass is a property of a closed surface: the frosting needs an interior to
/// sample the backdrop through, so a glyph, a plotted series or an open line is
/// refused rather than clipped to a box it does not fill.
fn validate_glass(node: &Node, path: &str, issues: &mut Vec<ValidationIssue>) {
    let Some(glass) = &node.glass else {
        return;
    };
    let closed = match &node.kind {
        NodeKind::Rectangle { .. } | NodeKind::Ellipse => true,
        NodeKind::Path { closed, .. } => *closed,
        NodeKind::Line
        | NodeKind::Group
        | NodeKind::Text { .. }
        | NodeKind::Chart { .. }
        | NodeKind::Image { .. }
        | NodeKind::Video { .. } => false,
    };
    if !closed {
        issues.push(issue(
            format!("{path}/glass"),
            "Only a closed shape can carry a glass backdrop.",
        ));
    }
    if !glass.blur_radius.is_finite()
        || !(0.0..=crate::document::MAX_GLASS_BLUR_RADIUS).contains(&glass.blur_radius)
    {
        issues.push(issue(
            format!("{path}/glass/blurRadius"),
            format!("A glass blur is from 0 to {} artboard units.", crate::document::MAX_GLASS_BLUR_RADIUS),
        ));
    }
}

fn validate_node_kind(
    document: &ThemeDocument,
    node: &Node,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let bindings = document.bindings.get(&node.id);
    match &node.kind {
        NodeKind::Text { content } => {
            for (index, run) in content.runs.iter().enumerate() {
                let run_path = format!("{path}/text/runs/{index}");
                let (type_token, style, binding_id) = match run {
                    crate::document::TextRun::Literal {
                        type_token, style, ..
                    } => (type_token, style, None),
                    crate::document::TextRun::Value {
                        type_token,
                        style,
                        binding_id,
                        ..
                    } => (type_token, style, Some(binding_id)),
                };
                validate_type_token_ref(
                    document,
                    type_token,
                    &format!("{run_path}/typeToken"),
                    issues,
                );
                validate_style_map(document, style, &format!("{run_path}/style"), issues);
                if let Some(binding_id) = binding_id {
                    let found = bindings
                        .is_some_and(|entries| entries.iter().any(|entry| &entry.id == binding_id));
                    if !found {
                        issues.push(issue(
                            format!("{run_path}/bindingId"),
                            "A value run references a binding on the same node.",
                        ));
                    }
                }
            }
        }
        NodeKind::Image { content } => {
            validate_asset_ref(
                document,
                &content.asset_id,
                &format!("{path}/image/assetId"),
                issues,
            );
            if let Some(monochrome) = &content.monochrome {
                validate_style_value(
                    document,
                    monochrome,
                    &format!("{path}/image/monochrome"),
                    issues,
                );
            }
        }
        NodeKind::Video { content } => {
            validate_asset_ref(
                document,
                &content.asset_id,
                &format!("{path}/video/assetId"),
                issues,
            );
        }
        _ => {}
    }
}

fn validate_asset_ref(
    document: &ThemeDocument,
    asset_id: &str,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if asset_by_id(document, asset_id).is_none() {
        issues.push(issue(path, "The asset is not declared."));
    }
}

fn validate_type_token_ref(
    document: &ThemeDocument,
    reference: &str,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(id) = reference.strip_prefix("typeTokens.") else {
        issues.push(issue(path, "A type token is typeTokens.<id>."));
        return;
    };
    if !is_stable_id(id) || !document.type_tokens.contains_key(id) {
        issues.push(issue(path, "The type token is not declared."));
    }
}

fn validate_style_map(
    document: &ThemeDocument,
    style: &StyleMap,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    for (key, value) in style {
        validate_style_value(document, value, &format!("{path}/{key}"), issues);
    }
}

fn validate_style_value(
    document: &ThemeDocument,
    value: &StyleValue,
    path: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    let StyleValue::Ref(reference) = value else {
        return;
    };
    if resolve_style_ref(document, &reference.r#ref).is_none() {
        issues.push(issue(
            format!("{path}/ref"),
            "A style reference names a declared palette or type token.",
        ));
    }
}

fn validate_bindings(document: &ThemeDocument, issues: &mut Vec<ValidationIssue>) {
    let node_ids: BTreeSet<&str> = {
        let mut ids = BTreeSet::new();
        let mut stack: Vec<&Node> = document.nodes.iter().collect();
        while let Some(node) = stack.pop() {
            ids.insert(node.id.as_str());
            stack.extend(node.children.iter());
        }
        ids
    };

    for (node_id, bindings) in &document.bindings {
        if !node_ids.contains(node_id.as_str()) {
            issues.push(issue(
                format!("bindings/{node_id}"),
                "A binding key is a node id.",
            ));
        }
        let mut seen = BTreeSet::new();
        for (index, binding) in bindings.iter().enumerate() {
            validate_binding(
                binding,
                &format!("bindings/{node_id}/{index}"),
                &mut seen,
                issues,
            );
        }
    }

    // Chart caps live on the node kind, but report at the binding path so one
    // rule has one place to read.
    let mut stack: Vec<&Node> = document.nodes.iter().collect();
    while let Some(node) = stack.pop() {
        if let NodeKind::Chart { content } = &node.kind {
            let count = document.bindings.get(&node.id).map_or(0, Vec::len);
            let (max, message) = match content.family {
                ChartFamily::Gauge => (1usize, "A gauge reads one binding."),
                ChartFamily::Radar => (12usize, "A radar chart draws at most 12 bindings."),
                _ => continue,
            };
            if count > max {
                let path = format!("bindings/{}/0", node.id);
                if !issues.iter().any(|issue| issue.path == path) {
                    issues.push(issue(path, message));
                }
            }
        }
        stack.extend(node.children.iter());
    }
}

fn validate_binding<'a>(
    binding: &'a Binding,
    path: &str,
    seen: &mut BTreeSet<&'a str>,
    issues: &mut Vec<ValidationIssue>,
) {
    if !is_stable_id(&binding.id) {
        issues.push(issue(
            format!("{path}/id"),
            "A binding id must match [A-Za-z0-9_-]{1,64}.",
        ));
    }
    if !seen.insert(binding.id.as_str()) {
        issues.push(issue(format!("{path}/id"), "A binding id is used twice."));
    }
    if let Some(refusal) = semantic_key_refusal(&binding.semantic_key) {
        issues.push(issue(format!("{path}/semanticKey"), refusal));
    }
    if let Some(precision) = binding.precision
        && precision > 6
    {
        issues.push(issue(
            format!("{path}/precision"),
            "precision must be an integer from 0 to 6.",
        ));
    }
    if let Some(scale) = binding.scale
        && !scale.is_finite()
    {
        issues.push(issue(format!("{path}/scale"), "A scale must be finite."));
    }
    if let Some(offset) = binding.offset
        && !offset.is_finite()
    {
        issues.push(issue(format!("{path}/offset"), "An offset must be finite."));
    }
    if binding.unit.as_ref().is_some_and(String::is_empty) {
        issues.push(issue(format!("{path}/unit"), "A unit is not empty."));
    }
}

/// Why a semantic key is not this format's, or `None` when it is. The editor
/// asks before it writes, so a refused key never reaches a save.
pub fn semantic_key_refusal(value: &str) -> Option<&'static str> {
    if is_semantic_key(value) {
        None
    } else {
        Some("A semantic key is family.quantity[.qualifier] in lower case.")
    }
}

fn is_semantic_key(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

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

    fn group(id: &str, children: Vec<Node>) -> Node {
        Node {
            id: id.into(),
            name: None,
            kind: NodeKind::Group,
            transform: Transform::default(),
            visible: true,
            locked: false,
            glass: None,
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
    fn glass_is_read_on_a_closed_shape() {
        let document = parse_document(
            r#"{"schemaVersion":"1.0.0","id":"t","artboard":{"width":10,"height":10},
                "nodes":[{"id":"p","kind":{"type":"rectangle"},
                "transform":{"width":10,"height":10},
                "glass":{"blurRadius":24,"sampling":"static"}}]}"#,
        )
        .expect("a frosted rectangle is a theme");
        let glass = document.nodes[0].glass.as_ref().expect("the treatment");
        assert_eq!(glass.blur_radius, 24.0);
        assert_eq!(glass.sampling, Some(GlassSampling::Static));
    }

    fn frosted(kind: NodeKind, blur_radius: f64) -> Node {
        Node {
            id: "p".into(),
            name: None,
            kind,
            transform: Transform {
                width: 10.0,
                height: 10.0,
                ..Transform::default()
            },
            visible: true,
            locked: false,
            glass: Some(GlassTreatment {
                blur_radius,
                sampling: None,
            }),
            style: StyleMap::new(),
            children: Vec::new(),
        }
    }

    #[test]
    fn glass_is_refused_where_there_is_no_interior_to_sample() {
        let document = with_nodes(vec![frosted(NodeKind::Line, 8.0)]);
        assert_eq!(paths(validate_document(&document)), vec!["nodes/0/glass"]);
    }

    #[test]
    fn a_glass_blur_is_bounded() {
        let document = with_nodes(vec![frosted(NodeKind::Rectangle { corner_radius: 4.0 }, 60.0)]);
        assert_eq!(
            paths(validate_document(&document)),
            vec!["nodes/0/glass/blurRadius"]
        );
    }

    fn binding(id: &str, semantic_key: &str) -> Binding {
        Binding {
            id: id.into(),
            semantic_key: semantic_key.into(),
            precision: None,
            unit_display: None,
            scale: None,
            offset: None,
            unit: None,
            auto_scale: None,
            format: None,
            time_zone: None,
        }
    }

    fn asset(id: &str, kind: AssetKind, path: &str) -> AssetReference {
        AssetReference {
            id: id.into(),
            kind,
            path: path.into(),
            sha256: None,
            source_url: None,
            license: None,
            family: None,
            weight: None,
            style: None,
            format: None,
            subset: None,
        }
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
        document.artboard = Artboard {
            width: 480.0,
            height: 480.0,
            ..document.artboard
        };
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
        document.nodes.push(group("bad id", vec![]));
        assert_eq!(paths(validate_document(&document)), vec!["nodes/0/id"]);
    }

    #[test]
    fn accepts_5000_nodes() {
        let nodes = (0..5000)
            .map(|index| group(&format!("n{index}"), vec![]))
            .collect();
        assert!(validate_document(&with_nodes(nodes)).is_ok());
    }

    #[test]
    fn refuses_5001_nodes() {
        let nodes = (0..5001)
            .map(|index| group(&format!("n{index}"), vec![]))
            .collect();
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
        assert!(
            issues
                .iter()
                .any(|issue| issue.message.contains("Node depth"))
        );
    }

    /// A tree deep enough to overflow a recursive validator never reaches one:
    /// the JSON reader caps nesting first, so the depth rule is a product bound
    /// and the reader's own limit is the safety one.
    #[test]
    fn refuses_a_document_nested_deeper_than_the_reader_allows() {
        let mut json = String::from(
            r#"{"schemaVersion":"1.0.0","id":"theme_1","artboard":{"width":800,"height":480},"nodes":["#,
        );
        for _ in 0..300 {
            json.push_str(r#"{"id":"n","kind":{"type":"group"},"children":["#);
        }
        for _ in 0..300 {
            json.push_str("]}");
        }
        json.push_str("]}");
        parse_document(&json).expect_err("deep nesting is refused");
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
            kind: NodeKind::Chart {
                content: ChartContent {
                    family: ChartFamily::Gauge,
                    settings: ChartSettings::default(),
                },
            },
            ..group("chart", vec![])
        }]);
        document.bindings.insert(
            "chart".into(),
            vec![binding("a", "cpu.load"), binding("b", "ram.used")],
        );
        assert_eq!(
            paths(validate_document(&document)),
            vec!["bindings/chart/0"]
        );
    }

    #[test]
    fn refuses_radar_with_thirteen_bindings() {
        let mut document = with_nodes(vec![Node {
            kind: NodeKind::Chart {
                content: ChartContent {
                    family: ChartFamily::Radar,
                    settings: ChartSettings::default(),
                },
            },
            ..group("chart", vec![])
        }]);
        document.bindings.insert(
            "chart".into(),
            (0..13)
                .map(|index| binding(&format!("b{index}"), "gpu.load"))
                .collect(),
        );
        assert_eq!(
            paths(validate_document(&document)),
            vec!["bindings/chart/0"]
        );
    }

    #[test]
    fn refuses_precision_outside_zero_to_six() {
        let mut document = with_nodes(vec![group("cpu", vec![])]);
        document.bindings.insert(
            "cpu".into(),
            vec![Binding {
                precision: Some(7),
                ..binding("load", "cpu.load")
            }],
        );
        assert_eq!(
            paths(validate_document(&document)),
            vec!["bindings/cpu/0/precision"]
        );
    }

    #[test]
    fn refuses_text_value_run_without_binding() {
        let mut document = with_nodes(vec![Node {
            kind: NodeKind::Text {
                content: TextContent {
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
                },
            },
            ..group("label", vec![])
        }]);
        document.type_tokens.insert(
            "body".into(),
            TypeToken {
                family: "system-ui".into(),
                weight: None,
                face: None,
            },
        );
        document.bindings.insert("label".into(), vec![]);
        assert_eq!(
            paths(validate_document(&document)),
            vec!["nodes/0/text/runs/0/bindingId"]
        );
    }

    #[test]
    fn refuses_both_background_colors() {
        let mut document = minimal();
        document.background.color = Some(StyleValue::Literal(StyleLiteral {
            value: serde_json::json!("#000000"),
        }));
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(),
            color: Some(StyleValue::Literal(StyleLiteral {
                value: serde_json::json!("#111111"),
            })),
            bounds: BackgroundBounds::Theme,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            flip_x: false,
            flip_y: false,
            fallback_frame: None,
            composition: None,
        });
        document
            .assets
            .push(asset("bg", AssetKind::Image, "assets/bg.png"));
        assert_eq!(
            paths(validate_document(&document)),
            vec!["background/color"]
        );
    }

    #[test]
    fn refuses_composition_on_simple_background() {
        let mut document = minimal();
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(),
            color: None,
            bounds: BackgroundBounds::Theme,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            flip_x: false,
            flip_y: false,
            fallback_frame: None,
            composition: Some(BackgroundComposition {
                anchor: BackgroundAnchor::Top,
                extent: Some(0.5),
                fade: None,
            }),
        });
        document
            .assets
            .push(asset("bg", AssetKind::Image, "assets/bg.png"));
        assert_eq!(
            paths(validate_document(&document)),
            vec!["background/media/composition"]
        );
    }

    #[test]
    fn accepts_advanced_composition_with_defaults() {
        let mut document = minimal();
        document.background.mode = BackgroundMode::Advanced;
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(),
            color: None,
            bounds: BackgroundBounds::Theme,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            flip_x: false,
            flip_y: false,
            fallback_frame: None,
            composition: Some(BackgroundComposition {
                anchor: BackgroundAnchor::Center,
                extent: None,
                fade: None,
            }),
        });
        document
            .assets
            .push(asset("bg", AssetKind::Image, "assets/bg.png"));
        assert!(validate_document(&document).is_ok());
    }

    #[test]
    fn refuses_composition_ranges() {
        for (composition, expected) in [
            (
                BackgroundComposition {
                    anchor: BackgroundAnchor::Top,
                    extent: Some(0.09),
                    fade: None,
                },
                "background/media/composition/extent",
            ),
            (
                BackgroundComposition {
                    anchor: BackgroundAnchor::Top,
                    extent: Some(1.01),
                    fade: None,
                },
                "background/media/composition/extent",
            ),
            (
                BackgroundComposition {
                    anchor: BackgroundAnchor::Top,
                    extent: None,
                    fade: Some(0.009),
                },
                "background/media/composition/fade",
            ),
        ] {
            let mut document = minimal();
            document.background.mode = BackgroundMode::Advanced;
            document.background.media = Some(BackgroundMedia {
                asset_id: "bg".into(),
                color: None,
                bounds: BackgroundBounds::Theme,
                offset_x: 0.0,
                offset_y: 0.0,
                scale: 1.0,
                flip_x: false,
                flip_y: false,
                fallback_frame: None,
                composition: Some(composition),
            });
            document
                .assets
                .push(asset("bg", AssetKind::Image, "assets/bg.png"));
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
                asset_id: "bg".into(),
                color: None,
                bounds: BackgroundBounds::Theme,
                offset_x,
                offset_y,
                scale,
                flip_x: false,
                flip_y: false,
                fallback_frame: None,
                composition: None,
            });
            document
                .assets
                .push(asset("bg", AssetKind::Image, "assets/bg.png"));
            assert_eq!(paths(validate_document(&document)), vec![expected]);
        }
    }

    #[test]
    fn refuses_unresolved_style_refs() {
        let mut document = minimal();
        document.palette.insert(
            "accent".into(),
            crate::document::PaletteEntry {
                name: "Accent".into(),
                value: Paint::Solid {
                    color: "#f00".into(),
                },
            },
        );
        document.nodes.push(Node {
            style: StyleMap::from([(
                "fill".into(),
                StyleValue::Ref(StyleRef {
                    r#ref: "palette.missing".into(),
                }),
            )]),
            ..group("panel", vec![])
        });
        assert_eq!(
            paths(validate_document(&document)),
            vec!["nodes/0/style/fill/ref"]
        );

        document.nodes[0].style.insert(
            "fill".into(),
            StyleValue::Ref(StyleRef {
                r#ref: "typeTokens.body".into(),
            }),
        );
        assert_eq!(
            paths(validate_document(&document)),
            vec!["nodes/0/style/fill/ref"]
        );
    }

    #[test]
    fn refuses_asset_paths_outside_assets() {
        for path in [
            "img/a.png",
            "assets/../a.png",
            "assets//a.png",
            "assets/./a.png",
            "assets/",
        ] {
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
        document
            .assets
            .push(asset("a", AssetKind::Video, "assets/a.png"));
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
        ]
        .into_iter()
        .enumerate()
        {
            let mut entry = asset(&format!("a{index}"), kind, path);
            if kind == AssetKind::Font {
                entry.family = Some("Inter".into());
                entry.weight = Some(crate::document::TypeWeight::Name("regular".into()));
                entry.style = Some(crate::document::FontStyle::Normal);
                entry.format = Some(crate::document::FontFormat::Woff2);
            }
            document.assets.push(entry);
        }
        assert!(validate_document(&document).is_ok());
    }

    #[test]
    fn refuses_a_font_asset_without_a_face() {
        let mut document = minimal();
        document
            .assets
            .push(asset("face", AssetKind::Font, "assets/a.woff2"));
        assert_eq!(
            paths(validate_document(&document)),
            vec![
                "assets/0/family",
                "assets/0/format",
                "assets/0/style",
                "assets/0/weight"
            ]
        );
    }

    #[test]
    fn refuses_duplicate_and_invalid_asset_ids() {
        let mut document = minimal();
        document
            .assets
            .push(asset("same", AssetKind::Image, "assets/a.png"));
        document
            .assets
            .push(asset("same", AssetKind::Image, "assets/b.png"));
        assert_eq!(paths(validate_document(&document)), vec!["assets/1/id"]);

        let mut document = minimal();
        document
            .assets
            .push(asset("bad id", AssetKind::Image, "assets/a.png"));
        assert_eq!(paths(validate_document(&document)), vec!["assets/0/id"]);
    }

    #[test]
    fn refuses_media_asset_that_is_not_declared() {
        let mut document = minimal();
        document.background.media = Some(BackgroundMedia {
            asset_id: "missing".into(),
            color: None,
            bounds: BackgroundBounds::Theme,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            flip_x: false,
            flip_y: false,
            fallback_frame: None,
            composition: None,
        });
        assert_eq!(
            paths(validate_document(&document)),
            vec!["background/media/assetId"]
        );
    }

    #[test]
    fn refuses_fallback_frame_on_non_video_asset() {
        let mut document = minimal();
        document
            .assets
            .push(asset("bg", AssetKind::Image, "assets/bg.png"));
        document.background.media = Some(BackgroundMedia {
            asset_id: "bg".into(),
            color: None,
            bounds: BackgroundBounds::Theme,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            flip_x: false,
            flip_y: false,
            fallback_frame: Some(0),
            composition: None,
        });
        assert_eq!(
            paths(validate_document(&document)),
            vec!["background/media/fallbackFrame"]
        );
    }

    #[test]
    fn refuses_bad_binding_values() {
        let cases: Vec<(Binding, &str)> = vec![
            (binding("x", "CPU Load"), "bindings/cpu/0/semanticKey"),
            (
                Binding {
                    scale: Some(f64::INFINITY),
                    ..binding("x", "cpu.load")
                },
                "bindings/cpu/0/scale",
            ),
            (
                Binding {
                    unit: Some(String::new()),
                    ..binding("x", "cpu.load")
                },
                "bindings/cpu/0/unit",
            ),
        ];
        for (entry, expected) in cases {
            let mut document = with_nodes(vec![group("cpu", vec![])]);
            document.bindings.insert("cpu".into(), vec![entry]);
            assert_eq!(paths(validate_document(&document)), vec![expected]);
        }
    }

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
}
