//! Radar chart.
//!
//! The frame is drawn whenever the node has bindings: one spoke per binding,
//! `splitNumber` concentric rings, and the binding's semantic key at each
//! spoke. The request carries one reading, so it is drawn on every spoke;
//! per-binding readings are out of scope until the request carries them.

use crate::chart::{
    ChartFill, ChartRequest, LabelStyle, boolean, chart_fill, circle, filled, label, number,
    point_on, position,
};
use crate::text::TextAlign;
use crate::{Color, Op, Paint, PathCommand, PlanIssue, Shape, Stroke};

const MIN_DEFAULT: f32 = 0.0;
const MAX_DEFAULT: f32 = 100.0;
const SPLIT_DEFAULT: f32 = 4.0;
const LINE_DEFAULT: f32 = 2.0;
const MARKER_DEFAULT: f32 = 4.0;
/// ECharts draws a radar inside 75% of the shorter side, leaving room for the
/// names this frame puts at the spoke ends.
const RADIUS_PERCENT: f32 = 0.75;
const LABEL_GAP: f32 = 8.0;
const LABEL_SIZE: f32 = 12.0;

const STROKE_DEFAULT: Color = Color::rgb(0x00, 0xb8, 0xd9);
const AXIS_DEFAULT: Color = Color::rgb(0xbb, 0xbb, 0xbb);
const GRID_DEFAULT: Color = Color::rgb(0xcc, 0xcc, 0xcc);
const LABEL_DEFAULT: Color = Color::rgb(0x6e, 0x70, 0x79);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameShape {
    Polygon,
    Circle,
}

pub fn build(
    request: &ChartRequest<'_>,
    fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    let settings = request.settings;
    let names: Vec<String> = request
        .document
        .bindings
        .get(request.node_id)
        .map(|bindings| {
            bindings
                .iter()
                .map(|binding| binding.semantic_key.clone())
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        return Vec::new();
    }

    let centre = (request.width / 2.0, request.height / 2.0);
    let radius = request.width.min(request.height).max(0.0) / 2.0 * RADIUS_PERCENT;
    if radius <= 0.0 {
        return Vec::new();
    }

    let count = names.len();
    let angles: Vec<f32> = (0..count)
        .map(|index| 90.0 - index as f32 * 360.0 / count as f32)
        .collect();
    let opacity = request.opacity;
    let split = number(settings, "splitNumber", SPLIT_DEFAULT)
        .round()
        .clamp(1.0, 10.0) as u32;
    let shape = match settings.get("shape").and_then(|value| value.as_str()) {
        Some("circle") => FrameShape::Circle,
        _ => FrameShape::Polygon,
    };
    let show_axes = boolean(settings, "showAxes", true);

    let path = |key: &str| format!("nodes/{}/settings/{key}", request.node_id);
    let stroke = match settings.get("stroke") {
        None => Some(ChartFill::Solid(STROKE_DEFAULT).with_opacity(opacity)),
        Some(value) => chart_fill(
            request.document,
            Some(value),
            request.node_id,
            &path("stroke"),
            issues,
        )
        .map(|fill| fill.with_opacity(opacity)),
    };
    let area = chart_fill(
        request.document,
        settings.get("area"),
        request.node_id,
        &path("area"),
        issues,
    )
    .map(|fill| fill.with_opacity(opacity));

    let mut ops = Vec::new();
    if show_axes {
        for ring in 1..=split {
            let ring_radius = radius * ring as f32 / split as f32;
            let commands = frame(shape, centre, ring_radius, &angles);
            if let Some(op) = stroked(commands, true, faded(GRID_DEFAULT, opacity), 1.0) {
                ops.push(op);
            }
        }
        for angle in &angles {
            let edge = point_on(centre, radius, *angle);
            let commands = vec![
                PathCommand::MoveTo(centre.0, centre.1),
                PathCommand::LineTo(edge.0, edge.1),
            ];
            if let Some(op) = stroked(commands, false, faded(AXIS_DEFAULT, opacity), 1.0) {
                ops.push(op);
            }
        }
        let color = faded(LABEL_DEFAULT, opacity);
        for (name, angle) in names.iter().zip(&angles) {
            let at = point_on(centre, radius + LABEL_GAP, *angle);
            let align = if at.0 > centre.0 + 1.0 {
                TextAlign::Left
            } else if at.0 < centre.0 - 1.0 {
                TextAlign::Right
            } else {
                TextAlign::Center
            };
            let style = LabelStyle {
                size: LABEL_SIZE,
                color,
                weight: 400,
                family: None,
                align,
            };
            let anchor = (at.0, at.1 - LABEL_SIZE / 2.0);
            if let Some(op) = label(fonts, name, anchor, &style) {
                ops.push(op);
            }
        }
    }

    let min = number(settings, "min", MIN_DEFAULT);
    let max = number(settings, "max", MAX_DEFAULT);
    let reading = request.reading.filter(|value| value.is_finite());
    // A polygon needs a resolved outline, three spokes and a real scale; a
    // missing reading is never drawn as zero.
    let drawable = reading.is_some() && stroke.is_some() && count >= 3 && max > min;
    if drawable {
        let Some(reading) = reading else { return ops };
        let at = position(reading, min, max);
        let vertices: Vec<(f32, f32)> = angles
            .iter()
            .map(|angle| point_on(centre, radius * at, *angle))
            .collect();

        if let Some(area) = &area {
            ops.extend(filled(polygon(&vertices), ChartFill::Solid(area.at(1.0))));
        }
        if let Some(stroke) = &stroke {
            let color = stroke.at(1.0);
            let width = number(settings, "lineWidth", LINE_DEFAULT).max(0.0);
            if let Some(op) = stroked(polygon(&vertices), true, color, width) {
                ops.push(op);
            }
            if boolean(settings, "showMarkers", true) {
                let marker = number(settings, "markerSize", MARKER_DEFAULT).max(0.0) / 2.0;
                if marker > 0.0 {
                    for vertex in &vertices {
                        if let Some(op) = filled(circle(*vertex, marker), ChartFill::Solid(color)) {
                            ops.push(op);
                        }
                    }
                }
            }
        }
    }

    ops
}

fn frame(shape: FrameShape, centre: (f32, f32), radius: f32, angles: &[f32]) -> Vec<PathCommand> {
    match shape {
        FrameShape::Circle => circle(centre, radius),
        FrameShape::Polygon => {
            let vertices: Vec<(f32, f32)> = angles
                .iter()
                .map(|angle| point_on(centre, radius, *angle))
                .collect();
            polygon(&vertices)
        }
    }
}

fn polygon(vertices: &[(f32, f32)]) -> Vec<PathCommand> {
    if vertices.len() < 2 {
        return Vec::new();
    }
    let mut commands = Vec::with_capacity(vertices.len() + 1);
    commands.push(PathCommand::MoveTo(vertices[0].0, vertices[0].1));
    for (x, y) in &vertices[1..] {
        commands.push(PathCommand::LineTo(*x, *y));
    }
    commands.push(PathCommand::Close);
    commands
}

fn stroked(commands: Vec<PathCommand>, closed: bool, color: Color, width: f32) -> Option<Op> {
    if commands.is_empty() || width <= 0.0 {
        return None;
    }
    Some(Op {
        transform: crate::Affine::IDENTITY,
        shape: Shape::Path { commands, closed },
        fill: None,
        stroke: Some(Stroke {
            paint: Paint::Solid(color),
            width,
        }),
    })
}

fn faded(color: Color, opacity: f32) -> Color {
    color.with_alpha((f32::from(color.a) * opacity.clamp(0.0, 1.0)).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::Shape;
    use yunari_model::document::{NodeKind, ThemeDocument, parse_document};

    fn document(settings: &str) -> ThemeDocument {
        parse_document(&format!(
            r##"{{"schemaVersion":"1.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "palette":{{
                    "good":{{"name":"Good","value":{{"kind":"solid","color":"#00ff00"}}}},
                    "bad":{{"name":"Bad","value":{{"kind":"solid","color":"#ff0000"}}}}
                }},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"radar","settings":{settings}}}}},
                    "transform":{{"width":200,"height":200}}}}],
                "bindings":{{"c":[
                    {{"id":"a","semanticKey":"cpu.load"}},
                    {{"id":"b","semanticKey":"gpu.load"}},
                    {{"id":"c2","semanticKey":"ram.used"}},
                    {{"id":"d","semanticKey":"disk.used"}}
                ]}}}}"##
        ))
        .expect("fixture parses")
    }

    fn build(settings: &str, reading: Option<f64>) -> (Vec<Op>, Vec<PlanIssue>) {
        let document = document(settings);
        let node = &document.nodes[0];
        let NodeKind::Chart { content } = &node.kind else {
            panic!("expected a chart");
        };
        let request = ChartRequest {
            document: &document,
            node_id: &node.id,
            width: 200.0,
            height: 200.0,
            reading,
            opacity: 1.0,
            settings: &content.settings.0,
        };
        let mut issues = Vec::new();
        let ops = super::build(&request, &mut crate::Fonts::new(), &mut issues);
        (ops, issues)
    }

    fn points(op: &Op) -> Vec<(f32, f32)> {
        let Shape::Path { commands, .. } = &op.shape else {
            panic!("expected a path");
        };
        commands
            .iter()
            .filter_map(|command| match *command {
                crate::PathCommand::MoveTo(x, y)
                | crate::PathCommand::LineTo(x, y)
                | crate::PathCommand::CubicTo(_, _, _, _, x, y) => Some((x, y)),
                _ => None,
            })
            .collect()
    }

    fn distance(point: (f32, f32)) -> f32 {
        ((point.0 - 100.0).powi(2) + (point.1 - 100.0).powi(2)).sqrt()
    }

    #[test]
    fn a_radar_without_a_reading_draws_its_frame_only() {
        let (ops, issues) = build("{}", None);
        assert!(issues.is_empty(), "{issues:?}");
        // Four rings, four spokes and four labels.
        assert_eq!(ops.len(), 12);
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op.shape, Shape::Text(_)))
                .count(),
            4
        );
        assert!(ops.iter().all(|op| op.fill.is_none()));
    }

    #[test]
    fn split_number_sets_the_ring_count() {
        let (ops, issues) = build(r#"{"splitNumber":2}"#, None);
        assert!(issues.is_empty(), "{issues:?}");
        // Two rings, four spokes and four labels.
        assert_eq!(ops.len(), 10);
    }

    #[test]
    fn a_reading_adds_a_polygon_and_its_markers() {
        let (ops, issues) = build("{}", Some(50.0));
        assert!(issues.is_empty(), "{issues:?}");
        // The frame, the outline and one marker per spoke.
        assert_eq!(ops.len(), 17);
        let outline = ops
            .iter()
            .find(|op| {
                matches!(op.shape, Shape::Path { closed: true, .. })
                    && op
                        .stroke
                        .as_ref()
                        .is_some_and(|stroke| stroke.width == LINE_DEFAULT)
            })
            .expect("a polygon outline");
        // Half the 75-unit radius.
        for point in points(outline) {
            assert!((37.4..=37.6).contains(&distance(point)), "{point:?}");
        }
        assert_eq!(
            ops.last().expect("a marker").fill,
            Some(Paint::Solid(STROKE_DEFAULT))
        );
    }

    #[test]
    fn hiding_the_axes_leaves_only_the_polygon() {
        let (ops, issues) = build(r#"{"showAxes":false,"showMarkers":false}"#, Some(50.0));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 1);
        assert!(ops[0].stroke.is_some());
    }

    #[test]
    fn a_threshold_stroke_takes_its_top_band() {
        let (ops, issues) = build(
            r#"{"showAxes":false,"showMarkers":false,"stroke":{"kind":"thresholds","bands":[
                {"offset":0.5,"ref":"palette.good"},{"offset":1,"ref":"palette.bad"}]}}"#,
            Some(50.0),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 1);
        let stroke = ops[0].stroke.as_ref().expect("an outline");
        assert_eq!(stroke.paint, Paint::Solid(Color::rgb(0xff, 0x00, 0x00)));
    }

    #[test]
    fn an_unresolved_stroke_draws_no_polygon_and_reports_it() {
        let (ops, issues) = build(
            r#"{"showAxes":false,"stroke":{"ref":"palette.missing"}}"#,
            Some(50.0),
        );
        assert!(ops.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, crate::IssueCode::UnresolvedGlobal);
    }

    #[test]
    fn a_circle_frame_uses_round_rings() {
        let (ops, issues) = build(r#"{"shape":"circle","splitNumber":1}"#, None);
        assert!(issues.is_empty(), "{issues:?}");
        // One ring, four spokes and four labels.
        assert_eq!(ops.len(), 9);
        let ring = &ops[0];
        assert!(matches!(ring.shape, Shape::Path { closed: true, .. }));
        assert!(points(ring).len() > 4, "a circle is four cubics");
    }
}
