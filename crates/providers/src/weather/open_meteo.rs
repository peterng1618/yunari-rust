//! Open-Meteo's current-conditions endpoint, normalized to one reading.
//!
//! The request names only the fields the weather keys need, and the answer is
//! bounded in size and time by [`crate::http`]. A reason never carries the URL:
//! the reason reaches every display on the network.

use std::sync::Arc;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde_json::Value;

use super::wire;

pub const ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";

/// Open-Meteo's licence (CC BY 4.0) asks for this credit, linking to their site.
pub const ATTRIBUTION: &str = "Weather data by Open-Meteo.com";
pub const TERMS_URL: &str = "https://open-meteo.com/en/licence";

const TIMEOUT_MS: u64 = 10_000;
const LABEL: &str = "Open-Meteo";

/// How a GET is performed. A seam so a test never reaches the network.
pub type Fetch = Arc<dyn Fn(&str, u64) -> Result<String, String> + Send + Sync>;

/// A place, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

/// One acquisition's worth of weather, in the vocabulary's own units.
#[derive(Debug, Clone, PartialEq)]
pub struct Weather {
    pub temp_c: f64,
    pub feels_like_c: f64,
    pub high_c: f64,
    pub low_c: f64,
    pub humidity: f64,
    pub pressure_hpa: f64,
    pub precipitation_mm: f64,
    pub wind_kph: f64,
    pub wind_deg: f64,
    pub code: i64,
    pub is_day: bool,
    pub sunrise: String,
    pub sunset: String,
}

/// One bounded request, then one parse.
pub fn fetch(location: &Location, fetch: &Fetch) -> Result<Weather, String> {
    let payload = super::wire::json(fetch, &url_for(location), TIMEOUT_MS, LABEL)?;
    parse(&payload)
}

/// The request the weather keys need: metric units are the endpoint's default.
pub fn url_for(location: &Location) -> String {
    format!(
        "{ENDPOINT}?latitude={}&longitude={}&current=temperature_2m,apparent_temperature,relative_humidity_2m,precipitation,weather_code,wind_speed_10m,wind_direction_10m,is_day,pressure_msl&daily=temperature_2m_max,temperature_2m_min,sunrise,sunset&timezone=auto&forecast_days=1",
        location.latitude, location.longitude
    )
}

/// One answer, as the fields the keys read.
pub fn parse(payload: &Value) -> Result<Weather, String> {
    let current = wire::object(payload, "current", LABEL)?;
    let daily = wire::object(payload, "daily", LABEL)?;
    let offset = wire::number(payload, "utc_offset_seconds", LABEL)? as i64;
    let daylight = wire::number(current, "is_day", LABEL)?;
    if daylight != 0.0 && daylight != 1.0 {
        return Err(format!("{LABEL}'s answer has an unreadable is_day."));
    }
    Ok(Weather {
        temp_c: wire::number(current, "temperature_2m", LABEL)?,
        feels_like_c: wire::number(current, "apparent_temperature", LABEL)?,
        high_c: first_number(daily, "temperature_2m_max")?,
        low_c: first_number(daily, "temperature_2m_min")?,
        humidity: wire::number(current, "relative_humidity_2m", LABEL)?,
        pressure_hpa: wire::number(current, "pressure_msl", LABEL)?,
        precipitation_mm: wire::number(current, "precipitation", LABEL)?,
        wind_kph: wire::number(current, "wind_speed_10m", LABEL)?,
        wind_deg: wire::number(current, "wind_direction_10m", LABEL)?,
        code: wire::number(current, "weather_code", LABEL)? as i64,
        is_day: daylight == 1.0,
        sunrise: first_instant(daily, "sunrise", offset)?,
        sunset: first_instant(daily, "sunset", offset)?,
    })
}

/// A local wall time (`2026-10-24T07:47`) plus the answer's own UTC offset is
/// one instant.
///
/// ponytail: a single offset is applied to both sun times. On a DST-change day
/// one of them can be an hour off; the upgrade is a per-day offset if the API
/// ever offers one.
pub fn instant_of(wall: &str, offset_seconds: i64) -> Result<String, String> {
    let with_seconds = if wall.len() == 16 {
        format!("{wall}:00")
    } else {
        wall.to_owned()
    };
    let wall: jiff::civil::DateTime = with_seconds
        .parse()
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?;
    let timestamp = wall
        .to_zoned(TimeZone::UTC)
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?
        .timestamp();
    let millis = timestamp
        .as_millisecond()
        .checked_sub(offset_seconds * 1000)
        .ok_or_else(|| format!("{LABEL}'s answer has an unreadable sun time."))?;
    let instant = Timestamp::from_millisecond(millis)
        .map_err(|_| format!("{LABEL}'s answer has an unreadable sun time."))?;
    Ok(instant.to_string())
}

fn first<'a>(record: &'a Value, field: &str) -> Result<&'a Value, String> {
    wire::first(record, field, LABEL)
}

fn first_number(record: &Value, field: &str) -> Result<f64, String> {
    first(record, field)?
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{LABEL}'s answer is missing {field}."))
}

fn first_instant(record: &Value, field: &str, offset_seconds: i64) -> Result<String, String> {
    let wall = first(record, field)?
        .as_str()
        .ok_or_else(|| format!("{LABEL}'s answer is missing a sun time."))?;
    instant_of(wall, offset_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Open-Meteo answers with, trimmed to the fields read here.
    fn payload(offset_seconds: i64) -> Value {
        serde_json::json!({
            "utc_offset_seconds": offset_seconds,
            "current": {
                "temperature_2m": 18.4,
                "apparent_temperature": 17.9,
                "relative_humidity_2m": 72.0,
                "precipitation": 0.0,
                "weather_code": 3.0,
                "wind_speed_10m": 11.2,
                "wind_direction_10m": 225.0,
                "is_day": 1.0,
                "pressure_msl": 1013.2
            },
            "daily": {
                "temperature_2m_max": [21.0],
                "temperature_2m_min": [12.5],
                "sunrise": ["2026-10-24T07:47"],
                "sunset": ["2026-10-24T17:52"]
            }
        })
    }

    #[test]
    fn an_answer_maps_to_every_field_the_keys_read() {
        let weather = parse(&payload(7_200)).expect("an answer");
        assert_eq!(weather.temp_c, 18.4);
        assert_eq!(weather.feels_like_c, 17.9);
        assert_eq!(weather.high_c, 21.0);
        assert_eq!(weather.low_c, 12.5);
        assert_eq!(weather.humidity, 72.0);
        assert_eq!(weather.pressure_hpa, 1013.2);
        assert_eq!(weather.wind_kph, 11.2);
        assert_eq!(weather.wind_deg, 225.0);
        assert_eq!(weather.code, 3);
        assert!(weather.is_day);
        // 07:47 local at UTC+2 is 05:47 UTC.
        assert_eq!(weather.sunrise, "2026-10-24T05:47:00Z");
        assert_eq!(weather.sunset, "2026-10-24T15:52:00Z");
    }

    #[test]
    fn a_missing_field_fails_the_acquisition_rather_than_reading_zero() {
        let mut payload = payload(0);
        payload["current"]
            .as_object_mut()
            .unwrap()
            .remove("temperature_2m");
        assert_eq!(
            parse(&payload).unwrap_err(),
            "Open-Meteo's answer is missing temperature_2m."
        );
        assert!(parse(&serde_json::json!({"current": {}})).is_err());
    }

    #[test]
    fn an_unreadable_day_flag_or_sun_time_is_refused() {
        let mut payload = payload(0);
        payload["current"]["is_day"] = serde_json::json!(2.0);
        assert_eq!(
            parse(&payload).unwrap_err(),
            "Open-Meteo's answer has an unreadable is_day."
        );

        let mut payload = self::payload(0);
        payload["daily"]["sunrise"] = serde_json::json!(["noon"]);
        assert_eq!(
            parse(&payload).unwrap_err(),
            "Open-Meteo's answer has an unreadable sun time."
        );
    }

    #[test]
    fn the_request_names_the_place_and_only_the_fields_read() {
        let url = url_for(&Location {
            latitude: 52.52,
            longitude: 13.405,
        });
        assert!(url.starts_with("https://api.open-meteo.com/v1/forecast?"));
        assert!(url.contains("latitude=52.52"));
        assert!(url.contains("longitude=13.405"));
        assert!(url.contains("weather_code"));
        assert!(url.contains("pressure_msl"));
        assert!(url.contains("forecast_days=1"));
    }

    #[test]
    fn an_unreachable_endpoint_is_a_reason_not_a_panic() {
        let get: Fetch = Arc::new(|_url, _timeout| Err("connection refused".to_owned()));
        let error = fetch_weather(&get).unwrap_err();
        assert!(error.contains("could not be reached"));
        assert!(!error.contains("https://"), "{error}");
    }

    fn fetch_weather(get: &Fetch) -> Result<Weather, String> {
        super::fetch(
            &Location {
                latitude: 0.0,
                longitude: 0.0,
            },
            get,
        )
    }
}
