//! Text layout: a text node becomes glyph-run geometry.
//!
//! One owner for every decision a backend must not make: which font a run
//! resolves to, how wide a line is, where a line breaks, and where each
//! resulting segment sits inside the node's box. Backends shape the segment
//! they are handed and paint it; they never re-wrap.

use serde::Serialize;

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, LayoutGlyph, Metrics, Shaping,
    Style as TextPosture, Weight,
};

// Re-exported so a backend shapes with the same settings this module measured
// with, without depending on the shaper itself.
pub use cosmic_text::{
    Buffer as ShapeBuffer, Color as TextColor, Metrics as TextMetrics, Shaping as TextShaping,
    SwashCache,
};

use crate::color::Color;

/// The system font database plus the shaping state every layout needs.
///
/// One per renderer, not one per frame: loading the database is the expensive
/// part, and a layout borrows it for the length of a shaping call.
pub struct Fonts {
    system: FontSystem,
}

impl Fonts {
    pub fn new() -> Self {
        Self {
            system: FontSystem::new(),
        }
    }

    /// The font database a backend shapes with. Text positions come from this
    /// crate, so a backend that shapes with the same database advances glyphs
    /// the same way the layout measured them.
    pub fn system_mut(&mut self) -> &mut FontSystem {
        &mut self.system
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

/// How a run is drawn: the face it names and the size and colour it resolves
/// to. Positions are the layout's business, so they are not here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextStyle {
    /// The family the run's type token names. Absent takes the shaper's own
    /// sans-serif choice.
    pub family: Option<String>,
    /// A CSS-style weight: 400 is regular, 700 is bold.
    pub weight: u16,
    pub italic: bool,
    /// In artboard units, so scaling is the backend's.
    pub size: f32,
    pub color: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: None,
            weight: 400,
            italic: false,
            size: 16.0,
            color: Color::rgb(255, 255, 255),
        }
    }
}

/// A piece of the paragraph, before layout.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRunPlan {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflow {
    Visible,
    Clip,
    Ellipsis,
}

/// A laid-out paragraph in the text node's own box space.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextLayout {
    /// Segments in paint order, each with its left edge and line top.
    pub segments: Vec<TextSegment>,
    /// The box an overflowing paragraph is cut to, when it is cut.
    pub clip: Option<(f32, f32)>,
}

/// One run's share of one line, positioned and ready to shape.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextSegment {
    /// Left edge and line top, in the box's own coordinates. A backend that
    /// paints baselines reads the ascent from its own shaping, so the line box
    /// is the unit both sides agree on.
    pub origin: (f32, f32),
    pub text: String,
    pub style: TextStyle,
}

/// Everything a text node says about how its paragraph is arranged.
pub struct TextRequest<'a> {
    pub runs: &'a [TextRunPlan],
    /// The wrapping width and clipping height, when the node declares a box.
    /// Absent is auto width: a paragraph takes exactly the space it needs.
    pub box_: Option<(f32, f32)>,
    pub align: TextAlign,
    pub vertical_align: VerticalAlign,
    pub overflow: TextOverflow,
    /// Stack characters upright, one per line.
    pub vertical: bool,
    /// Line spacing as a multiple of the largest size on the line.
    pub line_height: f32,
}

const DEFAULT_LINE_HEIGHT: f32 = 1.2;

/// Shape, wrap and place the paragraph.
pub fn layout(fonts: &mut Fonts, request: &TextRequest<'_>) -> TextLayout {
    let line_height = if request.line_height > 0.0 {
        request.line_height
    } else {
        DEFAULT_LINE_HEIGHT
    };
    if request.vertical {
        return vertical(fonts, request, line_height);
    }

    let size = request
        .runs
        .iter()
        .map(|run| run.style.size)
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let width = request.box_.map(|(width, _)| width);
    let metrics = Metrics::new(size, size * line_height);
    let mut buffer = Buffer::new(&mut fonts.system, metrics);
    {
        let mut buffer = buffer.borrow_with(&mut fonts.system);
        buffer.set_size(width, None);
        buffer.set_rich_text(
            request.runs.iter().enumerate().map(|(index, run)| {
                (
                    run.text.as_str(),
                    attrs(&run.style).metadata(index),
                )
            }),
            &Attrs::new(),
            Shaping::Advanced,
            Some(align(request.align)),
        );
        buffer.shape_until_scroll(false);
    }
    buffer.shape_until_scroll(&mut fonts.system, false);

    let mut segments: Vec<TextSegment> = Vec::new();
    let mut lines = 0.0_f32;
    let mut dropped = false;
    for run in buffer.layout_runs() {
        if let Some((_, box_height)) = request.box_
            && run.line_top + run.line_height > box_height + 0.5
        {
            dropped = true;
            break;
        }
        lines += 1.0;
        // One span's share of one line: contiguous by construction, so the
        // first glyph's start and the last glyph's end bracket the text.
        let mut spans: Vec<usize> = Vec::new();
        for glyph in run.glyphs {
            if spans.last() != Some(&glyph.metadata) {
                spans.push(glyph.metadata);
            }
        }
        for span in spans {
            let glyphs: Vec<&LayoutGlyph> = run
                .glyphs
                .iter()
                .filter(|glyph| glyph.metadata == span)
                .collect();
            let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
                continue;
            };
            segments.push(TextSegment {
                origin: (first.x, run.line_top),
                text: run.text[first.start..last.end].to_string(),
                style: request.runs[span].style.clone(),
            });
        }
    }

    let mut layout = TextLayout {
        segments,
        clip: match (request.overflow, request.box_) {
            (TextOverflow::Clip, Some(box_)) | (TextOverflow::Ellipsis, Some(box_)) => Some(box_),
            _ => None,
        },
    };
    if dropped && request.overflow == TextOverflow::Ellipsis {
        elide(fonts, &mut layout, request);
    }
    let total = lines * size * line_height;
    if let Some((_, box_height)) = request.box_ {
        let offset = vertical_offset(request.vertical_align, box_height, total);
        if offset != 0.0 {
            for segment in &mut layout.segments {
                segment.origin.1 += offset;
            }
        }
    }
    layout
}

/// The shaping attributes a segment's style asks for.
pub fn attrs(style: &TextStyle) -> Attrs<'_> {
    let mut attrs = Attrs::new()
        .weight(Weight(style.weight))
        .color(cosmic_color(style.color));
    if let Some(family) = &style.family {
        attrs = attrs.family(Family::Name(family));
    }
    if style.italic {
        attrs = attrs.style(TextPosture::Italic);
    }
    attrs.metrics(Metrics::new(style.size, style.size * DEFAULT_LINE_HEIGHT))
}

fn cosmic_color(color: Color) -> cosmic_text::Color {
    cosmic_text::Color::rgba(color.r, color.g, color.b, color.a)
}

fn align(align: TextAlign) -> Align {
    match align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    }
}

fn vertical_offset(align: VerticalAlign, box_height: f32, total: f32) -> f32 {
    let spare = box_height - total;
    match align {
        VerticalAlign::Top => 0.0,
        VerticalAlign::Middle => spare / 2.0,
        VerticalAlign::Bottom => spare,
    }
}

/// Trim the last kept line until it and an ellipsis fit, then draw the
/// ellipsis. The lines after it are already dropped, so this is the only
/// marking a truncated paragraph gets.
fn elide(fonts: &mut Fonts, layout: &mut TextLayout, request: &TextRequest<'_>) {
    let Some((box_width, _)) = request.box_ else {
        return;
    };
    let Some(last) = layout.segments.last_mut() else {
        return;
    };
    let style = last.style.clone();
    let original = last.text.clone();
    let mut candidate = original.clone();
    while !candidate.is_empty() {
        candidate.pop();
        let measured = measure(fonts, &candidate, &style);
        if measured + measure(fonts, "\u{2026}", &style) <= box_width - last.origin.0 {
            last.text = format!("{candidate}\u{2026}");
            return;
        }
    }
    last.text = "\u{2026}".into();
}

/// The width one line of this style occupies, for a caller placing something
/// else beside it.
pub fn advance_width(fonts: &mut Fonts, text: &str, style: &TextStyle) -> f32 {
    measure(fonts, text, style)
}

fn measure(fonts: &mut Fonts, text: &str, style: &TextStyle) -> f32 {
    let metrics = Metrics::new(style.size, style.size);
    let mut buffer = Buffer::new(&mut fonts.system, metrics);
    {
        let mut buffer = buffer.borrow_with(&mut fonts.system);
        buffer.set_size(None, None);
        buffer.set_text(text, &attrs(style), Shaping::Advanced, None);
    }
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0_f32, f32::max)
}

/// Characters stacked upright, one line each, so a vertical clock reads down
/// the box instead of across it.
fn vertical(fonts: &mut Fonts, request: &TextRequest<'_>, line_height: f32) -> TextLayout {
    let metrics_of = |style: &TextStyle| Metrics::new(style.size, style.size * line_height);
    let mut segments = Vec::new();
    let mut y = 0.0;
    for run in request.runs {
        for character in run.text.chars() {
            let text = character.to_string();
            let metrics = metrics_of(&run.style);
            let mut buffer = Buffer::new(&mut fonts.system, metrics);
            let width = {
                let mut buffer = buffer.borrow_with(&mut fonts.system);
                buffer.set_size(None, None);
                buffer.set_text(&text, &attrs(&run.style), Shaping::Advanced, None);
                buffer.layout_runs().next().map_or(0.0, |line| line.line_w)
            };
            let x = match (request.align, request.box_) {
                (TextAlign::Left, _) => 0.0,
                (TextAlign::Center, Some((box_width, _))) => (box_width - width) / 2.0,
                (TextAlign::Right, Some((box_width, _))) => box_width - width,
                (TextAlign::Center | TextAlign::Right, None) => 0.0,
            };
            segments.push(TextSegment {
                origin: (x, y),
                text,
                style: run.style.clone(),
            });
            y += run.style.size * line_height;
        }
    }
    let total = y;
    if let Some((_, box_height)) = request.box_ {
        let offset = vertical_offset(request.vertical_align, box_height, total);
        for segment in &mut segments {
            segment.origin.1 += offset;
        }
    }
    TextLayout { segments, clip: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::new()
    }

    fn run(text: &str, size: f32) -> TextRunPlan {
        TextRunPlan {
            text: text.into(),
            style: TextStyle {
                size,
                ..TextStyle::default()
            },
        }
    }

    fn request<'a>(runs: &'a [TextRunPlan]) -> TextRequest<'a> {
        TextRequest {
            runs,
            box_: None,
            align: TextAlign::Left,
            vertical_align: VerticalAlign::Top,
            overflow: TextOverflow::Visible,
            vertical: false,
            line_height: DEFAULT_LINE_HEIGHT,
        }
    }

    #[test]
    fn a_single_run_keeps_its_text_and_sits_at_the_origin() {
        let runs = vec![run("Hello", 20.0)];
        let laid = layout(&mut fonts(), &request(&runs));
        assert_eq!(laid.segments.len(), 1);
        assert_eq!(laid.segments[0].text, "Hello");
        assert_eq!(laid.segments[0].origin.0, 0.0);
        assert_eq!(laid.segments[0].style.size, 20.0);
    }

    #[test]
    fn two_runs_on_one_line_are_two_segments() {
        let runs = vec![run("72", 20.0), run(" C", 10.0)];
        let laid = layout(&mut fonts(), &request(&runs));
        assert_eq!(laid.segments.len(), 2);
        assert!(laid.segments[1].origin.0 > 0.0);
        assert_eq!(laid.segments[1].origin.1, laid.segments[0].origin.1);
    }

    #[test]
    fn a_narrow_box_wraps_onto_a_second_line() {
        let runs = vec![run("one two three four", 20.0)];
        let mut request = request(&runs);
        request.box_ = Some((60.0, 200.0));
        let laid = layout(&mut fonts(), &request);
        let baselines: std::collections::BTreeSet<i32> = laid
            .segments
            .iter()
            .map(|segment| segment.origin.1 as i32)
            .collect();
        assert!(baselines.len() > 1, "{:?}", laid.segments);
    }

    #[test]
    fn centring_pushes_the_line_inward() {
        let runs = vec![run("hi", 20.0)];
        let mut centred = request(&runs);
        centred.box_ = Some((200.0, 40.0));
        centred.align = TextAlign::Center;
        let laid = layout(&mut fonts(), &centred);
        assert!(laid.segments[0].origin.0 > 0.0);

        let mut right = request(&runs);
        right.box_ = Some((200.0, 40.0));
        right.align = TextAlign::Right;
        let laid = layout(&mut fonts(), &right);
        assert!(laid.segments[0].origin.0 > 50.0);
    }

    #[test]
    fn middle_alignment_moves_the_paragraph_down_the_box() {
        let runs = vec![run("hi", 20.0)];
        let mut request = request(&runs);
        request.box_ = Some((100.0, 100.0));
        request.vertical_align = VerticalAlign::Middle;
        let laid = layout(&mut fonts(), &request);
        assert!(laid.segments[0].origin.1 > 20.0);
    }

    #[test]
    fn a_clipped_box_cuts_the_paragraph_and_says_where() {
        let runs = vec![run("one two three four five six", 20.0)];
        let mut request = request(&runs);
        request.box_ = Some((60.0, 40.0));
        request.overflow = TextOverflow::Clip;
        let laid = layout(&mut fonts(), &request);
        assert_eq!(laid.clip, Some((60.0, 40.0)));
        let baselines: std::collections::BTreeSet<i32> = laid
            .segments
            .iter()
            .map(|segment| segment.origin.1 as i32)
            .collect();
        assert!(baselines.len() <= 2, "{:?}", laid.segments);
    }

    #[test]
    fn an_elided_paragraph_ends_in_an_ellipsis() {
        let runs = vec![run("one two three four five six", 20.0)];
        let mut request = request(&runs);
        request.box_ = Some((60.0, 40.0));
        request.overflow = TextOverflow::Ellipsis;
        let laid = layout(&mut fonts(), &request);
        let last = laid.segments.last().expect("a line survives");
        assert!(last.text.ends_with('\u{2026}'), "{:?}", last.text);
    }

    #[test]
    fn vertical_mode_stacks_one_character_per_line() {
        let runs = vec![run("12:34", 20.0)];
        let mut request = request(&runs);
        request.vertical = true;
        let laid = layout(&mut fonts(), &request);
        assert_eq!(laid.segments.len(), 5);
        assert_eq!(laid.segments[0].text, "1");
        assert!(laid.segments[1].origin.1 > laid.segments[0].origin.1);
    }

    #[test]
    fn a_zero_line_height_falls_back_rather_than_overlapping_lines() {
        let runs = vec![run("a b", 20.0)];
        let mut request = request(&runs);
        request.line_height = 0.0;
        request.box_ = Some((20.0, 100.0));
        let laid = layout(&mut fonts(), &request);
        assert!(laid.segments.len() > 1);
        assert!(laid.segments[1].origin.1 > laid.segments[0].origin.1);
    }
}
