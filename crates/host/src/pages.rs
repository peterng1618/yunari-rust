//! The pages this host serves itself, with no build step in between.

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

use crate::http::App;

const PAIR_HTML: &str = include_str!("../assets/pair.html");
const PLAYER_HTML: &str = include_str!("../assets/player.html");
const PLAYER_JS: &str = include_str!("../assets/player.js");
const FAVICON: &str = include_str!("../assets/favicon.svg");

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
