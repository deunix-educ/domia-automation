// SPDX-License-Identifier: AGPL-3.0-or-later
//! Envoi de commandes aux appareils et ouverture de l'appairage Zigbee2MQTT (R3 lot 5, ADR 0029).
//!
//! Le serveur publie sur le broker, QoS 1, sans retenue ; l'effet se voit quand Zigbee2MQTT republie l'etat de l'appareil.
//! Rien n'est mis en attente : sans lien avec le broker la commande est refusee tout de suite (`Unavailable`).

use dominet_domain::device::DeviceId;
use dominet_domain::repo::DeviceRepo;
use dominet_proto::CommandOutcome;
use serde_json::{Value, json};

use crate::AppState;

/// Publie `command` (objet JSON deja valide) sur `<topic de l'appareil>/set`.
pub async fn send_command(state: &AppState, device: DeviceId, command: &Value) -> CommandOutcome {
    let found = match state.store.devices().get(device).await {
        Ok(Some(d)) if d.active => d,
        Ok(_) => return CommandOutcome::NotFound,
        Err(e) => {
            tracing::warn!(device_id = device.0, error = %e, "unreadable device for a command");
            return CommandOutcome::Unavailable;
        }
    };
    let Some(client) = &state.mqtt else {
        return CommandOutcome::Unavailable;
    };
    match client.publish(&found.command_topic(), command.to_string(), 1, false) {
        Ok(()) => CommandOutcome::Sent,
        Err(e) => {
            tracing::warn!(device_id = device.0, error = %e, "command not published");
            CommandOutcome::Unavailable
        }
    }
}

/// Duree d'ouverture de l'appairage, en secondes (maximum accepte par Zigbee2MQTT).
pub const PERMIT_JOIN_SECS: u8 = 254;

/// Ouvre ou ferme l'appairage de nouveaux appareils (`<prefixe>/bridge/request/permit_join`). Comme en reference, l'etat
/// affiche est mis a jour tout de suite ; `bridge/info` le confirmera.
///
/// Zigbee2MQTT 2.x n'accepte plus `{"value": true}` (reponse « Invalid payload », constate en production le 2026-11-13) : il
/// faut une duree en secondes, `{"time": N}`, 0 pour fermer et 254 au plus. L'appairage se referme donc seul apres
/// [`PERMIT_JOIN_SECS`] ; `bridge/info` remet alors l'etat affiche a « ferme ».
pub async fn set_permit_join(state: &AppState, enabled: bool) -> CommandOutcome {
    let Some(client) = &state.mqtt else {
        return CommandOutcome::Unavailable;
    };
    let base = crate::mqtt::z2m_base(state).await;
    let topic = format!("{base}/bridge/request/permit_join");
    let time = if enabled { PERMIT_JOIN_SECS } else { 0 };
    match client.publish(&topic, json!({"time": time}).to_string(), 1, false) {
        Ok(()) => {
            state
                .permit_join
                .store(enabled, std::sync::atomic::Ordering::Relaxed);
            CommandOutcome::Sent
        }
        Err(e) => {
            tracing::warn!(error = %e, "permit_join not published");
            CommandOutcome::Unavailable
        }
    }
}
