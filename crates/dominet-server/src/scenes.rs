// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scene activation (R4 lot 2, ADR 0031).
//!
//! Activating a scene = publishing each command on its device's command topic, **in order**, QoS 1 (like
//! the reference implementation). A failing command does not stop the following ones; the failures are reported.
//! The automation engine (lot 3) and the script API (lot 5) go through here.

use dominet_domain::repo::{DeviceRepo, SceneRepo};
use dominet_domain::scene::{Scene, SceneId};
use dominet_proto::{CommandOutcome, SceneFailureDto, SceneOutcome};

use crate::AppState;
use crate::control::send_command;

/// Publishes the commands of `scene`, in order.
pub async fn activate(state: &AppState, scene: &Scene) -> SceneOutcome {
    let mut sent = 0u32;
    let mut failed = vec![];
    for action in &scene.actions {
        match send_command(state, action.device, &action.payload).await {
            CommandOutcome::Sent => sent += 1,
            outcome => {
                let device = match state.store.devices().get(action.device).await {
                    Ok(Some(d)) => d.friendly_name,
                    _ => format!("#{}", action.device.0),
                };
                failed.push(SceneFailureDto { device, outcome });
            }
        }
    }
    tracing::info!(
        scene_id = scene.id.0,
        scene = %scene.name,
        sent,
        failed = failed.len(),
        "scene activated"
    );
    SceneOutcome::Activated { sent, failed }
}

/// Activates scene `id` if it exists and is active.
pub async fn activate_by_id(state: &AppState, id: SceneId) -> SceneOutcome {
    match state.store.scenes().get(id).await {
        Ok(Some(scene)) if scene.active => activate(state, &scene).await,
        Ok(_) => SceneOutcome::NotFound,
        Err(e) => {
            tracing::warn!(scene_id = id.0, error = %e, "unreadable scene");
            SceneOutcome::Activated {
                sent: 0,
                failed: vec![SceneFailureDto {
                    device: String::new(),
                    outcome: CommandOutcome::Unavailable,
                }],
            }
        }
    }
}

/// Activates the scene of that name (script API, lot 5).
pub async fn activate_by_name(state: &AppState, name: &str) -> SceneOutcome {
    match state.store.scenes().find_by_name(name).await {
        Ok(Some(scene)) if scene.active => activate(state, &scene).await,
        _ => SceneOutcome::NotFound,
    }
}
