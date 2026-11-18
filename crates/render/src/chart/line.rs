//! Line chart: cartesian axes, grid and each binding's series mark.
//!
//! The request carries one figure per binding. A series with history is drawn
//! as a polyline with the area beneath its first series; a lone reading is a
//! point, and is only drawn when the family's `showMarkers` asks for it. Axis
//! furniture is drawn even when every reading is absent; nothing is made from a
//! zero that was not measured.
//!
//! Upstream hides axis tick labels (`axisLabel.show: false`), so this port
//! draws the axes and split lines without text.

use crate::chart::{ChartFill, ChartRequest, boolean, chart_fill, circle, number, position};
use crate::{Affine, Color, GradientStop, Op, Paint, PathCommand, PlanIssue, Shape, Stroke};

/// Copied from `defaultLineSettings.stroke`.
const STROKE_DEFAULT: Color = Color::rgb(0x00, 0xb8, 0xd9);
/// ECharts' default axis line colour, which upstream never overrides; the
/// product initialises ECharts with no theme.
const AXIS_COLOR: Color = Color::rgb(0x6e, 0x70, 0x79);
const SPLIT_COLOR: Color = Color::rgb(0xe0, 0xe6, 0xf1);
/// Upstream's grid inset, and its always-on top inset.
const GRID_INSET: f32 = 8.0;
/// ECharts' default value-axis `splitNumber`.
const Y_SPLITS: usize = 5;
/// ECharts' default line width.
const LINE_WIDTH_DEFAULT: f32 = 2.0;

/// The plot rectangle inside the node's own box.
#[derive(Debug, Clone, Copy)]
struct Plot {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

pub fn build(
    request: &ChartRequest<'_>,
    _fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    let settings = request.settings;
    let show_axes = boolean(settings, "showAxes", true);
    let window_seconds = finite(settings, "windowSeconds", 60.0).max(1.0);

    // A reading that is not a finite number is an absence, never a zero. The
    // scale spans every sample the window holds, so a line never leaves its
    // own plot.
    let values: Vec<f32> = request
        .readings
        .iter()
        .filter_map(|value| value.filter(|value| value.is_finite()))
        .map(|value| value as f32)
        .chain(
            request
                .series
                .iter()
                .flatten()
                .map(|(_, value)| *value as f32)
                .filter(|value| value.is_finite()),
        )
        .collect();
    let range = value_range(settings, &values);

    let width = request.width.max(0.0);
    let height = request.height.max(0.0);
    let inset = if show_axes { GRID_INSET } else { 0.0 };
    let plot = Plot {
        left: inset.min(width),
        right: (width - inset).max(inset.min(width)),
        top: GRID_INSET.min(height),
        bottom: (height - inset).max(GRID_INSET.min(height)),
    };

    let mut ops = Vec::new();
    if show_axes {
        let axis = alpha(AXIS_COLOR, request.opacity);
        let split = alpha(SPLIT_COLOR, request.opacity);
        for index in 1..=Y_SPLITS {
            let at = index as f32 / Y_SPLITS as f32;
            let y = plot.bottom - at * (plot.bottom - plot.top);
            push_stroke(
                &mut ops,
                segment((plot.left, y), (plot.right, y)),
                Paint::Solid(split),
                1.0,
            );
        }
        push_stroke(
            &mut ops,
            segment((plot.left, plot.bottom), (plot.right, plot.bottom)),
            Paint::Solid(axis),
            1.0,
        );
        push_stroke(
            &mut ops,
            segment((plot.left, plot.top), (plot.left, plot.bottom)),
            Paint::Solid(axis),
            1.0,
        );
    }

    // `showSymbol` upstream follows this flag, and the family's own default is
    // false. The area is the family's own gradient unless one is authored, and
    // only the first series gets one (stacked areas hide the crossings).
    let show_markers = boolean(settings, "showMarkers", false);
    let area = match settings.get("area") {
        Some(value) => chart_fill(
            request.document,
            Some(value),
            request.node_id,
            &format!("nodes/{}/settings/area", request.node_id),
            issues,
        )
        .map(|fill| fill.with_opacity(request.opacity)),
        None => Some(default_area(request.opacity)),
    };

    if let Some((low, high)) = range {
        for (index, reading) in request.readings.iter().enumerate() {
            let Some(value) = reading
                .filter(|value| value.is_finite())
                .map(|value| value as f32)
            else {
                continue;
            };
            let Some(stroke) = series_fill(request, settings, index, issues) else {
                continue;
            };
            // A series with two or more samples is a line across the window; the
            // newest sample sits at the right edge, and its age places the rest.
            if let Some(samples) = request
                .series
                .get(index)
                .filter(|samples| samples.len() > 1)
            {
                let newest = samples.iter().map(|(at, _)| *at).max().unwrap_or(0);
                let width = plot.right - plot.left;
                let mut vertices = Vec::with_capacity(samples.len());
                for (at, value) in samples {
                    let age = newest.saturating_sub(*at) as f64 / 1000.0;
                    let fraction = (age / f64::from(window_seconds)).clamp(0.0, 1.0) as f32;
                    let x = plot.right - fraction * width;
                    let y = plot.bottom - position(*value, low, high) * (plot.bottom - plot.top);
                    vertices.push((x, y));
                }
                if index == 0
                    && let Some(fill) = &area
                {
                    push_area(&mut ops, &vertices, plot, fill);
                }
                let commands = vertices
                    .iter()
                    .enumerate()
                    .map(|(step, (x, y))| {
                        if step == 0 {
                            PathCommand::MoveTo(*x, *y)
                        } else {
                            PathCommand::LineTo(*x, *y)
                        }
                    })
                    .collect();
                let line_width = finite(settings, "lineWidth", LINE_WIDTH_DEFAULT).max(0.0);
                push_stroke(
                    &mut ops,
                    commands,
                    Paint::Solid(stroke.for_mark(1.0, true)),
                    line_width,
                );
            }
            if show_markers {
                let at = position(f64::from(value), low, high);
                let point = (plot.right, plot.bottom - at * (plot.bottom - plot.top));
                let size = clamp_length(finite(settings, "markerSize", 4.0), plot);
                push_symbol(&mut ops, point, size, stroke.for_mark(1.0, true));
            }
        }
    }

    ops
}

/// The area under a series, closed on the axis. A gradient travels down the
/// area's own extent, as ECharts reads `{x: 0, y: 0, x2: 0, y2: 1}`.
fn push_area(ops: &mut Vec<Op>, vertices: &[(f32, f32)], plot: Plot, fill: &ChartFill) {
    let (Some(first), Some(last)) = (vertices.first(), vertices.last()) else {
        return;
    };
    let bottom = plot.bottom;
    let mut commands: Vec<PathCommand> = vertices
        .iter()
        .enumerate()
        .map(|(step, (x, y))| {
            if step == 0 {
                PathCommand::MoveTo(*x, *y)
            } else {
                PathCommand::LineTo(*x, *y)
            }
        })
        .collect();
    commands.push(PathCommand::LineTo(last.0, bottom));
    commands.push(PathCommand::LineTo(first.0, bottom));
    let paint = match fill {
        ChartFill::Gradient(stops) => {
            let top = vertices
                .iter()
                .map(|(_, y)| *y)
                .fold(f32::INFINITY, f32::min);
            Paint::Linear {
                from: (first.0, top),
                to: (first.0, bottom),
                stops: stops
                    .iter()
                    .map(|(offset, color)| GradientStop {
                        offset: *offset,
                        color: *color,
                    })
                    .collect(),
            }
        }
        // A threshold area resolves to its top band, as `toEngineColor` does.
        fill => Paint::Solid(fill.at(1.0)),
    };
    push_filled(ops, commands, paint);
}

/// `defaultLineSettings.area`: the family's own #00b8d9 -> transparent fill.
fn default_area(opacity: f32) -> ChartFill {
    ChartFill::Gradient(vec![
        (0.0, STROKE_DEFAULT),
        (1.0, STROKE_DEFAULT.with_alpha(0)),
    ])
    .with_opacity(opacity)
}

/// One series' paint: the palette entry at its own index when present, else
/// `stroke`. An authored paint that cannot resolve is an absence, not a
/// substituted colour.
fn series_fill(
    request: &ChartRequest<'_>,
    settings: &serde_json::Map<String, serde_json::Value>,
    index: usize,
    issues: &mut Vec<PlanIssue>,
) -> Option<ChartFill> {
    let palette_entry = settings
        .get("palette")
        .and_then(|value| value.as_array())
        .filter(|entries| !entries.is_empty())
        .map(|entries| (entries, index % entries.len()));
    let (value, key) = match palette_entry {
        Some((entries, at)) => (Some(&entries[at]), format!("palette.{at}")),
        None => (settings.get("stroke"), "stroke".to_owned()),
    };
    let path = format!("nodes/{}/settings/{key}", request.node_id);
    match value {
        Some(value) => chart_fill(
            request.document,
            Some(value),
            request.node_id,
            &path,
            issues,
        )
        .map(|fill| fill.with_opacity(request.opacity)),
        None => Some(ChartFill::Solid(STROKE_DEFAULT).with_opacity(request.opacity)),
    }
}

/// A reading chooses the scale when the document names no bounds; a missing
/// reading leaves no honest extent. Authored bounds win, and a degenerate one
/// grows rather than divides by zero.
fn value_range(
    settings: &serde_json::Map<String, serde_json::Value>,
    values: &[f32],
) -> Option<(f32, f32)> {
    let min = optional(settings, "min");
    let max = optional(settings, "max");
    match (min, max) {
        (Some(low), Some(high)) => (high > low).then_some((low, high)),
        _ => {
            let first = *values.first()?;
            let lowest = values.iter().copied().fold(first, f32::min);
            let highest = values.iter().copied().fold(first, f32::max);
            let mut low = min.unwrap_or_else(|| lowest.min(0.0));
            let mut high = max.unwrap_or_else(|| highest.max(0.0));
            if high <= low {
                if min.is_some() {
                    high = low + 1.0;
                } else if max.is_some() {
                    low = high - 1.0;
                } else {
                    low = first - 1.0;
                    high = first + 1.0;
                }
            }
            Some((low, high))
        }
    }
}

fn push_stroke(ops: &mut Vec<Op>, commands: Vec<PathCommand>, paint: Paint, width: f32) {
    if commands.is_empty() || width <= 0.0 {
        return;
    }
    ops.push(Op {
        transform: Affine::IDENTITY,
        shape: Shape::Path {
            commands,
            closed: false,
        },
        fill: None,
        stroke: Some(Stroke { paint, width }),
    });
}

fn push_filled(ops: &mut Vec<Op>, commands: Vec<PathCommand>, paint: Paint) {
    if commands.is_empty() {
        return;
    }
    ops.push(Op {
        transform: Affine::IDENTITY,
        shape: Shape::Path {
            commands,
            closed: true,
        },
        fill: Some(paint),
        stroke: None,
    });
}

fn push_symbol(ops: &mut Vec<Op>, at: (f32, f32), size: f32, color: Color) {
    let radius = (size / 2.0).max(0.0);
    if radius > 0.0 {
        push_filled(ops, circle(at, radius), Paint::Solid(color));
    }
}

fn segment(from: (f32, f32), to: (f32, f32)) -> Vec<PathCommand> {
    vec![
        PathCommand::MoveTo(from.0, from.1),
        PathCommand::LineTo(to.0, to.1),
    ]
}

/// A setting's finite number, or the family's own default.
fn finite(settings: &serde_json::Map<String, serde_json::Value>, key: &str, default: f32) -> f32 {
    let value = number(settings, key, default);
    if value.is_finite() { value } else { default }
}

fn optional(settings: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<f32> {
    settings
        .get(key)
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite())
}

/// A width or diameter cannot exceed the plot it is drawn in.
fn clamp_length(value: f32, plot: Plot) -> f32 {
    let bound = (plot.right - plot.left)
        .abs()
        .max((plot.bottom - plot.top).abs());
    value.max(0.0).min(bound)
}

fn alpha(color: Color, opacity: f32) -> Color {
    if opacity >= 1.0 {
        return color;
    }
    let alpha = (f32::from(color.a) * opacity.clamp(0.0, 1.0)).round() as u8;
    color.with_alpha(alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_model::document::parse_document;

    /// A single-figure fixture, read as one figure per binding.
    fn readings_of(
        document: &yunari_model::document::ThemeDocument,
        reading: Option<f64>,
    ) -> Vec<Option<f64>> {
        vec![reading; document.bindings.get("c").map_or(1, Vec::len)]
    }

    fn document(settings: &str) -> yunari_model::document::ThemeDocument {
        parse_document(&format!(
            r##"{{"schemaVersion":"3.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"line","settings":{settings}}}}},
                "transform":{{"width":200,"height":200}}}}]}}"##
        ))
        .expect("fixture parses")
    }

    fn render(
        settings: &str,
        readings: &[Option<f64>],
        series: &[Vec<(u64, f64)>],
    ) -> (Vec<Op>, Vec<PlanIssue>) {
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
            readings,
            series,
            opacity: 1.0,
            settings: &content.settings.0,
        };
        let mut issues = Vec::new();
        let ops = super::build(&request, &mut crate::Fonts::new(), &mut issues);
        (ops, issues)
    }

    fn built(settings: &str, reading: Option<f64>) -> (Vec<Op>, Vec<PlanIssue>) {
        let document = document(settings);
        let node = &document.nodes[0];
        let yunari_model::document::NodeKind::Chart { content } = &node.kind else {
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

    fn ops(settings: &str, reading: Option<f64>) -> Vec<Op> {
        let (ops, issues) = built(settings, reading);
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
                PathCommand::MoveTo(x, y)
                | PathCommand::LineTo(x, y)
                | PathCommand::CubicTo(_, _, _, _, x, y) => Some((x, y)),
                _ => None,
            })
            .collect()
    }

    /// The series mark is the last filled op; an area fill, when present, is
    /// painted beneath the line.
    fn marker(ops: &[Op]) -> ((f32, f32), f32) {
        let op = ops
            .iter()
            .rev()
            .find(|op| op.fill.is_some())
            .expect("a reading draws a mark");
        let points = points(op);
        let (min_x, max_x) = bounds(points.iter().map(|(x, _)| *x));
        let (min_y, max_y) = bounds(points.iter().map(|(_, y)| *y));
        (
            ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0),
            (max_x - min_x) / 2.0,
        )
    }

    /// Filled ops drawn as circles: the per-point symbols.
    fn symbols(ops: &[Op]) -> Vec<&Op> {
        ops.iter()
            .filter(|op| op.fill.is_some())
            .filter(|op| {
                matches!(&op.shape,
                    Shape::Path { commands, .. }
                        if commands.iter().any(|command| matches!(command, PathCommand::CubicTo(..)))
                            && !commands.iter().any(|command| matches!(command, PathCommand::LineTo(..))))
            })
            .collect()
    }

    /// Filled ops drawn as polygons: the area under a series.
    fn areas(ops: &[Op]) -> Vec<&Op> {
        ops.iter()
            .filter(|op| op.fill.is_some())
            .filter(|op| {
                matches!(&op.shape,
                    Shape::Path { commands, .. }
                        if commands.iter().any(|command| matches!(command, PathCommand::LineTo(..))))
            })
            .collect()
    }

    fn bounds(values: impl Iterator<Item = f32>) -> (f32, f32) {
        values.fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
            (low.min(value), high.max(value))
        })
    }

    #[test]
    fn every_binding_draws_its_own_series_mark() {
        let document =
            document(r#"{"showAxes": false, "showMarkers": true, "min": 0, "max": 100}"#);
        let node = &document.nodes[0];
        let yunari_model::document::NodeKind::Chart { content } = &node.kind else {
            panic!("expected a chart");
        };
        let readings = [Some(25.0), Some(75.0)];
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
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(ops.len(), 2);
        let centres: Vec<f32> = ops
            .iter()
            .map(|op| marker(std::slice::from_ref(op)).0.1)
            .collect();
        assert!((centres[0] - 152.0).abs() < 0.01, "{centres:?}");
        assert!((centres[1] - 56.0).abs() < 0.01, "{centres:?}");
    }

    #[test]
    fn a_series_draws_a_line_across_its_window() {
        let document =
            document(r#"{"showAxes": false, "showMarkers": true, "min": 0, "max": 100}"#);
        let node = &document.nodes[0];
        let yunari_model::document::NodeKind::Chart { content } = &node.kind else {
            panic!("expected a chart");
        };
        let readings = [Some(75.0)];
        let series = [vec![(0, 25.0), (30_000, 50.0), (60_000, 75.0)]];
        let request = ChartRequest {
            document: &document,
            node_id: &node.id,
            width: 200.0,
            height: 200.0,
            readings: &readings,
            series: &series,
            opacity: 1.0,
            settings: &content.settings.0,
        };
        let mut issues = Vec::new();
        let ops = super::build(&request, &mut crate::Fonts::new(), &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        // One stroke across the window and the newest sample's mark.
        let stroke = ops
            .iter()
            .find(|op| op.stroke.is_some())
            .expect("a series line");
        let points = points(stroke);
        assert_eq!(points.len(), 3, "one vertex per sample: {points:?}");
        assert!((points[0].0 - 0.0).abs() < 0.01, "{points:?}");
        assert!((points[2].0 - 200.0).abs() < 0.01, "{points:?}");
        // Oldest at 25 on the scale, newest at 75.
        assert!(points[0].1 > points[2].1, "{points:?}");
        let ((x, y), _) = marker(&ops);
        assert!(
            (x - 200.0).abs() < 0.01 && (y - 56.0).abs() < 0.01,
            "{x} {y}"
        );
    }

    #[test]
    fn a_missing_reading_draws_axes_and_grid_but_no_series() {
        let ops = ops(r#"{"min": 0, "max": 100}"#, None);
        assert_eq!(
            ops.iter().filter(|op| op.stroke.is_some()).count(),
            2 + Y_SPLITS
        );
        assert!(
            ops.iter().all(|op| op.fill.is_none()),
            "nothing is fabricated from a missing reading"
        );
    }

    #[test]
    fn a_reading_sits_at_the_right_edge_on_a_pinned_scale() {
        // No axes keeps the plot deterministic: top inset 8, full height below.
        let ops = ops(
            r#"{"showAxes": false, "showMarkers": true, "min": 0, "max": 100}"#,
            Some(50.0),
        );
        assert_eq!(ops.len(), 1);
        let ((x, y), radius) = marker(&ops);
        assert!((x - 200.0).abs() < 0.01, "{x}");
        assert!((y - 104.0).abs() < 0.01, "{y}");
        assert!((radius - 2.0).abs() < 0.01, "{radius}");
        assert_eq!(
            ops[0].fill,
            Some(Paint::Solid(Color::rgb(0x00, 0xb8, 0xd9))),
            "the family's own stroke colour"
        );
    }

    #[test]
    fn an_authored_stroke_paints_the_mark() {
        let ops = ops(
            r##"{"showAxes": false, "showMarkers": true, "stroke": {"kind": "solid", "color": "#ff0000"}}"##,
            Some(25.0),
        );
        assert_eq!(
            ops[0].fill,
            Some(Paint::Solid(Color::rgb(0xff, 0x00, 0x00)))
        );
    }

    #[test]
    fn an_unresolvable_paint_is_an_issue_not_a_colour() {
        let (ops, issues) = built(
            r#"{"showAxes": false, "stroke": {"ref": "palette.missing"}}"#,
            Some(10.0),
        );
        assert!(ops.is_empty(), "no substituted stroke");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, crate::IssueCode::UnresolvedGlobal);
    }

    #[test]
    fn axis_tick_labels_are_hidden() {
        // Upstream sets `axisLabel: { show: false }` on both axes (line.ts:240,
        // line.ts:249), so a line chart draws no tick text at all.
        let ops = ops(r#"{"min": 0, "max": 100}"#, Some(50.0));
        assert!(
            !ops.iter().any(|op| matches!(op.shape, Shape::Text(_))),
            "no x or y tick text"
        );
    }

    #[test]
    fn show_markers_off_draws_no_sample_symbol() {
        // Upstream maps `showSymbol` from `showMarkers` (line.ts:255).
        let settings = r#"{"showAxes": false, "showMarkers": false, "min": 0, "max": 100}"#;
        let readings = [Some(75.0)];
        let series = [vec![(0, 25.0), (30_000, 50.0), (60_000, 75.0)]];
        let (ops, issues) = render(settings, &readings, &series);
        assert!(issues.is_empty(), "{issues:?}");
        assert!(symbols(&ops).is_empty(), "showMarkers is false");
    }

    #[test]
    fn area_fill_paints_under_the_first_series_only() {
        // Upstream attaches `areaStyle` to the first series only and fades an
        // area gradient toward the axis (line.ts:283-291, fill.ts gradientAxis).
        let settings = r##"{"showAxes": false, "min": 0, "max": 100,
            "area": {"kind": "gradient", "stops": [
                {"offset": 0, "color": "#00b8d9"},
                {"offset": 1, "color": "#00b8d900"}]}}"##;
        let readings = [Some(75.0), Some(25.0)];
        let series = [
            vec![(0, 25.0), (30_000, 50.0), (60_000, 75.0)],
            vec![(0, 75.0), (30_000, 50.0), (60_000, 25.0)],
        ];
        let (ops, issues) = render(settings, &readings, &series);
        assert!(issues.is_empty(), "{issues:?}");
        let areas = areas(&ops);
        assert_eq!(areas.len(), 1, "one area for the first series only");
        let Some(Paint::Linear { from, to, stops }) = &areas[0].fill else {
            panic!("an area gradient is a linear paint");
        };
        assert_eq!(from.0, to.0, "vertical gradient: {from:?} {to:?}");
        assert!(to.1 > from.1, "fades toward the axis: {from:?} {to:?}");
        assert!(
            (to.1 - 200.0).abs() < 0.01,
            "reaches the plot bottom: {to:?}"
        );
        assert_eq!(
            stops.first().map(|stop| (stop.offset, stop.color)),
            Some((0.0, Color::rgb(0x00, 0xb8, 0xd9)))
        );
        assert_eq!(stops.last().map(|stop| stop.color.a), Some(0));
    }

    #[test]
    fn the_default_line_fills_the_area_under_its_first_series() {
        // `defaultLineSettings.area` is the same #00b8d9 -> transparent
        // gradient (line.ts:54-60), so an omitted setting still fills.
        let settings = r#"{"showAxes": false, "min": 0, "max": 100}"#;
        let readings = [Some(75.0)];
        let series = [vec![(0, 25.0), (30_000, 50.0), (60_000, 75.0)]];
        let (ops, issues) = render(settings, &readings, &series);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(areas(&ops).len(), 1, "the family's own area gradient");
    }

    #[test]
    fn axes_stay_inside_the_box() {
        let ops = ops(r#"{"showMarkers": true, "min": 0, "max": 100}"#, Some(50.0));
        for op in ops.iter().filter(|op| op.stroke.is_some()) {
            for (x, y) in points(op) {
                assert!((0.0..=200.0).contains(&x), "x {x}");
                assert!((0.0..=200.0).contains(&y), "y {y}");
            }
        }
        let ((x, _), _) = marker(&ops);
        assert!((x - 192.0).abs() < 0.01, "right inset 8: {x}");
    }
}
