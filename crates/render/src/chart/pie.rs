//! Pie chart.
//!
//! One binding is one slice, with a fixed total leaving a remainder slice. A
//! binding nothing read is absent rather than a zero share.
//!
//! Upstream draws no engine text for a pie and offers no setting to turn it on
//! (`label: { show: false }` is unconditional), so no labels are drawn here.

use crate::chart::segments::{segment_count, segment_divisor_of, segment_spacing_percent_of};
use crate::chart::{ChartFill, ChartRequest, arc, chart_fill, filled, number, point_on, ring};
use crate::{Color, Op, PathCommand, PlanIssue};

const INNER_DEFAULT: f32 = 60.0;
const OUTER_DEFAULT: f32 = 100.0;
const START_DEFAULT: f32 = 90.0;
const PAD_DEFAULT: f32 = 2.0;
const PALETTE_FALLBACK: Color = Color::rgb(0x89, 0x93, 0xa4);
const REMAINDER_DEFAULT: Color = Color::rgb(0x2a, 0x2f, 0x3a);

/// The family's own slice colours, used only when the document names none.
const PALETTE_DEFAULT: [Color; 5] = [
    Color::rgb(0x00, 0xb8, 0xd9),
    Color::rgb(0x65, 0x54, 0xc0),
    Color::rgb(0x36, 0xb3, 0x7e),
    Color::rgb(0xff, 0xab, 0x00),
    Color::rgb(0xff, 0x56, 0x30),
];

/// One drawn piece of the composition and the share its paint resolves at.
struct Slice {
    /// The share of the whole this slice stands for, 0–1.
    share: f32,
    /// Where its paint resolves: a slice at its share, a remainder at the top.
    paint_at: f32,
    fill: Option<ChartFill>,
}

pub fn build(
    request: &ChartRequest<'_>,
    _fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    let settings = request.settings;

    // One binding is one slice. A negative or non-finite part has no angle;
    // upstream treats it as missing rather than fabricating a zero.
    let values: Vec<f32> = request
        .readings
        .iter()
        .filter_map(|value| value.filter(|value| value.is_finite() && *value >= 0.0))
        .map(|value| value as f32)
        .collect();
    if values.is_empty() {
        return Vec::new();
    }
    let sum: f32 = values.iter().sum();

    let start = number(settings, "startAngle", START_DEFAULT);
    let end = settings
        .get("endAngle")
        .and_then(|value| value.as_f64())
        .map(|value| value as f32);
    // ECharts runs clockwise from `startAngle`. An absent endAngle is a full
    // turn; an explicit one is clamped into one turn the way ECharts wraps it.
    let sweep = match end {
        Some(end) => {
            let raw = start - end;
            let clockwise = if raw >= 360.0 {
                360.0
            } else {
                raw.rem_euclid(360.0)
            };
            -clockwise
        }
        None => -360.0,
    };
    let span = sweep.abs();
    if span < 0.01 {
        return Vec::new();
    }

    let centre = (request.width / 2.0, request.height / 2.0);
    let half = request.width.min(request.height).max(0.0) / 2.0;
    let outer =
        half * number(settings, "outerRadiusPercent", OUTER_DEFAULT).clamp(0.0, 100.0) / 100.0;
    let inner = (half * number(settings, "innerRadiusPercent", INNER_DEFAULT).clamp(0.0, 100.0)
        / 100.0)
        .min(outer);
    if outer <= 0.0 {
        return Vec::new();
    }

    let fixed = fixed_total(settings);
    let overflow = fixed.is_some_and(|total| sum > total);
    let whole = match fixed {
        Some(total) if !overflow => total,
        _ => sum,
    };
    let remainder = match fixed {
        Some(total) if !overflow => Some(total - sum),
        _ => None,
    };

    let mut items = Vec::new();
    if whole > 0.0 {
        for (index, value) in values.iter().enumerate() {
            let share = value / whole;
            if share > 0.0 {
                items.push(Slice {
                    share,
                    paint_at: share,
                    fill: palette_fill(request, index, issues),
                });
            }
        }
    }
    if let Some(remainder) = remainder.filter(|value| *value > 0.0) {
        items.push(Slice {
            share: remainder / whole,
            // Upstream resolves the remainder paint at the top of its scale.
            paint_at: 1.0,
            fill: remainder_fill(request, issues),
        });
    }
    if items.is_empty() {
        return Vec::new();
    }

    let segments = segment_count(segment_divisor_of(settings)).max(1);
    let pad = number(settings, "padAngle", PAD_DEFAULT).max(0.0);
    let direction = if sweep < 0.0 { -1.0 } else { 1.0 };
    let mut ops = Vec::new();
    let mut cursor = start;

    if segments == 1 {
        // Continuous: one sector per composition part.
        let gap = if items.len() > 1 {
            pad.min(0.9 * span / items.len() as f32)
        } else {
            0.0
        };
        let available = (span - gap * items.len() as f32).max(0.0);
        for item in &items {
            let piece = available * item.share;
            if piece > 0.01 {
                push_sector(
                    &mut ops,
                    centre,
                    inner,
                    outer,
                    cursor,
                    cursor + direction * piece,
                    item,
                );
            }
            cursor += direction * (piece + gap);
        }
        return ops;
    }

    // Segmented: each slice is cut into equal pieces with a gap between them,
    // and the authored pad separates slices. This is upstream's spacer
    // arithmetic with the authored sweep in place of 360°.
    let q = segment_spacing_percent_of(settings).max(0.0) / 100.0;
    let count = segments as f32;
    let boundary = (items.len() as f32 * pad / span).min(0.9) * span / items.len() as f32;
    let denominator = count + (count - 1.0) * q;
    if denominator <= 0.0 {
        return ops;
    }
    let unit = (span - items.len() as f32 * boundary) / denominator;
    for item in &items {
        let piece = unit * item.share;
        let gap = q * piece;
        for index in 0..segments {
            let to = cursor + direction * piece;
            if piece > 0.01 {
                push_sector(&mut ops, centre, inner, outer, cursor, to, item);
            }
            cursor = to;
            if index + 1 < segments {
                cursor += direction * gap;
            }
        }
        cursor += direction * boundary;
    }
    ops
}

fn push_sector(
    ops: &mut Vec<Op>,
    centre: (f32, f32),
    inner: f32,
    outer: f32,
    from: f32,
    to: f32,
    item: &Slice,
) {
    let Some(fill) = item.fill.as_ref() else {
        return;
    };
    let commands = sector(centre, inner, outer, from, to);
    if let Some(op) = filled(commands, ChartFill::Solid(fill.at(item.paint_at))) {
        ops.push(op);
    }
}

/// An annulus sector, or a pie wedge when the inner radius is zero.
fn sector(centre: (f32, f32), inner: f32, outer: f32, from: f32, to: f32) -> Vec<PathCommand> {
    if (to - from).abs() < 0.01 || outer <= 0.0 || inner >= outer {
        return Vec::new();
    }
    if inner <= 0.01 {
        let mut commands = vec![PathCommand::MoveTo(centre.0, centre.1)];
        let start = point_on(centre, outer, from);
        commands.push(PathCommand::LineTo(start.0, start.1));
        arc(&mut commands, centre, outer, from, to);
        commands.push(PathCommand::LineTo(centre.0, centre.1));
        commands.push(PathCommand::Close);
        return commands;
    }
    ring(
        centre,
        (inner + outer) / 2.0,
        outer - inner,
        from,
        to,
        false,
    )
}

fn fixed_total(settings: &serde_json::Map<String, serde_json::Value>) -> Option<f32> {
    let total = settings.get("total")?;
    if total.get("kind").and_then(|kind| kind.as_str()) != Some("fixed") {
        return None;
    }
    Some(
        total
            .get("value")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
            .max(0.0) as f32,
    )
}

/// The first slice paint, cycled. An authored entry that cannot resolve is an
/// absence; only a document that names no palette gets the family's colours.
fn palette_fill(
    request: &ChartRequest<'_>,
    index: usize,
    issues: &mut Vec<PlanIssue>,
) -> Option<ChartFill> {
    match request.settings.get("palette") {
        None => Some(solid(
            PALETTE_DEFAULT[index % PALETTE_DEFAULT.len()],
            request.opacity,
        )),
        Some(serde_json::Value::Array(items)) if items.is_empty() => {
            Some(solid(PALETTE_FALLBACK, request.opacity))
        }
        Some(serde_json::Value::Array(items)) => {
            let path = format!("nodes/{}/settings/palette/{index}", request.node_id);
            chart_fill(
                request.document,
                Some(&items[index % items.len()]),
                request.node_id,
                &path,
                issues,
            )
            .map(|fill| fill.with_opacity(request.opacity))
        }
        Some(_) => Some(solid(
            PALETTE_DEFAULT[index % PALETTE_DEFAULT.len()],
            request.opacity,
        )),
    }
}

fn remainder_fill(request: &ChartRequest<'_>, issues: &mut Vec<PlanIssue>) -> Option<ChartFill> {
    let value = request.settings.get("remainderFill");
    let path = format!("nodes/{}/settings/remainderFill", request.node_id);
    match chart_fill(request.document, value, request.node_id, &path, issues) {
        Some(fill) => Some(fill.with_opacity(request.opacity)),
        None if value.is_none() => Some(solid(REMAINDER_DEFAULT, request.opacity)),
        None => None,
    }
}

fn solid(color: Color, opacity: f32) -> ChartFill {
    ChartFill::Solid(color).with_opacity(opacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::Shape;
    use yunari_model::document::{NodeKind, ThemeDocument, parse_document};

    /// A single-figure fixture, read as one figure per binding.
    fn readings_of(
        document: &yunari_model::document::ThemeDocument,
        reading: Option<f64>,
    ) -> Vec<Option<f64>> {
        vec![reading; document.bindings.get("c").map_or(1, Vec::len)]
    }

    fn document(settings: &str) -> ThemeDocument {
        parse_document(&format!(
            r##"{{"schemaVersion":"3.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"pie","settings":{settings}}}}},
                "transform":{{"width":200,"height":200}}}}]}}"##
        ))
        .expect("fixture parses")
    }

    fn build(settings: &str, reading: Option<f64>) -> (Vec<Op>, Vec<PlanIssue>) {
        let document = document(settings);
        let node = &document.nodes[0];
        let NodeKind::Chart { content } = &node.kind else {
            panic!("expected a chart");
        };
        let readings = readings_of(&document, reading);
        let request = ChartRequest {
            document: &document,
            node_id: &node.id,
            width: 200.0,
            height: 200.0,
            readings: &readings,
            series: &[],
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
    fn a_pie_without_a_reading_draws_nothing() {
        let (ops, issues) = build("{}", None);
        assert!(ops.is_empty());
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn a_donut_spans_the_authored_radii() {
        let (ops, issues) = build("{}", Some(50.0));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 1);
        let painted = points(&ops[0]);
        for point in &painted {
            assert!((59.0..=101.0).contains(&distance(*point)), "{point:?}");
        }
        assert!(
            painted.iter().any(|point| point.1 < 1.0),
            "starts at the top"
        );
    }

    #[test]
    fn a_pie_without_an_inner_radius_reaches_the_centre() {
        let (ops, issues) = build(r#"{"innerRadiusPercent":0}"#, Some(50.0));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 1);
        assert!(points(&ops[0]).iter().any(|point| distance(*point) < 0.01));
    }

    #[test]
    fn a_fixed_total_draws_a_remainder_in_its_own_paint() {
        let (ops, issues) = build(
            r#"{"total":{"kind":"fixed","value":200},"padAngle":0}"#,
            Some(50.0),
        );
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 2);
        assert_eq!(
            ops[0].fill,
            Some(crate::Paint::Solid(Color::rgb(0x00, 0xb8, 0xd9)))
        );
        assert_eq!(ops[1].fill, Some(crate::Paint::Solid(REMAINDER_DEFAULT)));
        // A quarter turn from 12 o'clock: the slice stays in the top-right.
        for point in points(&ops[0]) {
            assert!(point.0 >= 99.9 && point.1 <= 100.1, "{point:?}");
        }
    }

    #[test]
    fn a_segment_divisor_cuts_one_slice_into_wedges() {
        let (ops, issues) = build(r#"{"segmentDivisor":10}"#, Some(50.0));
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 10);
    }

    #[test]
    fn an_unresolved_slice_paint_is_an_absence_not_a_colour() {
        let (ops, issues) = build(r#"{"palette":[{"ref":"palette.missing"}]}"#, Some(1.0));
        assert!(ops.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, crate::IssueCode::UnresolvedGlobal);
    }
}
