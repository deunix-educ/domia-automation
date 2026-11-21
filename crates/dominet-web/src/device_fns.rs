// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fonctions serveur des appareils : commandes, appairage, informations d'affichage (R3 lot 5, ADR 0029).

use dominet_proto::{CommandOutcome, GrafanaOutcome};
use leptos::prelude::*;

/// Envoie une commande (objet JSON, ex. `{"state":"ON"}`) a un appareil : publiee sur `<appareil>/set`. Tout compte connecte
/// peut commander (comme en Reference : la famille allume une lampe).
///
/// # Errors
/// Refus (401) sans session.
#[server(name = SendDeviceCommand, prefix = "/sf", endpoint = "devices/command")]
pub async fn send_device_command(
    device_id: i64,
    command: String,
) -> Result<CommandOutcome, ServerFnError> {
    use dominet_domain::command::parse_command;
    use dominet_domain::device::DeviceId;

    use crate::server_fns::require_user;

    let state = expect_context::<dominet_server::AppState>();
    let user = require_user(&state).await?;
    let Ok(value) = parse_command(&command) else {
        return Ok(CommandOutcome::Invalid);
    };
    let outcome = dominet_server::control::send_command(&state, DeviceId(device_id), &value).await;
    tracing::info!(user_id = user.id.0, device_id, ?outcome, "device command");
    Ok(outcome)
}

/// Ouvre ou ferme l'appairage de nouveaux appareils Zigbee2MQTT (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur.
#[server(name = SetPermitJoin, prefix = "/sf", endpoint = "devices/permit-join")]
pub async fn set_permit_join(enabled: bool) -> Result<CommandOutcome, ServerFnError> {
    use crate::server_fns::require_admin;

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let outcome = dominet_server::control::set_permit_join(&state, enabled).await;
    tracing::info!(admin_id = admin.id.0, enabled, ?outcome, "permit_join");
    Ok(outcome)
}

/// Etat connu de l'appairage (mis a jour par `bridge/info` et par `set_permit_join`).
///
/// # Errors
/// Refus (401) sans session.
#[server(name = PermitJoinState, prefix = "/sf", endpoint = "devices/permit-join/state")]
pub async fn permit_join_state() -> Result<bool, ServerFnError> {
    use crate::server_fns::require_user;

    let state = expect_context::<dominet_server::AppState>();
    require_user(&state).await?;
    Ok(state.permit_join.load(std::sync::atomic::Ordering::Relaxed))
}

/// Ecrit le tableau Grafana de l'appareil (un panneau par cle suivie) dans le dossier surveille par Grafana, et coche
/// « tableau Grafana » sur l'appareil (administrateurs).
///
/// # Errors
/// Refus (403) pour un non-administrateur ; erreur interne si la base echoue.
#[server(name = GenerateDeviceDashboard, prefix = "/sf", endpoint = "devices/grafana")]
pub async fn generate_device_dashboard(device_id: i64) -> Result<GrafanaOutcome, ServerFnError> {
    use dominet_domain::device::{DeviceDraft, DeviceId};
    use dominet_domain::repo::DeviceRepo;

    use crate::server_fns::{internal, require_admin};

    let state = expect_context::<dominet_server::AppState>();
    let admin = require_admin(&state).await?;
    let Some(device) = state
        .store
        .devices()
        .get(DeviceId(device_id))
        .await
        .map_err(|e| internal("reading a device", &e))?
    else {
        return Ok(GrafanaOutcome::NotFound);
    };
    let Some(dir) = state.config.grafana_dashboards_dir.clone() else {
        return Ok(GrafanaOutcome::NotConfigured);
    };
    match dominet_server::grafana::write_dashboard_async(dir.into(), device.clone()).await {
        Ok(Some(path)) => {
            tracing::info!(admin_id = admin.id.0, device_id, file = %path.display(), "Grafana dashboard written");
            if !device.grafana_dashboard {
                let mut draft = DeviceDraft::new(&device.friendly_name);
                draft.description = device.description.clone();
                draft.device_type = device.device_type;
                draft.group = device.group;
                draft.location = device.location;
                draft.base_topic = device.base_topic.clone();
                draft.active = device.active;
                draft.icon = device.icon.clone();
                draft.image_url = device.image_url.clone();
                draft.tracked_keys = device.tracked_keys.clone();
                draft.hidden_keys = device.hidden_keys.clone();
                draft.grafana_dashboard = true;
                state
                    .store
                    .devices()
                    .update(device.id, &draft)
                    .await
                    .map_err(|e| internal("updating a device", &e))?;
            }
            Ok(GrafanaOutcome::Written)
        }
        Ok(None) => Ok(GrafanaOutcome::NoTrackedKeys),
        Err(e) => {
            tracing::warn!(admin_id = admin.id.0, device_id, error = %e, "Grafana dashboard not written");
            Ok(GrafanaOutcome::NotConfigured)
        }
    }
}
