//! Gauge: an arc, a track and the reading's progress.

use crate::chart::segments::{
    SegmentFill, segment_count, segment_divisor_of, segment_fill_of,
};
use crate::chart::{
    ChartFill, ChartRequest, boolean, chart_fill, filled, gap_of, number, position, ring,
};
use crate::{Color, Op, PlanIssue};

/// The family's own colours, used only when the document names none: a paint
/// that was authored but cannot be resolved is an absence, not a default.
const TRACK_DEFAULT: Color = Color::rgb(0x2a, 0x2f, 0x3a);
const PROGRESS_DEFAULT: Color = Color::rgb(0x00, 0xb8, 0xd9);

pub fn build(request: &ChartRequest<'_>, issues: &mut Vec<PlanIssue>) -> Vec<Op> {
    let settings = request.settings;
    let start = number(settings, "startAngle", 225.0);
    let end = number(settings, "endAngle", -45.0);
    let min = number(settings, "min", 0.0);
    let max = number(settings, "max", 100.0);
    let thickness = number(settings, "thickness", 18.0);
    let round_cap = boolean(settings, "roundCap", true);
    let centre = (request.width / 2.0, request.height / 2.0);
    let radius = request.width.min(request.height) / 2.0;
    let sweep = end - start;

    let path = |key: &str| format!("nodes/{}/settings/{key}", request.node_id);
    let Some(track) = fill_or_default(
        request,
        settings,
        "track",
        &path("track"),
        TRACK_DEFAULT,
        issues,
    ) else {
        return Vec::new();
    };
    let progress = fill_or_default(
        request,
        settings,
        "progress",
        &path("progress"),
        PROGRESS_DEFAULT,
        issues,
    );

    // One condition for the whole arc: a missing reading shows only the track,
    // and a paint that resolves to nothing is the same absence.
    let reading = request.reading;
    let drawn = reading.is_some() && progress.is_some();
    let at = reading.map_or(0.0, |value| position(value, min, max));

    let count = segment_count(segment_divisor_of(settings));
    if count <= 1 {
        let mut ops = vec![];
        push_solid(
            &mut ops,
            ring(centre, radius, thickness, start, start + sweep, round_cap),
            track.for_mark(0.5, false),
        );
        if drawn {
            let Some(progress) = progress else {
                return ops;
            };
            push_progress(
                &mut ops,
                centre,
                radius,
                thickness,
                start,
                sweep,
                at,
                round_cap,
                progress,
                number(settings, "gradientSegments", 64.0),
            );
        }
        return ops;
    }

    let gap = gap_of(request, sweep, count);
    let band = sweep / count as f32;
    let fill_mode = segment_fill_of(settings, SegmentFill::Whole);
    let mut ops = Vec::new();
    for index in 0..count {
        let band_from = start + index as f32 * band;
        let band_to = start + (index as f32 + 1.0) * band - gap;
        let band_start = index as f32 / count as f32;
        let band_end = (index as f32 + 1.0) / count as f32 - gap / sweep.abs().max(f32::EPSILON);
        let midpoint = (band_start + band_end) / 2.0;

        if fill_mode == SegmentFill::Partial && drawn {
            let Some(progress) = &progress else {
                continue;
            };
            if at <= band_start {
                push_solid(&mut ops, ring(centre, radius, thickness, band_from, band_to, round_cap), track.for_mark(band_start, false));
            } else if at >= band_end {
                push_solid(&mut ops, ring(centre, radius, thickness, band_from, band_to, round_cap), progress.for_mark(midpoint, true));
            } else {
                let inside = start + at * sweep;
                push_solid(&mut ops, ring(centre, radius, thickness, band_from, inside, round_cap), progress.for_mark(at, true));
                push_solid(&mut ops, ring(centre, radius, thickness, inside, band_to, round_cap), track.for_mark(midpoint, false));
            }
        } else {
            let reached = drawn && band_end <= at;
            let paint = if reached {
                progress.as_ref().map_or(track.for_mark(midpoint, false), |progress| {
                    progress.for_mark(midpoint, true)
                })
            } else {
                track.for_mark(midpoint, false)
            };
            push_solid(&mut ops, ring(centre, radius, thickness, band_from, band_to, round_cap), paint);
        }
    }
    ops
}

/// A paint the document authors, or the family's own when it authors none.
fn fill_or_default(
    request: &ChartRequest<'_>,
    settings: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    path: &str,
    default: Color,
    issues: &mut Vec<PlanIssue>,
) -> Option<ChartFill> {
    match chart_fill(request.document, settings.get(key), request.node_id, path, issues) {
        Some(fill) => Some(fill.with_opacity(request.opacity)),
        None if settings.get(key).is_none() => {
            Some(ChartFill::Solid(default).with_opacity(request.opacity))
        }
        None => None,
    }
}

fn push_solid(ops: &mut Vec<Op>, commands: Vec<crate::PathCommand>, color: Color) {
    if let Some(op) = filled(commands, ChartFill::Solid(color)) {
        ops.push(op);
    }
}

/// The reading's progress arc. A gradient is drawn as bounded flat slices, so
/// the ring follows the gradient rather than repeating one colour.
#[allow(clippy::too_many_arguments)]
fn push_progress(
    ops: &mut Vec<Op>,
    centre: (f32, f32),
    radius: f32,
    thickness: f32,
    start: f32,
    sweep: f32,
    at: f32,
    round_cap: bool,
    progress: ChartFill,
    gradient_segments: f32,
) {
    let end = start + sweep * at;
    match &progress {
        ChartFill::Gradient(_) => {
            let count = gradient_segments.clamp(2.0, 256.0) as usize;
            for index in 0..count {
                let from = index as f32 / count as f32;
                let to = (index as f32 + 1.0) / count as f32;
                if to > at {
                    break;
                }
                let midpoint = (from + to) / 2.0;
                push_solid(
                    ops,
                    ring(
                        centre,
                        radius,
                        thickness,
                        start + sweep * from,
                        start + sweep * to,
                        false,
                    ),
                    progress.at(midpoint),
                );
            }
        }
        _ => push_solid(
            ops,
            ring(centre, radius, thickness, start, end, round_cap),
            progress.at(at),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Affine;
    use crate::display_list::Shape;
    use yunari_model::document::parse_document;

    fn document(settings: &str) -> yunari_model::document::ThemeDocument {
        parse_document(&format!(
            r##"{{"schemaVersion":"1.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"gauge","settings":{settings}}}}},
                "transform":{{"width":200,"height":200}}}}]}}"##
        ))
        .expect("fixture parses")
    }

    fn ops(settings: &str, reading: Option<f64>) -> Vec<Op> {
        let document = document(settings);
        let node = &document.nodes[0];
        let yunari_model::document::NodeKind::Chart { content } = &node.kind else {
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
        let ops = super::build(&request, &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        ops
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

    #[test]
    fn a_gauge_without_a_reading_draws_only_its_track() {
        let ops = ops(r#"{"thickness": 18}"#, None);
        assert_eq!(ops.len(), 1);
        let painted = points(&ops[0]);
        assert!(painted.len() > 4);
        for (x, y) in painted {
            assert!((-1.0..=201.0).contains(&x) && (-1.0..=201.0).contains(&y), "{x},{y}");
        }
    }

    #[test]
    fn a_reading_adds_a_shorter_progress_arc() {
        let ops = ops(r#"{"thickness": 18}"#, Some(50.0));
        assert_eq!(ops.len(), 2);
        let track = points(&ops[0]).len();
        let progress = points(&ops[1]).len();
        assert!(progress < track, "progress {progress} vs track {track}");
        assert_eq!(ops[1].fill, Some(crate::Paint::Solid(Color::rgb(0x00, 0xb8, 0xd9))));
    }

    #[test]
    fn a_segmented_gauge_draws_one_band_per_slice() {
        let ops = ops(r#"{"thickness": 18, "segmentDivisor": 10}"#, None);
        assert_eq!(ops.len(), 10);
    }

    #[test]
    fn a_reading_lights_the_slices_it_reaches() {
        let ops = ops(r#"{"thickness": 18, "segmentDivisor": 10}"#, Some(50.0));
        let track = Color::rgb(0x2a, 0x2f, 0x3a);
        let progress = Color::rgb(0x00, 0xb8, 0xd9);
        let colours: Vec<_> = ops
            .iter()
            .map(|op| op.fill.clone().expect("a band is filled"))
            .collect();
        assert_eq!(colours[0], crate::Paint::Solid(progress));
        assert_eq!(colours[4], crate::Paint::Solid(progress));
        assert_eq!(colours[9], crate::Paint::Solid(track));
    }

    #[test]
    fn the_arc_keeps_its_centre_and_thickness() {
        let ops = ops(r#"{"thickness": 20, "startAngle": 90, "endAngle": 0}"#, None);
        let painted = points(&ops[0]);
        // A quarter arc from straight up to the right: every point sits on the
        // band between radius 90 and 110 around the box centre.
        for (x, y) in &painted {
            let distance = ((x - 100.0).powi(2) + (y - 100.0).powi(2)).sqrt();
            assert!((89.0..=111.0).contains(&distance), "{x},{y}");
        }
        assert!(painted.iter().any(|(_, y)| *y < 1.0), "the arc starts at the top");
        assert!(painted.iter().any(|(x, _)| *x > 209.0), "and ends at the right");
    }

    #[test]
    fn a_ring_is_a_closed_path_around_its_centre() {
        let op = filled(
            ring((10.0, 10.0), 8.0, 2.0, 0.0, 180.0, true),
            ChartFill::Solid(Color::rgb(1, 2, 3)),
        )
        .expect("a ring is drawable");
        assert_eq!(op.transform, Affine::IDENTITY);
        assert!(matches!(op.shape, Shape::Path { closed: true, .. }));
    }
}
