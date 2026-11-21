// SPDX-License-Identifier: AGPL-3.0-or-later
//! Crochets de l'ingestion qui alimentent les automations (R4 lot 3) : quels messages sont annonces au `Sink` comme messages
//! (`mqtt_message`) et comme etats d'appareil (`device_state`).

use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use dominet_automation::ingest::{Ingestor, Sink};
use dominet_domain::device::{Device, DeviceDraft, DeviceType, MqttSource};
use dominet_domain::repo::DeviceRepo;
use dominet_mqtt::{Incoming, Payload, decode_payload};
use dominet_store::PgStore;
use serde_json::Value;
use sqlx::PgPool;

#[derive(Default)]
struct Seen {
    messages: Mutex<Vec<String>>,
    states: Mutex<Vec<(String, Value)>>,
    handlers: Mutex<Vec<(String, String)>>,
    skipped: Mutex<Vec<(String, Value)>>,
    bridge_errors: Mutex<Vec<String>>,
}

struct Hooks(Arc<Seen>);

impl Sink for Hooks {
    fn device_changed(&self, _device: &Device) {}
    fn permit_join(&self, _enabled: bool) {}
    fn bridge_error(&self, message: &str) {
        self.0
            .bridge_errors
            .lock()
            .unwrap()
            .push(message.to_string());
    }
    fn message(&self, topic: &str, _payload: &Payload) {
        self.0.messages.lock().unwrap().push(topic.to_string());
    }
    fn source_handler(&self, source: &MqttSource, topic: &str, _payload: &Payload) {
        self.0
            .handlers
            .lock()
            .unwrap()
            .push((source.name.clone(), topic.to_string()));
    }
    fn state_skipped(&self, name: &str, _topic: &str, state: &Value) {
        self.0
            .skipped
            .lock()
            .unwrap()
            .push((name.to_string(), state.clone()));
    }
    fn device_state(&self, device: &Device, state: &Value) {
        self.0
            .states
            .lock()
            .unwrap()
            .push((device.friendly_name.clone(), state.clone()));
    }
}

fn msg(topic: &str, payload: &str) -> Incoming {
    Incoming {
        topic: topic.to_string(),
        payload: decode_payload(payload.as_bytes()),
        retain: false,
        qos: 0,
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn automations_see_states_availability_and_source_messages_but_not_bridge_noise_or_commands(
    pool: PgPool,
) {
    let store = PgStore::new(pool);
    let seen = Arc::new(Seen::default());
    let ing = Ingestor::with_clock(
        store.devices(),
        store.state_log(),
        Hooks(Arc::clone(&seen)),
        "zigbee2mqtt",
        Arc::new(|| Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()),
    );
    store
        .devices()
        .create(&DeviceDraft::new("Lampe"))
        .await
        .unwrap();
    let mut source = MqttSource::draft("Linky", "linky/#");
    source.handler_code = "print(topic)".into();
    source.auto_device = true;
    source.auto_device_type = Some(DeviceType::Sensor);
    ing.set_sources(vec![source]);

    for (topic, payload) in [
        ("zigbee2mqtt/Lampe", r#"{"state":"ON"}"#),
        ("zigbee2mqtt/Lampe/availability", r#"{"state":"online"}"#),
        ("zigbee2mqtt/Inconnue", r#"{"state":"ON"}"#),
        ("zigbee2mqtt/bridge/state", "online"),
        ("zigbee2mqtt/bridge/info", r#"{"permit_join":true}"#),
        ("zigbee2mqtt/bridge/devices", "[]"),
        (
            "zigbee2mqtt/bridge/logging",
            r#"{"level":"info","message":"z2m: MQTT publish: topic 'zigbee2mqtt/Lampe'","namespace":"z2m"}"#,
        ),
        (
            "zigbee2mqtt/bridge/logging",
            r#"{"level":"error","message":"z2m: Publish 'set' 'state' to 'Lampe' failed: 'Error: ZCL command failed'","namespace":"z2m"}"#,
        ),
        ("zigbee2mqtt/Lampe/set", r#"{"state":"OFF"}"#),
        ("linky/compteur", r#"{"papp":420}"#),
        ("autre/chose", "texte"),
    ] {
        ing.handle(msg(topic, payload)).await;
    }
    assert_eq!(
        *seen.messages.lock().unwrap(),
        [
            "zigbee2mqtt/Lampe",
            "zigbee2mqtt/Lampe/availability",
            "zigbee2mqtt/Inconnue",
            "linky/compteur",
            "autre/chose"
        ]
    );
    // Zigbee2MQTT n'a signale qu'une erreur (un ordre non remis) : elle seule est remontee, pas les lignes d'information.
    assert_eq!(
        *seen.bridge_errors.lock().unwrap(),
        ["z2m: Publish 'set' 'state' to 'Lampe' failed: 'Error: ZCL command failed'"]
    );
    // Le code de traitement n'est annonce que pour les messages de la source qui en a un.
    assert_eq!(
        *seen.handlers.lock().unwrap(),
        [("Linky".to_string(), "linky/compteur".to_string())]
    );
    // Les etats : l'appareil connu et l'appareil automatique d'une source ; pas l'appareil inconnu (aucune ligne en base).
    assert_eq!(
        *seen.states.lock().unwrap(),
        [
            ("Lampe".to_string(), serde_json::json!({"state": "ON"})),
            (
                "linky/compteur".to_string(),
                serde_json::json!({"papp": 420})
            ),
        ]
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_state_coalesced_in_a_burst_is_still_announced_to_the_automations(pool: PgPool) {
    // Une telecommande envoie `{"action":"1_single"}` puis `{"action":""}` a quelques millisecondes d'ecart : dans un meme paquet
    // et la meme seconde, l'ingestion ne garde que le dernier etat, mais la touche ne doit pas se perdre pour les automations.
    let store = PgStore::new(pool);
    let seen = Arc::new(Seen::default());
    let ing = Ingestor::with_clock(
        store.devices(),
        store.state_log(),
        Hooks(Arc::clone(&seen)),
        "zigbee2mqtt",
        Arc::new(|| Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()),
    );
    store
        .devices()
        .create(&DeviceDraft::new("Tele-commande"))
        .await
        .unwrap();
    ing.handle_batch(vec![
        msg("zigbee2mqtt/Tele-commande", r#"{"action":"1_single"}"#),
        msg("zigbee2mqtt/Tele-commande", r#"{"action":""}"#),
    ])
    .await;
    assert_eq!(
        *seen.skipped.lock().unwrap(),
        [(
            "Tele-commande".to_string(),
            serde_json::json!({"action": "1_single"})
        )]
    );
    assert_eq!(
        *seen.states.lock().unwrap(),
        [(
            "Tele-commande".to_string(),
            serde_json::json!({"action": ""})
        )],
        "seul le dernier etat est enregistre"
    );
}
