// SPDX-License-Identifier: AGPL-3.0-or-later
//! Routeur HTTP : pile de middlewares et routes.

mod auth;
mod csrf;
mod custom;
mod detections;
mod docs;
mod energy;
pub mod health;
mod media;
mod ptz;
mod recordings;
mod render_guard;
mod settings;
mod snapshots;
mod system;
mod ws;

pub use energy::{DEFAULT_DAYS, MAX_DAYS, days_from};
pub use media::{ImageProblem, process_upload};
pub use ws::{CLOSE_NO_HEARTBEAT, CLOSE_SESSION_ENDED};

use std::time::Duration;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use tower::{Layer, ServiceBuilder};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::AppState;
use crate::auth::session;

/// Chemin a ecrire dans les journaux : le jeton d'un appareil Android (secret) n'y figure jamais.
#[must_use]
pub fn loggable_path(path: &str) -> std::borrow::Cow<'_, str> {
    if path.starts_with("/ws/android/device/") {
        return "/ws/android/device/***/".into();
    }
    path.into()
}

const REQUEST_ID: &str = "x-request-id";

fn header_layer(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

/// Re-export : conversion compte -> DTO, reutilisee par les fonctions serveur de l'interface.
pub use auth::user_dto;

/// Routeur sans interface (API, sondes) : utilise par les tests.
pub fn router(state: AppState) -> Router {
    build(state, None, None)
}

/// Routeur complet : l'API et les sondes, puis l'interface (`ui`) pour tout le reste. Les middlewares
/// (session, CSRF, en-tetes de securite) enveloppent aussi l'interface.
pub fn router_with_ui(state: AppState, ui: Router) -> Router {
    build(state, Some(ui), None)
}

/// Comme [`router_with_ui`], avec des routes supplementaires (serveur MCP, R11) qui passent par les memes middlewares :
/// journal, identifiant de requete, delai maximal, en-tetes de securite. Un client hors navigateur n'est pas concerne par le CSRF.
pub fn router_with_extra(state: AppState, ui: Router, extra: Option<Router<AppState>>) -> Router {
    build(state, Some(ui), extra)
}

/// Construit le routeur. Le premier `layer` d'un `ServiceBuilder` est le plus externe.
fn build(state: AppState, ui: Option<Router>, extra: Option<Router<AppState>>) -> Router {
    let sessions = session::layer(state.store.sessions(), &state.config);
    let hsts = state.config.cookie_secure;

    let api = auth::routes()
        .merge(settings::routes())
        .merge(system::routes())
        .merge(energy::routes())
        .merge(media::api_routes())
        .layer(header_layer(header::CACHE_CONTROL, "no-store"));

    let mut app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .nest("/api", api)
        .merge(media::media_routes())
        .merge(ws::routes())
        .merge(ptz::routes())
        .merge(snapshots::routes())
        .merge(recordings::routes())
        .merge(detections::routes())
        .merge(crate::android::routes())
        .merge(custom::routes())
        .merge(docs::routes(&state));
    if let Some(extra) = extra {
        app = app.merge(extra);
    }
    if let Some(ui) = ui {
        // Le rendu des pages est borne dans le temps et relance s'il se fige (voir `render_guard`, ADR 0021).
        app = app.fallback_service(render_guard::RenderGuardLayer::new().layer(ui));
    }
    let mut app = app
        .layer(from_fn_with_state(state.clone(), csrf::check_origin))
        .layer(sessions)
        .layer(header_layer(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header_layer(header::X_FRAME_OPTIONS, "DENY"))
        .layer(header_layer(header::REFERRER_POLICY, "same-origin"))
        .layer(header_layer(
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ))
        .layer(header_layer(
            HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        ))
        .layer(header_layer(
            HeaderName::from_static("permissions-policy"),
            "camera=(), microphone=(), geolocation=()",
        ));
    if hsts {
        // Sert derriere TLS (cookie Secure) : le navigateur doit s'en souvenir.
        app = app.layer(header_layer(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000",
        ));
    }

    let middleware = ServiceBuilder::new()
        .layer(axum::middleware::from_fn(crate::metrics::track))
        .layer(SetRequestIdLayer::new(HeaderName::from_static(REQUEST_ID), MakeRequestUuid))
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request<_>| {
            let request_id = req.headers().get(REQUEST_ID).and_then(|v| v.to_str().ok()).unwrap_or("-");
            tracing::info_span!("http", method = %req.method(), path = %loggable_path(req.uri().path()), request_id = %request_id)
        }))
        .layer(PropagateRequestIdLayer::new(HeaderName::from_static(REQUEST_ID)))
        .layer(TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(30)));

    app.layer(middleware).with_state(state)
}

#[cfg(test)]
mod tests {
    use super::loggable_path;

    #[test]
    fn the_android_token_never_reaches_the_logs() {
        let token = "0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b";
        assert_eq!(
            loggable_path(&format!("/ws/android/device/{token}/")),
            "/ws/android/device/***/"
        );
        assert_eq!(
            loggable_path("/ws/android/device/n-importe-quoi"),
            "/ws/android/device/***/"
        );
        assert_eq!(loggable_path("/api/auth/login"), "/api/auth/login");
        assert_eq!(
            loggable_path("/ws/android/browser/3/"),
            "/ws/android/browser/3/"
        );
    }
}
