//! The pages this host serves itself, with no build step in between.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::http::App;

const PAIR_HTML: &str = include_str!("../assets/pair.html");
const PLAYER_HTML: &str = include_str!("../assets/player.html");
const PLAYER_JS: &str = include_str!("../assets/player.js");
const FAVICON: &str = include_str!("../assets/favicon.svg");

/// Every icon this build serves, keyed by the slug a reading names.
static WEATHER_ICONS: &[(&str, &[u8])] = &[
    (
        "clear-day",
        include_bytes!("../assets/weather-icons/clear-day.svg"),
    ),
    (
        "clear-night",
        include_bytes!("../assets/weather-icons/clear-night.svg"),
    ),
    (
        "drizzle",
        include_bytes!("../assets/weather-icons/drizzle.svg"),
    ),
    ("fog", include_bytes!("../assets/weather-icons/fog.svg")),
    (
        "fog-day",
        include_bytes!("../assets/weather-icons/fog-day.svg"),
    ),
    (
        "fog-night",
        include_bytes!("../assets/weather-icons/fog-night.svg"),
    ),
    (
        "mostly-clear-day",
        include_bytes!("../assets/weather-icons/mostly-clear-day.svg"),
    ),
    (
        "mostly-clear-night",
        include_bytes!("../assets/weather-icons/mostly-clear-night.svg"),
    ),
    (
        "overcast-day",
        include_bytes!("../assets/weather-icons/overcast-day.svg"),
    ),
    (
        "overcast-night",
        include_bytes!("../assets/weather-icons/overcast-night.svg"),
    ),
    (
        "partly-cloudy-day",
        include_bytes!("../assets/weather-icons/partly-cloudy-day.svg"),
    ),
    (
        "partly-cloudy-day-rain",
        include_bytes!("../assets/weather-icons/partly-cloudy-day-rain.svg"),
    ),
    (
        "partly-cloudy-day-snow",
        include_bytes!("../assets/weather-icons/partly-cloudy-day-snow.svg"),
    ),
    (
        "partly-cloudy-night",
        include_bytes!("../assets/weather-icons/partly-cloudy-night.svg"),
    ),
    (
        "partly-cloudy-night-rain",
        include_bytes!("../assets/weather-icons/partly-cloudy-night-rain.svg"),
    ),
    (
        "partly-cloudy-night-snow",
        include_bytes!("../assets/weather-icons/partly-cloudy-night-snow.svg"),
    ),
    ("rain", include_bytes!("../assets/weather-icons/rain.svg")),
    ("sleet", include_bytes!("../assets/weather-icons/sleet.svg")),
    ("snow", include_bytes!("../assets/weather-icons/snow.svg")),
    (
        "thunderstorms-day",
        include_bytes!("../assets/weather-icons/thunderstorms-day.svg"),
    ),
    (
        "thunderstorms-day-rain",
        include_bytes!("../assets/weather-icons/thunderstorms-day-rain.svg"),
    ),
    (
        "thunderstorms-night",
        include_bytes!("../assets/weather-icons/thunderstorms-night.svg"),
    ),
    (
        "thunderstorms-night-rain",
        include_bytes!("../assets/weather-icons/thunderstorms-night-rain.svg"),
    ),
];

const MANIFEST: &str = r##"{"name":"Yunari","short_name":"Yunari","display":"standalone","start_url":"/","scope":"/","background_color":"#000000","theme_color":"#000000","icons":[{"src":"/favicon.svg","sizes":"any","type":"image/svg+xml","purpose":"any"}]}"##;

pub fn pair_page(_app: &App, _code: Option<&str>, _reason: Option<&str>) -> Response {
    html(PAIR_HTML)
}

pub fn player_page() -> Response {
    html(PLAYER_HTML)
}

pub fn player_script() -> Response {
    let mut response = PLAYER_JS.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    response
}

/// A weather icon, or nothing when the slug is not one this build vendored.
pub fn weather_icon(file: &str) -> Response {
    let Some(slug) = file.strip_suffix(".svg") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((_, bytes)) = WEATHER_ICONS.iter().find(|(name, _)| *name == slug) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = (*bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // The bytes only change with a new build, so a display may keep them.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    response
}

/// One artwork file the cache wrote; the content-addressed name is the whole
/// allow-list, and the bytes are checked against it before they are served.
pub fn media_artwork(directory: &std::path::Path, name: &str) -> Response {
    let cache = yunari_providers::media::artwork::Artwork::new(directory);
    let Some(bytes) = cache.read(name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(yunari_providers::media::artwork::content_type(name)),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // The name is the digest, so the bytes under it never change.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
}

pub fn favicon() -> Response {
    let mut response = FAVICON.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml"),
    );
    response
}

fn html(body: &'static str) -> Response {
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub fn manifest() -> Response {
    let mut response = MANIFEST.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/manifest+json"),
    );
    response
}
