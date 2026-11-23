// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/system-stats/`: machine resources (logged-in account), in the reference implementation's format.
//!
//! The response is the last message of the hub's `system` subject (see `system_stats`): no on-demand measurement, hence
//! no extra load when several clients poll.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use dominet_proto::{SystemStatsDto, Topic, WsServerMsg};

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::{AppError, AppResult};

pub fn routes() -> Router<AppState> {
    Router::new().route("/system-stats/", get(system_stats))
}

async fn system_stats(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
) -> AppResult<Json<SystemStatsDto>> {
    match state.hub.retained(&Topic::System).as_deref() {
        Some(WsServerMsg::SystemStats(latest)) => Ok(Json(*latest)),
        // Collector not yet run (start-up) or disabled: we do not guess.
        _ => Err(AppError::Unavailable("no measurement available yet".into())),
    }
}
