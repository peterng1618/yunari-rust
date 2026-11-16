//! The canonical condition vocabulary every weather backend maps into.
//!
//! WMO 4677 present-weather codes, the set Open-Meteo reports. An unknown code
//! says "Unknown" rather than putting a number where prose belongs.
//!
//! A closed vocabulary, not a translation platform: English, German and
//! Vietnamese tables, and anything else reads in English.

/// The words a code reads as, or `Unknown` for a code outside the table.
pub fn condition_text(code: i64) -> &'static str {
    condition_text_in(None, code)
}

/// The words a code reads as in one language, falling back to English.
pub fn condition_text_in(language: Option<&str>, code: i64) -> &'static str {
    let table: &'static [(i64, &'static str)] = match primary_subtag(language).as_str() {
        "de" => DE_CONDITIONS,
        "vi" => VI_CONDITIONS,
        _ => EN_CONDITIONS,
    };
    table
        .iter()
        .find(|(key, _)| *key == code)
        .map(|(_, text)| *text)
        .unwrap_or("Unknown")
}

/// `de-DE` reads the German table: a region tag differs from its language only.
fn primary_subtag(language: Option<&str>) -> String {
    language
        .unwrap_or_default()
        .split('-')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

const EN_CONDITIONS: &[(i64, &str)] = &[
    (0, "Clear sky"),
    (1, "Mainly clear"),
    (2, "Partly cloudy"),
    (3, "Overcast"),
    (45, "Fog"),
    (48, "Depositing rime fog"),
    (51, "Light drizzle"),
    (53, "Moderate drizzle"),
    (55, "Dense drizzle"),
    (56, "Light freezing drizzle"),
    (57, "Dense freezing drizzle"),
    (61, "Slight rain"),
    (63, "Moderate rain"),
    (65, "Heavy rain"),
    (66, "Light freezing rain"),
    (67, "Heavy freezing rain"),
    (71, "Slight snow"),
    (73, "Moderate snow"),
    (75, "Heavy snow"),
    (77, "Snow grains"),
    (80, "Slight rain showers"),
    (81, "Moderate rain showers"),
    (82, "Violent rain showers"),
    (85, "Slight snow showers"),
    (86, "Heavy snow showers"),
    (95, "Thunderstorm"),
    (96, "Thunderstorm with slight hail"),
    (99, "Thunderstorm with heavy hail"),
];

/// Kept beside the source vocabulary rather than translated at runtime: a
/// table is what a reviewer can check.
const DE_CONDITIONS: &[(i64, &str)] = &[
    (0, "Klarer Himmel"),
    (1, "Überwiegend klar"),
    (2, "Teilweise bewölkt"),
    (3, "Bedeckt"),
    (45, "Nebel"),
    (48, "Reifnebel"),
    (51, "Leichter Nieselregen"),
    (53, "Mäßiger Nieselregen"),
    (55, "Dichter Nieselregen"),
    (56, "Leichter gefrierender Nieselregen"),
    (57, "Dichter gefrierender Nieselregen"),
    (61, "Leichter Regen"),
    (63, "Mäßiger Regen"),
    (65, "Starker Regen"),
    (66, "Leichter gefrierender Regen"),
    (67, "Starker gefrierender Regen"),
    (71, "Leichter Schneefall"),
    (73, "Mäßiger Schneefall"),
    (75, "Starker Schneefall"),
    (77, "Schneegriesel"),
    (80, "Leichte Regenschauer"),
    (81, "Mäßige Regenschauer"),
    (82, "Heftige Regenschauer"),
    (85, "Leichte Schneeschauer"),
    (86, "Starke Schneeschauer"),
    (95, "Gewitter"),
    (96, "Gewitter mit leichtem Hagel"),
    (99, "Gewitter mit starkem Hagel"),
];

const VI_CONDITIONS: &[(i64, &str)] = &[
    (0, "Trời quang"),
    (1, "Chủ yếu quang"),
    (2, "Có mây rải rác"),
    (3, "U ám"),
    (45, "Sương mù"),
    (48, "Sương mù đọng giá"),
    (51, "Mưa phùn nhẹ"),
    (53, "Mưa phùn vừa"),
    (55, "Mưa phùn dày"),
    (56, "Mưa phùn đóng băng nhẹ"),
    (57, "Mưa phùn đóng băng dày"),
    (61, "Mưa nhẹ"),
    (63, "Mưa vừa"),
    (65, "Mưa to"),
    (66, "Mưa đóng băng nhẹ"),
    (67, "Mưa đóng băng to"),
    (71, "Tuyết rơi nhẹ"),
    (73, "Tuyết rơi vừa"),
    (75, "Tuyết rơi dày"),
    (77, "Hạt tuyết"),
    (80, "Mưa rào nhẹ"),
    (81, "Mưa rào vừa"),
    (82, "Mưa rào mạnh"),
    (85, "Mưa tuyết nhẹ"),
    (86, "Mưa tuyết mạnh"),
    (95, "Dông"),
    (96, "Dông kèm mưa đá nhẹ"),
    (99, "Dông kèm mưa đá mạnh"),
];

/// The words a place the host located itself reads as. A place the author chose
/// is their data and is shown as written.
pub fn location_text() -> &'static str {
    location_text_in(None)
}

pub fn location_text_in(language: Option<&str>) -> &'static str {
    match primary_subtag(language).as_str() {
        "de" => "Windows-Standort",
        "vi" => "Vị trí Windows",
        _ => "Windows location",
    }
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
    fn a_language_reads_its_own_table_and_anything_else_reads_english() {
        assert_eq!(condition_text_in(Some("de-DE"), 3), "Bedeckt");
        assert_eq!(condition_text_in(Some("vi"), 61), "Mưa nhẹ");
        assert_eq!(condition_text_in(Some("fr"), 3), "Overcast");
        assert_eq!(condition_text_in(None, 3), "Overcast");
        // An absent code is still unknown in every table.
        assert_eq!(condition_text_in(Some("de"), 1234), "Unknown");
        assert_eq!(location_text_in(Some("vi-VN")), "Vị trí Windows");
        assert_eq!(location_text_in(Some("de")), "Windows-Standort");
        assert_eq!(location_text_in(None), "Windows location");
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
