//! Polar bar: a reading drawn as a radial band around the box centre.

use crate::chart::bar::{
    AXIS_COLOR, FILL_DEFAULT, GRID_COLOR, TRACK_DEFAULT, faded, gradient_paint, paint_setting,
    setting_number, shape_op, stroke_op,
};
use crate::chart::{ChartFill, ChartRequest, arc, boolean, circle, number, point_on, position};
use crate::display_list::{PathCommand, Shape};
use crate::{Op, Paint, PlanIssue};

const START_ANGLE_DEFAULT: f32 = 90.0;
const INNER_RADIUS_DEFAULT: f32 = 20.0;
const OUTER_RADIUS_DEFAULT: f32 = 80.0;
const CORNER_RADIUS_DEFAULT: f32 = 7.0;
const CATEGORY_GAP_DEFAULT: f32 = 40.0;
const SPLIT_NUMBER: usize = 5;

pub fn build(
    request: &ChartRequest<'_>,
    _fonts: &mut crate::Fonts,
    issues: &mut Vec<PlanIssue>,
) -> Vec<Op> {
    let settings = request.settings;
    let centre = (request.width / 2.0, request.height / 2.0);
    let radius = request.width.min(request.height) / 2.0;
    let start = finite(
        number(settings, "startAngle", START_ANGLE_DEFAULT),
        START_ANGLE_DEFAULT,
    );
    let inner = radius
        * number(settings, "innerRadiusPercent", INNER_RADIUS_DEFAULT).clamp(0.0, 100.0)
        / 100.0;
    let outer = radius
        * number(settings, "outerRadiusPercent", OUTER_RADIUS_DEFAULT).clamp(0.0, 100.0)
        / 100.0;
    let show_axes = boolean(settings, "showAxes", false);
    let corner = number(settings, "cornerRadius", CORNER_RADIUS_DEFAULT).max(0.0);
    let track_corner = number(settings, "trackCornerRadius", CORNER_RADIUS_DEFAULT).max(0.0);

    let mut ops = Vec::new();
    if show_axes && outer > inner {
        draw_axes(&mut ops, request, centre, inner, outer, start);
    }

    // One binding is one category: its own slice of the turn, with the gap a
    // share left empty on each side of the bar.
    let categories = request.readings.len();
    if categories == 0 || outer <= inner {
        return ops;
    }
    let slot = 360.0 / categories as f32;
    let gap = number(settings, "categoryGapPercent", CATEGORY_GAP_DEFAULT).clamp(0.0, 100.0);
    let width = setting_number(settings, "barWidth")
        .map(|width| width.abs())
        .unwrap_or(slot * (1.0 - gap / 100.0))
        .clamp(0.0, slot);
    if width <= 0.0 {
        return ops;
    }

    let track = paint_setting(request, "track", TRACK_DEFAULT, issues);
    let fill = paint_setting(request, "fill", FILL_DEFAULT, issues);
    let min = finite(number(settings, "min", 0.0), 0.0);
    let max = finite(number(settings, "max", 100.0), 100.0);

    for (index, reading) in request.readings.iter().enumerate() {
        let reading = reading.filter(|value| value.is_finite());
        let at = reading.map_or(0.0, |value| position(value, min, max));
        // Bars run clockwise from the category's start, centred in its slot.
        let midpoint = start - slot * (index as f32 + 0.5);
        let from = midpoint + width / 2.0;
        let to = midpoint - width / 2.0;

        let band = Sector {
            centre,
            inner,
            outer,
            from,
            to,
        };
        if let Some(track) = &track {
            let paint = band.paint(track, false, 0.0);
            if let Some(op) = sector_op(band.path(track_corner), paint) {
                ops.push(op);
            }
        }

        if fill.is_some() && reading.is_some() && at > 0.0 {
            let Some(fill) = &fill else {
                return ops;
            };
            let filled = Sector {
                outer: inner + (outer - inner) * at,
                ..band
            };
            let paint = filled.paint(fill, true, at);
            if let Some(op) = sector_op(filled.path(corner), paint) {
                ops.push(op);
            }
        }
    }
    ops
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// One annular sector: a radial span and a clockwise angle span.
#[derive(Clone, Copy)]
struct Sector {
    centre: (f32, f32),
    inner: f32,
    outer: f32,
    from: f32,
    to: f32,
}

impl Sector {
    /// A gradient is linear in screen space (`to-top`, the direction the
    /// upstream adapter hands a polar bar); everything else is flat.
    fn paint(&self, fill: &ChartFill, filled: bool, at: f32) -> Paint {
        match fill {
            ChartFill::Gradient(stops) => {
                let (_, low, _, high) = self.bounds();
                gradient_paint(stops, (self.centre.0, high), (self.centre.0, low))
            }
            _ => Paint::Solid(fill.for_mark(if filled { at } else { 1.0 }, filled)),
        }
    }

    /// The outer edge clockwise, the inner edge back. `corner` rounds the four
    /// joins with quadratic fillets, clamped so a fillet cannot cross the band
    /// or its own arc.
    fn path(&self, corner: f32) -> Vec<PathCommand> {
        let Self {
            centre,
            inner,
            outer,
            from,
            to,
        } = *self;
        let sweep = to - from;
        if outer <= inner || sweep.abs() < 0.01 {
            return Vec::new();
        }
        let sweep_radians = sweep.abs().to_radians();
        let mut fillet = corner.max(0.0).min((outer - inner) / 2.0);
        fillet = fillet.min(outer * sweep_radians / 4.0);
        if inner > 0.0 {
            fillet = fillet.min(inner * sweep_radians / 4.0);
        }
        let direction = sweep.signum();
        let out_step = if outer > 0.0 { fillet / outer } else { 0.0 };
        let in_step = if inner > 0.0 { fillet / inner } else { 0.0 };
        let out_start = from + direction * out_step.to_degrees();
        let out_end = to - direction * out_step.to_degrees();
        let in_start = to - direction * in_step.to_degrees();
        let in_end = from + direction * in_step.to_degrees();

        let mut commands = Vec::new();
        if fillet > 0.0 {
            let (x, y) = point_on(centre, outer - fillet, from);
            commands.push(PathCommand::MoveTo(x, y));
            let (cx, cy) = point_on(centre, outer, from);
            let (x, y) = point_on(centre, outer, out_start);
            commands.push(PathCommand::QuadTo(cx, cy, x, y));
        } else {
            let (x, y) = point_on(centre, outer, from);
            commands.push(PathCommand::MoveTo(x, y));
        }
        arc(&mut commands, centre, outer, out_start, out_end);
        if fillet > 0.0 {
            let (cx, cy) = point_on(centre, outer, to);
            let (x, y) = point_on(centre, outer - fillet, to);
            commands.push(PathCommand::QuadTo(cx, cy, x, y));
            let (x, y) = point_on(centre, inner + fillet, to);
            commands.push(PathCommand::LineTo(x, y));
            let (cx, cy) = point_on(centre, inner, to);
            let (x, y) = point_on(centre, inner, in_start);
            commands.push(PathCommand::QuadTo(cx, cy, x, y));
        } else {
            let (x, y) = point_on(centre, inner, to);
            commands.push(PathCommand::LineTo(x, y));
        }
        arc(&mut commands, centre, inner, in_start, in_end);
        if fillet > 0.0 {
            let (cx, cy) = point_on(centre, inner, from);
            let (x, y) = point_on(centre, inner + fillet, from);
            commands.push(PathCommand::QuadTo(cx, cy, x, y));
        }
        commands.push(PathCommand::Close);
        commands
    }

    /// The band's bounding box, sampled around both arcs: a screen-space
    /// gradient spans the shape it paints, not the whole circle.
    fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut low_x = f32::INFINITY;
        let mut low_y = f32::INFINITY;
        let mut high_x = f32::NEG_INFINITY;
        let mut high_y = f32::NEG_INFINITY;
        for index in 0..=32 {
            let angle = self.from + (self.to - self.from) * index as f32 / 32.0;
            for radius in [self.inner, self.outer] {
                let (x, y) = point_on(self.centre, radius, angle);
                low_x = low_x.min(x);
                low_y = low_y.min(y);
                high_x = high_x.max(x);
                high_y = high_y.max(y);
            }
        }
        (low_x, low_y, high_x, high_y)
    }
}

fn sector_op(commands: Vec<PathCommand>, paint: Paint) -> Option<Op> {
    if commands.len() < 2 {
        return None;
    }
    Some(shape_op(
        Shape::Path {
            commands,
            closed: true,
        },
        paint,
    ))
}

/// The angle axis line, a radial spoke and the circular split grid. Labels are
/// hidden upstream, so none are drawn here.
fn draw_axes(
    ops: &mut Vec<Op>,
    request: &ChartRequest<'_>,
    centre: (f32, f32),
    inner: f32,
    outer: f32,
    start: f32,
) {
    let axis = faded(AXIS_COLOR, request.opacity);
    let grid = faded(GRID_COLOR, request.opacity);
    ops.push(stroke_op(circle(centre, outer), axis, 1.0));
    let (x, y) = point_on(centre, inner, start);
    let (x2, y2) = point_on(centre, outer, start);
    ops.push(stroke_op(
        vec![PathCommand::MoveTo(x, y), PathCommand::LineTo(x2, y2)],
        axis,
        1.0,
    ));
    for index in 1..=SPLIT_NUMBER {
        let radius = inner + (outer - inner) * index as f32 / SPLIT_NUMBER as f32;
        ops.push(stroke_op(circle(centre, radius), grid, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-figure fixture, read as one figure per binding.
    fn readings_of(
        document: &yunari_model::document::ThemeDocument,
        reading: Option<f64>,
    ) -> Vec<Option<f64>> {
        vec![reading; document.bindings.get("c").map_or(1, Vec::len)]
    }

    fn document(settings: &str) -> yunari_model::document::ThemeDocument {
        yunari_model::document::parse_document(&format!(
            r##"{{"schemaVersion":"3.0.0","id":"t","artboard":{{"width":200,"height":200}},
                "nodes":[{{"id":"c","kind":{{"type":"chart","content":{{"family":"polarBar","settings":{settings}}}}},
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
        let (ops, issues) = build_with(settings, reading);
        assert!(issues.is_empty(), "{issues:?}");
        ops
    }

    fn endpoints(op: &Op) -> Vec<(f32, f32)> {
        let Shape::Path { commands, .. } = &op.shape else {
            panic!("expected a path");
        };
        commands
            .iter()
            .filter_map(|command| match *command {
                PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) => Some((x, y)),
                PathCommand::CubicTo(_, _, _, _, x, y) => Some((x, y)),
                _ => None,
            })
            .collect()
    }

    fn distances(op: &Op) -> Vec<f32> {
        endpoints(op)
            .into_iter()
            .map(|(x, y)| ((x - 100.0).powi(2) + (y - 100.0).powi(2)).sqrt())
            .filter(|distance| *distance > 0.5)
            .collect()
    }

    fn angular_span(op: &Op, midpoint: f32) -> f32 {
        let angles: Vec<f32> = endpoints(op)
            .into_iter()
            .filter(|(x, y)| ((x - 100.0).powi(2) + (y - 100.0).powi(2)).sqrt() > 0.5)
            .map(|(x, y)| {
                let angle = (-(y - 100.0)).atan2(x - 100.0).to_degrees();
                let delta = (angle - midpoint + 180.0).rem_euclid(360.0) - 180.0;
                midpoint + delta
            })
            .collect();
        let low = angles.iter().copied().fold(f32::INFINITY, f32::min);
        let high = angles.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        high - low
    }

    #[test]
    fn a_reading_draws_a_radial_band_over_its_track() {
        let ops = ops("{}", Some(50.0));
        assert_eq!(ops.len(), 2);
        let track = distances(&ops[0]);
        let fill = distances(&ops[1]);
        assert!(track.iter().any(|d| (*d - 80.0).abs() < 0.1), "{track:?}");
        assert!(track.iter().any(|d| (*d - 20.0).abs() < 0.1), "{track:?}");
        assert!(fill.iter().all(|d| *d <= 50.1), "{fill:?}");
        assert!(fill.iter().any(|d| (*d - 50.0).abs() < 0.1), "{fill:?}");
        assert_eq!(ops[0].fill, Some(Paint::Solid(TRACK_DEFAULT)));
        assert_eq!(ops[1].fill, Some(Paint::Solid(FILL_DEFAULT)));
    }

    #[test]
    fn a_missing_reading_draws_the_track_only() {
        let ops = ops("{}", None);
        assert_eq!(ops.len(), 1);
        assert!(distances(&ops[0]).iter().all(|d| (19.9..=80.1).contains(d)));
    }

    #[test]
    fn the_category_gap_sizes_the_angular_band() {
        let ops = ops(r#"{"cornerRadius":0,"trackCornerRadius":0}"#, None);
        // A 40% gap leaves 60% of the single 360° slot: 216°.
        let span = angular_span(&ops[0], -90.0);
        assert!((span - 216.0).abs() < 0.2, "{span}");
    }

    #[test]
    fn an_empty_gap_draws_no_band() {
        assert!(ops(r#"{"categoryGapPercent":100}"#, Some(50.0)).is_empty());
    }

    #[test]
    fn the_radii_follow_the_authored_percentages() {
        let ops = ops(
            r#"{"innerRadiusPercent":10,"outerRadiusPercent":60,"cornerRadius":0,"trackCornerRadius":0}"#,
            Some(50.0),
        );
        let track = distances(&ops[0]);
        assert!(track.iter().any(|d| (*d - 10.0).abs() < 0.1), "{track:?}");
        assert!(track.iter().any(|d| (*d - 60.0).abs() < 0.1), "{track:?}");
        // Half the range: 10 + (60 - 10) / 2 = 35.
        assert!(distances(&ops[1]).iter().any(|d| (*d - 35.0).abs() < 0.1));
    }

    #[test]
    fn axes_add_rings_and_a_spoke() {
        let ops = ops(r#"{"showAxes":true}"#, None);
        let strokes = ops.iter().filter(|op| op.stroke.is_some()).count();
        assert_eq!(strokes, SPLIT_NUMBER + 2);
        assert_eq!(ops.len(), SPLIT_NUMBER + 3);
    }

    #[test]
    fn a_polar_gradient_stays_linear_in_screen_space() {
        let ops = ops(
            r##"{"fill":{"kind":"gradient","stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}}"##,
            Some(50.0),
        );
        let Some(Paint::Linear { from, to, .. }) = &ops[1].fill else {
            panic!("expected a linear gradient");
        };
        assert!(from.1 >= to.1, "to-top: {from:?} -> {to:?}");
    }

    #[test]
    fn the_start_angle_moves_the_whole_sector() {
        let default = ops(r#"{"cornerRadius":0,"trackCornerRadius":0}"#, None);
        let moved = ops(
            r#"{"startAngle":0,"cornerRadius":0,"trackCornerRadius":0}"#,
            None,
        );
        assert!((angular_span(&default[0], -90.0) - 216.0).abs() < 0.2);
        assert!((angular_span(&moved[0], -180.0) - 216.0).abs() < 0.2);
    }
}
