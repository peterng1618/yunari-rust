//! The pages this host serves itself, with no build step in between.

use axum::response::{IntoResponse, Response};

use crate::http::App;

pub fn pair_page(_app: &App, _code: Option<&str>, _reason: Option<&str>) -> Response {
    "Pairing page arrives with the phone player.".to_owned().into_response()
}
