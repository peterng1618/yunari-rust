//! OpenWeatherMap's current and five-day forecast endpoints, normalized to one
//! reading.
//!
//! The key rides in the query string by this API's design, so it is read only
//! here and never named in a reason. Two requests are needed because the current
//! answer has no daily high or low; both go through the shared bounded request.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;

use super::open_meteo::{Fetch, Location, Weather};
use super::wire;

pub const CURRENT_ENDPOINT: &str = "https://api.openweathermap.org/data/2.5/weather";
pub const FORECAST_ENDPOINT: &str = "https://api.openweathermap.org/data/2.5/forecast";
/// Verified 2026-10-24 from OpenWeather's FAQ: the required wording is
/// "Weather data provided by OpenWeather".
pub const ATTRIBUTION: &str = "Weather data provided by OpenWeather";
pub const TERMS_URL: &str = "https://openweathermap.org/";
/// The credential id this backend's key is stored under.
pub const CREDENTIAL_ID: &str = "openweathermap";

const LABEL: &str = "OpenWeatherMap";

/// OpenWeatherMap's condition id to the canonical WMO code.
const CONDITION_BY_ID: &[(i64, i64)] = &[
    (800, 0),
    (801, 1),
    (802, 2),
    (803, 3),
    (804, 3),
    (200, 95),
    (201, 95),
    (202, 95),
    (210, 95),
    (211, 95),
    (212, 95),
    (221, 95),
    (230, 95),
    (231, 95),
    (232, 96),
    (300, 51),
    (301, 51),
    (302, 53),
    (310, 53),
    (311, 53),
    (312, 55),
    (313, 55),
    (314, 55),
    (321, 55),
    (500, 61),
    (501, 63),
    (502, 65),
    (503, 65),
    (504, 65),
    (511, 66),
    (520, 80),
    (521, 81),
    (522, 82),
    (531, 81),
    (600, 71),
    (601, 73),
    (602, 75),
    (611, 66),
    (612, 66),
    (613, 67),
    (615, 66),
    (616, 67),
    (620, 85),
    (621, 86),
    (622, 86),
    (701, 45),
    (711, 45),
    (721, 45),
    (731, 45),
    (741, 45),
    (751, 45),
    (761, 45),
    (762, 45),
];

/// Two bounded requests, then one parse. `now_ms` is this host's clock, which
/// the answer's own sun times and forecast buckets are compared against.
pub fn fetch(
    location: &Location,
    fetch: &Fetch,
    key: &str,
    now_ms: u64,
) -> Result<Weather, String> {
    let current = wire::json(
        fetch,
        &url_for(CURRENT_ENDPOINT, location, key),
        wire::TIMEOUT_MS,
        LABEL,
    )?;
    let forecast = wire::json(
        fetch,
        &url_for(FORECAST_ENDPOINT, location, key),
        wire::TIMEOUT_MS,
        LABEL,
    )?;
    parse(&current, &forecast, now_ms)
}

pub fn url_for(endpoint: &str, location: &Location, key: &str) -> String {
    format!(
        "{endpoint}?lat={}&lon={}&units=metric&appid={}",
        location.latitude,
        location.longitude,
        crate::http::encode(key)
    )
}

/// A required credential, named by the store id it lives under.
pub fn required_key(key: Option<&str>) -> Result<String, String> {
    match key.map(str::trim) {
        Some(key) if !key.is_empty() => Ok(key.to_owned()),
        _ => Err(format!("Set {CREDENTIAL_ID} in Settings.")),
    }
}

pub fn parse(current: &Value, forecast: &Value, now_ms: u64) -> Result<Weather, String> {
    let condition = wire::record(
        wire::list(current, "weather", LABEL)?.first(),
        "weather",
        LABEL,
    )?;
    let main = wire::object(current, "main", LABEL)?;
    let wind = wire::object(current, "wind", LABEL)?;
    let sys = wire::object(current, "sys", LABEL)?;
    let sunrise = wire::number(sys, "sunrise", LABEL)? as i64;
    let sunset = wire::number(sys, "sunset", LABEL)? as i64;
    let range = today_range(forecast, now_ms, main)?;

    Ok(Weather {
        temp_c: wire::number(main, "temp", LABEL)?,
        feels_like_c: wire::number(main, "feels_like", LABEL)?,
        high_c: range.0,
        low_c: range.1,
        humidity: wire::number(main, "humidity", LABEL)?,
        pressure_hpa: wire::number(main, "pressure", LABEL)?,
        precipitation_mm: precipitation_of(current)?,
        // The metric request reports wind in metres per second; the vocabulary's
        // unit is km/h.
        wind_kph: wire::number(wind, "speed", LABEL)? * 3.6,
        wind_deg: wire::number(wind, "deg", LABEL)?,
        code: canonical_code(wire::number(condition, "id", LABEL)? as i64)?,
        is_day: now_ms >= (sunrise.max(0) as u64) * 1000 && now_ms < (sunset.max(0) as u64) * 1000,
        sunrise: instant_of_seconds(sunrise)?,
        sunset: instant_of_seconds(sunset)?,
    })
}

/// The Connections check: one current-conditions request for London.
pub fn verify(key: &str, fetch: &Fetch) -> Result<(), String> {
    let url = format!(
        "{CURRENT_ENDPOINT}?q=London&units=metric&appid={}",
        crate::http::encode(key)
    );
    wire::json(fetch, &url, wire::TIMEOUT_MS, LABEL).map(|_| ())
}

fn canonical_code(id: i64) -> Result<i64, String> {
    CONDITION_BY_ID
        .iter()
        .find(|(from, _)| *from == id)
        .map(|(_, to)| *to)
        .ok_or_else(|| format!("{LABEL}'s condition id {id} has no canonical condition."))
}

fn instant_of_seconds(seconds: i64) -> Result<String, String> {
    let millis = seconds
        .checked_mul(1000)
        .ok_or_else(|| format!("{LABEL}'s answer has an unreadable sun time."))?;
    let instant = Timestamp::from_millisecond(millis)
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?;
    Ok(instant.to_string())
}

/// An absent `rain` key is this API's way of saying it reported no rain, and
/// 0 mm is that statement rather than a reading invented in its place.
fn precipitation_of(current: &Value) -> Result<f64, String> {
    let Some(rain) = current.get("rain") else {
        return Ok(0.0);
    };
    let rain = wire::record(Some(rain), "rain", LABEL)?;
    match rain.get("1h") {
        None => Ok(0.0),
        Some(_) => wire::number(rain, "1h", LABEL),
    }
}

/// The local calendar date a forecast entry belongs to. `city.timezone` is a
/// seconds offset from UTC, and the day boundaries a person sees are local
/// ones: bucketing in UTC would put a 23:00 entry on the wrong day.
fn local_date(dt_seconds: i64, offset_seconds: i64) -> Option<String> {
    let millis = dt_seconds.checked_add(offset_seconds)?.checked_mul(1000)?;
    let instant = Timestamp::from_millisecond(millis).ok()?;
    Some(instant.to_zoned(TimeZone::UTC).date().to_string())
}

/// Today's high and low from the three-hourly list, by the city's own date.
fn today_range(forecast: &Value, now_ms: u64, current: &Value) -> Result<(f64, f64), String> {
    let city = wire::object(forecast, "city", LABEL)?;
    let offset = wire::number(city, "timezone", LABEL)? as i64;
    let now_seconds = (now_ms / 1000) as i64;
    let Some(today) = local_date(now_seconds, offset) else {
        return Err(format!("{LABEL}'s answer has an unreadable time."));
    };
    let mut high: Option<f64> = None;
    let mut low: Option<f64> = None;
    for entry in wire::list(forecast, "list", LABEL)? {
        let row = wire::record(Some(entry), "list", LABEL)?;
        let dt = wire::number(row, "dt", LABEL)? as i64;
        let Some(date) = local_date(dt, offset) else {
            return Err(format!("{LABEL}'s answer has an unreadable time."));
        };
        if date != today {
            continue;
        }
        let main = wire::object(row, "main", LABEL)?;
        let entry_high = wire::number(main, "temp_max", LABEL)?;
        let entry_low = wire::number(main, "temp_min", LABEL)?;
        high = Some(high.map_or(entry_high, |value: f64| value.max(entry_high)));
        low = Some(low.map_or(entry_low, |value: f64| value.min(entry_low)));
    }
    // ponytail: just before local midnight the three-hourly list can hold no
    // row for today, and failing the whole acquisition daily is worse than the
    // current answer's own range. The upgrade is a forecast list that names
    // today explicitly.
    match (high, low) {
        (Some(high), Some(low)) => Ok((high, low)),
        _ => Ok((
            wire::number(current, "temp_max", LABEL)?,
            wire::number(current, "temp_min", LABEL)?,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-24 12:00 UTC; Berlin is two hours ahead.
    const NOW_MS: u64 = 1_792_843_200_000;

    fn current() -> Value {
        serde_json::json!({
            "weather": [{"id": 500}],
            "main": {
                "temp": 18.4,
                "feels_like": 17.9,
                "humidity": 72.0,
                "pressure": 1013.2,
                "temp_max": 19.0,
                "temp_min": 13.0
            },
            "wind": {"speed": 3.1, "deg": 225.0},
            "rain": {"1h": 0.4},
            "sys": {"sunrise": 1_792_836_000_i64, "sunset": 1_792_870_000_i64}
        })
    }

    fn forecast() -> Value {
        serde_json::json!({
            "city": {"timezone": 7200},
            "list": [
                {"dt": 1_792_836_000_i64, "main": {"temp_max": 21.0, "temp_min": 12.5}},
                {"dt": 1_792_846_800_i64, "main": {"temp_max": 23.0, "temp_min": 11.0}},
                // The next local day, which must not widen today's range.
                {"dt": 1_792_914_000_i64, "main": {"temp_max": 30.0, "temp_min": 1.0}}
            ]
        })
    }

    #[test]
    fn an_answer_maps_to_every_field_the_keys_read() {
        let weather = parse(&current(), &forecast(), NOW_MS).expect("an answer");
        assert_eq!(weather.temp_c, 18.4);
        assert_eq!(weather.high_c, 23.0);
        assert_eq!(weather.low_c, 11.0);
        assert_eq!(weather.code, 61);
        assert_eq!(weather.precipitation_mm, 0.4);
        assert!((weather.wind_kph - 11.16).abs() < 1e-9);
        assert_eq!(weather.sunrise, "2026-10-24T10:00:00Z");
        assert_eq!(weather.sunset, "2026-10-24T19:26:40Z");
        assert!(weather.is_day);
    }

    #[test]
    fn an_absent_rain_key_is_no_rain_and_an_unknown_id_is_refused() {
        let mut current = current();
        current.as_object_mut().unwrap().remove("rain");
        assert_eq!(
            parse(&current, &forecast(), NOW_MS)
                .unwrap()
                .precipitation_mm,
            0.0
        );

        let mut current = self::current();
        current["weather"][0]["id"] = serde_json::json!(42);
        assert_eq!(
            parse(&current, &forecast(), NOW_MS).unwrap_err(),
            "OpenWeatherMap's condition id 42 has no canonical condition."
        );
    }

    #[test]
    fn a_day_with_no_matching_forecast_row_uses_the_current_range() {
        let forecast = serde_json::json!({"city": {"timezone": 7200}, "list": []});
        let weather = parse(&current(), &forecast, NOW_MS).expect("an answer");
        assert_eq!(weather.high_c, 19.0);
        assert_eq!(weather.low_c, 13.0);
    }

    #[test]
    fn the_request_names_the_place_and_encodes_the_key() {
        let url = url_for(
            CURRENT_ENDPOINT,
            &Location {
                latitude: 52.52,
                longitude: 13.405,
            },
            "a b&c",
        );
        assert!(url.starts_with("https://api.openweathermap.org/data/2.5/weather?"));
        assert!(url.contains("lat=52.52"));
        assert!(url.contains("lon=13.405"));
        assert!(url.contains("appid=a%20b%26c"));
        assert_eq!(
            required_key(None).unwrap_err(),
            "Set openweathermap in Settings."
        );
    }
}
