// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service HTTP de l'interface (feature `ssr`) : pages rendues cote serveur, fonctions serveur et fichiers
//! statiques (`/pkg/...`). Il est monte en `fallback` du routeur principal, donc derriere la session, le
//! controle d'origine (CSRF) et les en-tetes de securite.

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

/// Construit le service de l'interface. `site_root` est le dossier produit par `cargo leptos build`.
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
            // L'etat de l'application est disponible dans les fonctions serveur via `expect_context`.
            move || provide_context(state.clone()),
            move || shell(shell_options.clone()),
        )
        // La page 404 est rendue par le meme composant `App` (barre laterale comprise) : elle a donc besoin du meme
        // contexte que les routes, sinon la fonction serveur de la mise en page ne trouve pas l'etat.
        .fallback(file_and_error_handler_with_context(
            move || provide_context(context_state.clone()),
            shell,
        ))
        .with_state(options)
        .layer(middleware::from_fn(cache_control))
        // Texte, JS, CSS et WebAssembly sont compresses selon `Accept-Encoding` (le WASM perd ~70 %). Aucun secret n'est
        // reflete dans les pages (la protection CSRF repose sur `Origin`), donc pas de risque BREACH.
        .layer(CompressionLayer::new())
}

/// Cache : les fichiers de `/pkg/` se revalident a chaque chargement (`ETag`, `Last-Modified`, reponse 304) car leurs noms ne
/// changent pas entre deux versions (un WASM perime avec un JS neuf casserait l'hydratation) ; tout le reste (pages,
/// fonctions serveur) est prive et n'est jamais mis en cache ni conserve dans l'historique de navigation.
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
