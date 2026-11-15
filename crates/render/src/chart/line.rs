//! Line chart: cartesian axes, grid and the reading's mark.
//!
//! The request carries one reading, so the series has one point: it is drawn
//! as a symbol, because a polyline through one point has no length. Axis
//! furniture is drawn even when the reading is absent; nothing is made from a
//! zero that was not measured.
//!
//! Upstream hides axis tick labels (`axisLabel.show: false`); this port draws
//! them from the family's defaults and reserves their margin so they stay
//! inside the node box.

use crate::chart::{
    ChartFill, ChartRequest, LabelStyle, boolean, chart_fill, circle, label, number, position,
    text_width,
};
use crate::text::TextAlign;
use crate::{Affine, Color, Op, Paint, PathCommand, PlanIssue, Shape, Stroke};

/// Copied from `defaultLineSettings.stroke`.
const STROKE_DEFAULT: Color = Color::rgb(0x00, 0xb8, 0xd9);
/// ECharts' default axis line and axis label colours, which upstream never
/// overrides; the product initialises ECharts with no theme.
const AXIS_COLOR: Color = Color::rgb(0x6e, 0x70, 0x79);
const SPLIT_COLOR: Color = Color::rgb(0xe0, 0xe6, 0xf1);
const LABEL_COLOR: Color = Color::rgb(0x6e, 0x70, 0x79);
const LABEL_SIZE: f32 = 12.0;
/// Upstream's grid inset, and its always-on top inset.
const GRID_INSET: f32 = 8.0;
const LABEL_GAP: f32 = 4.0;
/// ECharts' default value-axis `splitNumber`.
const Y_SPLITS: usize = 5;
const X_SPLITS: usize = 4;

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
    fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    let settings = request.settings;
    let show_axes = boolean(settings, "showAxes", true);
    let window_seconds = finite(settings, "windowSeconds", 60.0).max(1.0);

    // A reading that is not a finite number is an absence, never a zero.
    let reading = request
        .reading
        .filter(|value| value.is_finite())
        .map(|value| value as f32);
    let range = value_range(settings, reading);

    // Y tick texts first: the widest decides how far the plot moves right, so
    // the labels stay inside the node.
    let mut y_ticks = Vec::new();
    if let Some((low, high)) = range {
        for index in 0..=Y_SPLITS {
            let at = index as f32 / Y_SPLITS as f32;
            let value = low + (high - low) * at;
            y_ticks.push((at, crate::format_number(f64::from(value), None, '.')));
        }
    }

    let mut y_style = LabelStyle::new(LABEL_SIZE, alpha(LABEL_COLOR, request.opacity));
    y_style.align = TextAlign::Right;

    let y_band = if show_axes && !y_ticks.is_empty() {
        y_ticks
            .iter()
            .map(|(_, text)| text_width(fonts, text, &y_style))
            .fold(0.0, f32::max)
            + LABEL_GAP
    } else {
        0.0
    };
    let x_band = if show_axes {
        LABEL_GAP + LABEL_SIZE * 1.2
    } else {
        0.0
    };

    let width = request.width.max(0.0);
    let height = request.height.max(0.0);
    let inset = if show_axes { GRID_INSET } else { 0.0 };
    let plot = Plot {
        left: (inset + y_band).min(width),
        right: (width - inset).max((inset + y_band).min(width)),
        top: GRID_INSET.min(height),
        bottom: (height - inset - x_band).max(GRID_INSET.min(height)),
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

        for (at, text) in &y_ticks {
            let y = plot.bottom - at * (plot.bottom - plot.top);
            // The layout anchors a label at its line-box top; half a line
            // above the tick centres it on the grid line like ECharts.
            let centred = (y - LABEL_SIZE * 0.6).max(0.0);
            push_label(
                fonts,
                &mut ops,
                text,
                (plot.left - LABEL_GAP, centred),
                &y_style,
            );
        }

        let mut x_style = y_style.clone();
        x_style.align = TextAlign::Center;
        for index in 0..=X_SPLITS {
            let at = index as f32 / X_SPLITS as f32;
            let x = plot.left + at * (plot.right - plot.left);
            // The time axis ends at the newest sample; without a timestamp the
            // honest label is its distance from that edge.
            let text = if index == X_SPLITS {
                "0s".to_owned()
            } else {
                let ago = (1.0 - at) * window_seconds;
                format!("-{}s", crate::format_number(f64::from(ago), None, '.'))
            };
            push_label(
                fonts,
                &mut ops,
                &text,
                (x, plot.bottom + LABEL_GAP),
                &x_style,
            );
        }
    }

    if let (Some(value), Some((low, high)), Some(stroke)) =
        (reading, range, series_fill(request, settings, issues))
    {
        let at = position(f64::from(value), low, high);
        let point = (plot.right, plot.bottom - at * (plot.bottom - plot.top));
        // A lone point would otherwise be invisible, so it is always marked.
        let size = clamp_length(finite(settings, "markerSize", 4.0), plot);
        push_symbol(&mut ops, point, size, stroke.for_mark(1.0, true));
    }

    ops
}

/// The first series' paint: the palette entry when present, else `stroke`.
///
/// `palette` is a positional per-series list upstream; this single-reading
/// chart only draws series 0. An authored paint that cannot resolve is an
/// absence, not a substituted colour.
fn series_fill(
    request: &ChartRequest<'_>,
    settings: &serde_json::Map<String, serde_json::Value>,
    issues: &mut Vec<PlanIssue>,
) -> Option<ChartFill> {
    let palette_entry = settings
        .get("palette")
        .and_then(|value| value.as_array())
        .and_then(|entries| entries.first());
    let (value, key) = match palette_entry {
        Some(paint) => (Some(paint), "palette.0"),
        None => (settings.get("stroke"), "stroke"),
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
    reading: Option<f32>,
) -> Option<(f32, f32)> {
    let min = optional(settings, "min");
    let max = optional(settings, "max");
    match (min, max) {
        (Some(low), Some(high)) => (high > low).then_some((low, high)),
        _ => {
            let value = reading?;
            let mut low = min.unwrap_or_else(|| value.min(0.0));
            let mut high = max.unwrap_or_else(|| value.max(0.0));
            if high <= low {
                if min.is_some() {
                    high = low + 1.0;
                } else if max.is_some() {
                    low = high - 1.0;
                } else {
                    low = value - 1.0;
                    high = value + 1.0;
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

fn push_label(
    fonts: &mut crate::Fonts,
    ops: &mut Vec<Op>,
    text: &str,
    at: (f32, f32),
    style: &LabelStyle,
) {
    if let Some(op) = label(fonts, text, at, style) {
        ops.push(op);
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

    fn document(settings: &str) -> yunari_model::document::ThemeDocument {
        parse_document(&format!(
            r##"{{"schemaVersion":"1.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"line","settings":{settings}}}}},
                "transform":{{"width":200,"height":200}}}}]}}"##
        ))
        .expect("fixture parses")
    }

    fn built(settings: &str, reading: Option<f64>) -> (Vec<Op>, Vec<PlanIssue>) {
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

    /// The one filled op is the series mark; its bounds give centre and radius.
    fn marker(ops: &[Op]) -> ((f32, f32), f32) {
        let op = ops
            .iter()
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

    fn bounds(values: impl Iterator<Item = f32>) -> (f32, f32) {
        values.fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
            (low.min(value), high.max(value))
        })
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
        assert!(ops.iter().any(|op| matches!(op.shape, Shape::Text(_))));
    }

    #[test]
    fn a_reading_sits_at_the_right_edge_on_a_pinned_scale() {
        // No axes keeps the plot deterministic: top inset 8, full height below.
        let ops = ops(r#"{"showAxes": false, "min": 0, "max": 100}"#, Some(50.0));
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
            r##"{"showAxes": false, "stroke": {"kind": "solid", "color": "#ff0000"}}"##,
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
    fn axes_and_labels_stay_inside_the_box() {
        let ops = ops(r#"{"min": 0, "max": 100}"#, Some(50.0));
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
