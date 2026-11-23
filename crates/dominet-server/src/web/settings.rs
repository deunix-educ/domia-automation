// SPDX-License-Identifier: AGPL-3.0-or-later
//! Application settings routes (`/api/settings`), reserved to administrators.

use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use dominet_proto::{SettingDto, SettingUpdate};

use crate::AppState;
use crate::auth::extract::RequireAdmin;
use crate::error::AppResult;
use crate::settings::SettingView;

fn dto(v: SettingView) -> SettingDto {
    SettingDto {
        key: v.key.to_string(),
        value_type: v.value_type.to_string(),
        category: v.category.to_string(),
        description: v.description.to_string(),
        is_secret: v.is_secret,
        value: v.value,
        is_set: v.is_set,
        from_default: v.from_default,
        readable: v.readable,
        updated_at: v.updated_at.map(|t| t.to_rfc3339()),
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings", get(list))
        .route("/settings/{key}", put(update))
}

async fn list(
    State(state): State<AppState>,
    RequireAdmin(_): RequireAdmin,
) -> AppResult<Json<Vec<SettingDto>>> {
    Ok(Json(
        state.settings.list().await?.into_iter().map(dto).collect(),
    ))
}

async fn update(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    Path(key): Path<String>,
    Json(req): Json<SettingUpdate>,
) -> AppResult<Json<SettingDto>> {
    let view = state.settings.set(&key, &req.value).await?;
    // We log who changed what, never the value.
    tracing::info!(admin_id = admin.id.0, %key, secret = view.is_secret, "setting changed");
    Ok(Json(dto(view)))
}
