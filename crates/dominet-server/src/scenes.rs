// SPDX-License-Identifier: AGPL-3.0-or-later
//! Activation des scenes (R4 lot 2, ADR 0031).
//!
//! Activer une scene = publier chaque commande sur le topic de commande de son appareil, **dans l'ordre**, QoS 1 (comme
//! l'implementation de reference). Une commande en echec n'arrete pas les suivantes ; les echecs sont rapportes.
//! Le moteur d'automations (lot 3) et l'API de script (lot 5) passent par ici.

use dominet_domain::repo::{DeviceRepo, SceneRepo};
use dominet_domain::scene::{Scene, SceneId};
use dominet_proto::{CommandOutcome, SceneFailureDto, SceneOutcome};

use crate::AppState;
use crate::control::send_command;

/// Publie les commandes de `scene`, dans l'ordre.
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

/// Active la scene `id` si elle existe et est active.
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

/// Active la scene de ce nom (API de script, lot 5).
pub async fn activate_by_name(state: &AppState, name: &str) -> SceneOutcome {
    match state.store.scenes().find_by_name(name).await {
        Ok(Some(scene)) if scene.active => activate(state, &scene).await,
        _ => SceneOutcome::NotFound,
    }
}
