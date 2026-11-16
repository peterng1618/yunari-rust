//! The canonical condition vocabulary every weather backend maps into.
//!
//! WMO 4677 present-weather codes, the set Open-Meteo reports. An unknown code
//! says "Unknown" rather than putting a number where prose belongs.
//!
//! ponytail: English only. The host has no language setting yet; the upgraded
//! form carries the language through `Provider::sample` and adds the German and
//! Vietnamese tables beside this one.

/// The words a code reads as, or `Unknown` for a code outside the table.
pub fn condition_text(code: i64) -> &'static str {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Depositing rime fog",
        51 => "Light drizzle",
        53 => "Moderate drizzle",
        55 => "Dense drizzle",
        56 => "Light freezing drizzle",
        57 => "Dense freezing drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        66 => "Light freezing rain",
        67 => "Heavy freezing rain",
        71 => "Slight snow",
        73 => "Moderate snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Slight rain showers",
        81 => "Moderate rain showers",
        82 => "Violent rain showers",
        85 => "Slight snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 => "Thunderstorm with slight hail",
        99 => "Thunderstorm with heavy hail",
        _ => "Unknown",
    }
}

/// The words a place the host located itself reads as. A place the author chose
/// is their data and is shown as written.
pub fn location_text() -> &'static str {
    "Windows location"
}

/// Sixteen points, beginning at north.
const COMPASS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW",
    "NNW",
];

/// The nearest compass point a bearing names; 348.75° rounds to north.
pub fn wind_compass(degrees: f64) -> &'static str {
    if !degrees.is_finite() {
        return "N";
    }
    let index = (degrees / 22.5).round() as i64;
    COMPASS[(index.rem_euclid(COMPASS.len() as i64)) as usize]
}

/// WMO code → vendored Meteocons `fill` slug, one entry per canonical code.
///
/// The allow-list is also the icon route's allow-list: a slug that is not here
/// is not served, never a neighbouring picture.
const ICONS: &[(i64, &str, &str)] = &[
    (0, "clear-day", "clear-night"),
    (1, "mostly-clear-day", "mostly-clear-night"),
    (2, "partly-cloudy-day", "partly-cloudy-night"),
    (3, "overcast-day", "overcast-night"),
    (45, "fog-day", "fog-night"),
    (48, "fog", "fog"),
    (51, "drizzle", "drizzle"),
    (53, "drizzle", "drizzle"),
    (55, "drizzle", "drizzle"),
    (56, "sleet", "sleet"),
    (57, "sleet", "sleet"),
    (61, "rain", "rain"),
    (63, "rain", "rain"),
    (65, "rain", "rain"),
    (66, "sleet", "sleet"),
    (67, "sleet", "sleet"),
    (71, "snow", "snow"),
    (73, "snow", "snow"),
    (75, "snow", "snow"),
    (77, "snow", "snow"),
    (80, "partly-cloudy-day-rain", "partly-cloudy-night-rain"),
    (81, "rain", "rain"),
    (82, "rain", "rain"),
    (85, "partly-cloudy-day-snow", "partly-cloudy-night-snow"),
    (86, "snow", "snow"),
    (95, "thunderstorms-day", "thunderstorms-night"),
    (96, "thunderstorms-day-rain", "thunderstorms-night-rain"),
    (99, "thunderstorms-day-rain", "thunderstorms-night-rain"),
];

/// Every slug this build can serve.
pub fn vendored_icon_slugs() -> impl Iterator<Item = &'static str> {
    ICONS
        .iter()
        .flat_map(|(_, day, night)| [*day, *night])
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
}

pub fn is_vendored_icon(slug: &str) -> bool {
    ICONS
        .iter()
        .any(|(_, day, night)| *day == slug || *night == slug)
}

/// The host-relative icon a reading names, or nothing when no picture maps.
pub fn icon_url(code: i64, is_day: bool) -> Option<String> {
    let (_, day, night) = ICONS.iter().find(|(key, ..)| *key == code)?;
    let slug = if is_day { day } else { night };
    is_vendored_icon(slug).then(|| format!("/api/weather/icon/{slug}.svg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_reads_as_prose_and_an_unknown_code_is_unknown() {
        assert_eq!(condition_text(3), "Overcast");
        assert_eq!(condition_text(61), "Slight rain");
        assert_eq!(condition_text(1234), "Unknown");
    }

    #[test]
    fn a_bearing_names_the_nearest_compass_point() {
        assert_eq!(wind_compass(0.0), "N");
        assert_eq!(wind_compass(90.0), "E");
        assert_eq!(wind_compass(348.75), "N");
        assert_eq!(wind_compass(350.0), "N");
        assert_eq!(wind_compass(225.0), "SW");
        // A bearing outside the circle wraps rather than panicking.
        assert_eq!(wind_compass(-90.0), "W");
        assert_eq!(wind_compass(f64::NAN), "N");
    }

    #[test]
    fn an_icon_names_the_day_and_the_night_picture() {
        assert_eq!(
            icon_url(0, true).as_deref(),
            Some("/api/weather/icon/clear-day.svg")
        );
        assert_eq!(
            icon_url(0, false).as_deref(),
            Some("/api/weather/icon/clear-night.svg")
        );
        assert_eq!(
            icon_url(75, true).as_deref(),
            Some("/api/weather/icon/snow.svg")
        );
        // A code with no picture is a gap, not a neighbouring picture.
        assert_eq!(icon_url(1234, true), None);
    }

    #[test]
    fn every_named_slug_is_served_exactly_as_written() {
        assert!(is_vendored_icon("clear-day"));
        assert!(!is_vendored_icon("clear-day.svg"));
        assert!(!is_vendored_icon("../clear-day"));
        assert!(vendored_icon_slugs().all(is_vendored_icon));
    }
}
