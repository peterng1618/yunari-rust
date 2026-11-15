//! What a display shows: the consumer's unit choices, applied at presentation.
//!
//! Conversion happens here and never in the sample. A provider reports what it
//! measured, and a display derives the form a reader wants; storing a converted
//! value would put a derived number where a measurement belongs.
//!
//! Ported from `renderer-core/scene/measurement.ts`, decision for decision.

use crate::vocabulary::{self, UnitKind};

const BYTES_PER_GIB: f64 = 1_073_741_824.0;
const INHG_IN_HPA: f64 = 33.863_886_666_7;
const MPH_PER_KMH: f64 = 0.621_371_192_237_334;

/// The consumer's display choices. Absent fields fall back to the defaults, so
/// a caller with no preferences still gets the measured units.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitPreferences {
    pub measurement: Option<Measurement>,
    pub clock: Option<Clock>,
    /// Binary keeps the stored 2^30 figure; decimal converts it to 10^9.
    pub data: Option<Data>,
    pub data_scale: Option<DataScale>,
    pub rate: Option<Rate>,
    pub rate_scale: Option<RateScale>,
    pub wind_speed: Option<WindSpeed>,
    pub pressure: Option<Pressure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measurement {
    Metric,
    Imperial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    Mhz,
    Ghz,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Data {
    Binary,
    Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataScale {
    Auto,
    Mb,
    Gb,
    Tb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rate {
    Bits,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateScale {
    Auto,
    K,
    M,
    G,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindSpeed {
    Kmh,
    Mph,
    Ms,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Hpa,
    Inhg,
}

/// A binding's own unit choice, which wins over the consumer's preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Overrides<'a> {
    pub unit: Option<&'a str>,
    pub auto_scale: bool,
}

/// The value and unit to show, with the precision the chosen unit supplies
/// when the binding authored none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Converted<'a> {
    pub value: f64,
    pub unit: Option<&'a str>,
    pub precision: Option<u8>,
    /// The requested unit is not one this key can be shown in. The measured
    /// value is what gets shown, and the caller reports the refusal.
    pub unsupported: bool,
}

impl<'a> Converted<'a> {
    fn as_measured(value: f64, unit: Option<&'a str>) -> Self {
        Self {
            value,
            unit,
            precision: None,
            unsupported: false,
        }
    }
}

/// The value and unit a display shows for one reading.
///
/// Conversion runs once, at presentation, after the binding's authored
/// scale/offset. A unit that does not match the key's canonical unit is left as
/// measured rather than guessed at.
pub fn convert<'a>(
    key: &str,
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    let Some(described) = vocabulary::describe(key) else {
        return Converted::as_measured(value, unit);
    };
    if let Some(target) = overrides.unit
        && target != unit.unwrap_or_default()
        && !described.display_units.contains(&target)
    {
        // A plan-time issue, never a crash: the reading is still shown.
        return Converted {
            unsupported: true,
            ..Converted::as_measured(value, unit)
        };
    }
    let converted = match described.unit_kind {
        Some(UnitKind::Temperature) => temperature(value, unit, preferences, overrides),
        Some(UnitKind::Clock) => clock(value, unit, preferences, overrides),
        Some(UnitKind::Data) => data(value, unit, preferences, overrides),
        Some(UnitKind::DataRate) => rate(value, unit, preferences, overrides),
        Some(UnitKind::Speed) => wind_speed(value, unit, preferences, overrides),
        Some(UnitKind::Pressure) => pressure_of(value, unit, preferences, overrides),
        // Rotation and percent included: RPM is never guessed into a percentage.
        _ => Converted::as_measured(value, unit),
    };
    // An undeclared target the kind could not reach is still unsupported.
    if overrides.unit.is_some()
        && described.display_units.is_empty()
        && converted.unit != overrides.unit
    {
        return Converted {
            unsupported: true,
            ..Converted::as_measured(value, unit)
        };
    }
    converted
}

fn temperature<'a>(
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    if unit != Some("°C") {
        return Converted::as_measured(value, unit);
    }
    let imperial = match overrides.unit {
        Some("°F") => true,
        Some("°C") => false,
        _ => preferences.measurement == Some(Measurement::Imperial),
    };
    if imperial {
        Converted::as_measured(value * 1.8 + 32.0, Some("°F"))
    } else {
        Converted::as_measured(value, unit)
    }
}

fn clock<'a>(
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    if unit != Some("MHz") {
        return Converted::as_measured(value, unit);
    }
    let target = match overrides.unit {
        Some("GHz") => Clock::Ghz,
        Some("MHz") => Clock::Mhz,
        _ if overrides.auto_scale => Clock::Auto,
        _ => preferences.clock.unwrap_or(Clock::Auto),
    };
    if target == Clock::Ghz || (target == Clock::Auto && value >= 1000.0) {
        let value = value / 1000.0;
        // Below 10 GHz two decimals read; above, one. The value itself is never
        // rounded, so an authored precision can still take more.
        return Converted {
            value,
            unit: Some("GHz"),
            precision: Some(if value < 10.0 { 2 } else { 1 }),
            unsupported: false,
        };
    }
    Converted::as_measured(value, unit)
}

/// The bytes one display label stands for.
fn data_unit(label: &str, decimal: bool) -> Option<f64> {
    Some(match label {
        "MB" => {
            if decimal {
                1e6
            } else {
                u32::MAX as f64 + 1.0
            }
        }
        "MiB" => u32::MAX as f64 + 1.0,
        "GB" => {
            if decimal {
                1e9
            } else {
                BYTES_PER_GIB
            }
        }
        "GiB" => BYTES_PER_GIB,
        "TB" => {
            if decimal {
                1e12
            } else {
                1_099_511_627_776.0
            }
        }
        "TiB" => 1_099_511_627_776.0,
        _ => return None,
    })
}

fn data<'a>(
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    if unit != Some("GB") {
        return Converted::as_measured(value, unit);
    }
    let decimal = preferences.data == Some(Data::Decimal);
    let bytes = value * BYTES_PER_GIB;
    if let Some(target) = overrides.unit {
        return match data_unit(target, decimal) {
            Some(per_unit) => Converted::as_measured(bytes / per_unit, Some(target)),
            None => Converted::as_measured(value, unit),
        };
    }
    let scale = if overrides.auto_scale {
        DataScale::Auto
    } else {
        preferences.data_scale.unwrap_or(DataScale::Auto)
    };
    let label = match scale {
        // Binary crosses at 1024, decimal at 1000: the stored figure is
        // 2^30-based.
        DataScale::Auto => {
            let megabyte = if decimal { 1e9 } else { BYTES_PER_GIB };
            let gigabyte = if decimal { 1e12 } else { 1_099_511_627_776.0 };
            if bytes < megabyte {
                "MB"
            } else if bytes < gigabyte {
                "GB"
            } else {
                "TB"
            }
        }
        DataScale::Mb => "MB",
        DataScale::Gb => "GB",
        DataScale::Tb => "TB",
    };
    match data_unit(label, decimal) {
        Some(per_unit) => Converted::as_measured(bytes / per_unit, Some(label)),
        None => Converted::as_measured(value, unit),
    }
}

fn rate<'a>(
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    if unit != Some("Mb/s") && unit != Some("MB/s") {
        return Converted::as_measured(value, unit);
    }
    // Canonical rates are decimal bits or bytes per second, whichever the
    // family reports; both come back to bits so one scale rule governs them.
    let bits = if unit == Some("MB/s") { value * 8.0 } else { value };
    let (bytes, scale) = match overrides.unit {
        // `kb/s` is bits, `kB/s` is bytes; the leading letter is the magnitude.
        Some(explicit) => {
            let bytes = explicit.as_bytes().get(1) == Some(&b'B');
            let scale = if explicit.starts_with('k') {
                RateScale::K
            } else if explicit.starts_with('M') {
                RateScale::M
            } else {
                RateScale::G
            };
            (bytes, scale)
        }
        None => {
            // A byte-native family (disk throughput) stays in bytes; the
            // `rate` preference chooses the form of a bit-native one.
            let bytes =
                unit == Some("MB/s") || preferences.rate == Some(Rate::Bytes);
            let scale = if overrides.auto_scale {
                RateScale::Auto
            } else {
                preferences.rate_scale.unwrap_or(RateScale::Auto)
            };
            (bytes, scale)
        }
    };
    let magnitude = if bytes { bits / 8.0 } else { bits };
    let chosen = match scale {
        RateScale::Auto => {
            if magnitude < 1.0 {
                RateScale::K
            } else if magnitude < 1000.0 {
                RateScale::M
            } else {
                RateScale::G
            }
        }
        other => other,
    };
    let (shown, prefix) = match chosen {
        RateScale::K => (magnitude * 1000.0, "k"),
        RateScale::M => (magnitude, "M"),
        RateScale::Auto | RateScale::G => (magnitude / 1000.0, "G"),
    };
    let unit = match (prefix, bytes) {
        ("k", false) => "kb/s",
        ("k", true) => "kB/s",
        ("M", false) => "Mb/s",
        ("M", true) => "MB/s",
        (_, false) => "Gb/s",
        (_, true) => "GB/s",
    };
    Converted::as_measured(shown, Some(unit))
}

fn wind_speed<'a>(
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    if unit != Some("km/h") {
        return Converted::as_measured(value, unit);
    }
    let target = match overrides.unit {
        Some("mph") => WindSpeed::Mph,
        Some("m/s") => WindSpeed::Ms,
        Some("km/h") => WindSpeed::Kmh,
        _ => preferences.wind_speed.unwrap_or(WindSpeed::Kmh),
    };
    match target {
        WindSpeed::Mph => Converted::as_measured(value * MPH_PER_KMH, Some("mph")),
        WindSpeed::Ms => Converted::as_measured(value / 3.6, Some("m/s")),
        WindSpeed::Kmh => Converted::as_measured(value, unit),
    }
}

fn pressure_of<'a>(
    value: f64,
    unit: Option<&'a str>,
    preferences: &UnitPreferences,
    overrides: Overrides<'a>,
) -> Converted<'a> {
    if unit != Some("hPa") {
        return Converted::as_measured(value, unit);
    }
    let target = match overrides.unit {
        Some("inHg") => Pressure::Inhg,
        Some("hPa") => Pressure::Hpa,
        _ => preferences.pressure.unwrap_or(Pressure::Hpa),
    };
    if target == Pressure::Inhg {
        Converted::as_measured(value / INHG_IN_HPA, Some("inHg"))
    } else {
        Converted::as_measured(value, unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imperial() -> UnitPreferences {
        UnitPreferences {
            measurement: Some(Measurement::Imperial),
            ..UnitPreferences::default()
        }
    }

    #[test]
    fn a_temperature_follows_the_measurement_preference() {
        let converted = convert("cpu.temp", 25.0, Some("°C"), &imperial(), Overrides::default());
        assert_eq!(converted.unit, Some("°F"));
        assert!((converted.value - 77.0).abs() < 1e-9);
        assert!(!converted.unsupported);

        let metric = convert(
            "cpu.temp",
            25.0,
            Some("°C"),
            &UnitPreferences::default(),
            Overrides::default(),
        );
        assert_eq!((metric.value, metric.unit), (25.0, Some("°C")));
    }

    #[test]
    fn a_binding_unit_wins_over_the_preference() {
        let converted = convert(
            "cpu.temp",
            25.0,
            Some("°C"),
            &imperial(),
            Overrides {
                unit: Some("°C"),
                ..Overrides::default()
            },
        );
        assert_eq!(converted.unit, Some("°C"));
        assert_eq!(converted.value, 25.0);
    }

    #[test]
    fn a_clock_reads_gigahertz_with_the_decimals_that_fit() {
        let ghz = UnitPreferences {
            clock: Some(Clock::Ghz),
            ..UnitPreferences::default()
        };
        let converted = convert("cpu.clock", 3600.0, Some("MHz"), &ghz, Overrides::default());
        assert_eq!(converted.unit, Some("GHz"));
        assert!((converted.value - 3.6).abs() < 1e-9);
        assert_eq!(converted.precision, Some(2));

        let big = convert("cpu.clock", 12_500.0, Some("MHz"), &ghz, Overrides::default());
        assert_eq!(big.precision, Some(1));
    }

    #[test]
    fn an_auto_clock_switches_where_a_reading_stops_being_megahertz_sized() {
        let converted = convert(
            "cpu.clock",
            1200.0,
            Some("MHz"),
            &UnitPreferences::default(),
            Overrides::default(),
        );
        assert_eq!(converted.unit, Some("GHz"));
        let small = convert(
            "cpu.clock",
            500.0,
            Some("MHz"),
            &UnitPreferences::default(),
            Overrides::default(),
        );
        assert_eq!((small.value, small.unit), (500.0, Some("MHz")));
    }

    #[test]
    fn a_decimal_display_of_a_binary_figure_keeps_the_measured_bytes() {
        let decimal = UnitPreferences {
            data: Some(Data::Decimal),
            ..UnitPreferences::default()
        };
        let converted = convert(
            "disk.nvme0n1.used",
            1.0,
            Some("GB"),
            &decimal,
            Overrides::default(),
        );
        assert_eq!(converted.unit, Some("GB"));
        assert!((converted.value - 1.073_741_824).abs() < 1e-9);
    }

    #[test]
    fn a_binding_can_pin_a_binary_data_unit() {
        let converted = convert(
            "disk.nvme0n1.used",
            1.0,
            Some("GB"),
            &UnitPreferences::default(),
            Overrides {
                unit: Some("GiB"),
                ..Overrides::default()
            },
        );
        assert_eq!((converted.value, converted.unit), (1.0, Some("GiB")));
    }

    #[test]
    fn a_data_rate_keeps_a_byte_native_family_in_bytes() {
        let converted = convert(
            "disk.nvme0n1.read",
            250.0,
            Some("MB/s"),
            &UnitPreferences::default(),
            Overrides::default(),
        );
        assert_eq!(converted.unit, Some("MB/s"));
        assert_eq!(converted.value, 250.0);

        let network = convert(
            "network.ethernet.download",
            500.0,
            Some("Mb/s"),
            &UnitPreferences::default(),
            Overrides::default(),
        );
        assert_eq!((network.value, network.unit), (500.0, Some("Mb/s")));
    }

    #[test]
    fn a_unit_the_key_cannot_reach_is_refused_and_the_reading_kept() {
        let converted = convert(
            "cpu.load",
            42.0,
            Some("%"),
            &UnitPreferences::default(),
            Overrides {
                unit: Some("GiB"),
                ..Overrides::default()
            },
        );
        assert!(converted.unsupported);
        assert_eq!((converted.value, converted.unit), (42.0, Some("%")));
    }

    #[test]
    fn an_unknown_key_is_shown_as_measured() {
        let converted = convert(
            "nonsense.key",
            7.0,
            Some("widgets"),
            &imperial(),
            Overrides::default(),
        );
        assert_eq!((converted.value, converted.unit), (7.0, Some("widgets")));
        assert!(!converted.unsupported);
    }

    #[test]
    fn a_wind_speed_and_a_pressure_follow_their_preferences() {
        let converted = convert(
            "weather.windSpeed",
            36.0,
            Some("km/h"),
            &UnitPreferences {
                wind_speed: Some(WindSpeed::Ms),
                ..UnitPreferences::default()
            },
            Overrides::default(),
        );
        assert_eq!((converted.value, converted.unit), (10.0, Some("m/s")));

        let pressure = convert(
            "weather.pressure",
            1013.25,
            Some("hPa"),
            &UnitPreferences {
                pressure: Some(Pressure::Inhg),
                ..UnitPreferences::default()
            },
            Overrides::default(),
        );
        assert_eq!(pressure.unit, Some("inHg"));
        assert!((pressure.value - 29.921).abs() < 0.01);
    }
}
