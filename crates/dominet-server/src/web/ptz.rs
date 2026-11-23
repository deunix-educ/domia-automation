// SPDX-License-Identifier: AGPL-3.0-or-later
//! PTZ control WebSocket: `/ws/surveillance/ptz/<uuid>/` (R6 lot 5, ADR 0047).
//!
//! Protocol of the reference implementation (`PTZConsumer`), JSON messages:
//!
//! | Browser -> server | Effect |
//! |---|---|
//! | `{"action":"move","pan":-1..1,"tilt":-1..1,"speed_pt":0.05..1}` | `ContinuousMove` (axes multiplied by the speed) |
//! | `{"action":"zoom","z":-1..1,"speed_z":0.05..1}` | `ContinuousMove` on the zoom |
//! | `{"action":"stop"}` | `Stop` |
//! | `{"action":"home"}` | camera preset named `home` / `maison`, otherwise `GotoHomePosition`, or `AbsoluteMove` to the centre if the camera has no home position |
//! | `{"action":"get_presets"}` / `set_preset` (`name`) / `remove_preset` (`id`) | presets from the database, response `{"type":"presets","presets":[{id,name,token}]}` |
//! | `{"action":"goto_preset","id":N}` | `GotoPreset`, or `AbsoluteMove` to the saved position if the preset has no token |
//!
//! Responses: `{"type":"ready"}` when the camera is reached, `{"type":"error","message":"..."}` otherwise.
//!
//! Intended differences: administrators only (D9, the reference implementation allowed any logged-in account), refusal **before** the WebSocket opens
//! (unknown, inactive or non-PTZ camera), origin check as for the notifications, and **automatic stop**: a continuous movement
//! that nobody refreshes or stops (frozen tab, network cut) is stopped after 8 s (`PTZ_DEAD_MAN_SECS`), in addition to the stop on closing.

use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use dominet_domain::camera::{Camera, CameraId, CameraUuid, PtzPresetDraft, PtzPresetId};
use dominet_domain::repo::{CameraRepo, PtzPresetRepo};
use dominet_surveillance::onvif::ptz::{PtzControl, PtzPosition, clamp_speed};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::csrf;
use crate::AppState;
use crate::auth::extract::RequireAdmin;
use crate::error::AppError;

const MAX_MESSAGE_BYTES: usize = 4096;
const NAME_MAX: usize = 128;

pub fn routes() -> Router<AppState> {
    Router::new().route("/ws/surveillance/ptz/{uuid}/", get(handler))
}

async fn handler(
    State(state): State<AppState>,
    RequireAdmin(admin): RequireAdmin,
    headers: HeaderMap,
    Path(uuid): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if !csrf::verdict(&headers, &state.config.csrf_trusted_origins) {
        tracing::warn!(
            user_id = admin.id.0,
            "PTZ WebSocket refused: origin not allowed"
        );
        return Err(AppError::Forbidden);
    }
    let uuid: CameraUuid = uuid.parse().map_err(|_| AppError::NotFound)?;
    let camera = state
        .store
        .cameras()
        .get_by_uuid(uuid)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("reading a camera: {e}")))?
        .filter(|c| c.config.active && c.config.ptz)
        .ok_or(AppError::NotFound)?;
    Ok(ws
        .max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve(socket, state, camera))
        .into_response())
}

async fn send(socket: &mut WebSocket, value: &Value) -> bool {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .is_ok()
}

fn error(message: &str) -> Value {
    json!({"type": "error", "message": message})
}

/// Number read from a message: a JSON number or a numeric text (like the reference's `float()`); `default` if missing.
fn number(v: &Value, key: &str, default: f64) -> f64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

/// Preset number of a message (JSON integer or numeric text; 0, which does not exist, if missing).
fn preset_id(v: &Value) -> PtzPresetId {
    PtzPresetId(match v.get("id") {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

async fn serve(mut socket: WebSocket, state: AppState, camera: Camera) {
    let client = match state.cameras.onvif_client(&camera) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(camera = %camera.uuid, error = %e, "PTZ : client ONVIF impossible");
            let _ = send(&mut socket, &error(&e.to_string())).await;
            return;
        }
    };
    let ptz = match client.ptz().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(camera = %camera.uuid, error = %e, "PTZ: camera unreachable");
            let _ = send(&mut socket, &error(&e.to_string())).await;
            return;
        }
    };
    match ptz.capabilities().await {
        Ok(c) => tracing::info!(camera = %camera.uuid, capacites = %c, "PTZ node"),
        Err(e) => tracing::info!(camera = %camera.uuid, error = %e, "unreadable PTZ node"),
    }
    if !send(&mut socket, &json!({"type": "ready"})).await {
        return;
    }
    // When to stop a movement that nobody refreshes (`PTZ_DEAD_MAN_SECS`, 8 s by default).
    let dead_man = Duration::from_secs(u64::from(state.config.ptz_dead_man_secs));
    let mut stop_at: Option<Instant> = None;
    loop {
        let timer = async {
            match stop_at {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = timer => {
                stop_at = None;
                if let Err(e) = ptz.stop().await {
                    tracing::warn!(camera = %camera.uuid, error = %e, "PTZ: automatic stop failed");
                }
            }
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { break };
                let Message::Text(text) = msg else {
                    if matches!(msg, Message::Close(_)) { break; }
                    continue;
                };
                let Ok(data) = serde_json::from_str::<Value>(&text) else { continue };
                let action = data.get("action").and_then(Value::as_str).unwrap_or("").to_string();
                match act(&state, &camera, &ptz, &action, &data).await {
                    Ok(Reply::Moving) => stop_at = Some(Instant::now() + dead_man),
                    Ok(Reply::Stopped) => stop_at = None,
                    Ok(Reply::Presets(list)) => {
                        if !send(&mut socket, &json!({"type": "presets", "presets": list})).await { break; }
                    }
                    Ok(Reply::HomeSet) => {
                        if !send(&mut socket, &json!({"type": "home_set"})).await { break; }
                    }
                    Ok(Reply::Nothing) => {}
                    Err(message) => {
                        tracing::warn!(camera = %camera.uuid, action, error = %message, "PTZ: order failed");
                        if !send(&mut socket, &error(&message)).await { break; }
                    }
                }
            }
        }
    }
    // Closing: nothing must move any more.
    let _ = ptz.stop().await;
}

/// The preset that serves as the home position: "home" preferably, otherwise "maison".
fn home_preset(
    presets: &[dominet_surveillance::onvif::ptz::CameraPreset],
) -> Option<dominet_surveillance::onvif::ptz::CameraPreset> {
    let by_name = |wanted: &str| {
        presets
            .iter()
            .find(|p| p.name.trim().eq_ignore_ascii_case(wanted))
            .cloned()
    };
    by_name("home").or_else(|| by_name("maison"))
}

enum Reply {
    Moving,
    Stopped,
    Presets(Vec<Value>),
    HomeSet,
    Nothing,
}

async fn presets_json(state: &AppState, camera: CameraId) -> Result<Vec<Value>, String> {
    let list = state
        .store
        .ptz_presets()
        .list(camera)
        .await
        .map_err(|_| "cannot read the presets".to_string())?;
    Ok(list
        .into_iter()
        .map(|p| json!({"id": p.id.0, "name": p.draft.name, "token": p.draft.token}))
        .collect())
}

/// An order from the browser. Errors are sentences without a secret, sent back as is to the browser.
#[allow(
    clippy::too_many_lines,
    reason = "one action of the protocol per branch"
)]
async fn act(
    state: &AppState,
    camera: &Camera,
    ptz: &PtzControl<'_>,
    action: &str,
    data: &Value,
) -> Result<Reply, String> {
    let onvif = |e: dominet_surveillance::onvif::OnvifError| e.to_string();
    match action {
        "move" => {
            let s = clamp_speed(number(data, "speed_pt", 0.5));
            ptz.continuous_move(
                number(data, "pan", 0.0) * s,
                number(data, "tilt", 0.0) * s,
                0.0,
            )
            .await
            .map_err(onvif)?;
            Ok(Reply::Moving)
        }
        "zoom" => {
            let s = clamp_speed(number(data, "speed_z", 0.5));
            ptz.continuous_move(0.0, 0.0, number(data, "z", 0.0) * s)
                .await
                .map_err(onvif)?;
            Ok(Reply::Moving)
        }
        "stop" => {
            ptz.stop().await.map_err(onvif)?;
            Ok(Reply::Stopped)
        }
        "get_presets" => Ok(Reply::Presets(presets_json(state, camera.id).await?)),
        "set_preset" => {
            let name = data
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if name.is_empty() || name.chars().count() > NAME_MAX {
                return Err("invalid preset name (1 to 128 characters)".into());
            }
            // Current position (the NVT does not give it: zeros), then the preset in the camera, then the database (source of truth).
            let position = ptz
                .status()
                .await
                .ok()
                .and_then(|s| s.position)
                .unwrap_or_default();
            // Replacing a preset of the same name: the old token must not stay orphaned in the camera (the reference left it).
            if let Ok(existing) = state.store.ptz_presets().list(camera.id).await
                && let Some(old) = existing
                    .into_iter()
                    .find(|p| p.draft.name == name && !p.draft.token.is_empty())
            {
                let _ = ptz.remove_preset(&old.draft.token).await;
            }
            // Some cameras (128 fixed slots) answer a token that is not that of the slot taken: the camera's list,
            // queried by name, is authoritative; the response of `SetPreset` is only used as a fallback.
            let answered = ptz.set_preset(&name).await.unwrap_or_default();
            let on_camera = ptz.presets().await.unwrap_or_default();
            let token = on_camera
                .iter()
                .rev()
                .find(|p| p.name == name)
                .map_or_else(|| answered.clone(), |p| p.token.clone());
            tracing::info!(
                camera = %camera.uuid,
                answered = %answered,
                kept = %token,
                presets_on_camera = on_camera.len(),
                position_known = position != PtzPosition::default(),
                "PTZ preset saved"
            );
            let draft = PtzPresetDraft {
                name,
                token,
                pan: position.pan,
                tilt: position.tilt,
                zoom: position.zoom,
                position: 0,
            };
            state
                .store
                .ptz_presets()
                .upsert(camera.id, &draft)
                .await
                .map_err(|_| "cannot save the preset".to_string())?;
            Ok(Reply::Presets(presets_json(state, camera.id).await?))
        }
        "remove_preset" => {
            let id = preset_id(data);
            let found = state
                .store
                .ptz_presets()
                .get(id)
                .await
                .map_err(|_| "cannot read the preset".to_string())?
                .filter(|p| p.camera == camera.id);
            if let Some(p) = found {
                if !p.draft.token.is_empty() {
                    // Like the reference: the camera may no longer have it, that is not an error.
                    let _ = ptz.remove_preset(&p.draft.token).await;
                }
                state
                    .store
                    .ptz_presets()
                    .delete(p.id)
                    .await
                    .map_err(|_| "cannot delete the preset".to_string())?;
            }
            Ok(Reply::Presets(presets_json(state, camera.id).await?))
        }
        "set_home" => {
            // The home position is the camera's "home" preset: we overwrite the existing one ("home" or "maison"), otherwise we create it.
            let slot = home_preset(&ptz.presets().await.unwrap_or_default()).map(|p| p.token);
            let token = ptz
                .set_preset_at("home", slot.as_deref())
                .await
                .map_err(onvif)?;
            tracing::info!(camera = %camera.uuid, token = %slot.as_deref().unwrap_or(&token), "PTZ home saved");
            Ok(Reply::HomeSet)
        }
        "home" => {
            // A camera preset named "home" (or "maison") serves as the home position: many cameras have no ONVIF home
            // position. Otherwise `GotoHomePosition`, then, failing that, a return to the centre (0, 0) by `AbsoluteMove`.
            let named = home_preset(&ptz.presets().await.unwrap_or_default());
            if let Some(preset) = named {
                ptz.goto_preset(&preset.token).await.map_err(onvif)?;
            } else if let Err(e) = ptz.go_home().await {
                tracing::info!(camera = %camera.uuid, error = %e, "no home position: returning to the centre");
                let space = ptz
                    .status()
                    .await
                    .ok()
                    .and_then(|s| s.position)
                    .and_then(|p| p.space);
                ptz.absolute_move(&PtzPosition {
                    space,
                    ..PtzPosition::default()
                })
                .await
                .map_err(onvif)?;
            }
            Ok(Reply::Nothing)
        }
        "goto_preset" => {
            let id = preset_id(data);
            let preset = state
                .store
                .ptz_presets()
                .get(id)
                .await
                .map_err(|_| "cannot read the preset".to_string())?
                .filter(|p| p.camera == camera.id)
                .ok_or_else(|| "preset not found".to_string())?;
            tracing::info!(
                camera = %camera.uuid,
                token = %preset.draft.token,
                "go to PTZ preset"
            );
            if preset.draft.token.is_empty() {
                // No preset in the camera: `AbsoluteMove` to the saved position, in the space the camera announces
                // (Thingino: degrees; the default would be `[-1, 1]`).
                let space = ptz
                    .status()
                    .await
                    .ok()
                    .and_then(|s| s.position)
                    .and_then(|p| p.space);
                ptz.absolute_move(&PtzPosition {
                    pan: preset.draft.pan,
                    tilt: preset.draft.tilt,
                    zoom: preset.draft.zoom,
                    space,
                })
                .await
                .map_err(onvif)?;
            } else {
                ptz.goto_preset(&preset.draft.token).await.map_err(onvif)?;
            }
            Ok(Reply::Nothing)
        }
        _ => Ok(Reply::Nothing),
    }
}
