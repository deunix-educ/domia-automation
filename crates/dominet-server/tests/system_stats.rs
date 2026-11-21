// SPDX-License-Identifier: AGPL-3.0-or-later
//! Contrat de `GET /api/system-stats/` : memes cles que l'implementation de reference (+ `temperature`), compte connecte exigible,
//! valeur lue dans le dernier etat du sujet `system`.

mod common;

use axum::http::StatusCode;
use common::{Client, PASSWORD, add_user, code, state};
use dominet_domain::user::Role;
use dominet_proto::{Topic, WsServerMsg};
use dominet_server::system_stats;
use sqlx::PgPool;

fn keys(v: &serde_json::Value) -> Vec<&str> {
    let mut k: Vec<_> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn anonymous_gets_401(pool: PgPool) {
    let st = state(pool, &[]);
    let mut c = Client::new(&st);
    assert_eq!(
        c.get("/api/system-stats/").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_a_measure_yet_the_answer_is_503_not_zeros(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "marie", Role::User).await;
    let mut c = Client::new(&st);
    c.login("marie", PASSWORD).await;
    let r = c.get("/api/system-stats/").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(code(&r), "unavailable");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_ordinary_account_reads_the_reference_shaped_document(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "marie", Role::User).await;
    let stats = system_stats::sample_once().await.expect("mesure");
    st.hub
        .publish_retained(&Topic::System, WsServerMsg::SystemStats(stats));
    let mut c = Client::new(&st);
    c.login("marie", PASSWORD).await;
    let r = c.get("/api/system-stats/").await;
    assert_eq!(r.status, StatusCode::OK);
    let b = &r.body;
    // Les cles historiques des statistiques systeme, plus `temperature`.
    assert_eq!(
        keys(b),
        ["cpu", "disk", "memory", "temperature", "updated_at"]
    );
    assert_eq!(keys(&b["cpu"]), ["count", "percent"]);
    assert_eq!(
        keys(&b["memory"]),
        ["available_gb", "percent", "total_gb", "used_gb"]
    );
    assert_eq!(
        keys(&b["disk"]),
        ["free_gb", "percent", "total_gb", "used_gb"]
    );
    assert!(b["updated_at"].is_f64());
    assert!(b["memory"]["total_gb"].as_f64().unwrap() > 0.0);
    assert_eq!(r.headers["cache-control"], "no-store");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_endpoint_follows_the_latest_published_state(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "marie", Role::User).await;
    let mut stats = system_stats::sample_once().await.expect("mesure");
    stats.cpu.percent = 11.1;
    st.hub
        .publish_retained(&Topic::System, WsServerMsg::SystemStats(stats));
    let mut c = Client::new(&st);
    c.login("marie", PASSWORD).await;
    assert_eq!(
        c.get("/api/system-stats/").await.body["cpu"]["percent"],
        11.1
    );
    stats.cpu.percent = 88.8;
    st.hub
        .publish_retained(&Topic::System, WsServerMsg::SystemStats(stats));
    assert_eq!(
        c.get("/api/system-stats/").await.body["cpu"]["percent"],
        88.8
    );
}
