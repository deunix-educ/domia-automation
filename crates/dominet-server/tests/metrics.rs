// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/metrics` (R7 lot 1, ADR 0051): Prometheus format, bounded labels, separate listener, no secret.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, metrics, web};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn state() -> AppState {
    let config = Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://u:secret-db-pass@127.0.0.1:1/none".to_string())
    })
    .unwrap();
    let pool = PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy("postgres://u:secret-db-pass@127.0.0.1:1/none")
        .unwrap();
    AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    )
}

async fn text(app: axum::Router, path: &str) -> (StatusCode, String) {
    let resp = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn requests_are_counted_by_route_and_status_never_by_raw_url() {
    let st = state();
    let _ = metrics::handle();
    let app = web::router(st.clone());
    // A known route, an unknown one with an "identifier": only the family must appear.
    text(app.clone(), "/healthz").await;
    text(
        app.clone(),
        "/ws/android/device/0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b/",
    )
    .await;
    text(app, "/api/qui-n-existe-pas-12345").await;

    let (status, body) = text(metrics::router(st), "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"dominet_http_requests_total{method="GET",route="/healthz",status="200"}"#),
        "{body}"
    );
    assert!(
        body.contains("dominet_http_request_duration_seconds_bucket"),
        "histogramme de latence"
    );
    assert!(body.contains("# TYPE dominet_http_requests_total counter"));
    assert!(
        body.contains("dominet_build_info") || body.contains("dominet_http_requests_in_flight")
    );
    // Bounded cardinality and nothing sensitive: neither the identifier, nor the raw URL, nor the database password.
    assert!(
        !body.contains("0b9a0f0e"),
        "le jeton ne doit pas figurer dans les metriques"
    );
    assert!(
        !body.contains("qui-n-existe-pas"),
        "l'URL brute ne doit pas devenir une etiquette"
    );
    assert!(!body.contains("secret-db-pass"));
}

#[tokio::test]
async fn websocket_connections_are_a_gauge_that_goes_back_down() {
    let st = state();
    let _ = metrics::handle();
    let read = |st: AppState| async move {
        let (_, body) = text(metrics::router(st), "/metrics").await;
        body.lines()
            .find(|l| l.starts_with("dominet_ws_connections "))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let base = read(st.clone()).await;
    let guard = st.hub.try_connect(4242).expect("connection");
    assert!((read(st.clone()).await - (base + 1.0)).abs() < 0.5);
    drop(guard);
    assert!((read(st).await - base).abs() < 0.5);
}

#[tokio::test]
async fn the_listener_serves_metrics_on_its_own_port_and_the_public_router_does_not() {
    let st = state();
    let _ = metrics::handle();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    metrics::serve(listener, st.clone());
    let body = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap();
    assert_eq!(body.status(), 200);
    assert!(
        body.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/plain"),
        "type de contenu Prometheus"
    );
    // The public router does not expose /metrics (it treats it as an unknown URL).
    let (status, _) = text(web::router(st), "/metrics").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
