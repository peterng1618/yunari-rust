//! How a reading is written: the number, then the unit it is shown in.
//!
//! Ported from `renderer-core/scene/plan.ts`. Grouping is off on purpose: the
//! decimal separator is what a reader recognises, while grouping changes how
//! wide every four-digit reading is, and a dashboard lays its text out in boxes
//! an author sized by hand.

use yunari_model::document::UnitDisplay;

/// The reading in the theme's own number format.
///
/// `precision` keeps a fixed number of decimals from 0 to 6. Without it the
/// value keeps at most one decimal, and none where it has none, so a reading
/// ending `.0` never widens a text box the author sized by hand.
///
/// ponytail: the caller supplies the decimal separator instead of resolving a
/// BCP 47 tag here. Add `icu_decimal` when the editor offers a language choice;
/// grouping stays off then too.
pub fn format_number(value: f64, precision: Option<u8>, decimal_separator: char) -> String {
    match precision {
        Some(precision) => format_fixed(value, precision.min(6), decimal_separator),
        None => {
            let rounded = round_half_up(value * 10.0) / 10.0;
            let digits = if rounded.fract() == 0.0 { 0 } else { 1 };
            format_fixed(rounded, digits, decimal_separator)
        }
    }
}

/// `Math.round` in the source rounds a half up, which `f64::round` does not
/// for negative values; a reading of `-0.05` must print `0`, not `-0.1` in the
/// default (unpinned) branch.
fn round_half_up(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Rounds to `digits` decimals first, then writes the rounded integer, so the
/// decimal writer never rounds a second time.
///
/// Pinned precision goes through `Intl.NumberFormat` in the source, which
/// rounds a half away from zero: `-5.5` at zero decimals is `-6`, not `-5`.
fn format_fixed(value: f64, digits: u8, decimal_separator: char) -> String {
    let factor = 10f64.powi(i32::from(digits));
    let scaled = (value * factor).abs().round().copysign(value);
    let negative = scaled < 0.0;
    let mut text = format!("{:.0}", scaled.abs());
    if digits > 0 {
        let digits = usize::from(digits);
        while text.len() <= digits {
            text.insert(0, '0');
        }
        let split = text.len() - digits;
        text.insert(split, decimal_separator);
    }
    if negative && scaled != 0.0 {
        text.insert(0, '-');
    }
    text
}

/// The whole value, then the unit with the spacing a reader expects.
pub fn format_unit(
    unit: Option<&str>,
    display: UnitDisplay,
    long_units: Option<&std::collections::BTreeMap<String, String>>,
) -> String {
    let Some(unit) = unit.filter(|unit| !unit.is_empty()) else {
        return String::new();
    };
    if display == UnitDisplay::None {
        return String::new();
    }
    if display == UnitDisplay::Long {
        return match long_units.and_then(|long| long.get(unit)) {
            Some(long) => format!(" {long}"),
            None => spaced(unit),
        };
    }
    spaced(unit)
}

/// Percent and degree attach directly; word-like units get a leading space.
fn spaced(unit: &str) -> String {
    if unit.starts_with(['%', '°']) {
        unit.to_string()
    } else {
        format!(" {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn number(value: f64) -> String {
        format_number(value, None, '.')
    }

    #[test]
    fn keeps_at_most_one_decimal() {
        assert_eq!(number(17.64), "17.6");
        assert_eq!(number(17.04), "17");
        assert_eq!(number(17.0), "17");
    }

    #[test]
    fn rounds_half_up_like_the_source() {
        assert_eq!(number(0.05), "0.1");
        assert_eq!(number(-0.05), "0");
    }

    #[test]
    fn never_prints_negative_zero() {
        assert_eq!(number(-0.0), "0");
        assert_eq!(format_number(-0.0, Some(2), '.'), "0.00");
    }

    #[test]
    fn precision_pins_the_decimals() {
        assert_eq!(format_number(42.0, Some(2), '.'), "42.00");
        assert_eq!(format_number(42.5, Some(0), '.'), "43");
        assert_eq!(format_number(0.5, Some(1), '.'), "0.5");
    }

    #[test]
    fn precision_is_clamped_to_six() {
        assert_eq!(format_number(1.0, Some(9), '.'), "1.000000");
    }

    #[test]
    fn the_separator_is_the_callers() {
        assert_eq!(format_number(17.64, None, ','), "17,6");
        assert_eq!(format_number(42.0, Some(2), ','), "42,00");
    }

    #[test]
    fn a_unit_none_is_never_shown_is_absent() {
        assert_eq!(format_unit(Some("GB"), UnitDisplay::None, None), "");
        assert_eq!(format_unit(None, UnitDisplay::Short, None), "");
        assert_eq!(format_unit(Some(""), UnitDisplay::Short, None), "");
    }

    #[test]
    fn short_units_attach_percent_and_degrees() {
        assert_eq!(format_unit(Some("GB"), UnitDisplay::Short, None), " GB");
        assert_eq!(format_unit(Some("%"), UnitDisplay::Short, None), "%");
        assert_eq!(format_unit(Some("°"), UnitDisplay::Short, None), "°");
    }

    #[test]
    fn a_long_unit_is_the_ones_the_caller_has_and_the_symbol_otherwise() {
        let long = BTreeMap::from([("GB".to_string(), "gigabytes".to_string())]);
        assert_eq!(
            format_unit(Some("GB"), UnitDisplay::Long, Some(&long)),
            " gigabytes"
        );
        assert_eq!(format_unit(Some("GB"), UnitDisplay::Long, None), " GB");
    }
}
