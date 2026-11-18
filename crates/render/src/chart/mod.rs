//! Chart geometry: a chart node's settings become paint geometry.
//!
//! Charts are drawn, not delegated: ECharts is not ported, so every family's
//! arc, band, axis and mark is computed here as ordinary `Op`s in the node's
//! own box. A reading that is missing draws the structure it still has —
//! a track, an axis — and never a zero it did not measure.

pub mod bar;
pub mod gauge;
pub mod line;
pub mod pie;
pub mod polar_bar;
pub mod radar;
pub mod segments;

use yunari_model::document::{ThemeDocument, TypeWeight};

use crate::color::{Color, parse_color};
use crate::display_list::{PathCommand, Shape};

use self::segments::SegmentGap;
use crate::text::{
    Fonts, TextAlign, TextOverflow, TextRequest, TextRunPlan, TextStyle, VerticalAlign,
};

/// What a chart family is asked to draw.
pub struct ChartRequest<'a> {
    pub document: &'a ThemeDocument,
    pub node_id: &'a str,
    /// The node's box, in artboard units.
    pub width: f32,
    pub height: f32,
    /// What the node's bindings read, in binding order, with each binding's
    /// authored scale and offset applied. A binding nothing read is `None`, and
    /// every family treats that as an absence rather than a zero.
    pub readings: &'a [Option<f64>],
    /// Each binding's samples inside the chart's window, oldest first, with the
    /// binding's scale and offset already applied. Empty when no surface is
    /// recording history; a family that draws a window uses what it has and
    /// falls back to the one reading when a series is a single point.
    pub series: &'a [Vec<(u64, f64)>],
    pub opacity: f32,
    pub settings: &'a serde_json::Map<String, serde_json::Value>,
}

/// A paint a chart setting names, resolved to what can be drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum ChartFill {
    Solid(Color),
    /// Stops in the chart's own measure: 0 is the start of the sweep or bar,
    /// 1 the end. A gradient is decorative, never a threshold.
    Gradient(Vec<(f32, Color)>),
    /// Threshold bands: a value's band is its colour.
    Thresholds(Vec<(f32, Color)>),
}

impl ChartFill {
    /// The colour a position on the chart's own measure reads.
    pub fn at(&self, position: f32) -> Color {
        match self {
            ChartFill::Solid(color) => *color,
            ChartFill::Gradient(stops) | ChartFill::Thresholds(stops) => {
                stop_color_at(stops, position)
            }
        }
    }

    /// A filled mark answers the reading; an unfilled one is the track, whose
    /// threshold colour is the top of its own scale.
    pub fn for_mark(&self, position: f32, filled: bool) -> Color {
        match self {
            ChartFill::Thresholds(stops) if !filled => stop_color_at(stops, 1.0),
            _ => self.at(position),
        }
    }

    pub fn with_opacity(self, opacity: f32) -> Self {
        let scale = |color: Color| scale_alpha(color, opacity);
        match self {
            ChartFill::Solid(color) => ChartFill::Solid(scale(color)),
            ChartFill::Gradient(stops) => {
                ChartFill::Gradient(stops.into_iter().map(|(at, c)| (at, scale(c))).collect())
            }
            ChartFill::Thresholds(stops) => {
                ChartFill::Thresholds(stops.into_iter().map(|(at, c)| (at, scale(c))).collect())
            }
        }
    }
}

fn scale_alpha(color: Color, opacity: f32) -> Color {
    if opacity >= 1.0 {
        return color;
    }
    let alpha = (f32::from(color.a) * opacity.clamp(0.0, 1.0)).round();
    color.with_alpha(alpha as u8)
}

fn stop_color_at(stops: &[(f32, Color)], position: f32) -> Color {
    let position = position.clamp(0.0, 1.0);
    let mut previous = stops.first().map(|(at, color)| (*at, *color));
    for (at, color) in stops {
        if position <= *at {
            return match previous {
                Some((from_at, from_color)) if *at > from_at => {
                    mix(from_color, *color, (position - from_at) / (*at - from_at))
                }
                Some((_, from_color)) => from_color,
                None => *color,
            };
        }
        previous = Some((*at, *color));
    }
    stops
        .last()
        .map_or(Color::rgb(0, 0, 0), |(_, color)| *color)
}

fn mix(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let channel =
        |a: u8, b: u8| -> u8 { (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8 };
    Color {
        r: channel(from.r, to.r),
        g: channel(from.g, to.g),
        b: channel(from.b, to.b),
        a: channel(from.a, to.a),
    }
}

/// A paint setting, read from the settings bag. `None` with a reason means the
/// paint names something the document does not declare, and no colour is
/// substituted for it.
pub fn chart_fill(
    document: &ThemeDocument,
    value: Option<&serde_json::Value>,
    node_id: &str,
    path: &str,
    issues: &mut Vec<crate::PlanIssue>,
) -> Option<ChartFill> {
    let value = value?;
    let kind = value.get("kind").and_then(|kind| kind.as_str());
    match kind {
        Some("solid") => value
            .get("color")
            .and_then(|color| color.as_str())
            .and_then(parse_color)
            .or_else(|| {
                issues.push(unreadable(node_id, path));
                None
            })
            .map(ChartFill::Solid),
        Some("gradient") => Some(ChartFill::Gradient(stops_of(value))),
        Some("thresholds") => {
            // One band with no colour would be a threshold the author never
            // wrote, so the whole fill goes rather than renumbering the bands
            // around it.
            let bands = value
                .get("bands")
                .and_then(|bands| bands.as_array())
                .and_then(|bands| {
                    bands
                        .iter()
                        .map(|band| {
                            let offset = band.get("offset")?.as_f64()? as f32;
                            let reference = band.get("ref")?.as_str()?;
                            let color = palette_color(document, reference)?;
                            Some((offset, color))
                        })
                        .collect::<Option<Vec<_>>>()
                });
            match bands {
                Some(bands) if !bands.is_empty() => Some(ChartFill::Thresholds(normalize(bands))),
                _ => {
                    issues.push(unreadable(node_id, path));
                    None
                }
            }
        }
        _ => {
            let reference = value.get("ref")?.as_str()?;
            match palette_paint(document, reference) {
                Some(ChartFill::Solid(color)) => Some(ChartFill::Solid(color)),
                Some(ChartFill::Gradient(stops)) => Some(ChartFill::Gradient(stops)),
                Some(ChartFill::Thresholds(stops)) => Some(ChartFill::Thresholds(stops)),
                None => {
                    issues.push(crate::PlanIssue {
                        code: crate::IssueCode::UnresolvedGlobal,
                        node_id: Some(node_id.to_owned()),
                        detail: format!("{path}: {reference} is not declared."),
                    });
                    None
                }
            }
        }
    }
}

fn unreadable(node_id: &str, path: &str) -> crate::PlanIssue {
    crate::PlanIssue {
        code: crate::IssueCode::UnreadablePaint,
        node_id: Some(node_id.to_owned()),
        detail: format!("{path}: this paint cannot be read."),
    }
}

fn stops_of(value: &serde_json::Value) -> Vec<(f32, Color)> {
    let stops = value
        .get("stops")
        .and_then(|stops| stops.as_array())
        .map(|stops| {
            stops
                .iter()
                .filter_map(|stop| {
                    let offset = stop.get("offset")?.as_f64()? as f32;
                    let color = parse_color(stop.get("color")?.as_str()?)?;
                    Some((offset, color))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    normalize(stops)
}

/// Stops are read as ascending, unique offsets, which is what a sampler needs.
fn normalize(mut stops: Vec<(f32, Color)>) -> Vec<(f32, Color)> {
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    stops.dedup_by(|a, b| a.0 == b.0);
    stops
}

fn palette_color(document: &ThemeDocument, reference: &str) -> Option<Color> {
    match palette_paint(document, reference)? {
        ChartFill::Solid(color) => Some(color),
        _ => None,
    }
}

fn palette_paint(document: &ThemeDocument, reference: &str) -> Option<ChartFill> {
    let id = reference.strip_prefix("palette.")?;
    let entry = document.palette.get(id)?;
    match &entry.value {
        yunari_model::document::Paint::Solid { color } => parse_color(color).map(ChartFill::Solid),
        yunari_model::document::Paint::Gradient { stops, .. } => {
            Some(ChartFill::Gradient(normalize(
                stops
                    .iter()
                    .filter_map(|stop| {
                        let color = parse_color(&stop.color)?;
                        Some((stop.offset as f32, color))
                    })
                    .collect(),
            )))
        }
    }
}

/// A setting's number, or the family's own default.
pub fn number(
    settings: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: f32,
) -> f32 {
    settings
        .get(key)
        .and_then(|value| value.as_f64())
        .map_or(default, |value| value as f32)
}

pub fn boolean(
    settings: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> bool {
    settings
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(default)
}

pub fn text<'a>(
    settings: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    settings.get(key).and_then(|value| value.as_str())
}

/// A band of the ring or bar a value fills, in the chart's own measure.
pub struct Band {
    pub from: f32,
    pub to: f32,
    pub fill: ChartFill,
}

/// Arc geometry, in degrees, in the canvas's y-down space where 0 points right
/// and positive turns counter-clockwise — the convention gauge settings are
/// authored in.
pub fn point_on(centre: (f32, f32), radius: f32, degrees: f32) -> (f32, f32) {
    let radians = degrees.to_radians();
    (
        centre.0 + radius * radians.cos(),
        centre.1 - radius * radians.sin(),
    )
}

/// A ring slice: the outer edge forward, the inner edge back. `radius` is the
/// band's centre line, so the slice spans `radius ± thickness / 2`.
pub fn ring(
    centre: (f32, f32),
    radius: f32,
    thickness: f32,
    from: f32,
    to: f32,
    round_caps: bool,
) -> Vec<PathCommand> {
    let mut commands = Vec::new();
    if (to - from).abs() < 0.01 || thickness <= 0.0 {
        return commands;
    }
    let outer = radius + thickness / 2.0;
    let inner = (radius - thickness / 2.0).max(0.0);
    let start = point_on(centre, outer, from);
    commands.push(PathCommand::MoveTo(start.0, start.1));
    arc(&mut commands, centre, outer, from, to);
    let inner_end = point_on(centre, inner, to);
    commands.push(PathCommand::LineTo(inner_end.0, inner_end.1));
    arc(&mut commands, centre, inner, to, from);
    commands.push(PathCommand::Close);
    if round_caps {
        commands.extend(circle(point_on(centre, radius, from), thickness / 2.0));
        commands.extend(circle(point_on(centre, radius, to), thickness / 2.0));
    }
    commands
}

/// A circular arc as cubics, a quarter turn at a time so the approximation
/// stays within a fraction of a pixel at any size a display uses.
pub fn arc(commands: &mut Vec<PathCommand>, centre: (f32, f32), radius: f32, from: f32, to: f32) {
    let sweep = to - from;
    let steps = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
    let step = sweep / steps as f32;
    let mut angle = from;
    for _ in 0..steps {
        let next = angle + step;
        let kappa = 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
        let start = point_on(centre, radius, angle);
        let end = point_on(centre, radius, next);
        let start_tangent = tangent(radius, angle, kappa);
        let end_tangent = tangent(radius, next, kappa);
        commands.push(PathCommand::CubicTo(
            start.0 + start_tangent.0,
            start.1 + start_tangent.1,
            end.0 - end_tangent.0,
            end.1 - end_tangent.1,
            end.0,
            end.1,
        ));
        angle = next;
    }
}

fn tangent(radius: f32, degrees: f32, kappa: f32) -> (f32, f32) {
    let radians = degrees.to_radians();
    (
        -radius * radians.sin() * kappa,
        -radius * radians.cos() * kappa,
    )
}

/// A circle as four cubics, so a round cap is part of the same path.
pub fn circle(centre: (f32, f32), radius: f32) -> Vec<PathCommand> {
    let kappa = 0.552_284_8 * radius;
    let (x, y) = centre;
    vec![
        PathCommand::MoveTo(x + radius, y),
        PathCommand::CubicTo(x + radius, y + kappa, x + kappa, y + radius, x, y + radius),
        PathCommand::CubicTo(x - kappa, y + radius, x - radius, y + kappa, x - radius, y),
        PathCommand::CubicTo(x - radius, y - kappa, x - kappa, y - radius, x, y - radius),
        PathCommand::CubicTo(x + kappa, y - radius, x + radius, y - kappa, x + radius, y),
        PathCommand::Close,
    ]
}

/// Where along its own measure a reading sits, clamped into the range.
pub fn position(value: f64, min: f32, max: f32) -> f32 {
    if max <= min {
        return 0.0;
    }
    (((value as f32) - min) / (max - min)).clamp(0.0, 1.0)
}

/// A weight as the shaper's number, matching how text runs resolve theirs.
pub fn weight_of(weight: Option<&TypeWeight>) -> u16 {
    match weight {
        Some(TypeWeight::Number(weight)) => weight.round().clamp(1.0, 1000.0) as u16,
        Some(TypeWeight::Name(name)) => match name.to_ascii_lowercase().as_str() {
            "thin" => 100,
            "extralight" | "extra-light" => 200,
            "light" => 300,
            "medium" => 500,
            "semibold" | "semi-bold" | "demibold" => 600,
            "bold" => 700,
            "extrabold" | "extra-bold" => 800,
            "black" | "heavy" => 900,
            _ => 400,
        },
        None => 400,
    }
}

/// How a chart's own label is drawn: its size, colour and where the given
/// point sits on the text.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelStyle {
    pub size: f32,
    pub color: Color,
    pub weight: u16,
    pub family: Option<String>,
    /// What the anchor point marks: the left edge, the centre or the right.
    pub align: TextAlign,
}

impl LabelStyle {
    pub fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            color,
            weight: 400,
            family: None,
            align: TextAlign::Left,
        }
    }
}

fn text_style(style: &LabelStyle) -> TextStyle {
    TextStyle {
        family: style.family.clone(),
        weight: style.weight,
        italic: false,
        size: style.size,
        color: style.color,
        ..TextStyle::default()
    }
}

/// The width a chart label occupies, for the caller that has to centre or
/// right-align it against a point it computed.
pub fn text_width(fonts: &mut Fonts, text: &str, style: &LabelStyle) -> f32 {
    crate::text::advance_width(fonts, text, &text_style(style))
}

/// A chart's own label — an axis tick, a value, a legend entry — placed so the
/// anchor point lands where the caller wants it.
pub fn label(
    fonts: &mut Fonts,
    text: &str,
    at: (f32, f32),
    style: &LabelStyle,
) -> Option<crate::Op> {
    if text.is_empty() {
        return None;
    }
    let runs = [TextRunPlan {
        text: text.to_owned(),
        style: text_style(style),
    }];
    let mut laid = crate::text::layout(
        fonts,
        &TextRequest {
            runs: &runs,
            box_: None,
            align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            overflow: TextOverflow::Visible,
            vertical: false,
            line_height: 1.2,
        },
    );
    let width = crate::text::advance_width(fonts, text, &text_style(style));
    let shift = match style.align {
        TextAlign::Left => 0.0,
        TextAlign::Center => width / 2.0,
        TextAlign::Right => width,
    };
    for segment in &mut laid.segments {
        segment.origin.0 += at.0 - shift;
        segment.origin.1 += at.1;
    }
    Some(crate::Op {
        transform: crate::Affine::IDENTITY,
        shape: Shape::Text(laid),
        fill: None,
        stroke: None,
    })
}

/// A filled op from finished commands.
pub fn filled(commands: Vec<PathCommand>, fill: ChartFill) -> Option<crate::Op> {
    if commands.is_empty() {
        return None;
    }
    Some(crate::Op {
        transform: crate::Affine::IDENTITY,
        shape: Shape::Path {
            commands,
            closed: true,
        },
        fill: Some(crate::Paint::Solid(fill.at(0.0))),
        stroke: None,
    })
}

/// The gap helper a family's bands are cut with.
pub fn gap_of(request: &ChartRequest<'_>, sweep: f32, count: u32) -> f32 {
    let thickness = number(request.settings, "thickness", 18.0);
    let round_cap = boolean(request.settings, "roundCap", true);
    let spacing = segments::segment_spacing_percent_of(request.settings);
    segments::gauge_gap(
        SegmentGap {
            thickness,
            round_cap,
            spacing_percent: spacing,
        },
        sweep,
        count,
        Some(request.width.min(request.height)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_model::document::parse_document;

    fn document() -> ThemeDocument {
        parse_document(
            r##"{"schemaVersion":"3.0.0","id":"t","artboard":{"width":200,"height":200},
                "palette":{"good":{"name":"Good","value":{"kind":"solid","color":"#36b37e"}}}}"##,
        )
        .expect("fixture parses")
    }

    #[test]
    fn a_threshold_band_with_an_undeclared_colour_refuses_the_whole_fill() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"kind":"thresholds","bands":[
                {"offset":0.5,"ref":"palette.good"},
                {"offset":1,"ref":"palette.missing"}]}"#,
        )
        .expect("bands parse");
        let mut issues = Vec::new();
        let fill = chart_fill(&document(), Some(&value), "c", "progress", &mut issues);
        assert_eq!(fill, None, "an unresolved band refuses the fill");
    }
}
