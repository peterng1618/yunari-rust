//! Where a background picture sits and how it dissolves into the fill.
//!
//! Ported decision-for-decision from the source renderer: a simple background
//! covers the box, a centred one keeps its long edge and fades the two gaps
//! the aspect leaves, and an edge anchor is a band that stays flush while the
//! picture pans inside it. The arithmetic runs before a picture's own size is
//! known, so the area and its ramps are computable from the document alone;
//! only the crop waits for the bytes.

use yunari_model::document::{BackgroundAnchor, BackgroundComposition, BackgroundMedia};

/// The area's share of the box when an edge anchor names no extent.
pub const DEFAULT_EXTENT: f32 = 0.5;
pub const MIN_EXTENT: f32 = 0.1;
pub const MAX_EXTENT: f32 = 1.0;
pub const MAX_FADE: f32 = 1.0;
pub const MIN_SCALE: f32 = 1.0;
pub const MAX_SCALE: f32 = 5.0;
pub const MIN_OFFSET: f32 = -5.0;
pub const MAX_OFFSET: f32 = 5.0;
/// How many stops approximate the eased ramp, so a backend that can only take
/// gradient stops draws the same curve a sampler does.
pub const RAMP_STOPS: usize = 8;

/// A box in artboard units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// A window of the source picture, in source pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Where one picture goes: the rect it covers, the rect the box lets through,
/// and the source window that fills the latter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub cover: Rect,
    pub dest: Rect,
    pub crop: Crop,
}

/// One ramp: transparent at the first point, opaque at the second, along the
/// axis the two share.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mask {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    /// The media area in box units. It may extend past the box on any side.
    pub region: Rect,
    /// One ramp per side that meets the fill; none for a full cover.
    pub masks: Vec<Mask>,
    /// Absent until the source's own size is known.
    pub media: Option<Placed>,
}

/// The authored scale and pan, clamped to the document's ranges. A non-finite
/// value is a refused reading, not a number to round.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

impl Geometry {
    pub fn of(media: Option<&BackgroundMedia>) -> Self {
        Self {
            scale: bounded(
                media.map(|media| media.scale as f32),
                Self::default().scale,
                MIN_SCALE,
                MAX_SCALE,
            ),
            offset_x: bounded(
                media.map(|media| media.offset_x as f32),
                Self::default().offset_x,
                MIN_OFFSET,
                MAX_OFFSET,
            ),
            offset_y: bounded(
                media.map(|media| media.offset_y as f32),
                Self::default().offset_y,
                MIN_OFFSET,
                MAX_OFFSET,
            ),
        }
    }
}

/// The composition a background actually composes. A simple background keeps
/// the full cover it was authored with, whatever stray keys its media carries;
/// only an advanced one positions its media and fades it into the fill.
pub fn active(background: &yunari_model::document::Background) -> Option<&BackgroundComposition> {
    match background.mode {
        yunari_model::document::BackgroundMode::Advanced => {
            background.media.as_ref()?.composition.as_ref()
        }
        yunari_model::document::BackgroundMode::Simple => None,
    }
}

/// Turns an anchor and the authored geometry into the one picture every
/// surface reads.
pub fn resolve(
    box_: (f32, f32),
    composition: Option<&BackgroundComposition>,
    source: Option<(f32, f32)>,
    geometry: Geometry,
) -> Composition {
    let known = known_size(source, box_);
    let Some(composition) = composition else {
        let region = scaled(box_rect(box_), BackgroundAnchor::Center, geometry.scale, box_);
        return Composition {
            media: known.map(|known| place_media(region, box_, known, geometry)),
            region,
            masks: Vec::new(),
        };
    };
    let anchor = composition.anchor;
    let fade = fade_of(composition.fade.map(|fade| fade as f32));
    if anchor == BackgroundAnchor::Center {
        let mode = known.and_then(|known| centre_mode(box_, known));
        let base = match (known, mode) {
            (Some(known), Some(_)) => centre_rect(box_, known),
            _ => box_rect(box_),
        };
        let region = scaled(base, BackgroundAnchor::Center, geometry.scale, box_);
        let masks = match mode {
            Some(mode) => centre_masks(region, box_, fade, mode),
            None => Vec::new(),
        };
        return Composition {
            media: known.map(|known| place_media(region, box_, known, geometry)),
            region,
            masks,
        };
    }
    let base = edge_band(
        box_,
        anchor,
        extent_of(composition.extent.map(|extent| extent as f32)),
    );
    let region = scaled(base, anchor, geometry.scale, box_);
    let masks = edge_masks(region, box_, anchor, fade);
    Composition {
        media: known.map(|known| place_media(region, box_, known, geometry)),
        region,
        masks,
    }
}

/// The eased ramp: 0 at the transparent end and 1 at the opaque one.
/// Smootherstep has a zero derivative at both ends, which is what removes the
/// crease a straight ramp leaves where a fade starts and stops.
pub fn blend_ramp(t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// The ramp as evenly spaced stops, for a shader that only takes stops.
pub fn ramp_stops() -> impl Iterator<Item = (f32, f32)> {
    (0..RAMP_STOPS).map(|index| {
        let offset = index as f32 / (RAMP_STOPS - 1) as f32;
        (offset, blend_ramp(offset))
    })
}

/// The source size when both it and the box can be divided by.
fn known_size(source: Option<(f32, f32)>, box_: (f32, f32)) -> Option<(f32, f32)> {
    let (width, height) = source?;
    (width > 0.0 && height > 0.0 && box_.0 > 0.0 && box_.1 > 0.0).then_some((width, height))
}

fn box_rect(box_: (f32, f32)) -> Rect {
    Rect {
        left: 0.0,
        top: 0.0,
        width: box_.0,
        height: box_.1,
    }
}

/// The picture's long edge fits the box, so its aspect decides which axis
/// leaves the two gaps.
fn centre_rect(box_: (f32, f32), source: (f32, f32)) -> Rect {
    let box_aspect = box_.0 / box_.1;
    let source_aspect = source.0 / source.1;
    if source_aspect > box_aspect {
        Rect {
            left: 0.0,
            top: (box_.1 - box_.0 / source_aspect) / 2.0,
            width: box_.0,
            height: box_.0 / source_aspect,
        }
    } else {
        Rect {
            left: (box_.0 - box_.1 * source_aspect) / 2.0,
            top: 0.0,
            width: box_.1 * source_aspect,
            height: box_.1,
        }
    }
}

/// Which way a centred picture leaves its gaps, or `None` when the aspects
/// match and it covers the box.
fn centre_mode(box_: (f32, f32), source: (f32, f32)) -> Option<bool> {
    let box_aspect = box_.0 / box_.1;
    let source_aspect = source.0 / source.1;
    if (source_aspect - box_aspect).abs() < 1e-9 {
        None
    } else {
        Some(source_aspect > box_aspect)
    }
}

/// Scale multiplies the base area about its anchor. A centred area grows about
/// the box centre; a band grows across the box from the edge it is pinned to,
/// keeping the perpendicular at the box's own extent.
fn scaled(base: Rect, anchor: BackgroundAnchor, scale: f32, box_: (f32, f32)) -> Rect {
    match anchor {
        BackgroundAnchor::Left => Rect {
            left: base.left,
            top: 0.0,
            width: base.width * scale,
            height: box_.1,
        },
        BackgroundAnchor::Right => Rect {
            left: box_.0 - base.width * scale,
            top: 0.0,
            width: base.width * scale,
            height: box_.1,
        },
        BackgroundAnchor::Top => Rect {
            left: 0.0,
            top: base.top,
            width: box_.0,
            height: base.height * scale,
        },
        BackgroundAnchor::Bottom => Rect {
            left: 0.0,
            top: box_.1 - base.height * scale,
            width: box_.0,
            height: base.height * scale,
        },
        BackgroundAnchor::Center => {
            let centre_x = base.left + base.width / 2.0;
            let centre_y = base.top + base.height / 2.0;
            Rect {
                left: centre_x - (base.width * scale) / 2.0,
                top: centre_y - (base.height * scale) / 2.0,
                width: base.width * scale,
                height: base.height * scale,
            }
        }
    }
}

/// The part of `region` the box lets through, never smaller than a pixel.
pub fn visible_area(region: Rect, box_: (f32, f32)) -> Rect {
    let left = region.left.max(0.0);
    let top = region.top.max(0.0);
    let right = box_.0.min(region.left + region.width);
    let bottom = box_.1.min(region.top + region.height);
    Rect {
        left,
        top,
        width: 1.0_f32.max((box_.0 - left).min(right - left)),
        height: 1.0_f32.max((box_.1 - top).min(bottom - top)),
    }
}

/// The picture that covers `region`, panned by the authored offsets, and the
/// source window the box sees.
///
/// The offset never moves the area: the picture's cover is inflated just enough
/// for the pan the offset asks for — `1 + 2·|offset|` of the area's own span,
/// one uniform factor so nothing distorts — and the pan then stops at the
/// overhang that inflation made.
fn place_media(region: Rect, box_: (f32, f32), source: (f32, f32), geometry: Geometry) -> Placed {
    let factor = (region.width / source.0)
        .max(region.height / source.1)
        .max((region.width * (1.0 + 2.0 * geometry.offset_x.abs())) / source.0)
        .max((region.height * (1.0 + 2.0 * geometry.offset_y.abs())) / source.1);
    let painted = (source.0 * factor, source.1 * factor);
    let travel_x = ((painted.0 - region.width) / 2.0).max(0.0);
    let travel_y = ((painted.1 - region.height) / 2.0).max(0.0);
    let cover = Rect {
        left: region.left
            + (region.width - painted.0) / 2.0
            + (geometry.offset_x * region.width).clamp(-travel_x, travel_x),
        top: region.top
            + (region.height - painted.1) / 2.0
            + (geometry.offset_y * region.height).clamp(-travel_y, travel_y),
        width: painted.0,
        height: painted.1,
    };
    // The area layer clips, so what the box shows is the area it lets through —
    // not the larger cover the area was cut from.
    let dest = visible_area(region, box_);
    let x = ((dest.left - cover.left) / factor)
        .round()
        .clamp(0.0, source.0 - 1.0);
    let y = ((dest.top - cover.top) / factor)
        .round()
        .clamp(0.0, source.1 - 1.0);
    let width = (dest.width / factor).round().clamp(1.0, source.0 - x);
    let height = (dest.height / factor).round().clamp(1.0, source.1 - y);
    Placed {
        cover,
        dest,
        crop: Crop {
            x,
            y,
            width,
            height,
        },
    }
}

fn centre_masks(region: Rect, box_: (f32, f32), fade: Option<f32>, wide: bool) -> Vec<Mask> {
    let Some(fade) = fade else {
        return Vec::new();
    };
    let across = fade * box_.0;
    let down = fade * box_.1;
    // The two gaps are the pair of sides the picture's short axis leaves.
    if wide {
        vec![
            Mask {
                x1: region.left,
                y1: region.top,
                x2: region.left,
                y2: (region.top + down).min(region.top + region.height),
            },
            Mask {
                x1: region.left,
                y1: region.top + region.height,
                x2: region.left,
                y2: (region.top + region.height - down).max(region.top),
            },
        ]
    } else {
        vec![
            Mask {
                x1: region.left,
                y1: region.top,
                x2: (region.left + across).min(region.left + region.width),
                y2: region.top,
            },
            Mask {
                x1: region.left + region.width,
                y1: region.top,
                x2: (region.left + region.width - across).max(region.left),
                y2: region.top,
            },
        ]
    }
}

fn edge_band(box_: (f32, f32), anchor: BackgroundAnchor, extent: f32) -> Rect {
    match anchor {
        BackgroundAnchor::Left => Rect {
            left: 0.0,
            top: 0.0,
            width: box_.0 * extent,
            height: box_.1,
        },
        BackgroundAnchor::Right => Rect {
            left: box_.0 * (1.0 - extent),
            top: 0.0,
            width: box_.0 * extent,
            height: box_.1,
        },
        BackgroundAnchor::Top => Rect {
            left: 0.0,
            top: 0.0,
            width: box_.0,
            height: box_.1 * extent,
        },
        BackgroundAnchor::Bottom => Rect {
            left: 0.0,
            top: box_.1 * (1.0 - extent),
            width: box_.0,
            height: box_.1 * extent,
        },
        // A centred background has no band; its caller never asks for one.
        BackgroundAnchor::Center => box_rect(box_),
    }
}

fn edge_masks(
    region: Rect,
    box_: (f32, f32),
    anchor: BackgroundAnchor,
    fade: Option<f32>,
) -> Vec<Mask> {
    let Some(fade) = fade else {
        return Vec::new();
    };
    let across = fade * box_.0;
    let down = fade * box_.1;
    let mask = match anchor {
        BackgroundAnchor::Left => Mask {
            x1: region.left + region.width,
            y1: region.top,
            x2: (region.left + region.width - across).max(region.left),
            y2: region.top,
        },
        BackgroundAnchor::Right => Mask {
            x1: region.left,
            y1: region.top,
            x2: (region.left + across).min(region.left + region.width),
            y2: region.top,
        },
        BackgroundAnchor::Top => Mask {
            x1: region.left,
            y1: region.top + region.height,
            x2: region.left,
            y2: (region.top + region.height - down).max(region.top),
        },
        BackgroundAnchor::Bottom => Mask {
            x1: region.left,
            y1: region.top,
            x2: region.left,
            y2: (region.top + down).min(region.top + region.height),
        },
        BackgroundAnchor::Center => return Vec::new(),
    };
    vec![mask]
}

fn extent_of(value: Option<f32>) -> f32 {
    value
        .filter(|extent| extent.is_finite())
        .map_or(DEFAULT_EXTENT, |extent| extent.clamp(MIN_EXTENT, MAX_EXTENT))
}

fn fade_of(value: Option<f32>) -> Option<f32> {
    value
        .filter(|fade| fade.is_finite() && *fade > 0.0)
        .map(|fade| fade.min(MAX_FADE))
}

fn bounded(value: Option<f32>, fallback: f32, minimum: f32, maximum: f32) -> f32 {
    value
        .filter(|value| value.is_finite())
        .map_or(fallback, |value| value.clamp(minimum, maximum))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunari_model::document::{BackgroundAnchor, BackgroundComposition};

    const BOX: (f32, f32) = (400.0, 100.0);
    const SOURCE: (f32, f32) = (160.0, 90.0);

    fn composition(anchor: BackgroundAnchor, extent: Option<f32>, fade: Option<f32>) -> BackgroundComposition {
        BackgroundComposition {
            anchor,
            extent: extent.map(f64::from),
            fade: fade.map(f64::from),
        }
    }

    fn at(geometry: Geometry) -> Composition {
        resolve(
            BOX,
            Some(&composition(BackgroundAnchor::Center, None, None)),
            Some(SOURCE),
            geometry,
        )
    }

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-2, "{actual} != {expected}");
    }

    #[test]
    fn a_simple_background_keeps_the_old_cover() {
        let resolved = resolve(BOX, None, Some(SOURCE), Geometry::default());
        assert_eq!(
            resolved.region,
            Rect { left: 0.0, top: 0.0, width: 400.0, height: 100.0 }
        );
        let media = resolved.media.expect("a known source places the picture");
        assert_eq!(media.dest, Rect { left: 0.0, top: 0.0, width: 400.0, height: 100.0 });
        assert_eq!(media.crop, Crop { x: 0.0, y: 25.0, width: 160.0, height: 40.0 });
        assert!(resolved.masks.is_empty());
    }

    #[test]
    fn scale_grows_the_simple_area_about_the_centre() {
        let resolved = resolve(
            BOX,
            None,
            Some(SOURCE),
            Geometry { scale: 2.0, ..Geometry::default() },
        );
        assert_eq!(
            resolved.region,
            Rect { left: -200.0, top: -50.0, width: 800.0, height: 200.0 }
        );
        let media = resolved.media.expect("a known source places the picture");
        assert_eq!(media.crop, Crop { x: 40.0, y: 35.0, width: 80.0, height: 20.0 });
    }

    #[test]
    fn a_narrower_source_is_letterboxed_at_the_centre_anchor() {
        let resolved = at(Geometry::default());
        close(resolved.region.left, 111.111);
        close(resolved.region.width, 177.778);
        close(resolved.region.height, 100.0);
        let media = resolved.media.expect("a known source places the picture");
        assert_eq!(media.crop, Crop { x: 0.0, y: 0.0, width: 160.0, height: 90.0 });
    }

    #[test]
    fn a_centred_picture_pans_inside_its_area_and_stops_at_the_source_edge() {
        let still = at(Geometry::default());
        let panned = at(Geometry { offset_x: 5.0, ..Geometry::default() });
        close(panned.region.left, still.region.left);
        assert_eq!(
            panned.media.expect("a known source places the picture").crop.x,
            0.0
        );
    }

    #[test]
    fn an_edge_band_stays_flush_while_its_picture_pans() {
        let top = composition(BackgroundAnchor::Top, Some(0.5), None);
        let panned = resolve(
            BOX,
            Some(&top),
            Some(SOURCE),
            Geometry { offset_y: -0.5, ..Geometry::default() },
        );
        assert_eq!(
            panned.region,
            Rect { left: 0.0, top: 0.0, width: 400.0, height: 50.0 }
        );
        assert_eq!(
            panned.media.expect("a known source places the picture").crop,
            Crop { x: 0.0, y: 45.0, width: 160.0, height: 20.0 }
        );
    }

    #[test]
    fn a_band_shows_itself_and_not_the_whole_cover_it_was_cut_from() {
        let top = composition(BackgroundAnchor::Top, Some(0.5), None);
        let resolved = resolve(BOX, Some(&top), Some(SOURCE), Geometry::default());
        let media = resolved.media.expect("a known source places the picture");
        assert_eq!(media.dest, Rect { left: 0.0, top: 0.0, width: 400.0, height: 50.0 });
        assert_eq!(media.crop, Crop { x: 0.0, y: 35.0, width: 160.0, height: 20.0 });
    }

    #[test]
    fn a_centred_band_fades_both_of_its_gaps() {
        let centred = composition(BackgroundAnchor::Center, None, Some(0.25));
        let resolved = resolve(BOX, Some(&centred), Some(SOURCE), Geometry::default());
        assert_eq!(resolved.masks.len(), 2);
        close(resolved.masks[0].x1, 111.111);
        close(resolved.masks[0].x2, 211.111);
        close(resolved.masks[1].x1, 288.889);
    }

    #[test]
    fn an_edge_band_fades_where_it_meets_the_fill() {
        let left = composition(BackgroundAnchor::Left, Some(0.35), Some(0.25));
        let resolved = resolve(BOX, Some(&left), None, Geometry::default());
        assert_eq!(
            resolved.region,
            Rect { left: 0.0, top: 0.0, width: 140.0, height: 100.0 }
        );
        assert_eq!(
            resolved.masks,
            vec![Mask { x1: 140.0, y1: 0.0, x2: 40.0, y2: 0.0 }]
        );
    }

    #[test]
    fn a_fade_longer_than_the_band_stops_at_the_opposite_edge() {
        let left = composition(BackgroundAnchor::Left, Some(0.25), Some(1.0));
        let resolved = resolve(BOX, Some(&left), None, Geometry::default());
        assert_eq!(resolved.masks, vec![Mask { x1: 100.0, y1: 0.0, x2: 0.0, y2: 0.0 }]);
    }

    #[test]
    fn the_opposite_edges_mirror_their_bands() {
        let right = composition(BackgroundAnchor::Right, Some(0.25), None);
        assert_eq!(
            resolve(BOX, Some(&right), None, Geometry::default()).region,
            Rect { left: 300.0, top: 0.0, width: 100.0, height: 100.0 }
        );
        let bottom = composition(BackgroundAnchor::Bottom, Some(0.25), None);
        assert_eq!(
            resolve(BOX, Some(&bottom), None, Geometry::default()).region,
            Rect { left: 0.0, top: 75.0, width: 400.0, height: 25.0 }
        );
    }

    #[test]
    fn a_matching_aspect_covers_the_box_without_a_ramp() {
        let centred = composition(BackgroundAnchor::Center, None, Some(0.1));
        let resolved = resolve(
            (800.0, 480.0),
            Some(&centred),
            Some((800.0, 480.0)),
            Geometry::default(),
        );
        assert_eq!(
            resolved.region,
            Rect { left: 0.0, top: 0.0, width: 800.0, height: 480.0 }
        );
        assert!(resolved.masks.is_empty());
    }

    #[test]
    fn no_media_is_placed_until_the_source_size_is_known() {
        let resolved = resolve(BOX, Some(&composition(BackgroundAnchor::Center, None, None)), None, Geometry::default());
        assert!(resolved.media.is_none());
    }

    #[test]
    fn the_authored_geometry_is_clamped_and_non_finite_values_fall_back() {
        let media = BackgroundMedia {
            asset_id: "a".into(),
            color: None,
            bounds: yunari_model::document::BackgroundBounds::Theme,
            offset_x: -9.0,
            offset_y: f64::NAN,
            scale: 9.0,
            flip_x: false,
            flip_y: false,
            fallback_frame: None,
            composition: None,
        };
        assert_eq!(
            Geometry::of(Some(&media)),
            Geometry { scale: 5.0, offset_x: -5.0, offset_y: 0.0 }
        );
        assert_eq!(Geometry::of(None), Geometry::default());
    }

    #[test]
    fn the_ramp_eases_without_a_crease_at_either_end() {
        let stops: Vec<(f32, f32)> = ramp_stops().collect();
        assert!(stops.len() > 2);
        assert_eq!(stops[0], (0.0, 0.0));
        assert_eq!(stops[stops.len() - 1], (1.0, 1.0));
        for pair in stops.windows(2) {
            assert!(pair[1].1 > pair[0].1);
        }
        close(blend_ramp(2.0 / 7.0), 0.145);
    }
}
