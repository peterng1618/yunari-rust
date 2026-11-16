//! Weather as readings, acquired on the provider's own clock rather than inside
//! the host's poll.
//!
//! `sample()` is a cache read: a display asking for a weather key records demand
//! and gets whatever the last acquisition produced. One refresher fetches on the
//! consumer's cadence, backs off on failure, and stops once no display has asked
//! for longer than the window that makes a reading stale.

pub mod conditions;
pub mod location;
pub mod open_meteo;
pub mod openweathermap;
pub mod weatherapi;
pub mod wire;

use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::provider::{Descriptor, Health, Provider};
use crate::sample::Reading;
use crate::vocabulary;
use open_meteo::{Fetch, Location, Weather};

pub const KEYS: &[&str] = &[
    "weather.location",
    "weather.temp",
    "weather.feelsLike",
    "weather.conditionCode",
    "weather.condition",
    "weather.icon",
    "weather.humidity",
    "weather.windSpeed",
    "weather.windDirection",
    "weather.windDirection.compass",
    "weather.pressure",
    "weather.precipitation",
    "weather.isDay",
    "weather.sunrise",
    "weather.sunset",
    "weather.high",
    "weather.low",
    "weather.attribution",
];

/// What a display is told when no place has been chosen yet.
pub const NO_PLACE_MESSAGE: &str = "Set a place in Settings, or turn on Windows location.";
pub const NO_ICON_MESSAGE: &str = "No icon is available for this condition.";
pub const WAITING_MESSAGE: &str = "Waiting for the first weather reading.";
pub const STALE_MESSAGE: &str = "The last weather reading is out of date.";
pub const UNSUPPORTED_BACKEND_MESSAGE: &str = "That weather provider is not one this host knows.";
/// Windows location is reused for this long before asking again.
pub const POSITION_TTL_MS: u64 = 24 * 60 * 60_000;
const NO_KEY_MESSAGE: &str = "This host does not read that weather key.";

const MIN_IDLE_WINDOW_MS: u64 = 30 * 60_000;
const MAX_BACKOFF_MS: u64 = 60 * 60_000;
/// How often the refresher checks whether a fetch is due.
const WORKER_TICK: Duration = Duration::from_millis(500);

/// Where this PC's weather comes from, and the place it is for.
#[derive(Debug, Clone, PartialEq)]
pub struct WeatherSettings {
    pub provider: String,
    /// The manual place's label. Empty means no manual place has been chosen.
    pub label: String,
    pub latitude: f64,
    pub longitude: f64,
    pub use_windows_location: bool,
    pub poll_interval_ms: u64,
}

impl Default for WeatherSettings {
    fn default() -> Self {
        Self {
            provider: "open-meteo".to_owned(),
            label: String::new(),
            latitude: 0.0,
            longitude: 0.0,
            use_windows_location: true,
            poll_interval_ms: 600_000,
        }
    }
}

impl WeatherSettings {
    /// A stored file read into the fields this build knows. A field that is
    /// absent or out of range takes its default: a wrong setting may not stop
    /// the host, and it may not fail a display's poll either.
    pub fn from_json(value: &Value) -> Self {
        let defaults = Self::default();
        let Some(record) = value.as_object() else {
            return defaults;
        };
        let text = |field: &str| record.get(field).and_then(Value::as_str).map(str::to_owned);
        let bounded = |field: &str, fallback: f64, min: f64, max: f64| {
            record
                .get(field)
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value >= min && *value <= max)
                .unwrap_or(fallback)
        };
        Self {
            provider: text("provider").unwrap_or(defaults.provider),
            label: text("label").unwrap_or(defaults.label),
            latitude: bounded("latitude", defaults.latitude, -90.0, 90.0),
            longitude: bounded("longitude", defaults.longitude, -180.0, 180.0),
            use_windows_location: record
                .get("useWindowsLocation")
                .and_then(Value::as_bool)
                .unwrap_or(defaults.use_windows_location),
            poll_interval_ms: bounded(
                "pollIntervalMs",
                defaults.poll_interval_ms as f64,
                60_000.0,
                86_400_000.0,
            ) as u64,
        }
    }

    /// The shape [`Self::from_json`] reads back, for the settings writer.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "provider": self.provider,
            "label": self.label,
            "latitude": self.latitude,
            "longitude": self.longitude,
            "useWindowsLocation": self.use_windows_location,
            "pollIntervalMs": self.poll_interval_ms,
        })
    }

    /// The manual place, when one has been chosen.
    fn place(&self) -> Option<Location> {
        (!self.label.is_empty()).then_some(Location {
            latitude: self.latitude,
            longitude: self.longitude,
        })
    }

    /// The window past which the last good reading is stale, and past which an
    /// undemanded provider stops fetching: whichever is longer.
    fn idle_window_ms(&self) -> u64 {
        (3 * self.poll_interval_ms).max(MIN_IDLE_WINDOW_MS)
    }
}

struct Cached {
    weather: Weather,
    fetched_at_ms: u64,
}

#[derive(Default)]
struct State {
    last_good: Option<Cached>,
    /// Redacted; may reach a browser through `health()` and a reading.
    failure: Option<String>,
    failures: u32,
    last_demand_ms: Option<u64>,
    next_due_ms: u64,
    /// A Windows fix and when it was taken; reused for a day.
    position: Option<(location::Position, u64)>,
    /// Backend plus place; a change drops the reading and the backoff.
    identity: Option<String>,
}

/// How settings are read. A seam so a test holds them still, and so a settings
/// file written mid-session is picked up on the next poll.
pub type SettingsLoader = Arc<dyn Fn() -> WeatherSettings + Send + Sync>;

/// How a backend's credential is read. A key is fetched only for a poll that
/// can use it, and never stored in the provider.
pub type KeysLoader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct WeatherProvider {
    settings: SettingsLoader,
    keys: KeysLoader,
    locate: location::Reader,
    fetch: Fetch,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    state: Arc<Mutex<State>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// False in tests: acquisition is driven by [`WeatherProvider::refresh`].
    auto: bool,
}

impl WeatherProvider {
    pub fn new(settings: SettingsLoader) -> Self {
        Self::with_fetch(settings, Arc::new(crate::http::get), Arc::new(now_ms), true)
            .with_locator(location::reader())
    }

    /// The credential a backend's key lives under, read on every poll.
    pub fn with_credentials(mut self, credentials: crate::weather::KeysLoader) -> Self {
        self.keys = credentials;
        self
    }

    /// How a Windows fix is produced.
    pub fn with_locator(mut self, reader: location::Reader) -> Self {
        self.locate = reader;
        self
    }

    pub fn with_fetch(
        settings: SettingsLoader,
        fetch: Fetch,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        auto: bool,
    ) -> Self {
        Self {
            settings,
            keys: Arc::new(|_| None),
            locate: Arc::new(|| Err(location::UNAVAILABLE_REASON.to_owned())),
            fetch,
            clock,
            state: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
            auto,
        }
    }

    /// One acquisition, when one is due: called by the refresher and directly by
    /// a test. True when a reading was stored.
    pub fn refresh(&self, now_ms: u64) -> bool {
        acquire(
            &self.state,
            &(self.settings)(),
            &self.keys,
            &self.locate,
            &self.fetch,
            now_ms,
        )
    }

    /// Starts the refresher unless one is already running. It ends by itself
    /// when no display has asked for a while, and the next ask starts another.
    fn ensure_worker(&self) {
        let mut worker = self.worker.lock().expect("the weather worker");
        if worker.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        let settings = Arc::clone(&self.settings);
        let keys = Arc::clone(&self.keys);
        let locate = Arc::clone(&self.locate);
        let state = Arc::clone(&self.state);
        let clock = Arc::clone(&self.clock);
        let fetch = Arc::clone(&self.fetch);
        // The loop owns only the shared pieces, so it outlives no borrow of
        // `self`.
        *worker = Some(thread::spawn(move || {
            loop {
                let now = clock();
                let settings_now = settings();
                let demanded = state
                    .lock()
                    .expect("the weather state")
                    .last_demand_ms
                    .is_some_and(|last| now.saturating_sub(last) <= settings_now.idle_window_ms());
                if !demanded {
                    return;
                }
                let due = now >= state.lock().expect("the weather state").next_due_ms;
                if due {
                    acquire(&state, &settings_now, &keys, &locate, &fetch, now);
                }
                thread::sleep(WORKER_TICK);
            }
        }));
    }
}

/// One acquisition, when one is due. True when a reading was stored.
fn acquire(
    state: &Mutex<State>,
    settings: &WeatherSettings,
    keys: &KeysLoader,
    locate: &location::Reader,
    fetch: &Fetch,
    now_ms: u64,
) -> bool {
    let identity = identity_of(settings);
    {
        let mut state = state.lock().expect("the weather state");
        remember(&mut state, identity);
        if now_ms < state.next_due_ms {
            return false;
        }
    }
    let interval = settings.poll_interval_ms;
    let attempt = place_of(state, settings, locate, now_ms)
        .and_then(|place| backend(settings, keys, &place, fetch, now_ms));
    match attempt {
        Ok(weather) => {
            let mut state = state.lock().expect("the weather state");
            state.last_good = Some(Cached {
                weather,
                fetched_at_ms: now_ms,
            });
            state.failure = None;
            state.failures = 0;
            state.next_due_ms = now_ms.saturating_add(interval);
            true
        }
        Err(refusal) => {
            let mut state = state.lock().expect("the weather state");
            state.failures = state.failures.saturating_add(1);
            state.failure = Some(refusal);
            let backoff = interval
                .saturating_mul(1_u64 << state.failures.min(10))
                .min(MAX_BACKOFF_MS);
            state.next_due_ms = now_ms.saturating_add(backoff);
            false
        }
    }
}

/// The place this poll is for. A manual place always wins, so location is
/// never asked for beside it; a Windows fix is reused for a day.
fn place_of(
    state: &Mutex<State>,
    settings: &WeatherSettings,
    locate: &location::Reader,
    now_ms: u64,
) -> Result<Location, String> {
    if let Some(place) = settings.place() {
        return Ok(place);
    }
    if !settings.use_windows_location {
        return Err(NO_PLACE_MESSAGE.to_owned());
    }
    {
        let state = state.lock().expect("the weather state");
        if let Some((position, taken_at)) = state.position
            && now_ms.saturating_sub(taken_at) < POSITION_TTL_MS
        {
            return Ok(Location {
                latitude: position.latitude,
                longitude: position.longitude,
            });
        }
    }
    let position = locate()?;
    state.lock().expect("the weather state").position = Some((position, now_ms));
    Ok(Location {
        latitude: position.latitude,
        longitude: position.longitude,
    })
}

/// One backend's acquisition. Its key is resolved before the request so a
/// missing credential is a sentence, not a request nobody can authorise.
fn backend(
    settings: &WeatherSettings,
    keys: &KeysLoader,
    place: &Location,
    fetch: &Fetch,
    now_ms: u64,
) -> Result<Weather, String> {
    match settings.provider.as_str() {
        "open-meteo" => open_meteo::fetch(place, fetch),
        "weatherapi" => {
            let key = weatherapi::required_key(keys(weatherapi::CREDENTIAL_ID).as_deref())?;
            weatherapi::fetch(place, fetch, &key, now_ms)
        }
        "openweathermap" => {
            let key = openweathermap::required_key(keys(openweathermap::CREDENTIAL_ID).as_deref())?;
            openweathermap::fetch(place, fetch, &key, now_ms)
        }
        _ => Err(UNSUPPORTED_BACKEND_MESSAGE.to_owned()),
    }
}

/// The credit a backend's licence asks for.
fn attribution_of(provider: &str) -> &'static str {
    match provider {
        "weatherapi" => weatherapi::ATTRIBUTION,
        "openweathermap" => openweathermap::ATTRIBUTION,
        _ => open_meteo::ATTRIBUTION,
    }
}

/// A change of backend or place drops everything derived from the old one.
fn remember(state: &mut State, identity: String) {
    if state.identity.as_deref() == Some(identity.as_str()) {
        return;
    }
    let first = state.identity.is_none();
    state.identity = Some(identity);
    if first {
        return;
    }
    state.last_good = None;
    state.failure = None;
    state.failures = 0;
    state.next_due_ms = 0;
}

fn identity_of(settings: &WeatherSettings) -> String {
    format!(
        "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        settings.provider,
        settings.label,
        settings.latitude,
        settings.longitude,
        settings.use_windows_location
    )
}

impl Provider for WeatherProvider {
    fn id(&self) -> &str {
        "weather"
    }

    fn label(&self) -> &str {
        "Weather"
    }

    fn describe(&self) -> Vec<Descriptor> {
        KEYS.iter()
            .map(|key| {
                Descriptor::new(*key, vocabulary::describe(key).and_then(|entry| entry.unit))
            })
            .collect()
    }

    fn sample(&self, keys: &[String], now_ms: u64) -> Result<Vec<Reading>, String> {
        let wanted: Vec<&String> = keys
            .iter()
            .filter(|key| KEYS.contains(&key.as_str()))
            .collect();
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let settings = (self.settings)();

        // Demand is what starts and keeps the clock; a sample that asks for no
        // weather key must not spend this PC's request.
        {
            let mut state = self.state.lock().expect("the weather state");
            state.last_demand_ms = Some(now_ms);
            remember(&mut state, identity_of(&settings));
        }
        if self.auto {
            self.ensure_worker();
        }

        let state = self.state.lock().expect("the weather state");
        Ok(wanted
            .into_iter()
            .map(|key| reading_of(&state, &settings, key, now_ms))
            .collect())
    }

    fn health(&self) -> Health {
        match self
            .state
            .lock()
            .expect("the weather state")
            .failure
            .clone()
        {
            Some(reason) => Health::Unavailable(reason),
            None => Health::Ok,
        }
    }
}

/// One key's reading, from the cache the refetcher fills.
fn reading_of(state: &State, settings: &WeatherSettings, key: &str, now_ms: u64) -> Reading {
    if settings.label.is_empty() && !settings.use_windows_location {
        return Reading::missing(key, NO_PLACE_MESSAGE);
    }
    let Some(cached) = &state.last_good else {
        return Reading::missing(
            key,
            state
                .failure
                .clone()
                .unwrap_or_else(|| WAITING_MESSAGE.to_owned()),
        );
    };
    if now_ms.saturating_sub(cached.fetched_at_ms) > settings.idle_window_ms() {
        return Reading::missing(key, STALE_MESSAGE);
    }
    let weather = &cached.weather;
    let unit = vocabulary::describe(key)
        .and_then(|entry| entry.unit)
        .unwrap_or_default();
    match key {
        "weather.location" => Reading::text(
            key,
            if settings.label.is_empty() {
                conditions::location_text().to_owned()
            } else {
                settings.label.clone()
            },
        ),
        "weather.temp" => Reading::number(key, weather.temp_c, unit),
        "weather.feelsLike" => Reading::number(key, weather.feels_like_c, unit),
        "weather.high" => Reading::number(key, weather.high_c, unit),
        "weather.low" => Reading::number(key, weather.low_c, unit),
        "weather.conditionCode" => Reading::number(key, weather.code as f64, unit),
        "weather.condition" => Reading::text(key, conditions::condition_text(weather.code)),
        "weather.icon" => match conditions::icon_url(weather.code, weather.is_day) {
            Some(url) => Reading::text(key, url),
            None => Reading::missing(key, NO_ICON_MESSAGE),
        },
        "weather.humidity" => Reading::number(key, weather.humidity, unit),
        "weather.pressure" => Reading::number(key, weather.pressure_hpa, unit),
        "weather.precipitation" => Reading::number(key, weather.precipitation_mm, unit),
        "weather.windSpeed" => Reading::number(key, weather.wind_kph, unit),
        "weather.windDirection" => Reading::number(key, weather.wind_deg, unit),
        "weather.windDirection.compass" => {
            Reading::text(key, conditions::wind_compass(weather.wind_deg))
        }
        // The source's renderer shows a boolean as these two words, so the
        // reading carries the rendered form.
        "weather.isDay" => Reading::text(key, if weather.is_day { "on" } else { "off" }),
        "weather.sunrise" => Reading::text(key, weather.sunrise.clone()),
        "weather.sunset" => Reading::text(key, weather.sunset.clone()),
        "weather.attribution" => Reading::text(key, attribution_of(&settings.provider)),
        _ => Reading::missing(key, NO_KEY_MESSAGE),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::sample::Status;

    fn payload() -> String {
        serde_json::json!({
            "utc_offset_seconds": 0,
            "current": {
                "temperature_2m": 18.4,
                "apparent_temperature": 17.9,
                "relative_humidity_2m": 72.0,
                "precipitation": 1.5,
                "weather_code": 61.0,
                "wind_speed_10m": 11.2,
                "wind_direction_10m": 225.0,
                "is_day": 0.0,
                "pressure_msl": 1013.2
            },
            "daily": {
                "temperature_2m_max": [21.0],
                "temperature_2m_min": [12.5],
                "sunrise": ["2026-10-24T07:47"],
                "sunset": ["2026-10-24T17:52"]
            }
        })
        .to_string()
    }

    fn settings(label: &str) -> WeatherSettings {
        WeatherSettings {
            label: label.to_owned(),
            latitude: 52.52,
            longitude: 13.405,
            use_windows_location: false,
            ..WeatherSettings::default()
        }
    }

    fn provider(
        settings: WeatherSettings,
        fetch: impl Fn(&str, u64) -> Result<String, String> + Send + Sync + 'static,
    ) -> (WeatherProvider, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let fetch: Fetch = Arc::new(move |url, timeout| {
            counted.fetch_add(1, Ordering::SeqCst);
            fetch(url, timeout)
        });
        let provider = WeatherProvider::with_fetch(
            Arc::new(move || settings.to_owned()),
            fetch,
            Arc::new(|| 0),
            false,
        );
        (provider, calls)
    }

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_reading_is_served_from_the_cache_once_it_is_fetched() {
        let (provider, calls) = provider(settings("Berlin"), |_url, _timeout| Ok(payload()));
        assert_eq!(
            provider.sample(&keys(&["weather.temp"]), 1_000).unwrap()[0].status,
            Status::Missing
        );
        assert!(provider.refresh(1_000));
        let readings = provider
            .sample(
                &keys(&["weather.temp", "weather.condition", "weather.temp"]),
                1_100,
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(readings[0].value, Some(18.4));
        assert_eq!(readings[0].unit.as_deref(), Some("°C"));
        assert_eq!(readings[1].text.as_deref(), Some("Slight rain"));
    }

    #[test]
    fn the_place_condition_wind_icon_and_day_flag_read_as_the_source_does() {
        let (provider, _) = provider(settings("Berlin"), |_url, _timeout| Ok(payload()));
        provider.refresh(1_000);
        let readings = provider
            .sample(
                &keys(&[
                    "weather.location",
                    "weather.windDirection.compass",
                    "weather.icon",
                    "weather.isDay",
                    "weather.attribution",
                ]),
                1_100,
            )
            .unwrap();
        assert_eq!(readings[0].text.as_deref(), Some("Berlin"));
        assert_eq!(readings[1].text.as_deref(), Some("SW"));
        assert_eq!(
            readings[2].text.as_deref(),
            Some("/api/weather/icon/rain.svg")
        );
        assert_eq!(readings[3].text.as_deref(), Some("off"));
        assert_eq!(
            readings[4].text.as_deref(),
            Some("Weather data by Open-Meteo.com")
        );
    }

    #[test]
    fn no_place_is_said_rather_than_fetched() {
        let mut none = settings("");
        none.use_windows_location = false;
        let (provider, calls) = provider(none, |_url, _timeout| Ok(payload()));
        let readings = provider.sample(&keys(&["weather.temp"]), 1_000).unwrap();
        assert_eq!(readings[0].message.as_deref(), Some(NO_PLACE_MESSAGE));
        assert!(!provider.refresh(1_000));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_windows_fallback_is_named_rather_than_guessed() {
        let mut windows = settings("");
        windows.use_windows_location = true;
        let (provider, calls) = provider(windows, |_url, _timeout| Ok(payload()));
        let readings = provider.sample(&keys(&["weather.temp"]), 1_000).unwrap();
        // A display is told why before the refetcher runs.
        assert_eq!(readings[0].status, Status::Missing);
        provider.refresh(1_000);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(matches!(provider.health(), Health::Unavailable(_)));
    }

    #[test]
    fn a_failed_fetch_is_a_reason_and_the_next_one_waits_for_the_backoff() {
        let (provider, calls) = provider(settings("Berlin"), |_url, _timeout| {
            Err("connection refused".to_owned())
        });
        assert!(!provider.refresh(1_000));
        let readings = provider.sample(&keys(&["weather.temp"]), 1_100).unwrap();
        assert_eq!(readings[0].status, Status::Missing);
        assert!(
            readings[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("connection refused")),
            "{:?}",
            readings[0].message
        );
        assert!(matches!(provider.health(), Health::Unavailable(_)));

        // The first backoff doubles the ten-minute cadence: not due yet.
        assert!(!provider.refresh(60_000));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!provider.refresh(21 * 60_000));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_reading_older_than_the_idle_window_is_stale() {
        let (provider, _) = provider(settings("Berlin"), |_url, _timeout| Ok(payload()));
        provider.refresh(1_000);
        let fresh = provider.sample(&keys(&["weather.temp"]), 2_000).unwrap();
        assert_eq!(fresh[0].status, Status::Ok);
        // Ten minutes of cadence means half an hour of tolerance.
        let stale = provider
            .sample(&keys(&["weather.temp"]), 31 * 60_000)
            .unwrap();
        assert_eq!(stale[0].status, Status::Missing);
        assert_eq!(stale[0].message.as_deref(), Some(STALE_MESSAGE));
    }

    #[test]
    fn a_moved_place_drops_the_old_reading() {
        let place = Arc::new(Mutex::new(settings("Berlin")));
        let loader_place = Arc::clone(&place);
        let fetch: Fetch = Arc::new(|_url, _timeout| Ok(payload()));
        let provider = WeatherProvider::with_fetch(
            Arc::new(move || loader_place.lock().unwrap().clone()),
            fetch,
            Arc::new(|| 0),
            false,
        );
        provider.refresh(1_000);
        assert_eq!(
            provider.sample(&keys(&["weather.temp"]), 1_100).unwrap()[0].status,
            Status::Ok
        );
        *place.lock().unwrap() = settings("Paris");
        let readings = provider.sample(&keys(&["weather.temp"]), 1_200).unwrap();
        assert_eq!(readings[0].status, Status::Missing);
        assert_eq!(readings[0].message.as_deref(), Some(WAITING_MESSAGE));
    }

    #[test]
    fn only_the_keys_asked_for_are_answered() {
        let (provider, _) = provider(settings("Berlin"), |_url, _timeout| Ok(payload()));
        let readings = provider
            .sample(&keys(&["weather.temp", "cpu.load"]), 1_000)
            .unwrap();
        assert_eq!(readings.len(), 1);
        assert_eq!(provider.sample(&[], 1_000).unwrap().len(), 0);
    }

    #[test]
    fn a_settings_file_with_wrong_fields_falls_back_field_by_field() {
        let stored = serde_json::json!({
            "provider": "weatherapi",
            "label": "Berlin",
            "latitude": "52.5",
            "longitude": 400,
            "pollIntervalMs": 1
        });
        let settings = WeatherSettings::from_json(&stored);
        assert_eq!(settings.provider, "weatherapi");
        assert_eq!(settings.label, "Berlin");
        // Out of range or the wrong type is the default, not a zero nobody
        // measured.
        assert_eq!(settings.latitude, 0.0);
        assert_eq!(settings.longitude, 0.0);
        // Below the bounds is the default cadence, not a one-millisecond poll.
        assert_eq!(settings.poll_interval_ms, 600_000);
        // Windows location is on until a person says otherwise, as in the source.
        assert!(settings.use_windows_location);
        assert_eq!(
            WeatherSettings::from_json(&serde_json::json!("not an object")),
            WeatherSettings::default()
        );
        // What the writer produces reads back as the same settings.
        assert_eq!(WeatherSettings::from_json(&settings.to_json()), settings);
    }

    fn weatherapi_payload() -> String {
        serde_json::json!({
            "location": {"localtime": "2026-10-24 14:00", "localtime_epoch": 1_792_850_400_i64},
            "current": {
                "temp_c": 18.4,
                "feelslike_c": 17.9,
                "humidity": 72.0,
                "pressure_mb": 1013.2,
                "precip_mm": 0.0,
                "wind_kph": 11.2,
                "wind_degree": 225.0,
                "is_day": 1,
                "condition": {"code": 1009}
            },
            "forecast": {"forecastday": [{
                "day": {"maxtemp_c": 21.0, "mintemp_c": 12.5},
                "astro": {"sunrise": "07:47 AM", "sunset": "05:52 PM"}
            }]}
        })
        .to_string()
    }

    #[test]
    fn a_backend_with_no_key_says_which_credential_to_set() {
        let mut other = settings("Berlin");
        other.provider = "weatherapi".to_owned();
        let (provider, calls) = provider(other, |_url, _timeout| Ok(weatherapi_payload()));
        assert!(!provider.refresh(1_000));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let readings = provider.sample(&keys(&["weather.temp"]), 1_100).unwrap();
        assert_eq!(
            readings[0].message.as_deref(),
            Some("Set weatherapi in Settings.")
        );

        let mut unknown = self::settings("Berlin");
        unknown.provider = "tomorrow-io".to_owned();
        let (provider, _) = self::provider(unknown, |_url, _timeout| Ok(payload()));
        assert!(!provider.refresh(1_000));
        let readings = provider.sample(&keys(&["weather.temp"]), 1_100).unwrap();
        assert_eq!(
            readings[0].message.as_deref(),
            Some(UNSUPPORTED_BACKEND_MESSAGE)
        );
    }

    #[test]
    fn a_keyed_backend_reads_its_own_answer_and_names_its_own_credit() {
        let mut keyed = settings("Berlin");
        keyed.provider = "weatherapi".to_owned();
        let (mut provider, calls) = provider(keyed, |url, _timeout| {
            assert!(url.contains("api.weatherapi.com"), "{url}");
            assert!(url.contains("key=stored-key"), "{url}");
            Ok(weatherapi_payload())
        });
        provider.keys = Arc::new(|id| (id == "weatherapi").then(|| "stored-key".to_owned()));
        assert!(provider.refresh(1_000));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let readings = provider
            .sample(&keys(&["weather.condition", "weather.attribution"]), 1_100)
            .unwrap();
        assert_eq!(readings[0].text.as_deref(), Some("Overcast"));
        assert_eq!(
            readings[1].text.as_deref(),
            Some("Powered by WeatherAPI.com")
        );
    }

    #[test]
    fn a_windows_fix_is_used_and_reused_before_asking_again() {
        let mut windows = settings("");
        windows.use_windows_location = true;
        let (mut provider, calls) = provider(windows, |_url, _timeout| Ok(payload()));
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&asked);
        provider.locate = Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(location::Position {
                latitude: 52.52,
                longitude: 13.405,
            })
        });
        assert!(provider.refresh(1_000));
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // The place is read once and the reading names the host's own lookup.
        let readings = provider
            .sample(&keys(&["weather.location", "weather.temp"]), 60_000)
            .unwrap();
        assert_eq!(readings[0].text.as_deref(), Some("Windows location"));
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_refused_location_is_its_own_reason_not_a_guessed_place() {
        let mut windows = settings("");
        windows.use_windows_location = true;
        let (mut provider, calls) = provider(windows, |_url, _timeout| Ok(payload()));
        provider.locate = Arc::new(|| Err(location::DENIED_REASON.to_owned()));
        assert!(!provider.refresh(1_000));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let readings = provider.sample(&keys(&["weather.temp"]), 1_100).unwrap();
        assert_eq!(
            readings[0].message.as_deref(),
            Some(location::DENIED_REASON)
        );
    }
}
