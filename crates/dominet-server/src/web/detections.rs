// SPDX-License-Identifier: AGPL-3.0-or-later
//! Image annotee d'une detection (R6 lot 8, ADR 0050) : `GET /surveillance/detections/<id>/frame/`, tout compte connecte.

use axum::Router;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use dominet_domain::camera::DetectionEventId;
use dominet_domain::repo::EventRepo;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::AppError;

pub fn routes() -> Router<AppState> {
    Router::new().route("/surveillance/detections/{id}/frame/", get(frame))
}

async fn frame(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let event = state
        .store
        .camera_events()
        .get_detection(DetectionEventId(id))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("reading a detection: {e}")))?
        .ok_or(AppError::NotFound)?;
    // Un nom ecrit par l'application (`<uuid>_<date>.jpg`) : une seule composante, rien d'autre.
    let name = std::path::Path::new(&event.frame_path);
    if event.frame_path.is_empty()
        || name.components().count() != 1
        || name.file_name().is_none_or(|n| n != name.as_os_str())
    {
        return Err(AppError::NotFound);
    }
    let bytes = tokio::fs::read(std::path::Path::new(&state.config.detections_dir).join(name))
        .await
        .map_err(|_| AppError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        bytes,
    )
        .into_response())
}
