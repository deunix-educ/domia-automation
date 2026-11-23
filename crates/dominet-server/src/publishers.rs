// SPDX-License-Identifier: AGPL-3.0-or-later
//! Weather and SolarMax publishers, hosted by the server (R3 lot 6, ADR 0030): supervised tasks of the `mqtt` role, which read an
//! external source and publish on the broker. Ingestion (MQTT sources) then turns the messages into devices.
//!
//! - **Weather**: always started; it only queries the API for the enabled stations (no station = no request).
//! - **SolarMax**: started only if `SOLARMAX_HOST` is set (never a default address).

use std::sync::Arc;
use std::time::Duration;

use dominet_integrations::run::{
    Publisher, SolarMaxSettings, run_solarmax, run_weather, supervise,
};
use dominet_integrations::weather::OpenMeteo;
use dominet_mqtt::MqttClient;

use crate::AppState;
use crate::config::SolarMaxConfig;

/// Publication through the server's MQTT client (QoS 1, not retained).
#[derive(Debug, Clone)]
pub struct MqttPublisher(pub MqttClient);

impl Publisher for MqttPublisher {
    fn publish(&self, topic: &str, payload: &str) -> Result<(), String> {
        self.0
            .publish(topic, payload, 1, false)
            .map_err(|e| e.to_string())
    }
}

/// SolarMax publisher settings from the configuration.
#[must_use]
pub fn solarmax_settings(cfg: &SolarMaxConfig) -> SolarMaxSettings {
    SolarMaxSettings {
        host: cfg.host.clone(),
        port: cfg.port,
        inverter: cfg.inverter,
        topic: cfg.topic.clone(),
        interval: Duration::from_secs(u64::from(cfg.interval_secs)),
        timeout: Duration::from_secs(10),
    }
}

/// MQTT client of the `publishers` role when the `mqtt` role is not running: it subscribes to nothing. Its identifier ends with
/// `-publishers` so as not to take over the `mqtt` role's session.
///
/// # Errors
/// Error if `MQTT_HOST` is not set.
pub fn own_client(state: &AppState) -> anyhow::Result<MqttClient> {
    use anyhow::Context;
    let mut cfg = state
        .config
        .mqtt
        .clone()
        .context("the publishers role needs MQTT_HOST")?;
    cfg.client_id = format!("{}-publishers", cfg.client_id);
    Ok(dominet_mqtt::start(&crate::mqtt::settings(&cfg)).client)
}

/// Starts the publishers (supervised: a panic restarts the task).
pub fn spawn(state: &AppState, client: &MqttClient) {
    match OpenMeteo::new(&state.config.open_meteo_url) {
        Ok(api) => {
            let (store, wake, out) = (
                state.store.clone(),
                Arc::clone(&state.stations_changed),
                MqttPublisher(client.clone()),
            );
            tokio::spawn(supervise("meteo", move || {
                run_weather_task(store.weather(), api.clone(), out.clone(), Arc::clone(&wake))
            }));
        }
        Err(e) => {
            tracing::warn!(error = %e, "weather publisher disabled: HTTP client cannot be built");
        }
    }
    if let Some(cfg) = &state.config.solarmax {
        let settings = solarmax_settings(cfg);
        let out = MqttPublisher(client.clone());
        tracing::info!(host = %cfg.host, port = cfg.port, topic = %cfg.topic, "SolarMax publisher active");
        tokio::spawn(supervise("solarmax", move || {
            run_solarmax_task(settings.clone(), out.clone())
        }));
    }
}

async fn run_weather_task(
    repo: dominet_store::PgWeather,
    api: OpenMeteo,
    out: MqttPublisher,
    wake: Arc<tokio::sync::Notify>,
) {
    run_weather(repo, api, out, wake).await;
}

async fn run_solarmax_task(settings: SolarMaxSettings, out: MqttPublisher) {
    run_solarmax(settings, out).await;
}
