// SPDX-License-Identifier: AGPL-3.0-or-later
//! Playback of a recorded segment (R6 lot 7, ADR 0049): `GET /surveillance/recordings/<uuid>/<id>/stream/` (like the reference), any logged-in
//! account. The local file is served with its `Range` requests (the browser can seek in the video); if it was deleted after
//! archiving, the segment is read from ReductStore, in memory, with the same `Range` support.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use dominet_domain::camera::{CameraUuid, RecordingId};
use dominet_domain::repo::{CameraRepo, RecordingRepo};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::AppState;
use crate::auth::extract::CurrentUser;
use crate::error::AppError;
use crate::recordings::SegmentBody;

pub fn routes() -> Router<AppState> {
    Router::new().route("/surveillance/recordings/{uuid}/{id}/stream/", get(stream))
}

/// `bytes=start-end` (a single range) on a content of `len` bytes: (start, end inclusive).
fn parse_range(headers: &HeaderMap, len: usize) -> Option<(usize, usize)> {
    let value = headers.get(header::RANGE)?.to_str().ok()?;
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = match (start.is_empty(), end.is_empty()) {
        // `-N`: the last N bytes.
        (true, false) => (len.saturating_sub(end.parse().ok()?), len - 1),
        (false, true) => (start.parse().ok()?, len - 1),
        (false, false) => (start.parse().ok()?, end.parse::<usize>().ok()?.min(len - 1)),
        (true, true) => return None,
    };
    (start <= end && start < len).then_some((start, end))
}

fn memory_response(headers: &HeaderMap, data: Vec<u8>) -> Response {
    let len = data.len();
    let base = |status: StatusCode, body: Vec<u8>, extra: Option<String>| {
        let mut r = (
            status,
            [
                (header::CONTENT_TYPE, "video/mp4".to_string()),
                (header::ACCEPT_RANGES, "bytes".to_string()),
            ],
            body,
        )
            .into_response();
        if let Some(range) = extra
            && let Ok(v) = range.parse()
        {
            r.headers_mut().insert(header::CONTENT_RANGE, v);
        }
        r
    };
    match parse_range(headers, len) {
        Some((start, end)) => base(
            StatusCode::PARTIAL_CONTENT,
            data[start..=end].to_vec(),
            Some(format!("bytes {start}-{end}/{len}")),
        ),
        None if headers.contains_key(header::RANGE) => {
            let mut r = base(StatusCode::RANGE_NOT_SATISFIABLE, vec![], None);
            if let Ok(v) = format!("bytes */{len}").parse() {
                r.headers_mut().insert(header::CONTENT_RANGE, v);
            }
            r
        }
        None => base(StatusCode::OK, data, None),
    }
}

async fn stream(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Path((uuid, id)): Path<(String, i64)>,
    request: Request,
) -> Result<Response, AppError> {
    let uuid: CameraUuid = uuid.parse().map_err(|_| AppError::NotFound)?;
    let camera = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("reading a camera: {e}")))?
        .ok_or(AppError::NotFound)?;
    let rec = state
        .store
        .recordings()
        .get(RecordingId(id))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("reading a recording: {e}")))?
        .filter(|r| r.camera == camera.id)
        .ok_or(AppError::NotFound)?;
    let headers = request.headers().clone();
    match state.recordings.open(camera.uuid, &rec).await {
        Ok(Some(SegmentBody::Local(path))) => {
            let response = ServeFile::new_with_mime(path, &"video/mp4".parse().unwrap())
                .oneshot(request)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("reading the file: {e}")))?;
            Ok(response.map(Body::new).into_response())
        }
        Ok(Some(SegmentBody::Memory(data))) => Ok(memory_response(&headers, data)),
        Ok(None) => Err(AppError::NotFound),
        Err(e) => Err(AppError::Internal(anyhow::anyhow!("{e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::RANGE, v.parse().unwrap());
        h
    }

    #[test]
    fn ranges_follow_http_semantics() {
        assert_eq!(parse_range(&range("bytes=0-9"), 100), Some((0, 9)));
        assert_eq!(parse_range(&range("bytes=90-"), 100), Some((90, 99)));
        assert_eq!(parse_range(&range("bytes=-10"), 100), Some((90, 99)));
        assert_eq!(parse_range(&range("bytes=50-500"), 100), Some((50, 99)));
        assert_eq!(parse_range(&range("bytes=100-"), 100), None);
        assert_eq!(parse_range(&range("bytes=5-1"), 100), None);
        assert_eq!(parse_range(&range("bytes=0-1,5-6"), 100), None);
        assert_eq!(parse_range(&HeaderMap::new(), 100), None);
    }
}
