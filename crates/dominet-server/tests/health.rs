// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests d'integration des sondes de sante et des middlewares (routeur complet, sans reseau).
//!
//! Les tests avec base utilisent `#[sqlx::test]` (base isolee, `DATABASE_URL` = stack de dev : `make dev-up`).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use http_body_util::BodyExt;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn test_config() -> Config {
    Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://u:p@127.0.0.1:1/none".to_string())
    })
    .unwrap()
}

fn state(pool: PgPool) -> AppState {
    AppState::with_hasher(
        pool,
        Arc::new(test_config()),
        Hasher::with_params(1024, 1, 1).unwrap(),
    )
}

/// Pool vers un port ferme : la base est "absente".
fn dead_pool() -> PgPool {
    PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy("postgres://u:p@127.0.0.1:1/none")
        .unwrap()
}

async fn get(
    app: axum::Router,
    path: &str,
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    let resp = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn healthz_answers_even_without_database() {
    let (status, _, body) = get(web::router(state(dead_pool())), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn readyz_is_503_without_database_and_hides_the_cause() {
    let (status, _, body) = get(web::router(state(dead_pool())), "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "unavailable");
    assert_eq!(body["checks"]["database"], "error");
    assert!(
        !body.to_string().contains("127.0.0.1"),
        "ne doit pas reveler l'adresse de la base"
    );
}

#[sqlx::test(migrations = false)]
async fn readyz_is_200_with_database(pool: PgPool) {
    let (status, _, body) = get(web::router(state(pool)), "/readyz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ready");
    assert_eq!(body["checks"]["database"], "ok");
}

#[tokio::test]
async fn every_response_carries_a_request_id() {
    let (_, headers, _) = get(web::router(state(dead_pool())), "/healthz").await;
    let id = headers["x-request-id"].to_str().unwrap();
    assert_eq!(id.len(), 36, "uuid attendu, recu {id}");
}

#[tokio::test]
async fn a_client_supplied_request_id_is_preserved() {
    let app = web::router(state(dead_pool()));
    let req = Request::get("/healthz")
        .header("x-request-id", "abc-123")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.headers()["x-request-id"], "abc-123");
}

#[tokio::test]
async fn unknown_route_is_404() {
    let (status, _, _) = get(web::router(state(dead_pool())), "/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
