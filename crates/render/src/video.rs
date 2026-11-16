//! The picture both device-played panel families bake: the theme's media
//! layer, its fill and ramps, and the display's mounting angle, as an ffmpeg
//! filter.
//!
//! **One filter, both wires.** The TURZX and the TUR USB differ in their own
//! geometry and encoder flags, not in what a composition looks like. The
//! resolver decides the area, the source window and the fill, so the device and
//! the page draw one picture. The turn is composed last, after the picture is
//! laid out in the box the mount asks for.

use crate::background::{self, Composition, Mask, Rect};
use crate::display_list::Paint;
use crate::{Color, GradientStop};

/// How a panel's clip is turned before it reaches the glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTurn {
    None,
    Cw,
    Half,
    Ccw,
}

/// The two display shapes the mount rule distinguishes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PanelShape {
    #[default]
    Rect,
    Circle,
}

/// What one display record asks the encoder to do with its clip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelRotation {
    Turn(PanelTurn),
    Angle(f64),
}

/// The source facts a region's cover and a finite output need.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SourceFacts {
    pub width: f32,
    pub height: f32,
    pub duration_ms: Option<f64>,
}

/// One ffmpeg picture, ready to be passed as `-vf` or `-filter_complex`.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeFilter {
    pub filter: String,
    /// The composed graph's fill and mask never end, so the output needs the
    /// source's own duration.
    pub complex: bool,
    pub duration_seconds: Option<f64>,
}

/// The authored placement of a panel's media layer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Placement {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Where the media sits inside its box and how it meets the fill. Present
    /// only for an advanced background.
    pub composition: Option<yunari_model::document::BackgroundComposition>,
    /// The resolved paint behind the media inside its box.
    pub fill: Option<Paint>,
    /// The display's shape, which decides whether a non-quarter angle can be
    /// composed.
    pub shape: PanelShape,
}

impl Placement {
    pub fn geometry(&self) -> background::Geometry {
        background::Geometry {
            scale: self.scale,
            offset_x: self.offset_x,
            offset_y: self.offset_y,
        }
    }
}

/// The turn a whole angle asks for, or `None` for one no panel composes.
pub fn video_turn(rotation: u16) -> Option<PanelTurn> {
    match rotation {
        0 => Some(PanelTurn::None),
        90 => Some(PanelTurn::Cw),
        180 => Some(PanelTurn::Half),
        270 => Some(PanelTurn::Ccw),
        _ => None,
    }
}

/// The rotation one display asks of its panel's clip, or `None` when the record
/// is not one the mount rule allows.
///
/// **One rule with the player.** A rectangle takes exactly the quarter turns; a
/// circle takes any whole degree below 360 and is composed by rotating it.
pub fn video_rotation(rotation: u16, shape: PanelShape) -> Option<PanelRotation> {
    if let Some(turn) = video_turn(rotation) {
        return Some(PanelRotation::Turn(turn));
    }
    if shape != PanelShape::Circle || rotation >= 360 {
        return None;
    }
    Some(PanelRotation::Angle(f64::from(rotation)))
}

/// The box the media layer is laid out in before it turns. A quarter turn
/// swaps the wire geometry's edges, because the turned layer has to cover the
/// same glass the upright one did.
pub fn turned_box(box_: (f32, f32), turn: PanelTurn) -> (f32, f32) {
    match turn {
        PanelTurn::Cw | PanelTurn::Ccw => (box_.1, box_.0),
        PanelTurn::None | PanelTurn::Half => box_,
    }
}

/// The layout box a whole rotation asks for.
pub fn rotation_box(box_: (f32, f32), rotation: PanelRotation) -> (f32, f32) {
    match rotation {
        PanelRotation::Turn(turn) => turned_box(box_, turn),
        PanelRotation::Angle(_) => box_,
    }
}

/// The fragment that turns the scaled picture onto the wire frame; empty when
/// the display is upright.
pub fn turn_filter(turn: PanelTurn) -> &'static str {
    match turn {
        PanelTurn::Cw => ",transpose=1",
        PanelTurn::Half => ",hflip,vflip",
        PanelTurn::Ccw => ",transpose=2",
        PanelTurn::None => "",
    }
}

/// The fragment that mirrors the placed media in its own frame. Applied before
/// the turn, because the page mirrors the layer inside the box and turns the
/// box as a whole.
pub fn flip_filter(flip_x: bool, flip_y: bool) -> String {
    format!(
        "{}{}",
        if flip_x { ",hflip" } else { "" },
        if flip_y { ",vflip" } else { "" }
    )
}

/// The fragment that turns an arbitrary circle angle, cropped back to the wire
/// frame afterwards. The corners the rotation exposes are outside a round
/// display's glass.
pub fn angle_filter(degrees: f64, box_: (f32, f32)) -> String {
    format!(
        ",rotate={}*PI/180:ow=rotw(iw):oh=roth(ih):c=black,scale={}:{}:force_original_aspect_ratio=increase:force_divisible_by=2,crop={}:{}",
        number(degrees),
        number(f64::from(box_.0)),
        number(f64::from(box_.1)),
        number(f64::from(box_.0)),
        number(f64::from(box_.1)),
    )
}

/// The sentence an encoder reports for an angle it cannot compose.
pub fn turn_refusal(rotation: u16) -> String {
    format!("The panel turns its clip by quarter turns only; {rotation}° is not one.")
}

/// The filter that puts a source onto the panel's box, already turned.
pub fn panel_filter(
    box_: (f32, f32),
    placement: &Placement,
    source: Option<SourceFacts>,
    rotation: PanelRotation,
) -> EncodeFilter {
    if placement.composition.is_some() || placement.fill.is_some() {
        return EncodeFilter {
            filter: compose_filter_graph(box_, placement, source, rotation),
            complex: true,
            duration_seconds: composed_seconds(source),
        };
    }

    let composition = background::resolve(
        box_,
        placement.composition.as_ref(),
        source.map(|source| (source.width, source.height)),
        placement.geometry(),
    );
    let picture = match composition.media {
        None => format!(
            "scale={}:{}:force_original_aspect_ratio=increase:force_divisible_by=2,crop={}:{}",
            number(f64::from(box_.0)),
            number(f64::from(box_.1)),
            number(f64::from(box_.0)),
            number(f64::from(box_.1)),
        ),
        Some(media) => format!(
            "crop={}:{}:{}:{},scale={}:{}:force_divisible_by=2",
            number(f64::from(media.crop.width)),
            number(f64::from(media.crop.height)),
            number(f64::from(media.crop.x)),
            number(f64::from(media.crop.y)),
            number(f64::from(box_.0)),
            number(f64::from(box_.1)),
        ),
    };
    EncodeFilter {
        // `setsar=1` pins the output to square pixels: `scale` keeps the
        // source's display aspect and would otherwise hand a non-square sample
        // aspect on.
        filter: format!(
            "{picture},setsar=1{}{}",
            flip_filter(placement.flip_x, placement.flip_y),
            turn_of(rotation, box_),
        ),
        complex: false,
        duration_seconds: None,
    }
}

/// The display's turn, as the fragment that composes it onto the wire frame.
fn turn_of(rotation: PanelRotation, box_: (f32, f32)) -> String {
    match rotation {
        PanelRotation::Turn(turn) => turn_filter(turn).to_owned(),
        PanelRotation::Angle(degrees) => angle_filter(degrees, box_),
    }
}

/// The composed graph, in `box` and with the turn applied last.
pub fn compose_filter_graph(
    box_: (f32, f32),
    placement: &Placement,
    source: Option<SourceFacts>,
    rotation: PanelRotation,
) -> String {
    let composition = background::resolve(
        box_,
        placement.composition.as_ref(),
        source.map(|source| (source.width, source.height)),
        placement.geometry(),
    );
    let (dest, picture) = match composition.media {
        Some(media) => (
            media.dest,
            format!(
                "crop={}:{}:{}:{},scale={}:{}:force_divisible_by=2",
                number(f64::from(media.crop.width)),
                number(f64::from(media.crop.height)),
                number(f64::from(media.crop.x)),
                number(f64::from(media.crop.y)),
                number(width_of(media.dest.width)),
                number(height_of(media.dest.height)),
            ),
        ),
        None => {
            let dest = background::visible_area(composition.region, box_);
            (
                dest,
                format!(
                    "scale={}:{}:force_original_aspect_ratio=increase:force_divisible_by=2,crop={}:{}",
                    number(width_of(dest.width)),
                    number(height_of(dest.height)),
                    number(width_of(dest.width)),
                    number(height_of(dest.height)),
                ),
            )
        }
    };
    let width = width_of(dest.width);
    let height = height_of(dest.height);
    let x = dest.left.round() as i64;
    let y = dest.top.round() as i64;

    let mut parts = vec![
        // The mirror belongs to the media, not the fill behind it, so it is
        // applied before the overlay and before the display's turn.
        format!(
            "[0:v]{picture},setsar=1{}[media]",
            flip_filter(placement.flip_x, placement.flip_y)
        ),
        format!(
            "{}[fill]",
            fill_source(placement.fill.as_ref(), box_.0, box_.1)
        ),
    ];

    // A centred band has two ramps. `alphamerge` replaces the alpha it is
    // given, so applying them one after another would keep only the last;
    // multiply the grayscale masks first, which is the DOM's `intersect`.
    let mut overlay_input = "media".to_owned();
    let masks: Vec<String> = composition
        .masks
        .iter()
        .filter_map(|mask| mask_source(mask, x, y, width, height))
        .collect();
    if !masks.is_empty() {
        for (index, mask) in masks.iter().enumerate() {
            parts.push(format!("{mask}[mask{index}]"));
        }
        // `alphamerge` copies luma into the alpha channel, so merging one mask
        // per source would leave only the last ramp. Multiply the grayscale
        // masks into one first: a centred box fades into the fill on both
        // opposite gaps.
        let mut mask_input = "mask0".to_owned();
        for index in 1..masks.len() {
            let combined = format!("masks{index}");
            parts.push(format!(
                "[{mask_input}][mask{index}]blend=all_mode=multiply[{combined}]"
            ));
            mask_input = combined;
        }
        parts.push(format!(
            // The mask never ends; the source does.
            "[{overlay_input}][{mask_input}]alphamerge=shortest=1[composed]"
        ));
        overlay_input = "composed".to_owned();
    }

    let turn = turn_of(rotation, box_);
    parts.push(format!(
        "[fill][{overlay_input}]overlay=x={x}:y={y}:shortest=1[turned]"
    ));
    parts.push(format!(
        "[turned]{}[out]",
        turn.strip_prefix(',').unwrap_or("null")
    ));
    parts.join(";")
}

fn width_of(value: f32) -> f64 {
    f64::from(value.max(1.0).round())
}

fn height_of(value: f32) -> f64 {
    f64::from(value.max(1.0).round())
}

/// A solid or linear-gradient source covering the whole box.
///
/// **The ramp is computed per pixel, not by `gradients`.** That source spaces
/// its colours evenly and accepts at most eight, so authored stop offsets and
/// the alpha ramp of a scrim came out as different bands and a different fade.
/// `geq` can read the authored stops directly, which makes the baked fill the
/// one the page draws.
fn fill_source(fill: Option<&Paint>, width: f32, height: f32) -> String {
    let size = format!("{}x{}", number(f64::from(width)), number(f64::from(height)));
    match fill {
        None => format!("color=c=0x000000:s={size}"),
        Some(Paint::Solid(color)) => format!("color=c={}:s={size}", ffmpeg_color(*color)),
        Some(Paint::Linear { from, to, stops }) => {
            gradient_source(*from, *to, stops, width, height)
        }
    }
}

fn ffmpeg_color(color: Color) -> String {
    if color.a == 255 {
        format!("0x{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "0x{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct Stop {
    offset: f64,
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}

fn stop_of(stop: &GradientStop) -> Stop {
    Stop {
        offset: f64::from(stop.offset).clamp(0.0, 1.0),
        red: f64::from(stop.color.r) / 255.0,
        green: f64::from(stop.color.g) / 255.0,
        blue: f64::from(stop.color.b) / 255.0,
        alpha: f64::from(stop.color.a) / 255.0,
    }
}

fn gradient_source(
    from: (f32, f32),
    to: (f32, f32),
    stops: &[GradientStop],
    width: f32,
    height: f32,
) -> String {
    let size = format!("{}x{}", number(f64::from(width)), number(f64::from(height)));
    let mut stops: Vec<Stop> = stops.iter().map(stop_of).collect();
    stops.sort_by(|first, second| first.offset.total_cmp(&second.offset));
    if stops.is_empty() {
        return format!("color=c=0x000000:s={size}");
    }
    if stops.len() == 1 {
        return format!("color=c={}", ffmpeg_color(stops[0].to_color()));
    }

    let x1 = f64::from(from.0);
    let y1 = f64::from(from.1);
    let dx = f64::from(to.0) - x1;
    let dy = f64::from(to.1) - y1;
    let length = dx * dx + dy * dy;
    let position = if length.is_finite() && length > 0.0 {
        format!(
            "clip(((X-{})*{}+(Y-{})*{})/{},0,1)",
            number(x1),
            number(dx),
            number(y1),
            number(dy),
            number(length)
        )
    } else {
        "0".to_owned()
    };

    let alpha = piecewise(&stops, &position, |stop| stop.alpha, None);
    // CSS and the canvas interpolate a gradient in premultiplied alpha; doing
    // the same keeps a translucent stop from pulling its neighbours toward
    // black.
    let channel = |index: usize| {
        piecewise(
            &stops,
            &position,
            |stop| [stop.red, stop.green, stop.blue][index] * stop.alpha,
            Some(&alpha),
        )
    };

    format!(
        "color=c=black@0.0:s={size},format=rgba,geq=r='255*{}':g='255*{}':b='255*{}':a='255*{}'",
        channel(0),
        channel(1),
        channel(2),
        alpha
    )
}

impl Stop {
    fn to_color(self) -> Color {
        Color {
            r: (self.red * 255.0).round() as u8,
            g: (self.green * 255.0).round() as u8,
            b: (self.blue * 255.0).round() as u8,
            a: (self.alpha * 255.0).round() as u8,
        }
    }
}

/// The stops as one nested ffmpeg expression over `position`, clamped at both
/// ends.
fn piecewise(
    stops: &[Stop],
    position: &str,
    value: impl Fn(&Stop) -> f64,
    divisor: Option<&str>,
) -> String {
    let lerp = |first: &Stop, second: &Stop| -> String {
        let span = second.offset - first.offset;
        let ratio = if span > 0.0 {
            format!(
                "clip(({position}-{})/{},0,1)",
                number(first.offset),
                number(span)
            )
        } else {
            "1".to_owned()
        };
        let (from, to) = (value(first), value(second));
        let mixed = format!(
            "{}+({}-{})*{}",
            number(from),
            number(to),
            number(from),
            ratio
        );
        match divisor {
            None => format!("({mixed})"),
            Some(divisor) => format!("if(gt({divisor},0),({mixed})/({divisor}),0)"),
        }
    };

    let mut expression = number(value(&stops[stops.len() - 1]));
    for index in (0..stops.len().saturating_sub(1)).rev() {
        let first = &stops[index];
        let second = &stops[index + 1];
        expression = format!(
            "if(lt({position},{}),{},{}))",
            number(second.offset),
            lerp(first, second),
            expression
        );
    }
    expression
}

/// A finite number as an ffmpeg expression literal.
fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let rounded = (value * 10_000.0).round() / 10_000.0;
    format!("{rounded}")
}

/// One alpha ramp, in the frame the media is laid in, or nothing when the box
/// cannot see the ramp at all.
fn mask_source(mask: &Mask, x: i64, y: i64, width: f64, height: f64) -> Option<String> {
    let vertical = mask.x1 == mask.x2;
    let size = if vertical { height } else { width };
    let across = if vertical { x as f64 } else { y as f64 };
    let from = f64::from(if vertical { mask.y1 } else { mask.x1 }) - across;
    let to = f64::from(if vertical { mask.y2 } else { mask.x2 }) - across;
    let ramp = visible_ramp(from, to, size)?;

    let stops: Vec<String> = (0..background::RAMP_STOPS)
        .map(|index| {
            let fraction = ramp.t0
                + ((ramp.t1 - ramp.t0) * index as f64) / (background::RAMP_STOPS - 1) as f64;
            let value = (background::blend_ramp(fraction as f32) * 255.0).round() as u8;
            format!("c{index}=0x{value:02x}{value:02x}{value:02x}")
        })
        .collect();
    let head = format!(
        "gradients=s={}x{}:{}:nb_colors={}",
        number(width),
        number(height),
        stops.join(":"),
        background::RAMP_STOPS
    );
    let across_point = point(
        if vertical {
            f64::from(mask.x1) - x as f64
        } else {
            f64::from(mask.y1) - y as f64
        },
        if vertical { width } else { height },
    );
    let start = point(ramp.start, size);
    let end = point(ramp.end, size);
    Some(if vertical {
        format!("{head}:x0={across_point}:y0={start}:x1={across_point}:y1={end},{FROZEN}")
    } else {
        format!("{head}:x0={start}:y0={across_point}:x1={end}:y1={across_point},{FROZEN}")
    })
}

struct Ramp {
    start: f64,
    end: f64,
    t0: f64,
    t1: f64,
}

/// The part of a ramp that lies inside a frame, with the ramp fraction it
/// covers.
fn visible_ramp(from: f64, to: f64, size: f64) -> Option<Ramp> {
    let span = to - from;
    if size <= 0.0 || span.abs() < 1e-9 {
        return None;
    }
    let at = |value: f64| (value - from) / span;
    let (t0, t1) = if span > 0.0 {
        (at(0.0).max(0.0), at(size).min(1.0))
    } else {
        (at(size).max(0.0), at(0.0).min(1.0))
    };
    if t1 <= t0 {
        return None;
    }
    Some(Ramp {
        start: from + span * t0,
        end: from + span * t1,
        t0,
        t1,
    })
}

/// A gradient line endpoint inside the source. ffmpeg's `gradients` reads the
/// line as coordinates: an endpoint exactly on the far edge is outside the
/// frame and paints the whole ramp wrong.
fn point(value: f64, size: f64) -> String {
    number(value.clamp(0.0, (size - 1.0).max(0.0)).round())
}

/// Holds a generated source on its first frame; `gradients` animates by
/// default and accepts no zero speed.
const FROZEN: &str = "loop=loop=-1:size=1:start=0";

/// A clip length the caller can bound a never-ending fill source with.
pub fn composed_seconds(source: Option<SourceFacts>) -> Option<f64> {
    source
        .and_then(|source| source.duration_ms)
        .filter(|duration| *duration > 0.0)
        .map(|duration| duration / 1000.0)
}

/// The composed graph's visible area, for callers that need the same rect.
pub fn destination(box_: (f32, f32), composition: &Composition) -> Rect {
    composition
        .media
        .map(|media| media.dest)
        .unwrap_or_else(|| background::visible_area(composition.region, box_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_model::document::{BackgroundAnchor, BackgroundComposition};

    fn placement() -> Placement {
        Placement {
            scale: 1.0,
            ..Placement::default()
        }
    }

    fn composition(
        anchor: BackgroundAnchor,
        extent: Option<f64>,
        fade: Option<f64>,
    ) -> BackgroundComposition {
        BackgroundComposition {
            anchor,
            extent,
            fade,
        }
    }

    fn solid(hex: Color) -> Paint {
        Paint::Solid(hex)
    }

    fn filter_for(
        box_: (f32, f32),
        placement: &Placement,
        rotation: u16,
        shape: PanelShape,
    ) -> EncodeFilter {
        let rotation = video_rotation(rotation, shape).expect("a composable rotation");
        // The caller lays the layer out in the box the mount asks for first.
        panel_filter(rotation_box(box_, rotation), placement, None, rotation)
    }

    #[test]
    fn fits_the_box_the_mount_puts_the_clip_in_then_turns_it() {
        let upright = filter_for((800.0, 480.0), &placement(), 0, PanelShape::Rect);
        assert_eq!(
            upright.filter,
            "scale=800:480:force_original_aspect_ratio=increase:force_divisible_by=2,crop=800:480,setsar=1"
        );

        // A quarter turn swaps the layer's box before it turns.
        let turned = filter_for((800.0, 480.0), &placement(), 90, PanelShape::Rect);
        assert_eq!(
            turned.filter,
            "scale=480:800:force_original_aspect_ratio=increase:force_divisible_by=2,crop=480:800,setsar=1,transpose=1"
        );

        let counter = filter_for((800.0, 480.0), &placement(), 270, PanelShape::Rect);
        assert!(counter.filter.ends_with(",transpose=2"));

        let upside_down = filter_for((800.0, 480.0), &placement(), 180, PanelShape::Rect);
        assert!(upside_down.filter.ends_with(",hflip,vflip"));
        assert!(upside_down.filter.contains("crop=800:480"));
    }

    #[test]
    fn refuses_a_rectangles_angle_with_the_sentence_naming_the_rule() {
        assert!(video_rotation(45, PanelShape::Rect).is_none());
        assert_eq!(
            turn_refusal(45),
            "The panel turns its clip by quarter turns only; 45° is not one."
        );
    }

    #[test]
    fn rotates_a_circles_non_quarter_angle_and_crops_back_to_the_frame() {
        let filter = filter_for((800.0, 480.0), &placement(), 45, PanelShape::Circle);
        assert!(
            filter.filter.contains("rotate=45*PI/180"),
            "{}",
            filter.filter
        );
        assert!(filter.filter.contains("crop=800:480"));
        assert!(!filter.filter.contains("transpose"));
    }

    #[test]
    fn crops_the_source_to_the_medias_band_and_composites_over_a_solid_fill() {
        let placed = Placement {
            composition: Some(composition(BackgroundAnchor::Left, Some(0.65), Some(0.12))),
            fill: Some(solid(Color::rgb(0x10, 0x14, 0x18))),
            ..placement()
        };
        let rotation = PanelRotation::Turn(PanelTurn::None);
        let filter = compose_filter_graph((800.0, 480.0), &placed, None, rotation);
        assert!(filter.contains("crop="), "{filter}");
        assert!(filter.contains("color=c=0x101418"), "{filter}");
        assert!(filter.contains("overlay"), "{filter}");
        // A left band keeping 65% of the 800px box is 520px wide and full
        // height.
        assert!(filter.contains("scale=520:480"), "{filter}");
    }

    #[test]
    fn draws_an_authored_gradient_as_a_geq_expression_over_the_stops() {
        let placed = Placement {
            composition: Some(composition(BackgroundAnchor::Top, Some(0.5), None)),
            fill: Some(Paint::Linear {
                from: (0.0, 0.0),
                to: (0.0, 480.0),
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: Color::rgb(0, 0, 0),
                    },
                    GradientStop {
                        offset: 1.0,
                        color: Color::rgb(255, 255, 255),
                    },
                ],
            }),
            ..placement()
        };
        let filter = compose_filter_graph(
            (800.0, 480.0),
            &placed,
            None,
            PanelRotation::Turn(PanelTurn::None),
        );
        assert!(filter.contains("geq="), "{filter}");
        assert!(!filter.contains("gradients="), "{filter}");
    }

    #[test]
    fn draws_the_shared_eased_ramp_and_holds_the_gradient_source_still() {
        let placed = Placement {
            composition: Some(composition(BackgroundAnchor::Left, Some(0.35), Some(0.4))),
            ..placement()
        };
        let filter = compose_filter_graph(
            (800.0, 480.0),
            &placed,
            None,
            PanelRotation::Turn(PanelTurn::None),
        );
        assert!(filter.contains("gradients="), "{filter}");
        assert!(filter.contains("nb_colors=8"), "{filter}");
        assert!(filter.contains("alphamerge=shortest=1"), "{filter}");
        assert!(filter.contains("loop=loop=-1:size=1:start=0"), "{filter}");
        for (_, alpha) in background::ramp_stops() {
            let value = (alpha * 255.0).round() as u8;
            let stop = format!("0x{value:02x}{value:02x}{value:02x}");
            assert!(filter.contains(&stop), "{stop} missing from {filter}");
        }
    }

    #[test]
    fn a_composed_graph_takes_the_sources_own_length() {
        let placed = Placement {
            composition: Some(composition(BackgroundAnchor::Top, Some(0.5), None)),
            ..placement()
        };
        let facts = SourceFacts {
            width: 320.0,
            height: 180.0,
            duration_ms: Some(2_000.0),
        };
        let filter = panel_filter(
            (800.0, 480.0),
            &placed,
            Some(facts),
            PanelRotation::Turn(PanelTurn::None),
        );
        assert!(filter.complex);
        assert_eq!(filter.duration_seconds, Some(2.0));
    }

    #[test]
    fn keeps_the_plain_path_when_there_is_no_composition() {
        let filter = filter_for((800.0, 480.0), &placement(), 0, PanelShape::Rect);
        assert!(!filter.complex);
        assert!(filter.duration_seconds.is_none());
    }
}
