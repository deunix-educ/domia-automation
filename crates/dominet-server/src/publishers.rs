// SPDX-License-Identifier: AGPL-3.0-or-later
//! Publishers meteo et SolarMax, hebergees par le serveur (R3 lot 6, ADR 0030) : des taches supervisees du role `mqtt`, qui lisent une
//! source externe et publient sur le broker. L'ingestion (sources MQTT) transforme ensuite les messages en appareils.
//!
//! - **Meteo** : toujours lancee ; elle n'interroge l'API que pour les stations activees (aucune station = aucune requete).
//! - **SolarMax** : lancee seulement si `SOLARMAX_HOST` est renseigne (jamais d'adresse par defaut).

use std::sync::Arc;
use std::time::Duration;

use dominet_integrations::run::{
    Publisher, SolarMaxSettings, run_solarmax, run_weather, supervise,
};
use dominet_integrations::weather::OpenMeteo;
use dominet_mqtt::MqttClient;

use crate::AppState;
use crate::config::SolarMaxConfig;

/// Publication par le client MQTT du serveur (QoS 1, sans retenue).
#[derive(Debug, Clone)]
pub struct MqttPublisher(pub MqttClient);

impl Publisher for MqttPublisher {
    fn publish(&self, topic: &str, payload: &str) -> Result<(), String> {
        self.0
            .publish(topic, payload, 1, false)
            .map_err(|e| e.to_string())
    }
}

/// Reglages du publisher SolarMax depuis la configuration.
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

/// Client MQTT du role `publishers` quand le role `mqtt` n'est pas lance : il ne s'abonne a rien. Son identifiant se termine par
/// `-publishers` pour ne pas prendre la session du role `mqtt`.
///
/// # Errors
/// Erreur si `MQTT_HOST` n'est pas renseigne.
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

/// Demarre les publishers (supervises : une panique relance la tache).
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
