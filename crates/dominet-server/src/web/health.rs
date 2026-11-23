// SPDX-License-Identifier: AGPL-3.0-or-later
//! Health probes: `/healthz` (the process answers) and `/readyz` (the dependencies answer).

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::AppState;

/// Liveness: no external dependency, answers as long as the process runs.
pub async fn healthz() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

/// The database answers in under 2 s (probe of `/readyz`, reused by the MCP tool `system_status`).
pub async fn database_ok(pool: &sqlx::PgPool) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query("SELECT 1").execute(pool)
        )
        .await,
        Ok(Ok(_))
    )
}

/// Readiness: 200 if the database answers in under 2 s, 503 otherwise (without detail on the cause).
pub async fn readyz(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let db_ok = database_ok(&state.pool).await;
    let status = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = json!({
        "status": if db_ok { "ready" } else { "unavailable" },
        "checks": { "database": if db_ok { "ok" } else { "error" } },
    });
    (status, Json(body))
}
