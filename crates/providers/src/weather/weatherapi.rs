//! WeatherAPI's `forecast.json`, normalized to one reading.
//!
//! The key rides in the query string by this API's design, so it is read only
//! here and never named in a reason. Its condition codes are not WMO: every code
//! the table below does not carry fails the acquisition rather than borrowing a
//! neighbouring condition.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;

use super::open_meteo::{Fetch, Location, Weather};
use super::wire;

pub const ENDPOINT: &str = "https://api.weatherapi.com/v1/forecast.json";
/// Verified 2026-10-24 from WeatherAPI's attribution page: the text option is
/// "Powered by WeatherAPI.com".
pub const ATTRIBUTION: &str = "Powered by WeatherAPI.com";
pub const TERMS_URL: &str = "https://www.weatherapi.com/";
/// The credential id this backend's key is stored under.
pub const CREDENTIAL_ID: &str = "weatherapi";

const LABEL: &str = "WeatherAPI";

/// WeatherAPI's condition code to the canonical WMO code.
const CONDITION_BY_CODE: &[(i64, i64)] = &[
    (1000, 0),
    (1003, 1),
    (1006, 2),
    (1009, 3),
    (1030, 45),
    (1135, 45),
    (1147, 48),
    (1087, 95),
    (1114, 75),
    (1117, 75),
    (1150, 51),
    (1153, 51),
    (1168, 56),
    (1171, 57),
    (1180, 61),
    (1183, 61),
    (1186, 63),
    (1189, 63),
    (1192, 65),
    (1195, 65),
    (1198, 66),
    (1201, 67),
    (1204, 66),
    (1207, 67),
    (1210, 71),
    (1213, 71),
    (1216, 73),
    (1219, 73),
    (1222, 75),
    (1225, 75),
    (1237, 77),
    (1240, 80),
    (1243, 81),
    (1246, 82),
    (1249, 66),
    (1252, 67),
    (1255, 85),
    (1258, 86),
    (1264, 86),
    (1261, 85),
    (1273, 95),
    (1276, 96),
    (1279, 95),
    (1282, 99),
];

/// One bounded request, then one parse. `now_ms` is this host's clock, which
/// the answer's own local time is compared against for the place's offset.
pub fn fetch(
    location: &Location,
    fetch: &Fetch,
    key: &str,
    now_ms: u64,
) -> Result<Weather, String> {
    let payload = wire::json(fetch, &url_for(location, key), wire::TIMEOUT_MS, LABEL)?;
    parse(&payload, now_ms)
}

pub fn url_for(location: &Location, key: &str) -> String {
    format!(
        "{ENDPOINT}?key={}&q={},{}&days=1&aqi=no&alerts=no&lang=en",
        crate::http::encode(key),
        location.latitude,
        location.longitude
    )
}

/// A required credential, named by the store id it lives under.
pub fn required_key(key: Option<&str>) -> Result<String, String> {
    match key.map(str::trim) {
        Some(key) if !key.is_empty() => Ok(key.to_owned()),
        _ => Err(format!("Set {CREDENTIAL_ID} in Settings.")),
    }
}

pub fn parse(payload: &Value, now_ms: u64) -> Result<Weather, String> {
    let place = wire::object(payload, "location", LABEL)?;
    let current = wire::object(payload, "current", LABEL)?;
    let forecast = wire::object(payload, "forecast", LABEL)?;
    let days = wire::list(forecast, "forecastday", LABEL)?;
    let today = wire::record(days.first(), "forecastday", LABEL)?;
    let day = wire::object(today, "day", LABEL)?;
    let astro = wire::object(today, "astro", LABEL)?;
    let condition = wire::object(current, "condition", LABEL)?;

    let localtime = wire::text(place, "localtime", LABEL)?;
    let local_date = localtime.get(..10).unwrap_or_default();
    if !is_local_date(local_date) {
        return Err(format!("{LABEL}'s answer has an unreadable local time."));
    }
    let offset_ms = wire::number(place, "localtime_epoch", LABEL)? as i64 * 1000 - now_ms as i64;
    let daylight = wire::number(current, "is_day", LABEL)?;
    if daylight != 0.0 && daylight != 1.0 {
        return Err(format!("{LABEL}'s answer has an unreadable is_day."));
    }

    Ok(Weather {
        temp_c: wire::number(current, "temp_c", LABEL)?,
        feels_like_c: wire::number(current, "feelslike_c", LABEL)?,
        high_c: wire::number(day, "maxtemp_c", LABEL)?,
        low_c: wire::number(day, "mintemp_c", LABEL)?,
        humidity: wire::number(current, "humidity", LABEL)?,
        pressure_hpa: wire::number(current, "pressure_mb", LABEL)?,
        precipitation_mm: wire::number(current, "precip_mm", LABEL)?,
        wind_kph: wire::number(current, "wind_kph", LABEL)?,
        wind_deg: wire::number(current, "wind_degree", LABEL)?,
        code: canonical_code(wire::number(condition, "code", LABEL)? as i64)?,
        is_day: daylight == 1.0,
        sunrise: sun_instant(&astro["sunrise"], local_date, offset_ms)?,
        sunset: sun_instant(&astro["sunset"], local_date, offset_ms)?,
    })
}

/// The Connections check: one request for London.
pub fn verify(key: &str, fetch: &Fetch) -> Result<(), String> {
    let url = format!(
        "{ENDPOINT}?key={}&q=London&days=1",
        crate::http::encode(key)
    );
    wire::json(fetch, &url, wire::TIMEOUT_MS, LABEL).map(|_| ())
}

fn is_local_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn canonical_code(code: i64) -> Result<i64, String> {
    CONDITION_BY_CODE
        .iter()
        .find(|(from, _)| *from == code)
        .map(|(_, to)| *to)
        .ok_or_else(|| format!("{LABEL}'s condition code {code} has no canonical condition."))
}

/// A 12-hour wall clock and the place's local date are one instant: the date
/// names the day, and the answer's own clock offset prices the wall time.
fn sun_instant(wall: &Value, local_date: &str, offset_ms: i64) -> Result<String, String> {
    let Some(wall) = wall.as_str() else {
        return Err(format!("{LABEL}'s answer is missing a sun time."));
    };
    let Some((clock, meridiem)) = wall.trim().split_once(' ') else {
        return Err(format!("{LABEL}'s answer has an unreadable sun time."));
    };
    let Some((hour, minute)) = clock.split_once(':') else {
        return Err(format!("{LABEL}'s answer has an unreadable sun time."));
    };
    let (Ok(hour), Ok(minute)) = (hour.parse::<u32>(), minute.parse::<u32>()) else {
        return Err(format!("{LABEL}'s answer has an unreadable sun time."));
    };
    if minute > 59 {
        return Err(format!("{LABEL}'s answer has an unreadable sun time."));
    }
    let afternoon = match meridiem.to_ascii_uppercase().as_str() {
        "AM" => false,
        "PM" => true,
        _ => return Err(format!("{LABEL}'s answer has an unreadable sun time.")),
    };
    let hour = hour % 12 + if afternoon { 12 } else { 0 };
    let wall = format!("{local_date}T{hour:02}:{minute:02}:00");
    let wall: jiff::civil::DateTime = wall
        .parse()
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?;
    let utc = wall
        .to_zoned(TimeZone::UTC)
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?
        .timestamp()
        .as_millisecond();
    let instant = Timestamp::from_millisecond(utc - offset_ms)
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?;
    Ok(instant.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-24 12:00 UTC, and noon in Berlin two hours later.
    const NOW_MS: u64 = 1_792_843_200_000;

    fn payload() -> Value {
        serde_json::json!({
            "location": {
                "localtime": "2026-10-24 14:00",
                "localtime_epoch": 1_792_850_400
            },
            "current": {
                "temp_c": 18.4,
                "feelslike_c": 17.9,
                "humidity": 72.0,
                "pressure_mb": 1013.2,
                "precip_mm": 0.4,
                "wind_kph": 11.2,
                "wind_degree": 225.0,
                "is_day": 1,
                "condition": {"code": 1183}
            },
            "forecast": {
                "forecastday": [{
                    "day": {"maxtemp_c": 21.0, "mintemp_c": 12.5},
                    "astro": {"sunrise": "07:47 AM", "sunset": "05:52 PM"}
                }]
            }
        })
    }

    #[test]
    fn an_answer_maps_to_every_field_the_keys_read() {
        let weather = parse(&payload(), NOW_MS).expect("an answer");
        assert_eq!(weather.temp_c, 18.4);
        assert_eq!(weather.high_c, 21.0);
        assert_eq!(weather.low_c, 12.5);
        assert_eq!(weather.code, 61);
        assert!(weather.is_day);
        // 07:47 local at UTC+2 is 05:47 UTC; 17:52 local is 15:52 UTC.
        assert_eq!(weather.sunrise, "2026-10-24T05:47:00Z");
        assert_eq!(weather.sunset, "2026-10-24T15:52:00Z");
    }

    #[test]
    fn an_unknown_condition_or_unreadable_sun_time_is_refused() {
        let mut payload = payload();
        payload["current"]["condition"]["code"] = serde_json::json!(1);
        assert_eq!(
            parse(&payload, NOW_MS).unwrap_err(),
            "WeatherAPI's condition code 1 has no canonical condition."
        );

        let mut payload = self::payload();
        payload["forecast"]["forecastday"][0]["astro"]["sunrise"] = serde_json::json!("noon");
        assert_eq!(
            parse(&payload, NOW_MS).unwrap_err(),
            "WeatherAPI's answer has an unreadable sun time."
        );

        let mut payload = self::payload();
        payload["location"]["localtime"] = serde_json::json!("monday");
        assert_eq!(
            parse(&payload, NOW_MS).unwrap_err(),
            "WeatherAPI's answer has an unreadable local time."
        );
    }

    #[test]
    fn the_request_names_the_place_and_encodes_the_key() {
        let url = url_for(
            &Location {
                latitude: 52.52,
                longitude: 13.405,
            },
            "a b&c",
        );
        assert!(url.starts_with("https://api.weatherapi.com/v1/forecast.json?"));
        assert!(url.contains("key=a%20b%26c"));
        assert!(url.contains("q=52.52,13.405"));
        assert!(url.contains("days=1"));
    }

    #[test]
    fn a_missing_key_is_a_settings_sentence_not_a_request() {
        assert_eq!(
            required_key(None).unwrap_err(),
            "Set weatherapi in Settings."
        );
        assert_eq!(
            required_key(Some("  ")).unwrap_err(),
            "Set weatherapi in Settings."
        );
        assert_eq!(required_key(Some("k")).unwrap(), "k");
    }
}
