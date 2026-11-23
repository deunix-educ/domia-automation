// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera images (R6 lot 6, ADR 0048): a camera's last image and the files of the indexed snapshots.
//!
//! - `GET /surveillance/camera/<uuid>/snapshot/` (like the reference): the camera's last image, any logged-in account. `?force=1` waits for
//!   an image **later than the request** (4 s at most; the `video` role writes one per second, no `ffmpeg` is started cold:
//!   ADR 0003); `?save=1` also records it in the gallery (administrators, like the live page's button).
//! - `GET /surveillance/snapshots/<id>/file/`: the file of an indexed snapshot, any logged-in account.
//!
//! `204` response when there is no recent image; never cached (`no-store`).

use std::collections::HashMap;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use dominet_domain::camera::{CameraUuid, SnapshotId};
use dominet_domain::repo::CameraRepo;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::AppError;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/surveillance/camera/{uuid}/snapshot/", get(latest))
        .route("/surveillance/snapshots/{id}/file/", get(file))
}

fn jpeg(bytes: Vec<u8>, age: Option<Duration>) -> Response {
    let mut r = (
        [
            (header::CONTENT_TYPE, "image/jpeg".to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response();
    if let Some(a) = age
        && let Ok(v) = a.as_secs().to_string().parse()
    {
        r.headers_mut().insert("x-frame-age", v);
    }
    r
}

async fn latest(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(uuid): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let uuid: CameraUuid = uuid.parse().map_err(|_| AppError::NotFound)?;
    let camera = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("reading a camera: {e}")))?
        .ok_or(AppError::NotFound)?;
    let save = q.get("save").is_some_and(|v| v == "1");
    if save && !user.role.is_admin() {
        return Err(AppError::Forbidden);
    }
    let image = if save || q.get("force").is_some_and(|v| v == "1") {
        state.snapshots.fresh(uuid, Duration::from_secs(4)).await
    } else {
        state.snapshots.latest(uuid).await
    };
    let Some(image) = image else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    if save
        && let Err(e) = state
            .snapshots
            .index(&camera, chrono::Utc::now(), &image.bytes)
            .await
    {
        tracing::warn!(camera = %uuid, error = %e, "snapshot not indexed");
    }
    Ok(jpeg(image.bytes, Some(image.age)))
}

async fn file(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    let id: i64 = id.parse().map_err(|_| AppError::NotFound)?;
    match state.snapshots.read(SnapshotId(id)).await {
        Ok(Some((_, bytes))) => Ok(jpeg(bytes, None)),
        Ok(None) => Err(AppError::NotFound),
        Err(e) => Err(AppError::Internal(anyhow::anyhow!(
            "reading a snapshot: {e}"
        ))),
    }
}
