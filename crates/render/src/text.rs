//! Text layout: a text node becomes glyph-run geometry.
//!
//! One owner for every decision a backend must not make: which font a run
//! resolves to, how wide a line is, where a line breaks, and where each
//! resulting segment sits inside the node's box. Backends shape the segment
//! they are handed and paint it; they never re-wrap.

use std::sync::Arc;

use serde::Serialize;

use cosmic_text::{
    Align, Attrs, Buffer, Family, FontSystem, LayoutGlyph, Metrics, Shaping, Style as TextPosture,
    Weight, fontdb,
};
use unicode_segmentation::UnicodeSegmentation;
use yunari_model::document::{AssetKind, FontFormat, ThemeDocument};

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

    /// Register every face `document` declares, reading each font asset's bytes
    /// through `read` (called with the path as the document writes it).
    ///
    /// A face whose bytes cannot be read, decompressed or parsed is answered as
    /// one refusal sentence and left unregistered: a run that names it falls
    /// back to the shaper's own family rather than painting wrong glyphs.
    pub fn load_faces(
        &mut self,
        document: &ThemeDocument,
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Vec<String> {
        let mut refusals = Vec::new();
        for asset in &document.assets {
            if asset.kind != AssetKind::Font {
                continue;
            }
            let Some(bytes) = read(&asset.path) else {
                refusals.push(format!("The face at {:?} could not be read.", asset.path));
                continue;
            };
            let format = asset.format.unwrap_or_else(|| format_of(&asset.path));
            let font = match format {
                FontFormat::Woff2 => wuff::decompress_woff2(&bytes).map_err(|_| "an invalid woff2"),
                FontFormat::Woff => wuff::decompress_woff1(&bytes).map_err(|_| "an invalid woff"),
                FontFormat::Ttf | FontFormat::Otf => Ok(bytes),
            };
            match font {
                Ok(font) => {
                    let source = fontdb::Source::Binary(Arc::new(font));
                    let ids = self.system.db_mut().load_font_source(source);
                    if ids.is_empty() {
                        refusals.push(format!(
                            "The face at {:?} carries no readable font.",
                            asset.path
                        ));
                    } else if let Some(declared) = asset
                        .family
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    {
                        alias_family(&mut self.system, &ids, declared);
                    }
                }
                Err(reason) => refusals.push(format!("The face at {:?} is {reason}.", asset.path)),
            }
        }
        refusals
    }
}

/// Register a packaged face under the family its theme declares.
///
/// A font's name table can disagree with the asset metadata: a face the theme
/// calls `Manrope` may name itself `Manrope ExtraLight`, and fontdb matches
/// only the name table, so a run that names the token family silently falls
/// back to a system face. fontdb has no mutable face API, so rebuild the
/// database with the alias as the first family on the named faces and rebuild
/// the shaping system from it. The same `Source::Binary` bytes are reused; no
/// face is re-parsed twice and no font bytes are copied.
fn alias_family(system: &mut FontSystem, ids: &[fontdb::ID], declared: &str) {
    let locale = system.locale().to_owned();
    let db = system.db();
    let generic = [
        (
            fontdb::Family::Serif,
            db.family_name(&fontdb::Family::Serif),
        ),
        (
            fontdb::Family::SansSerif,
            db.family_name(&fontdb::Family::SansSerif),
        ),
        (
            fontdb::Family::Cursive,
            db.family_name(&fontdb::Family::Cursive),
        ),
        (
            fontdb::Family::Fantasy,
            db.family_name(&fontdb::Family::Fantasy),
        ),
        (
            fontdb::Family::Monospace,
            db.family_name(&fontdb::Family::Monospace),
        ),
    ];
    let mut rebuilt = fontdb::Database::new();
    for (family, name) in generic {
        match family {
            fontdb::Family::Serif => rebuilt.set_serif_family(name),
            fontdb::Family::SansSerif => rebuilt.set_sans_serif_family(name),
            fontdb::Family::Cursive => rebuilt.set_cursive_family(name),
            fontdb::Family::Fantasy => rebuilt.set_fantasy_family(name),
            fontdb::Family::Monospace => rebuilt.set_monospace_family(name),
            _ => {}
        }
    }
    for mut face in db.faces().cloned().collect::<Vec<_>>() {
        if ids.contains(&face.id) && !face.families.iter().any(|(name, _)| name == declared) {
            face.families.insert(
                0,
                (declared.to_owned(), fontdb::Language::English_UnitedStates),
            );
        }
        rebuilt.push_face_info(face);
    }
    *system = FontSystem::new_with_locale_and_db(locale, rebuilt);
}

/// A format the document left out is read from the path's tail; the validator
/// refuses a font without one, so this only serves an unvalidated document.
fn format_of(path: &str) -> FontFormat {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("woff2") => FontFormat::Woff2,
        Some("woff") => FontFormat::Woff,
        Some("otf") => FontFormat::Otf,
        _ => FontFormat::Ttf,
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

/// One resolved font request. A CSS stack is split at the shared owner: the
/// first named face is exact-matched by fontdb, and a CSS generic that appears
/// in the stack becomes a cosmic-text generic so `sans-serif`/`monospace`
/// actually select a face instead of being treated as a literal family name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FontFamily {
    Named(String),
    Serif,
    SansSerif,
    Monospace,
    Cursive,
    Fantasy,
}

impl FontFamily {
    /// The first usable entry of a CSS font stack. Quoted names keep their
    /// spaces; generics map to cosmic-text's own family variants.
    pub fn parse(stack: &str) -> Option<Self> {
        let first = stack
            .split(',')
            .map(str::trim)
            .find(|entry| !entry.is_empty())?;
        let unquoted = first.trim_matches(['"', '\''].as_slice());
        Some(match unquoted.to_ascii_lowercase().as_str() {
            "serif" => Self::Serif,
            "sans-serif" => Self::SansSerif,
            "monospace" => Self::Monospace,
            "cursive" => Self::Cursive,
            "fantasy" => Self::Fantasy,
            _ => Self::Named(unquoted.to_owned()),
        })
    }

    /// The named face, when the request names one. A generic has no literal
    /// name for a caller to compare.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Named(name) => Some(name),
            _ => None,
        }
    }

    /// The family string a platform backend that takes a name uses. CSS
    /// generics map to gpui's system-family conventions, the same way they map
    /// to cosmic-text generics for the shaper.
    pub fn system_name(&self) -> &str {
        match self {
            Self::Named(name) => name,
            // gpui takes a literal family name; map a CSS generic to a face
            // this Windows-first product can rely on, not a keyword it cannot
            // load. The cosmic backend uses the generics directly instead.
            Self::Monospace => "Courier New",
            Self::Serif => "Times New Roman",
            Self::SansSerif | Self::Cursive | Self::Fantasy => ".SystemUIFont",
        }
    }
}

impl From<&str> for FontFamily {
    fn from(value: &str) -> Self {
        Self::parse(value).unwrap_or(Self::SansSerif)
    }
}

/// How a run is drawn: the face it names and the size and colour it resolves
/// to. Positions are the layout's business, so they are not here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextStyle {
    /// The family the run's type token names. Absent takes the shaper's own
    /// sans-serif choice.
    pub family: Option<FontFamily>,
    /// A CSS-style weight: 400 is regular, 700 is bold.
    pub weight: u16,
    pub italic: bool,
    /// In artboard units, so scaling is the backend's.
    pub size: f32,
    /// Extra advance after every glyph, in artboard units. Source paints it as
    /// Fabric's `charSpacing` (`scene-fabric/paint.ts:85-88`).
    pub letter_spacing: f32,
    pub color: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: None,
            weight: 400,
            italic: false,
            size: 16.0,
            letter_spacing: 0.0,
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

/// Fabric's own default (`fabric/dist/index.mjs:4175`), so a paragraph with no
/// authored line height lands on the editor's lines.
const DEFAULT_LINE_HEIGHT: f32 = 1.16;

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
            request
                .runs
                .iter()
                .enumerate()
                .map(|(index, run)| (run.text.as_str(), attrs(&run.style).metadata(index))),
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
    let clipping = request.overflow != TextOverflow::Visible;
    for run in buffer.layout_runs() {
        // Source paints every line when overflow is visible and clamps the
        // rest to at least one line (`scene-fabric/authored-box.ts:310-327`).
        if clipping
            && lines >= 1.0
            && let Some((_, box_height)) = request.box_
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
        attrs = attrs.family(match family {
            FontFamily::Named(name) => Family::Name(name),
            FontFamily::Serif => Family::Serif,
            FontFamily::SansSerif => Family::SansSerif,
            FontFamily::Monospace => Family::Monospace,
            FontFamily::Cursive => Family::Cursive,
            FontFamily::Fantasy => Family::Fantasy,
        });
    }
    if style.italic {
        attrs = attrs.style(TextPosture::Italic);
    }
    // No per-span metrics here: the layout's buffer owns line height, so a
    // span must not override it with a default and silently drop the authored
    // value. A backend shaping one segment at a time sets metrics itself.
    attrs.letter_spacing(style.letter_spacing)
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
/// the box instead of across it. Grapheme clusters are the unit, and authored
/// line breaks are dropped because every cluster already gets its own line
/// (`scene-fabric/text-runs.ts:55-65`).
fn vertical(fonts: &mut Fonts, request: &TextRequest<'_>, line_height: f32) -> TextLayout {
    let metrics_of = |style: &TextStyle| Metrics::new(style.size, style.size * line_height);
    let mut segments = Vec::new();
    let mut y = 0.0;
    for run in request.runs {
        for cluster in run
            .text
            .graphemes(true)
            .filter(|cluster| !matches!(*cluster, "\n" | "\r" | "\r\n"))
        {
            let text = cluster.to_string();
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
    TextLayout {
        segments,
        clip: None,
    }
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

    /// Fabric's own default is 1.16 (`fabric/dist/index.mjs:4175`); a native
    /// paragraph with no authored lineHeight must land on the same lines.
    #[test]
    fn an_unset_line_height_uses_fabric_1_16() {
        let runs = vec![run("one two three four", 20.0)];
        let mut request = request(&runs);
        request.box_ = Some((40.0, 200.0));
        request.line_height = 0.0;
        let laid = layout(&mut fonts(), &request);
        let tops: Vec<f32> = laid.segments.iter().map(|segment| segment.origin.1).collect();
        let lines: Vec<f32> = tops.into_iter().fold(Vec::new(), |mut lines, top| {
            if lines.last().is_none_or(|last| (*last - top).abs() > 0.5) {
                lines.push(top);
            }
            lines
        });
        assert!(lines.len() >= 2, "the narrow box wraps: {lines:?}");
        assert!(
            ((lines[1] - lines[0]) - 20.0 * 1.16).abs() < 0.5,
            "unset line height is 1.16 of the size: {lines:?}"
        );
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
    fn a_box_shorter_than_one_line_still_paints_one_line() {
        // Source clamps capacity with maxLines: Math.max(1, floor(height / line))
        // (scene-fabric/authored-box.ts:310-316), so a box that fits no full
        // line paints a clipped first line rather than nothing.
        let runs = vec![run("STATUS", 20.0)];
        let mut request = request(&runs);
        request.box_ = Some((100.0, 20.0));
        request.overflow = TextOverflow::Clip;
        let laid = layout(&mut fonts(), &request);
        assert_eq!(laid.segments.len(), 1, "{:?}", laid.segments);
        assert_eq!(laid.segments[0].text, "STATUS");
        assert_eq!(laid.clip, Some((100.0, 20.0)));
    }

    #[test]
    fn a_visible_box_keeps_the_lines_past_its_height() {
        // Source deletes the clip path for overflow "visible" and paints every
        // line (scene-fabric/authored-box.ts:320-327).
        let runs = vec![run(
            "one two three four five six seven eight nine ten",
            20.0,
        )];
        let mut unbounded = request(&runs);
        unbounded.box_ = Some((100.0, 10_000.0));
        let all = layout(&mut fonts(), &unbounded).segments.len();
        assert!(all > 2, "{all} wrapped lines");

        let mut short = request(&runs);
        short.box_ = Some((100.0, 48.0));
        short.overflow = TextOverflow::Visible;
        let laid = layout(&mut fonts(), &short);
        assert_eq!(laid.clip, None);
        assert_eq!(laid.segments.len(), all, "{:?}", laid.segments);
    }

    #[test]
    fn vertical_mode_stacks_grapheme_clusters_and_drops_line_breaks() {
        // Source splits with a grapheme splitter and filters "\n" and "\r"
        // (scene-fabric/text-runs.ts:55-65).
        let runs = vec![run(
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}A\r\nB",
            20.0,
        )];
        let mut request = request(&runs);
        request.vertical = true;
        let laid = layout(&mut fonts(), &request);
        let texts: Vec<&str> = laid
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(
            texts,
            [
                "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
                "A",
                "B"
            ]
        );
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

    fn document_with_font(path: &str, format: &str) -> ThemeDocument {
        yunari_model::document::parse_document(&format!(
            r##"{{"schemaVersion":"3.0.0","id":"t","artboard":{{"width":100,"height":100}},
                "nodes":[],
                "assets":[{{"id":"f","kind":"font","path":{path:?},"family":"Test Face",
                    "weight":400,"style":"normal","format":{format:?}}}]}}"##
        ))
        .expect("fixture parses")
    }

    #[test]
    fn a_woff2_face_that_will_not_decode_is_refused_by_its_own_format() {
        let document = document_with_font("assets/a.woff2", "woff2");
        let mut fonts = fonts();
        let refusals = fonts.load_faces(&document, |_| Some(b"not a font".to_vec()));
        assert_eq!(
            refusals,
            vec!["The face at \"assets/a.woff2\" is an invalid woff2.".to_owned()]
        );
    }

    #[test]
    fn a_face_whose_bytes_are_missing_is_refused_with_its_path() {
        let document = document_with_font("assets/a.ttf", "ttf");
        let refusals = fonts().load_faces(&document, |_| None);
        assert_eq!(
            refusals,
            vec!["The face at \"assets/a.ttf\" could not be read.".to_owned()]
        );
    }

    #[test]
    fn a_declared_ttf_is_registered_in_the_shaping_database() {
        // A system face stands in for a theme's own: the bytes go through the
        // same registration path. Absent on a host without it, the test says so
        // rather than failing.
        let path = [
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file());
        let Some(path) = path else {
            eprintln!("no system face to stand in for a theme's own");
            return;
        };
        let bytes = std::fs::read(path).expect("the face is readable");
        let document = document_with_font("assets/a.ttf", "ttf");
        let mut fonts = fonts();
        let before = fonts.system_mut().db().faces().count();
        let refusals = fonts.load_faces(&document, |_| Some(bytes.clone()));
        assert!(refusals.is_empty(), "{refusals:?}");
        assert!(fonts.system_mut().db().faces().count() > before);
    }
}
