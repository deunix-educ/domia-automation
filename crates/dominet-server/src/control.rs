// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sending commands to devices and opening Zigbee2MQTT pairing (R3 lot 5, ADR 0029).
//!
//! The server publishes on the broker, QoS 1, not retained; the effect shows when Zigbee2MQTT republishes the device's state.
//! Nothing is queued: without a link to the broker the command is refused right away (`Unavailable`).

use dominet_domain::device::DeviceId;
use dominet_domain::repo::DeviceRepo;
use dominet_proto::CommandOutcome;
use serde_json::{Value, json};

use crate::AppState;

/// Publishes `command` (an already validated JSON object) on `<device topic>/set`.
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

/// Duration the pairing stays open, in seconds (maximum accepted by Zigbee2MQTT).
pub const PERMIT_JOIN_SECS: u8 = 254;

/// Opens or closes the pairing of new devices (`<prefix>/bridge/request/permit_join`). As in the reference, the displayed
/// state is updated right away; `bridge/info` will confirm it.
///
/// Zigbee2MQTT 2.x no longer accepts `{"value": true}` (answer "Invalid payload", observed in production on 2026-11-13): it
/// needs a duration in seconds, `{"time": N}`, 0 to close and 254 at most. Pairing therefore closes by itself after
/// [`PERMIT_JOIN_SECS`]; `bridge/info` then puts the displayed state back to "closed".
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
