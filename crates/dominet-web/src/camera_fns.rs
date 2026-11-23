// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera server functions (R6 lot 2, ADR 0043 and 0044): viewing for any logged-in account, management for
//! administrators. No secret leaves the server: the interface only knows whether it is set.

use dominet_proto::{
    CameraCardDto, CameraDto, CameraInput, DeleteOutcome, MediaMtxSyncDto, SaveOutcome,
};
use leptos::prelude::*;

#[cfg(feature = "ssr")]
fn camera_dto(c: &dominet_domain::camera::Camera) -> CameraDto {
    let d = &c.config;
    CameraDto {
        id: c.id.0,
        uuid: c.uuid.to_string(),
        name: d.name.clone(),
        location_id: d.location_id,
        active: d.active,
        priority: d.priority,
        notes: d.notes.clone(),
        proto: d.proto.as_str().to_string(),
        ip: d.ip.clone(),
        port: d.port,
        path: d.path.clone(),
        rtsp_user: d.rtsp_user.clone(),
        has_rtsp_pass: !d.rtsp_pass.is_empty(),
        fps: d.fps,
        rtsp_any_port: d.rtsp_any_port,
        rtsp_transport_tcp: d.rtsp_transport_tcp,
        recording_mode: d.recording_mode.as_str().to_string(),
        night_mode_api: d.night_mode_api.as_str().to_string(),
        web_port: d.web_port,
        has_web_api_key: !d.web_api_key.is_empty(),
        web_user: d.web_user.clone(),
        has_web_pass: !d.web_pass.is_empty(),
        onvif: d.onvif,
        onvif_port: d.onvif_port,
        onvif_user: d.onvif_user.clone(),
        has_onvif_pass: !d.onvif_pass.is_empty(),
        ptz: d.ptz,
        yolo_enabled: d.yolo_enabled,
        yolo_classes: d.yolo_classes.join(", "),
        yolo_min_confidence_pct: pct(d.yolo_min_confidence),
        manufacturer: d.manufacturer.clone(),
        model: d.model.clone(),
        firmware: d.firmware.clone(),
        last_seen: c.last_seen.map(dominet_server::timefmt::rfc3339),
        main_token: d.main_token.clone(),
    }
}

#[cfg(feature = "ssr")]
#[allow(
    clippy::cast_possible_truncation,
    reason = "value bounded to 0..=100 by round() and clamp()"
)]
fn pct(confidence: f64) -> i32 {
    (confidence * 100.0).round().clamp(0.0, 100.0) as i32
}

/// Cameras with their configuration, without secrets (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = ListCameras, prefix = "/sf", endpoint = "cameras/list")]
pub async fn list_cameras() -> Result<Vec<CameraDto>, ServerFnError> {
    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let cameras = state
        .cameras
        .list()
        .await
        .map_err(|e| internal("camera list", &e))?;
    Ok(cameras.iter().map(camera_dto).collect())
}

/// Active cameras for the viewing page (any logged-in account): name, location, recording mode.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = ListCameraCards, prefix = "/sf", endpoint = "cameras/cards")]
pub async fn list_camera_cards() -> Result<Vec<CameraCardDto>, ServerFnError> {
    use dominet_domain::repo::LocationRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let cameras = state
        .cameras
        .list()
        .await
        .map_err(|e| internal("camera list", &e))?;
    let locations = state
        .store
        .locations()
        .list()
        .await
        .map_err(|e| internal("place list", &e))?;
    Ok(cameras
        .into_iter()
        .filter(|c| c.config.active)
        .map(|c| CameraCardDto {
            uuid: c.uuid.to_string(),
            location: c
                .config
                .location_id
                .and_then(|id| locations.iter().find(|l| l.id.0 == id))
                .map(|l| l.name.clone())
                .unwrap_or_default(),
            name: c.config.name,
            recording_mode: c.config.recording_mode.as_str().to_string(),
            ptz: c.config.ptz,
            last_seen: c.last_seen.map(dominet_server::timefmt::rfc3339),
        })
        .collect())
}

/// Creates (`id` absent) or replaces (`id` present) a camera. A secret left empty stays unchanged; MediaMTX is updated.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database or the encryption fails.
#[server(name = SaveCamera, prefix = "/sf", endpoint = "cameras/save", input = server_fn::codec::Json)]
pub async fn save_camera(input: CameraInput) -> Result<SaveOutcome<CameraDto>, ServerFnError> {
    use dominet_domain::Problem;
    use dominet_domain::camera::{CameraDraft, CameraId};
    use dominet_server::cameras::{CameraError, CameraInput as Domain, SecretInput};

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let mut draft = CameraDraft::new(input.name.trim());
    let mut bad: Vec<(&'static str, Problem)> = vec![];
    match input.proto.parse() {
        Ok(v) => draft.proto = v,
        Err(_) => bad.push(("proto", Problem::Invalid)),
    }
    match input.recording_mode.parse() {
        Ok(v) => draft.recording_mode = v,
        Err(_) => bad.push(("recording_mode", Problem::Invalid)),
    }
    match input.night_mode_api.parse() {
        Ok(v) => draft.night_mode_api = v,
        Err(_) => bad.push(("night_mode_api", Problem::Invalid)),
    }
    if !(0..=100).contains(&input.yolo_min_confidence_pct) {
        bad.push((
            "yolo_min_confidence_pct",
            Problem::OutOfRange { min: 0, max: 100 },
        ));
    }
    draft.location_id = input.location_id;
    draft.active = input.active;
    draft.priority = input.priority;
    draft.notes = input.notes.trim().to_string();
    draft.ip = input.ip.trim().to_string();
    draft.port = input.port;
    draft.path = input.path.trim().to_string();
    draft.rtsp_user = input.rtsp_user.trim().to_string();
    draft.fps = input.fps;
    draft.rtsp_any_port = input.rtsp_any_port;
    draft.rtsp_transport_tcp = input.rtsp_transport_tcp;
    draft.web_port = input.web_port;
    draft.web_user = input.web_user.trim().to_string();
    draft.onvif = input.onvif;
    draft.onvif_port = input.onvif_port;
    draft.onvif_user = input.onvif_user.trim().to_string();
    draft.ptz = input.ptz;
    draft.yolo_enabled = input.yolo_enabled;
    draft.yolo_classes = input
        .yolo_classes
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    draft.yolo_min_confidence = f64::from(input.yolo_min_confidence_pct) / 100.0;
    draft.main_token = input
        .main_token
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    // Malformed fields are returned with the other problems, in form order.
    let mut problems = draft.problems();
    problems.extend(bad);
    if !problems.is_empty() {
        return Ok(SaveOutcome::Invalid(field_problems(problems)));
    }
    let result = state
        .cameras
        .save(
            input.id.map(CameraId),
            Domain {
                draft,
                rtsp_pass: SecretInput::from_form(&input.rtsp_pass),
                onvif_pass: SecretInput::from_form(&input.onvif_pass),
                web_pass: SecretInput::from_form(&input.web_pass),
                web_api_key: SecretInput::from_form(&input.web_api_key),
            },
        )
        .await;
    match result {
        Ok((camera, sync)) => {
            tracing::info!(
                admin_id = admin.id.0,
                camera = %camera.uuid,
                created = input.id.is_none(),
                mediamtx = ?sync_label(&sync),
                "camera saved"
            );
            Ok(SaveOutcome::Saved(camera_dto(&camera)))
        }
        Err(CameraError::Invalid(p)) => Ok(SaveOutcome::Invalid(field_problems(p))),
        Err(CameraError::NotFound) => Ok(SaveOutcome::NotFound),
        Err(e) => Err(internal("saving a camera", &e)),
    }
}

#[cfg(feature = "ssr")]
fn sync_label(s: &dominet_server::cameras::SyncStatus) -> &'static str {
    use dominet_server::cameras::SyncStatus;
    match s {
        SyncStatus::Disabled => "disabled",
        SyncStatus::Synced => "ok",
        SyncStatus::Failed(_) => "failure",
    }
}

/// Deletes a camera, its recordings, snapshots and events, then its MediaMTX path.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteCamera, prefix = "/sf", endpoint = "cameras/delete")]
pub async fn delete_camera(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::camera::CameraId;
    use dominet_server::cameras::CameraError;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match state.cameras.delete(CameraId(id)).await {
        Ok(sync) => {
            tracing::info!(admin_id = admin.id.0, camera_id = id, mediamtx = ?sync_label(&sync), "camera deleted");
            Ok(DeleteOutcome::Deleted)
        }
        Err(CameraError::NotFound) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(internal("deleting a camera", &e)),
    }
}

/// Synchronises all the MediaMTX paths with the active cameras (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SyncCameras, prefix = "/sf", endpoint = "cameras/sync")]
pub async fn sync_cameras() -> Result<MediaMtxSyncDto, ServerFnError> {
    use dominet_server::cameras::SyncError;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let count = |v: &[String]| u32::try_from(v.len()).unwrap_or(u32::MAX);
    match state.cameras.sync_all().await {
        Ok(r) => {
            tracing::info!(
                admin_id = admin.id.0,
                failed = r.failed.len(),
                "MediaMTX synchronisation requested"
            );
            Ok(MediaMtxSyncDto::Done {
                added: count(&r.added),
                replaced: count(&r.replaced),
                removed: count(&r.removed),
                failed: r.failed,
            })
        }
        Err(SyncError::Disabled) => Ok(MediaMtxSyncDto::Disabled),
        Err(SyncError::MediaMtx(e)) => Ok(MediaMtxSyncDto::Unreachable(e.to_string())),
        Err(SyncError::Repo(e)) => Err(internal("MediaMTX synchronisation", &e)),
    }
}

/// Synchronises a camera with what ONVIF says: manufacturer, model, firmware, main profile, PTZ (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SyncCameraOnvif, prefix = "/sf", endpoint = "cameras/onvif-sync")]
pub async fn sync_camera_onvif(id: i64) -> Result<dominet_proto::OnvifSyncOutcome, ServerFnError> {
    use dominet_domain::camera::CameraId;
    use dominet_proto::OnvifSyncOutcome;
    use dominet_server::cameras::{CameraError, OnvifError};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match state.cameras.onvif_sync(CameraId(id)).await {
        Ok(r) => {
            tracing::info!(
                admin_id = admin.id.0,
                camera_id = id,
                "ONVIF synchronisation requested"
            );
            Ok(OnvifSyncOutcome::Done {
                manufacturer: r.info.manufacturer,
                model: r.info.model,
                firmware: r.info.firmware,
                main_token: r.main_token,
                ptz: r.ptz,
                stream_uri: r.stream_uri,
            })
        }
        Err(CameraError::NotFound) => Ok(OnvifSyncOutcome::NotFound),
        Err(CameraError::OnvifDisabled) => Ok(OnvifSyncOutcome::Disabled),
        Err(CameraError::Onvif(OnvifError::NotAuthorized)) => Ok(OnvifSyncOutcome::NotAuthorized),
        Err(CameraError::Onvif(e)) => Ok(OnvifSyncOutcome::Failed(e.to_string())),
        Err(e) => Err(internal("ONVIF synchronisation", &e)),
    }
}

/// Adds a camera by its IP address, or returns the one that already has this address (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = AddCameraByIp, prefix = "/sf", endpoint = "cameras/add-by-ip", input = server_fn::codec::Json)]
pub async fn add_camera_by_ip(
    input: dominet_proto::AddCameraByIp,
) -> Result<dominet_proto::AddByIpOutcome, ServerFnError> {
    use dominet_proto::AddByIpOutcome;
    use dominet_server::cameras::CameraError;

    use crate::server_fns::{field_problems, internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match state
        .cameras
        .add_by_ip(&input.ip, input.onvif_port, &input.user, &input.pass, None)
        .await
    {
        Ok((camera, created)) => {
            tracing::info!(admin_id = admin.id.0, camera = %camera.uuid, created, "camera added by address");
            Ok(AddByIpOutcome::Added {
                id: camera.id.0,
                name: camera.config.name,
                created,
                model: camera.config.model,
            })
        }
        Err(CameraError::Invalid(p)) => Ok(AddByIpOutcome::Invalid(field_problems(p))),
        Err(CameraError::NoOnvifService) => Ok(AddByIpOutcome::NoOnvif),
        Err(e) => Err(internal("adding a camera by address", &e)),
    }
}

/// Requests an ONVIF discovery from the `discovery` role; the results arrive through the `discovery` subject (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = StartDiscovery, prefix = "/sf", endpoint = "cameras/discovery")]
pub async fn start_discovery() -> Result<(), ServerFnError> {
    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    dominet_server::discovery::request(&state.pool)
        .await
        .map_err(|e| internal("discovery request", &e))?;
    tracing::info!(admin_id = admin.id.0, "ONVIF discovery requested");
    Ok(())
}

/// Host of the current request (`Host` header).
#[cfg(feature = "ssr")]
pub(crate) async fn request_host() -> Option<String> {
    let headers: axum::http::HeaderMap = leptos_axum::extract().await.ok()?;
    headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string)
}

/// What is needed to display a camera's live view (any logged-in account): `None` if it does not exist or is not active.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = CameraLive, prefix = "/sf", endpoint = "cameras/live")]
pub async fn camera_live(
    uuid: String,
) -> Result<Option<dominet_proto::CameraLiveDto>, ServerFnError> {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::CameraRepo;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let Ok(uuid) = uuid.parse::<CameraUuid>() else {
        return Ok(None);
    };
    let found = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| internal("reading a camera", &e))?
        .filter(|c| c.config.active);
    let host = request_host().await;
    Ok(match found {
        Some(c) => Some(live_dto(&state, &c, host.as_deref()).await),
        None => None,
    })
}

/// A camera's live view, as sent to the browser.
#[cfg(feature = "ssr")]
pub(crate) async fn live_dto(
    state: &dominet_server::AppState,
    camera: &dominet_domain::camera::Camera,
    request_host: Option<&str>,
) -> dominet_proto::CameraLiveDto {
    dominet_proto::CameraLiveDto {
        uuid: camera.uuid.to_string(),
        name: camera.config.name.clone(),
        whep_url: dominet_server::cameras::whep_url(state, camera.uuid, request_host).await,
        ptz: camera.config.ptz,
        recording_mode: camera.config.recording_mode.as_str().to_string(),
        night: camera.config.night_mode_api != dominet_domain::camera::NightApi::None,
    }
}

/// Day / night toggle, IR-cut filter or light of a camera, through the configured method (administrators). `action`: `daynight`,
/// `ircut` or `light`.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = CameraNight, prefix = "/sf", endpoint = "cameras/night")]
pub async fn camera_night(
    uuid: String,
    action: String,
) -> Result<dominet_proto::NightOutcomeDto, ServerFnError> {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::CameraRepo;
    use dominet_proto::NightOutcomeDto;
    use dominet_server::cameras::{CameraError, NightError};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let Ok(uuid) = uuid.parse::<CameraUuid>() else {
        return Ok(NightOutcomeDto::NotFound);
    };
    let Some(camera) = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| internal("reading a camera", &e))?
    else {
        return Ok(NightOutcomeDto::NotFound);
    };
    match state.cameras.night(camera.id, &action).await {
        Ok(outcome) => {
            tracing::info!(admin_id = admin.id.0, camera = %camera.uuid, action = %action, "day/night requested");
            Ok(NightOutcomeDto::Done {
                action: outcome.action.as_str().to_string(),
                details: outcome.to_json().to_string(),
            })
        }
        Err(CameraError::NotFound) => Ok(NightOutcomeDto::NotFound),
        Err(CameraError::Night(NightError::UnknownAction(_))) => Ok(NightOutcomeDto::InvalidAction),
        Err(CameraError::Night(NightError::NotConfigured)) => Ok(NightOutcomeDto::NotConfigured),
        Err(CameraError::Night(e)) => Ok(NightOutcomeDto::Failed(e.to_string())),
        Err(CameraError::Onvif(e)) => Ok(NightOutcomeDto::Failed(e.to_string())),
        Err(CameraError::Unreadable) => {
            Ok(NightOutcomeDto::Failed("unreadable camera secret".into()))
        }
        Err(e) => Err(internal("day/night command", &e)),
    }
}

// ------------------------------------------------------------------ snapshots (R6 lot 6)

#[cfg(feature = "ssr")]
fn snapshot_dto(
    s: &dominet_domain::camera::Snapshot,
    cameras: &[dominet_domain::camera::Camera],
) -> dominet_proto::SnapshotDto {
    let camera = cameras.iter().find(|c| c.id == s.camera);
    dominet_proto::SnapshotDto {
        id: s.id.0,
        camera_uuid: camera.map(|c| c.uuid.to_string()).unwrap_or_default(),
        camera_name: camera.map(|c| c.config.name.clone()).unwrap_or_default(),
        taken_at: dominet_server::timefmt::rfc3339(s.taken_at),
        size_bytes: s.size_bytes,
    }
}

/// Takes a snapshot and records it in the gallery (administrators): the camera's last image, or a fresh one.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database or the disk fails.
#[server(name = CaptureSnapshot, prefix = "/sf", endpoint = "snapshots/capture")]
pub async fn capture_snapshot(
    uuid: String,
) -> Result<dominet_proto::CaptureOutcome, ServerFnError> {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::CameraRepo;
    use dominet_proto::CaptureOutcome;
    use dominet_server::snapshots::SnapshotError;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let Ok(uuid) = uuid.parse::<CameraUuid>() else {
        return Ok(CaptureOutcome::NotFound);
    };
    let Some(camera) = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| internal("reading a camera", &e))?
    else {
        return Ok(CaptureOutcome::NotFound);
    };
    match state.snapshots.capture(&camera).await {
        Ok(s) => {
            tracing::info!(admin_id = admin.id.0, camera = %camera.uuid, snapshot = s.id.0, "snapshot taken");
            Ok(CaptureOutcome::Saved(snapshot_dto(
                &s,
                std::slice::from_ref(&camera),
            )))
        }
        Err(SnapshotError::NoImage) => Ok(CaptureOutcome::NoImage),
        Err(e) => Err(internal("snapshot", &e)),
    }
}

/// Indexed snapshots (any logged-in account): `camera` is a camera's public identifier (empty: all), `day` a day `YYYY-MM-DD`
/// (empty: all), read in the server's time zone; 200 at most, most recent first.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = ListSnapshots, prefix = "/sf", endpoint = "snapshots/list")]
pub async fn list_snapshots(
    camera: String,
    day: String,
) -> Result<Vec<dominet_proto::SnapshotDto>, ServerFnError> {
    use dominet_domain::camera::CameraUuid;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let cameras = state
        .cameras
        .list()
        .await
        .map_err(|e| internal("camera list", &e))?;
    let camera_id = if camera.trim().is_empty() {
        None
    } else {
        match camera
            .parse::<CameraUuid>()
            .ok()
            .and_then(|u| cameras.iter().find(|c| c.uuid == u))
        {
            Some(c) => Some(c.id),
            // An unknown camera: empty list (not all the cameras).
            None => return Ok(vec![]),
        }
    };
    let (from, to) = dominet_server::snapshots::day_bounds(&state.config.timezone, &day);
    let snapshots = state
        .snapshots
        .list(camera_id, from, to, 200)
        .await
        .map_err(|e| internal("snapshot list", &e))?;
    Ok(snapshots
        .iter()
        .map(|s| snapshot_dto(s, &cameras))
        .collect())
}

/// Deletes a snapshot (administrators): the row and the file.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = DeleteSnapshot, prefix = "/sf", endpoint = "snapshots/delete")]
pub async fn delete_snapshot(id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::RepoError;
    use dominet_domain::camera::SnapshotId;
    use dominet_server::snapshots::SnapshotError;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    match state.snapshots.delete(SnapshotId(id)).await {
        Ok(()) => {
            tracing::info!(admin_id = admin.id.0, snapshot = id, "snapshot deleted");
            Ok(DeleteOutcome::Deleted)
        }
        Err(SnapshotError::Repo(RepoError::NotFound)) => Ok(DeleteOutcome::NotFound),
        Err(e) => Err(internal("suppression d'un snapshot", &e)),
    }
}

// ------------------------------------------------------------------ recordings (R6 lot 7)

/// A camera's recordings for one day `YYYY-MM-DD` (empty: today, in the server's time zone), from oldest to
/// most recent. Any logged-in account; `None` if the camera does not exist.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = ListRecordings, prefix = "/sf", endpoint = "recordings/list")]
pub async fn list_recordings(
    uuid: String,
    day: String,
) -> Result<Option<dominet_proto::RecordingsDayDto>, ServerFnError> {
    use dominet_domain::camera::{CameraUuid, RecordingMode};
    use dominet_domain::repo::{CameraRepo, RecordingRepo};

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let Ok(uuid) = uuid.parse::<CameraUuid>() else {
        return Ok(None);
    };
    let Some(camera) = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| internal("reading a camera", &e))?
    else {
        return Ok(None);
    };
    let day = if day.trim().is_empty() {
        dominet_server::snapshots::local_today(&state.config.timezone)
    } else {
        day
    };
    let (from, to) = dominet_server::snapshots::day_bounds(&state.config.timezone, &day);
    let (Some(from), Some(to)) = (from, to) else {
        return Ok(Some(dominet_proto::RecordingsDayDto {
            camera_name: camera.config.name.clone(),
            recording_mode: camera.config.recording_mode.as_str().to_string(),
            manual_on: None,
            items: vec![],
        }));
    };
    let items = state
        .store
        .recordings()
        .list_between(camera.id, from, to)
        .await
        .map_err(|e| internal("recording list", &e))?;
    let manual_on = if camera.config.recording_mode == RecordingMode::Manual {
        state.recordings.manual_state(camera.uuid).await
    } else {
        None
    };
    Ok(Some(dominet_proto::RecordingsDayDto {
        camera_name: camera.config.name.clone(),
        recording_mode: camera.config.recording_mode.as_str().to_string(),
        manual_on,
        items: items
            .iter()
            .map(|r| dominet_proto::RecordingDto {
                id: r.id.0,
                started_at: dominet_server::snapshots::local_rfc3339(
                    &state.config.timezone,
                    r.started_at,
                ),
                duration_ms: r.duration_ms,
                event_type: r.event_type.as_str().to_string(),
                status: r.status.as_str().to_string(),
                size_bytes: r.size_bytes,
                error: r.error.clone(),
            })
            .collect(),
    }))
}

/// Deletes a recording (administrators): the local file, the ReductStore archive and the row.
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database or ReductStore fails.
#[server(name = DeleteRecording, prefix = "/sf", endpoint = "recordings/delete")]
pub async fn delete_recording(uuid: String, id: i64) -> Result<DeleteOutcome, ServerFnError> {
    use dominet_domain::camera::{CameraUuid, RecordingId};
    use dominet_domain::repo::{CameraRepo, RecordingRepo};

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let Ok(uuid) = uuid.parse::<CameraUuid>() else {
        return Ok(DeleteOutcome::NotFound);
    };
    let Some(camera) = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| internal("reading a camera", &e))?
    else {
        return Ok(DeleteOutcome::NotFound);
    };
    let Some(rec) = state
        .store
        .recordings()
        .get(RecordingId(id))
        .await
        .map_err(|e| internal("reading a recording", &e))?
        .filter(|r| r.camera == camera.id)
    else {
        return Ok(DeleteOutcome::NotFound);
    };
    state
        .recordings
        .delete(camera.uuid, &rec)
        .await
        .map_err(|e| ServerFnError::new(format!("deleting a recording: {e}")))?;
    tracing::info!(admin_id = admin.id.0, camera = %camera.uuid, recording = id, "recording deleted");
    Ok(DeleteOutcome::Deleted)
}

/// Starts or stops the recording of a camera in manual mode (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SetManualRecord, prefix = "/sf", endpoint = "recordings/manual")]
pub async fn set_manual_record(
    uuid: String,
    on: bool,
) -> Result<dominet_proto::ManualRecordOutcome, ServerFnError> {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::CameraRepo;
    use dominet_proto::ManualRecordOutcome as Out;
    use dominet_server::recordings::RecordingError;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let Ok(uuid) = uuid.parse::<CameraUuid>() else {
        return Ok(Out::NotFound);
    };
    let Some(camera) = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| internal("reading a camera", &e))?
    else {
        return Ok(Out::NotFound);
    };
    Ok(match state.recordings.set_manual(camera.id, on).await {
        Ok(()) => Out::Done { on },
        Err(RecordingError::NotFound) => Out::NotFound,
        Err(RecordingError::NotManual) => Out::NotManual,
        Err(RecordingError::Repo(e)) => return Err(internal("manual recording", &e)),
        Err(e) => Out::Unavailable(e.to_string()),
    })
}

// ------------------------------------------------------------------ events and detector (R6 lot 8)

/// Recent motions and detections (any logged-in account): `camera` is a camera's public identifier (empty: all), most
/// recent first, 100 at most.
///
/// # Errors
/// Refused (401) without a session; internal error if the database fails.
#[server(name = ListCameraEvents, prefix = "/sf", endpoint = "events/list")]
pub async fn list_camera_events(
    camera: String,
) -> Result<Vec<dominet_proto::CameraEventDto>, ServerFnError> {
    use dominet_domain::camera::CameraUuid;
    use dominet_domain::repo::EventRepo;
    use dominet_proto::CameraEventDto;

    use crate::server_fns::{internal, require_user};

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    let cameras = state
        .cameras
        .list()
        .await
        .map_err(|e| internal("camera list", &e))?;
    let chosen: Vec<_> = if camera.trim().is_empty() {
        cameras.iter().collect()
    } else {
        match camera
            .parse::<CameraUuid>()
            .ok()
            .and_then(|u| cameras.iter().find(|c| c.uuid == u))
        {
            Some(c) => vec![c],
            None => return Ok(vec![]),
        }
    };
    let tz = state.config.timezone.clone();
    let mut out: Vec<(chrono::DateTime<chrono::Utc>, CameraEventDto)> = vec![];
    for c in chosen {
        let events = state.store.camera_events();
        for m in events
            .list_motion(c.id, 100)
            .await
            .map_err(|e| internal("motion list", &e))?
        {
            out.push((
                m.detected_at,
                CameraEventDto {
                    kind: "motion".into(),
                    at: dominet_server::snapshots::local_rfc3339(&tz, m.detected_at),
                    camera_uuid: c.uuid.to_string(),
                    camera_name: c.config.name.clone(),
                    motion_pct: Some((m.confidence * 1000.0).round() / 10.0),
                    classes: String::new(),
                    frame_id: None,
                },
            ));
        }
        for d in events
            .list_detections(c.id, 100)
            .await
            .map_err(|e| internal("detection list", &e))?
        {
            out.push((
                d.detected_at,
                CameraEventDto {
                    kind: "detection".into(),
                    at: dominet_server::snapshots::local_rfc3339(&tz, d.detected_at),
                    camera_uuid: c.uuid.to_string(),
                    camera_name: c.config.name.clone(),
                    motion_pct: None,
                    classes: d.class_names().join(", "),
                    frame_id: (!d.frame_path.is_empty()).then_some(d.id.0),
                },
            ));
        }
    }
    out.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    Ok(out.into_iter().take(100).map(|(_, e)| e).collect())
}

/// Global setting of the detector (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = GetDetector, prefix = "/sf", endpoint = "detector/get")]
pub async fn get_detector() -> Result<dominet_proto::DetectorDto, ServerFnError> {
    use dominet_domain::repo::DetectorConfigRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    require_admin(&state).await?;
    let c = state
        .store
        .detector_config()
        .get()
        .await
        .map_err(|e| internal("reading the detector", &e))?;
    Ok(dominet_proto::DetectorDto {
        enabled: c.enabled,
        save_frames: c.save_frames,
    })
}

/// Enables or disables the object detector and the saving of annotated images (administrators).
///
/// # Errors
/// Refused (403) for a non-administrator; internal error if the database fails.
#[server(name = SetDetector, prefix = "/sf", endpoint = "detector/set")]
pub async fn set_detector(enabled: bool, save_frames: bool) -> Result<(), ServerFnError> {
    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    dominet_server::config_service::set_detector(&state, enabled, save_frames)
        .await
        .map_err(|e| internal("saving the detector", &e))?;
    tracing::info!(
        admin_id = admin.id.0,
        enabled,
        save_frames,
        "detector changed"
    );
    Ok(())
}
