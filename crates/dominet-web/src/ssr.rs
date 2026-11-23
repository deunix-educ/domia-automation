// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP service of the interface (`ssr` feature): server-rendered pages, server functions and static files
//! (`/pkg/...`). It is mounted as the main router's `fallback`, hence behind the session, the origin
//! check (CSRF) and the security headers.

use std::net::SocketAddr;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use dominet_server::AppState;
use leptos::config::{Env, LeptosOptions};
use leptos::prelude::provide_context;
use leptos_axum::{LeptosRoutes, file_and_error_handler_with_context, generate_route_list};
use tower_http::compression::CompressionLayer;

use crate::app::{App, shell};

/// Builds the interface's service. `site_root` is the folder produced by `cargo leptos build`.
pub fn ui_service(state: AppState, site_root: &str, addr: SocketAddr) -> Router {
    let options = LeptosOptions::builder()
        .output_name("dominet")
        .site_root(site_root)
        .site_pkg_dir("pkg")
        .site_addr(addr)
        .env(Env::PROD)
        .build();
    let routes = generate_route_list(App);
    let shell_options = options.clone();
    let context_state = state.clone();
    Router::new()
        .leptos_routes_with_context(
            &options,
            routes,
            // The application state is available in the server functions through `expect_context`.
            move || provide_context(state.clone()),
            move || shell(shell_options.clone()),
        )
        // The 404 page is rendered by the same `App` component (sidebar included): it therefore needs the same
        // context as the routes, otherwise the layout's server function does not find the state.
        .fallback(file_and_error_handler_with_context(
            move || provide_context(context_state.clone()),
            shell,
        ))
        .with_state(options)
        .layer(middleware::from_fn(cache_control))
        // Text, JS, CSS and WebAssembly are compressed according to `Accept-Encoding` (WASM loses ~70%). No secret is
        // reflected in the pages (CSRF protection relies on `Origin`), hence no BREACH risk.
        .layer(CompressionLayer::new())
}

/// Cache: the files of `/pkg/` are revalidated at each load (`ETag`, `Last-Modified`, 304 response) because their names do not
/// change between two versions (a stale WASM with a new JS would break hydration); everything else (pages,
/// server functions) is private and is never cached or kept in the navigation history.
async fn cache_control(req: Request, next: Next) -> Response {
    let asset = req.uri().path().starts_with("/pkg/");
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static(if asset {
            "no-cache"
        } else {
            "no-store"
        }));
    resp
}
