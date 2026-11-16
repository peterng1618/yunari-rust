//! Segmentation: a fill drawn as equal slices rather than one continuous shape.
//!
//! The author picks a *divisor* — how many slices a full measure is cut into —
//! and the shape still spans the reading, so one setting means the same thing
//! for a bar, a gauge arc and a pie slice. `100` is continuous.
//!
//! The gap arithmetic is the product's own: the authored share is of the
//! segment's *painted* width, and a round cap extends each band end by half the
//! thickness, so the empty space between two painted ends is smaller than the
//! stop gap by the clearance.

/// Every divisor an author can choose, ascending, ending at continuous.
pub const SEGMENT_DIVISORS: [u32; 9] = [1, 2, 4, 5, 10, 20, 25, 50, 100];

/// The gap an author who has not chosen one gets, as a share of the segment it
/// separates. At 100 the empty space is as wide as the painted segment.
pub const SEGMENT_SPACING_PERCENT_DEFAULT: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentFill {
    /// The reading lights the whole slice it reaches.
    Whole,
    /// The fill stops at the reading itself.
    Partial,
}

/// The authored spacing, or the family's own when the document carries none.
pub fn segment_spacing_percent_of(settings: &serde_json::Map<String, serde_json::Value>) -> f32 {
    settings
        .get("segmentSpacingPercent")
        .and_then(|value| value.as_f64())
        .map_or(SEGMENT_SPACING_PERCENT_DEFAULT, |value| value as f32)
}

/// The authored fill mode, or the family's own when the document carries none.
pub fn segment_fill_of(
    settings: &serde_json::Map<String, serde_json::Value>,
    fallback: SegmentFill,
) -> SegmentFill {
    match settings.get("segmentFill").and_then(|value| value.as_str()) {
        Some("whole") => SegmentFill::Whole,
        Some("partial") => SegmentFill::Partial,
        _ => fallback,
    }
}

/// The authored divisor, or continuous when the document carries none.
pub fn segment_divisor_of(settings: &serde_json::Map<String, serde_json::Value>) -> u32 {
    match settings
        .get("segmentDivisor")
        .and_then(|value| value.as_u64())
    {
        Some(divisor) if SEGMENT_DIVISORS.contains(&(divisor as u32)) => divisor as u32,
        _ => 100,
    }
}

/// How many slices a full measure is cut into. A divisor the list does not
/// carry draws continuously rather than refusing to draw.
pub fn segment_count(divisor: u32) -> u32 {
    if SEGMENT_DIVISORS.contains(&divisor) {
        100 / divisor
    } else {
        1
    }
}

/// What a gap's arithmetic needs from a family's settings.
#[derive(Debug, Clone, Copy)]
pub struct SegmentGap {
    pub thickness: f32,
    pub round_cap: bool,
    pub spacing_percent: f32,
}

/// The **stop** gap an angular band leaves, in degrees.
///
/// Writing `q` for the authored share, `b` for the band and `c` for the
/// clearance the caps eat:
///
/// ```text
/// empty = q · painted,  painted = b - stopGap + c
/// stopGap = (q (b + c) + c) / (1 + q)
/// ```
///
/// At `q = 0` the caps meet and the band is the whole band; at `q = 1` the
/// empty space and the painted band are both half of it.
pub fn gauge_gap(
    gap: SegmentGap,
    sweep_degrees: f32,
    count: u32,
    drawn_edge_px: Option<f32>,
) -> f32 {
    let band = sweep_degrees.abs() / count.max(1) as f32;
    let q = gap.spacing_percent.max(0.0) / 100.0;
    let clearance = match drawn_edge_px {
        Some(edge) if gap.round_cap && edge > 0.0 => {
            (gap.thickness / (edge / 2.0).max(1.0)) * (180.0 / std::f32::consts::PI)
        }
        _ => 0.0,
    };
    let stop = (q * (band + clearance) + clearance) / (1.0 + q);
    stop.min(band.max(0.0)).max(0.0)
}

/// The empty length a cartesian band leaves, in pixels. A bar has nothing
/// overhanging, so this is the gauge's equation with no clearance.
pub fn bar_gap_px(band_px: f32, spacing_percent: f32) -> f32 {
    let q = spacing_percent.max(0.0) / 100.0;
    (q * band_px.max(0.0)) / (1.0 + q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(spacing: f32, round_cap: bool) -> SegmentGap {
        SegmentGap {
            thickness: 18.0,
            round_cap,
            spacing_percent: spacing,
        }
    }

    #[test]
    fn a_full_divisor_draws_one_continuous_shape() {
        assert_eq!(segment_count(100), 1);
        assert_eq!(segment_count(10), 10);
        assert_eq!(segment_count(7), 1);
    }

    #[test]
    fn an_authored_divisor_off_the_list_is_continuous() {
        let mut settings = serde_json::Map::new();
        settings.insert("segmentDivisor".into(), 7.into());
        assert_eq!(segment_count(segment_divisor_of(&settings)), 1);
    }

    #[test]
    fn no_spacing_still_leaves_room_for_the_caps() {
        let stop = gauge_gap(gap(0.0, true), 270.0, 100, Some(200.0));
        assert!(stop > 0.0, "the caps cannot overlap: {stop}");
    }

    #[test]
    fn no_round_cap_means_no_clearance() {
        let stop = gauge_gap(gap(10.0, false), 270.0, 10, Some(200.0));
        // (0.1 · 27) / 1.1
        assert!((stop - 2.454_545).abs() < 1e-4, "{stop}");
    }

    #[test]
    fn a_gap_never_swallows_its_band() {
        let stop = gauge_gap(gap(500.0, true), 270.0, 10, Some(200.0));
        assert!(stop <= 27.0);
    }

    #[test]
    fn a_bar_gap_is_a_share_of_the_band() {
        // q = 0.1 → 10 / 1.1
        assert!((bar_gap_px(100.0, 10.0) - 9.090_909).abs() < 1e-4);
    }

    #[test]
    fn an_absent_setting_takes_the_default_spacing() {
        let settings = serde_json::Map::new();
        assert_eq!(segment_spacing_percent_of(&settings), 10.0);
        assert_eq!(
            segment_fill_of(&settings, SegmentFill::Whole),
            SegmentFill::Whole
        );
    }
}
