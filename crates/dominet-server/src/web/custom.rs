// SPDX-License-Identifier: AGPL-3.0-or-later
//! Customisation stylesheet (`/custom.css`): a file on the machine (`CUSTOM_CSS`), re-read at every request, loaded after the
//! main stylesheet. It lets you change colours, font and widths (CSS variables of `style/main.css`) **without rebuilding the image**.
//!
//! Public (the login page is styled too): it is only style. Absent, unreadable or too big: empty stylesheet, never an error.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::AppState;

/// Maximum size read (256 KiB): beyond it, the file is ignored.
const MAX_BYTES: u64 = 256 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new().route("/custom.css", get(custom_css))
}

async fn custom_css(State(state): State<AppState>) -> Response {
    let body = match state.config.custom_css.as_deref() {
        Some(path) => read_limited(path).await,
        None => String::new(),
    };
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            // A change to the file shows up on page reload.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}

async fn read_limited(path: &str) -> String {
    match tokio::fs::metadata(path).await {
        Ok(m) if m.is_file() && m.len() <= MAX_BYTES => {
            tokio::fs::read_to_string(path).await.unwrap_or_default()
        }
        Ok(m) if m.is_file() => {
            tracing::warn!(
                path,
                taille = m.len(),
                "custom.css ignored: more than 256 KiB"
            );
            String::new()
        }
        _ => String::new(),
    }
}
