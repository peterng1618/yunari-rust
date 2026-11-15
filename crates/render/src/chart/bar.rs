//! Bar chart: a value bar over a cartesian track, with optional axes.

use crate::chart::segments::{
    SegmentFill, bar_gap_px, segment_count, segment_divisor_of, segment_fill_of,
    segment_spacing_percent_of,
};
use crate::chart::{
    ChartFill, ChartRequest, LabelStyle, boolean, chart_fill, label, number, position, text,
};
use crate::display_list::{GradientStop, PathCommand, Shape};
use crate::text::TextAlign;
use crate::{Affine, Color, Op, Paint, PlanIssue, Stroke};

/// The family's own paints, used only when the document authors none.
pub(crate) const FILL_DEFAULT: Color = Color::rgb(0x00, 0xb8, 0xd9);
pub(crate) const TRACK_DEFAULT: Color = Color::rgb(0x2a, 0x2f, 0x3a);
/// ECharts' own axis defaults: bar.ts leaves axis styling to the engine.
pub(crate) const AXIS_COLOR: Color = Color::rgb(0x6e, 0x70, 0x79);
pub(crate) const GRID_COLOR: Color = Color::rgb(0xe0, 0xe6, 0xf1);
pub(crate) const LABEL_SIZE: f32 = 12.0;

const BAR_WIDTH_DEFAULT: f32 = 14.0;
const CORNER_RADIUS_DEFAULT: f32 = 7.0;
const SPLIT_NUMBER: usize = 5;
const AXIS_INSET: f32 = 8.0;

pub fn build(
    request: &ChartRequest<'_>,
    fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    let settings = request.settings;
    let horizontal = text(settings, "orientation") != Some("vertical");
    let min = number(settings, "min", 0.0);
    let max = number(settings, "max", 100.0);
    let show_axes = boolean(settings, "showAxes", false);
    let corner = number(settings, "cornerRadius", CORNER_RADIUS_DEFAULT).max(0.0);
    let track_corner = number(settings, "trackCornerRadius", CORNER_RADIUS_DEFAULT).max(0.0);
    // Track and value are independent: a track that cannot resolve is an
    // absence, not a reason to withhold a bar the document did resolve.
    let track = paint_setting(request, "track", TRACK_DEFAULT, issues);
    let fill = paint_setting(request, "fill", FILL_DEFAULT, issues);
    let reading = request.reading.filter(|value| value.is_finite());
    let at = reading.map_or(0.0, |value| position(value, min, max));

    let (x0, y0, x1, y1) = if show_axes {
        (AXIS_INSET, 0.0, request.width, request.height - AXIS_INSET)
    } else {
        (0.0, 0.0, request.width, request.height)
    };
    let span = if horizontal { x1 - x0 } else { y1 - y0 }.max(0.0);
    let across = if horizontal { y1 - y0 } else { x1 - x0 }.max(0.0);
    // An authored width wins. With no width, an authored gap asks for a share
    // of the category slot; neither authored takes the family's 14 px default.
    let gap = number(settings, "categoryGapPercent", 40.0).clamp(0.0, 100.0);
    let thickness = setting_number(settings, "barWidth")
        .unwrap_or(if settings.contains_key("categoryGapPercent") {
            across * (1.0 - gap / 100.0)
        } else {
            BAR_WIDTH_DEFAULT
        })
        .max(0.0)
        .min(across);
    let frame = Frame {
        horizontal,
        x0,
        y0,
        x1,
        y1,
        thickness,
    };

    let mut ops = Vec::new();
    if show_axes {
        draw_axes(
            &mut ops,
            fonts,
            request,
            horizontal,
            min,
            max,
            (x0, y0, x1, y1),
        );
    }

    let count = segment_count(segment_divisor_of(settings));
    if count <= 1 {
        if let Some(track) = &track {
            let paint = frame.mark_paint(0.0, span, track, false, 0.0);
            push(&mut ops, frame.rect(0.0, span, track_corner, paint));
        }
        if let (Some(fill), true) = (&fill, reading.is_some() && at > 0.0) {
            let paint = frame.mark_paint(0.0, at * span, fill, true, at);
            push(&mut ops, frame.rect(0.0, at * span, corner, paint));
        }
        return ops;
    }

    // Segmented: the reading decides reach, the bands only decide the cuts.
    let band = span / count as f32;
    let visible = (band - bar_gap_px(band, segment_spacing_percent_of(settings))).max(0.0);
    let whole = segment_fill_of(settings, SegmentFill::Partial) == SegmentFill::Whole;
    let reach = at * span;
    if let Some(track) = &track {
        for index in 0..count {
            let colour = band_colour(track, (index as f32 + 0.5) / count as f32, 1.0, false);
            push(
                &mut ops,
                frame.rect(
                    index as f32 * band,
                    visible,
                    track_corner,
                    Paint::Solid(colour),
                ),
            );
        }
    }
    if let (Some(fill), true) = (&fill, reading.is_some() && at > 0.0) {
        for index in 0..count {
            let offset = index as f32 * band;
            let length = (reach - offset).clamp(0.0, visible);
            if length <= 0.0 || (whole && length < visible) {
                continue;
            }
            let length = if whole { visible } else { length };
            let colour = band_colour(fill, (index as f32 + 0.5) / count as f32, at, true);
            push(
                &mut ops,
                frame.rect(offset, length, corner, Paint::Solid(colour)),
            );
        }
    }
    ops
}

/// A paint the document authors, or the family's own when it authors none. An
/// authored paint that cannot be resolved is an absence with a reason.
pub(crate) fn paint_setting(
    request: &ChartRequest<'_>,
    key: &str,
    default: Color,
    issues: &mut Vec<PlanIssue>,
) -> Option<ChartFill> {
    let path = format!("nodes/{}/settings/{key}", request.node_id);
    let before = issues.len();
    match chart_fill(
        request.document,
        request.settings.get(key),
        request.node_id,
        &path,
        issues,
    ) {
        Some(fill) => Some(fill.with_opacity(request.opacity)),
        None if request.settings.get(key).is_none() => {
            Some(ChartFill::Solid(default).with_opacity(request.opacity))
        }
        None => {
            if issues.len() == before {
                issues.push(PlanIssue {
                    code: crate::IssueCode::UnreadablePaint,
                    node_id: Some(request.node_id.to_owned()),
                    detail: format!("{path}: this paint cannot be read."),
                });
            }
            None
        }
    }
}

pub(crate) fn faded(color: Color, opacity: f32) -> Color {
    if opacity >= 1.0 {
        return color;
    }
    let alpha = (f32::from(color.a) * opacity.clamp(0.0, 1.0)).round();
    color.with_alpha(alpha as u8)
}

pub(crate) fn gradient_paint(stops: &[(f32, Color)], from: (f32, f32), to: (f32, f32)) -> Paint {
    Paint::Linear {
        from,
        to,
        stops: stops
            .iter()
            .map(|(offset, color)| GradientStop {
                offset: *offset,
                color: *color,
            })
            .collect(),
    }
}

/// A band's colour: a gradient follows the bar's own measure, thresholds the
/// reading, and a track resolves to the top of its scale.
pub(crate) fn band_colour(fill: &ChartFill, midpoint: f32, at: f32, filled: bool) -> Color {
    match fill {
        ChartFill::Gradient(_) => fill.at(midpoint),
        _ => fill.for_mark(if filled { at } else { 1.0 }, filled),
    }
}

pub(crate) fn shape_op(shape: Shape, paint: Paint) -> Op {
    Op {
        transform: Affine::IDENTITY,
        shape,
        fill: Some(paint),
        stroke: None,
    }
}

pub(crate) fn rect_op(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    corner_radius: f32,
    paint: Paint,
) -> Option<Op> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(Op {
        transform: Affine::translate(x, y),
        shape: Shape::Rect {
            width,
            height,
            corner_radius: corner_radius.clamp(0.0, width.min(height) / 2.0),
        },
        fill: Some(paint),
        stroke: None,
    })
}

pub(crate) fn stroke_op(commands: Vec<PathCommand>, color: Color, width: f32) -> Op {
    Op {
        transform: Affine::IDENTITY,
        // A grid line or axis is an open run, not an outline.
        shape: Shape::Path {
            commands,
            closed: false,
        },
        fill: None,
        stroke: Some(Stroke {
            paint: Paint::Solid(color),
            width,
        }),
    }
}

pub(crate) fn setting_number(
    settings: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<f32> {
    settings
        .get(key)
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .filter(|value| value.is_finite())
}

fn push(ops: &mut Vec<Op>, op: Option<Op>) {
    if let Some(op) = op {
        ops.push(op);
    }
}

fn line(from: (f32, f32), to: (f32, f32)) -> Vec<PathCommand> {
    vec![
        PathCommand::MoveTo(from.0, from.1),
        PathCommand::LineTo(to.0, to.1),
    ]
}

/// Where the bars live and which way they grow, in node coordinates.
struct Frame {
    horizontal: bool,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
}

impl Frame {
    fn span(&self) -> f32 {
        if self.horizontal {
            self.x1 - self.x0
        } else {
            self.y1 - self.y0
        }
        .max(0.0)
    }

    fn centre(&self) -> f32 {
        if self.horizontal {
            (self.y0 + self.y1) / 2.0
        } else {
            (self.x0 + self.x1) / 2.0
        }
    }

    /// A bar grows from the minimum edge: right for horizontal, up for vertical,
    /// which is why the vertical rect is placed from its far end.
    fn rect(&self, offset: f32, length: f32, radius: f32, paint: Paint) -> Option<Op> {
        let length = length.clamp(0.0, (self.span() - offset).max(0.0));
        let (x, y, width, height) = if self.horizontal {
            (
                self.x0 + offset,
                self.centre() - self.thickness / 2.0,
                length,
                self.thickness,
            )
        } else {
            (
                self.centre() - self.thickness / 2.0,
                self.y1 - offset - length,
                self.thickness,
                length,
            )
        };
        rect_op(x, y, width, height, radius, paint)
    }

    /// A gradient follows the mark's own extent in screen space; every other
    /// paint resolves to one colour at the reading.
    fn mark_paint(
        &self,
        offset: f32,
        length: f32,
        fill: &ChartFill,
        filled: bool,
        at: f32,
    ) -> Paint {
        let length = length.clamp(0.0, (self.span() - offset).max(0.0));
        match fill {
            ChartFill::Gradient(stops) => {
                let (a, b) = if self.horizontal {
                    (self.x0 + offset, self.x0 + offset + length)
                } else {
                    (self.y1 - offset, self.y1 - offset - length)
                };
                let (from, to) = if self.horizontal {
                    ((a, self.centre()), (b, self.centre()))
                } else {
                    ((self.centre(), a), (self.centre(), b))
                };
                gradient_paint(stops, from, to)
            }
            _ => Paint::Solid(fill.for_mark(if filled { at } else { 1.0 }, filled)),
        }
    }
}

/// The value axis, its split grid and its tick labels. Category labels are not
/// drawn: the upstream adapter hides them and a native text object carries a
/// caption instead.
fn draw_axes(
    ops: &mut Vec<Op>,
    fonts: &mut crate::Fonts,
    request: &ChartRequest<'_>,
    horizontal: bool,
    min: f32,
    max: f32,
    plot: (f32, f32, f32, f32),
) {
    let (x0, y0, x1, y1) = plot;
    let axis = faded(AXIS_COLOR, request.opacity);
    let grid = faded(GRID_COLOR, request.opacity);
    ops.push(stroke_op(
        if horizontal {
            line((x0, y1), (x1, y1))
        } else {
            line((x0, y1), (x0, y0))
        },
        axis,
        1.0,
    ));
    let range = max - min;
    if range <= 0.0 {
        return;
    }
    for index in 0..=SPLIT_NUMBER {
        let fraction = index as f32 / SPLIT_NUMBER as f32;
        let value = min + range * fraction;
        let align = match (horizontal, index) {
            (true, 0) => TextAlign::Left,
            (true, SPLIT_NUMBER) => TextAlign::Right,
            (true, _) => TextAlign::Center,
            (false, _) => TextAlign::Left,
        };
        let (grid_line, at) = if horizontal {
            let x = x0 + (x1 - x0) * fraction;
            (
                line((x, y0), (x, y1)),
                (x, (y1 - LABEL_SIZE - 2.0).max(0.0)),
            )
        } else {
            let y = y1 - (y1 - y0) * fraction;
            (
                line((x0, y), (x1, y)),
                (x0 + 3.0, (y - LABEL_SIZE / 2.0).max(0.0)),
            )
        };
        ops.push(stroke_op(grid_line, grid, 1.0));
        let text = crate::format::format_number(value as f64, None, '.');
        let style = LabelStyle {
            align,
            ..LabelStyle::new(LABEL_SIZE, axis)
        };
        push(ops, label(fonts, &text, at, &style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(settings: &str) -> yunari_model::document::ThemeDocument {
        yunari_model::document::parse_document(&format!(
            r##"{{"schemaVersion":"1.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"bar","settings":{settings}}}}},
                "transform":{{"width":200,"height":200}}}}]}}"##
        ))
        .expect("fixture parses")
    }

    fn build_with(settings: &str, reading: Option<f64>) -> (Vec<Op>, Vec<PlanIssue>) {
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
        let (ops, issues) = build_with(settings, reading);
        assert!(issues.is_empty(), "{issues:?}");
        ops
    }

    fn rect(op: &Op) -> (f32, f32, f32, f32, f32) {
        let Shape::Rect {
            width,
            height,
            corner_radius,
        } = &op.shape
        else {
            panic!("expected a rect");
        };
        (
            op.transform.e,
            op.transform.f,
            *width,
            *height,
            *corner_radius,
        )
    }

    fn unresolved(issues: &[PlanIssue]) -> bool {
        issues
            .iter()
            .any(|issue| issue.code == crate::IssueCode::UnresolvedGlobal)
    }

    #[test]
    fn a_reading_grows_a_horizontal_bar_from_the_minimum() {
        let drawn = ops("{}", Some(50.0));
        assert_eq!(drawn.len(), 2);
        assert_eq!(rect(&drawn[0]), (0.0, 93.0, 200.0, 14.0, 7.0));
        assert_eq!(rect(&drawn[1]), (0.0, 93.0, 100.0, 14.0, 7.0));
        assert_eq!(drawn[0].fill, Some(Paint::Solid(TRACK_DEFAULT)));
        assert_eq!(drawn[1].fill, Some(Paint::Solid(FILL_DEFAULT)));
        // No width authored but a gap is: the slot share sizes it.
        let gap_sized = ops(r#"{"categoryGapPercent":40}"#, Some(50.0));
        assert!((rect(&gap_sized[0]).3 - 120.0).abs() < 1e-3);
    }

    #[test]
    fn a_vertical_bar_grows_upward() {
        let ops = ops(r#"{"orientation":"vertical"}"#, Some(50.0));
        assert_eq!(rect(&ops[1]), (93.0, 100.0, 14.0, 100.0, 7.0));
    }

    #[test]
    fn a_missing_reading_draws_the_track_only() {
        let only_track = ops("{}", None);
        assert_eq!(only_track.len(), 1);
        assert_eq!(only_track[0].fill, Some(Paint::Solid(TRACK_DEFAULT)));
        assert_eq!(
            ops("{}", Some(0.0)).len(),
            1,
            "a reading at the minimum paints nothing"
        );
    }

    #[test]
    fn segments_cut_the_bar_with_the_authored_gap() {
        let ops = ops(r#"{"segmentDivisor":10}"#, Some(55.0));
        assert_eq!(ops.len(), 16);
        // Ten 20 px bands, each painting 20/1.1 px and leaving 20/11 empty.
        let (_, _, first, _, _) = rect(&ops[0]);
        assert!((first - 20.0 / 1.1).abs() < 1e-4, "{first}");
        // The partial sixth band follows the reading: 110 px - 5 bands.
        let (_, _, last, _, _) = rect(&ops[15]);
        assert!((last - 10.0).abs() < 1e-4, "{last}");
    }

    #[test]
    fn axes_draw_a_value_line_grid_and_tick_labels() {
        let ops = ops(r#"{"showAxes":true}"#, None);
        assert_eq!(
            ops.iter().filter(|op| op.stroke.is_some()).count(),
            SPLIT_NUMBER + 2
        );
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op.shape, Shape::Text(_)))
                .count(),
            SPLIT_NUMBER + 1
        );
    }

    #[test]
    fn unresolved_paints_leave_the_other_mark_and_a_reason() {
        let (only_track, issues) = build_with(r#"{"fill":{"ref":"palette.absent"}}"#, Some(50.0));
        assert_eq!(only_track.len(), 1, "only the track survives");
        assert!(unresolved(&issues));
        let (only_fill, issues) = build_with(r#"{"track":{"ref":"palette.absent"}}"#, Some(50.0));
        assert_eq!(only_fill[0].fill, Some(Paint::Solid(FILL_DEFAULT)));
        assert!(unresolved(&issues));
    }
}
