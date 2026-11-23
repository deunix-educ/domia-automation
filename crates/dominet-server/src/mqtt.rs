// SPDX-License-Identifier: AGPL-3.0-or-later
//! `mqtt` role: starts the supervised MQTT client (`dominet-mqtt`) and the ingestion of the devices' messages (ADR 0026 and 0027).
//!
//! Ingestion writes to the database, then broadcasts the state on the hub's `device:<id>` subject **with the last state kept** (the widgets
//! show it as soon as the page arrives). At start-up, the last known states from the database are republished: they therefore
//! survive a server restart.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Context;
use chrono::{DateTime, Utc};
use dominet_automation::alerts::{AlertEngine, AlertHandle};
use dominet_automation::engine::{Engine, EngineHandle};
use dominet_automation::ingest::{Ingestor, Sink};
use dominet_domain::device::{Device, LogEntry, MqttSource, Z2M_BASE_TOPIC};
use dominet_domain::repo::MqttSourceRepo;
use dominet_mqtt::{MqttClient, MqttSettings, Payload, start};
use dominet_proto::{DeviceStateDto, Topic, WsServerMsg};
use dominet_store::reduct::{IotRecord, IotWriter, ReductIot};

use crate::AppState;
use crate::config::MqttConfig;
use crate::hub::Hub;
use dominet_store::{
    PgAlertRules, PgAlerts, PgAutomationRuns, PgAutomations, PgDevices, PgStateLog,
};

use crate::alerts::AlertNotifier;
use crate::automation::ServerExecutor;

type ServerEngine = Engine<PgAutomations, PgAutomationRuns, ServerExecutor>;
type ServerAlerts = AlertEngine<PgAlertRules, PgAlerts, AlertNotifier>;

/// Client settings from the configuration (the password stays a secret up to here).
#[must_use]
pub fn settings(cfg: &MqttConfig) -> MqttSettings {
    let mut s = MqttSettings::new(&cfg.host, cfg.port, &cfg.client_id);
    s.user.clone_from(&cfg.user);
    s.password = cfg.password.as_ref().map(|p| p.expose().to_string());
    s
}

/// Server side of the ingestion: broadcast on the hub and memorisation of `permit_join`.
#[derive(Debug)]
pub struct HubSink {
    hub: Hub,
    permit_join: Arc<std::sync::atomic::AtomicBool>,
    iot: Option<IotWriter>,
    /// Automation engine (`mqtt_message` and `device_state` triggers), absent in some tests.
    engine: Option<EngineHandle>,
    /// Alert rule evaluation after each device state.
    alerts: Option<AlertHandle>,
    /// Last Zigbee2MQTT error shown as a banner (see [`BRIDGE_ERROR_QUIET`]).
    last_bridge_error: std::sync::Mutex<Option<(String, std::time::Instant)>>,
}

/// The same Zigbee2MQTT error is not shown twice within this delay (an unreachable device repeats it at every command).
const BRIDGE_ERROR_QUIET: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether the banner is due: a new message, or the same one after the quiet delay.
fn banner_due(
    last: &mut Option<(String, std::time::Instant)>,
    message: &str,
    now: std::time::Instant,
) -> bool {
    let repeated = last
        .as_ref()
        .is_some_and(|(m, at)| m == message && now.duration_since(*at) < BRIDGE_ERROR_QUIET);
    if !repeated {
        *last = Some((message.to_string(), now));
    }
    !repeated
}

impl HubSink {
    #[must_use]
    pub fn new(
        hub: Hub,
        permit_join: Arc<std::sync::atomic::AtomicBool>,
        iot: Option<IotWriter>,
    ) -> Self {
        Self {
            hub,
            permit_join,
            iot,
            engine: None,
            alerts: None,
            last_bridge_error: std::sync::Mutex::default(),
        }
    }

    /// Wires the alert evaluation.
    #[must_use]
    pub fn with_alerts(mut self, alerts: AlertHandle) -> Self {
        self.alerts = Some(alerts);
        self
    }

    /// Wires the automation engine.
    #[must_use]
    pub fn with_engine(mut self, engine: EngineHandle) -> Self {
        self.engine = Some(engine);
        self
    }
}

impl Sink for HubSink {
    fn device_changed(&self, device: &Device) {
        self.hub.publish_retained(
            &Topic::Device(device.id.0),
            WsServerMsg::DeviceState(DeviceStateDto {
                device_id: device.id.0,
                friendly_name: device.friendly_name.clone(),
                state: device.state.clone(),
                available: device.available,
            }),
        );
    }

    fn permit_join(&self, enabled: bool) {
        self.permit_join.store(enabled, Ordering::Relaxed);
    }

    fn bridge_error(&self, message: &str) {
        // The line is already in the server log (ingestion); connected users also get a banner, without repetition.
        let mut last = self
            .last_bridge_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !banner_due(&mut last, message, std::time::Instant::now()) {
            return;
        }
        self.hub.publish(
            &Topic::Notifications,
            WsServerMsg::Notification {
                level: dominet_proto::NotificationLevel::Warning,
                text: format!("Zigbee2MQTT : {message}")
                    .chars()
                    .take(200)
                    .collect(),
            },
        );
    }

    fn message(&self, topic: &str, payload: &Payload) {
        if let Some(engine) = &self.engine {
            engine.message(topic, payload);
        }
    }

    fn state_skipped(&self, name: &str, topic: &str, state: &serde_json::Value) {
        if let Some(engine) = &self.engine {
            engine.state(name, topic, state);
        }
    }

    fn device_state(&self, device: &Device, state: &serde_json::Value) {
        if let Some(engine) = &self.engine {
            engine.state(&device.friendly_name, &device.state_topic(), state);
        }
        if let Some(alerts) = &self.alerts {
            alerts.state(device.id, &device.friendly_name, state);
        }
    }

    fn source_handler(&self, source: &MqttSource, topic: &str, payload: &Payload) {
        if let Some(engine) = &self.engine {
            engine.handler(&source.name, &source.handler_code, topic, payload);
        }
    }

    fn archive(&self, device: &Device, at: DateTime<Utc>, entries: &[LogEntry]) {
        if let Some(iot) = &self.iot {
            iot.push(IotRecord {
                device: device.id,
                at,
                state: device.state.clone(),
                entries: entries.to_vec(),
            });
        }
    }
}

/// Wanted subscriptions: the whole Zigbee2MQTT prefix at QoS 1, plus the filter of each active source with its QoS (the highest
/// if the same filter comes back), plus the topic of each active `mqtt_message` automation (QoS 1): an automation that
/// waits for a topic outside the sources works without a source to create.
#[must_use]
pub fn subscriptions(
    z2m_base: &str,
    sources: &[MqttSource],
    automation_filters: &[String],
) -> BTreeMap<String, u8> {
    let mut out = BTreeMap::from([(format!("{z2m_base}/#"), 1)]);
    for filter in automation_filters {
        out.insert(filter.clone(), 1);
    }
    for source in sources.iter().filter(|s| s.active) {
        let qos = out
            .entry(source.topic_pattern.clone())
            .or_insert(source.qos);
        *qos = (*qos).max(source.qos);
    }
    out
}

/// ReductStore raw archive, if `REDUCTSTORE_URL` is set (written in a background task: it does not slow ingestion down).
fn iot_writer(state: &AppState) -> Option<IotWriter> {
    let cfg = state.config.reductstore.as_ref()?;
    let quota = u64::from(cfg.iot_quota_gb) * 1024 * 1024 * 1024;
    match ReductIot::with_bucket(
        &cfg.url,
        cfg.token.as_ref().map(crate::config::Secret::expose),
        &cfg.iot_bucket,
        quota,
    ) {
        Ok(archive) => Some(IotWriter::spawn(archive)),
        Err(e) => {
            // The raw archive is optional: the server starts without it.
            tracing::warn!(error = %e, "ReductStore archive disabled: HTTP client cannot be built");
            None
        }
    }
}

/// Safety reload interval of the sources (another process may have modified them).
const SOURCES_POLL: std::time::Duration = std::time::Duration::from_secs(60);

pub(crate) async fn z2m_base(state: &AppState) -> String {
    match state.settings.get_string("Z2M_BASE_TOPIC").await {
        Ok(b) if dominet_domain::device::is_valid_base_topic(b.trim()) && !b.trim().is_empty() => {
            b.trim().to_string()
        }
        _ => Z2M_BASE_TOPIC.to_string(),
    }
}

/// Starts the client and the ingestion; returns the client's handle (commands, link state).
///
/// # Errors
/// Error if `MQTT_HOST` is not set.
pub async fn spawn(state: &AppState) -> anyhow::Result<MqttClient> {
    let cfg = state
        .config
        .mqtt
        .clone()
        .context("the mqtt role needs MQTT_HOST")?;
    let started = start(&settings(&cfg));
    let base = z2m_base(state).await;
    // The automation engine publishes and activates scenes through this role's client: its state carries a copy of it.
    let mut engine_state = state.clone();
    engine_state.mqtt = Some(started.client.clone());
    let (engine, handle, queue) = Engine::new(
        state.store.automations(),
        state.store.automation_runs(),
        ServerExecutor::new(engine_state),
    );
    let (alert_engine, alert_handle, alert_queue) = AlertEngine::new(
        state.store.alert_rules(),
        state.store.alerts(),
        AlertNotifier::new(state),
    );
    alert_engine.load_firing().await;
    let ingestor = Arc::new(Ingestor::new(
        state.store.devices(),
        state.store.state_log(),
        HubSink::new(
            state.hub.clone(),
            state.permit_join.clone(),
            iot_writer(state),
        )
        .with_engine(handle.clone())
        .with_alerts(alert_handle),
        &base,
    ));
    ingestor.announce_known_states().await;
    let wiring = Wiring {
        state: state.clone(),
        ingestor: Arc::clone(&ingestor),
        client: started.client.clone(),
        engine: Arc::clone(&engine),
        handle,
        alert_engine: Arc::clone(&alert_engine),
    };
    wiring.refresh().await;
    tokio::spawn(engine.run(queue));
    tokio::spawn(alert_engine.run(alert_queue));
    tokio::spawn(Arc::clone(&ingestor).run(started.messages));
    // The prefix (setting) and the MQTT sources (administration) change live: subscriptions and routing follow.
    let mut changes = state.settings.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                key = changes.recv() => match key {
                    Ok(k) if k == "Z2M_BASE_TOPIC" => wiring.refresh().await,
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => wiring.refresh().await,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                () = wiring.state.sources_changed.notified() => wiring.refresh().await,
                () = wiring.state.automations_changed.notified() => wiring.refresh().await,
                () = wiring.state.alert_rules_changed.notified() => wiring.refresh().await,
                () = tokio::time::sleep(SOURCES_POLL) => wiring.refresh().await,
            }
        }
    });
    Ok(started.client)
}

/// What is needed to recompute subscriptions and routing.
struct Wiring {
    state: AppState,
    ingestor: Arc<Ingestor<PgDevices, PgStateLog, HubSink>>,
    client: MqttClient,
    engine: Arc<ServerEngine>,
    handle: EngineHandle,
    alert_engine: Arc<ServerAlerts>,
}

impl Wiring {
    /// Re-reads the prefix, the sources and the automations, then updates the routing and the subscriptions (idempotent: nothing
    /// changes if nothing changed, `set_subscriptions` only touches the difference).
    async fn refresh(&self) {
        self.engine.reload().await;
        self.alert_engine.reload().await;
        let base = z2m_base(&self.state).await;
        if base != self.ingestor.base_topic() {
            tracing::info!(%base, "Zigbee2MQTT prefix changed");
            self.ingestor.set_base_topic(&base);
        }
        let sources = match self.state.store.mqtt_sources().list_active().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "unreadable MQTT sources: subscriptions unchanged");
                return;
            }
        };
        self.client.set_subscriptions(subscriptions(
            &base,
            &sources,
            &self.handle.message_filters(),
        ));
        self.ingestor.set_sources(sources);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;

    #[test]
    fn a_repeated_bridge_error_is_shown_once_per_quiet_delay() {
        use std::time::{Duration, Instant};
        let (mut last, t0) = (None, Instant::now());
        assert!(banner_due(&mut last, "a", t0));
        assert!(!banner_due(&mut last, "a", t0 + Duration::from_secs(30)));
        assert!(banner_due(&mut last, "b", t0 + Duration::from_secs(31)));
        assert!(banner_due(&mut last, "a", t0 + Duration::from_secs(32)));
        assert!(!banner_due(&mut last, "a", t0 + Duration::from_secs(80)));
        assert!(banner_due(&mut last, "a", t0 + Duration::from_secs(93)));
    }

    #[test]
    fn settings_carry_credentials_and_client_id_without_showing_the_password() {
        let cfg = MqttConfig {
            host: "mosquitto".into(),
            port: 1884,
            user: Some("dominet".into()),
            password: Some(Secret::new("tres-secret".to_string())),
            client_id: "dominet".into(),
        };
        let s = settings(&cfg);
        assert_eq!(
            (s.host.as_str(), s.port, s.client_id.as_str()),
            ("mosquitto", 1884, "dominet")
        );
        assert_eq!(
            (s.user.as_deref(), s.password.as_deref()),
            (Some("dominet"), Some("tres-secret"))
        );
        assert!(!format!("{s:?}").contains("tres-secret"));
    }

    #[test]
    fn the_whole_z2m_prefix_is_subscribed_at_qos_1() {
        assert_eq!(
            subscriptions("zigbee2mqtt", &[], &[]),
            BTreeMap::from([("zigbee2mqtt/#".to_string(), 1)])
        );
    }

    #[test]
    fn active_sources_are_subscribed_with_the_highest_qos_for_a_shared_filter() {
        let mut a = MqttSource::draft("Linky", "linky/#");
        a.qos = 1;
        let mut b = MqttSource::draft("Linky bis", "linky/#");
        b.qos = 2;
        let mut off = MqttSource::draft("Off", "off/#");
        off.active = false;
        let z2m_dup = MqttSource::draft("Z2M", "zigbee2mqtt/#"); // qos 0: the prefix's QoS 1 is kept
        assert_eq!(
            subscriptions("zigbee2mqtt", &[a, b, off, z2m_dup], &[]),
            BTreeMap::from([("linky/#".to_string(), 2), ("zigbee2mqtt/#".to_string(), 1)])
        );
    }
}

#[cfg(test)]
mod automation_subscription_tests {
    use super::*;

    #[test]
    fn an_mqtt_message_automation_subscribes_to_its_topic_at_qos_1() {
        let mut source = MqttSource::draft("Linky", "linky/#");
        source.qos = 0;
        assert_eq!(
            subscriptions(
                "zigbee2mqtt",
                &[source],
                &["maison/+/inter".to_string(), "linky/#".to_string()]
            ),
            BTreeMap::from([
                ("linky/#".to_string(), 1),
                ("maison/+/inter".to_string(), 1),
                ("zigbee2mqtt/#".to_string(), 1)
            ])
        );
    }
}
