// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/system-stats/` : ressources de la machine (compte connecte), au format de l'implementation de reference.
//!
//! La reponse est le dernier message du sujet `system` du hub (voir `system_stats`) : pas de mesure a la demande, donc
//! pas de charge supplementaire quand plusieurs clients interrogent.

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
        // Collecteur pas encore passe (demarrage) ou desactive : on ne devine pas.
        _ => Err(AppError::Unavailable("no measurement available yet".into())),
    }
}
