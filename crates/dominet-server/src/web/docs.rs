// SPDX-License-Identifier: AGPL-3.0-or-later
//! Documentation (R7 lot 7, ADR 0056): the MkDocs site built at `docker build` (`DOCS_ROOT`), served on `/docs/` **behind the login**.
//!
//! A visitor who is not logged in is sent to `/login?next=/docs/` (like the interface pages). The site is static: no database access
//! beyond checking the session; if the folder does not exist (development without `make docs`), the pages answer 404.

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Redirect, Response};
use tower_http::services::ServeDir;

use crate::AppState;
use crate::auth::extract::CurrentUser;

pub fn routes(state: &AppState) -> Router<AppState> {
    let files = ServeDir::new(&state.config.docs_root).append_index_html_on_directories(true);
    Router::new()
        .nest_service("/docs", files)
        .route_layer(from_fn_with_state(state.clone(), require_login))
}

async fn require_login(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let (mut parts, body) = req.into_parts();
    if CurrentUser::from_request_parts(&mut parts, &state)
        .await
        .is_err()
    {
        let target = parts.uri.path_and_query().map_or("/docs/", |p| p.as_str());
        let encoded: String = url_encode(target);
        return Redirect::to(&format!("/login?next={encoded}")).into_response();
    }
    let mut res = next.run(Request::from_parts(parts, body)).await;
    // Internal documentation: no shared caching.
    if res.status() != StatusCode::NOT_MODIFIED {
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            "private, max-age=300".parse().expect("static header"),
        );
    }
    res
}

/// Minimal percent-encoding of a path for `?next=` (the non-alphanumeric bytes, except `-_.~`).
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::url_encode;

    #[test]
    fn encodes_path_for_next() {
        assert_eq!(url_encode("/docs/en/"), "%2Fdocs%2Fen%2F");
        assert_eq!(url_encode("/docs/a?b=1"), "%2Fdocs%2Fa%3Fb%3D1");
    }
}
