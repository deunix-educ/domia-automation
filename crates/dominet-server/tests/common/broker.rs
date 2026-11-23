// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test server linked to the dev Mosquitto (`make dev-up`) and listener of the messages published under a topic prefix of its
//! own (the broker keeps retained Zigbee2MQTT messages and the tests run in parallel).

use std::collections::BTreeMap;
use std::time::Duration;

use dominet_domain::device::{DeviceDraft, DeviceId};
use dominet_domain::repo::DeviceRepo;
use dominet_mqtt::{Incoming, MqttSettings, Payload, Status, start};
use dominet_server::{AppState, mqtt};
use serde_json::Value;
use sqlx::PgPool;

use super::state;

pub fn port() -> String {
    std::env::var("MQTT_TEST_PORT").unwrap_or_else(|_| "11883".into())
}

pub async fn connected(c: &dominet_mqtt::MqttClient) {
    let mut rx = c.watch_status();
    tokio::time::timeout(Duration::from_secs(10), async {
        while *rx.borrow_and_update() != Status::Connected {
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("connexion au broker de dev");
}

/// Test server linked to the broker, and a listener of the commands published under `prefix/`.
pub async fn setup(
    pool: PgPool,
    tag: &str,
    prefix: &str,
) -> (
    AppState,
    tokio::sync::mpsc::Receiver<Incoming>,
    dominet_mqtt::MqttClient,
) {
    let id = format!("dominet-test-{tag}-{}", std::process::id());
    let port = port();
    let mut st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
        ],
    );
    let listener = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("{id}-listen"),
    ));
    connected(&listener.client).await;
    listener
        .client
        .set_subscriptions(BTreeMap::from([(format!("{prefix}/#"), 1)]));
    tokio::time::sleep(Duration::from_millis(500)).await;
    // A separate Zigbee2MQTT base: the server's client does not receive the real dev bridge's retained messages (which
    // would create devices with the same names as the test's).
    st.settings
        .set("Z2M_BASE_TOPIC", &format!("{prefix}z2m"))
        .await
        .unwrap();
    st.mqtt = Some(mqtt::spawn(&st).await.unwrap());
    connected(st.mqtt.as_ref().unwrap()).await;
    (st, listener.messages, listener.client)
}

pub async fn next(rx: &mut tokio::sync::mpsc::Receiver<Incoming>) -> (String, Value) {
    let m = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("commande recue")
        .unwrap();
    let Payload::Json(v) = m.payload else {
        panic!("payload non JSON : {:?}", m.payload)
    };
    (m.topic, v)
}

pub async fn device(st: &AppState, name: &str, prefix: &str, active: bool) -> DeviceId {
    let mut draft = DeviceDraft::new(name);
    draft.base_topic = prefix.to_string();
    draft.active = active;
    st.store.devices().create(&draft).await.unwrap().id
}

/// A second client, to publish what the server must receive (device states, messages).
pub async fn publisher(tag: &str) -> dominet_mqtt::MqttClient {
    let client = start(&MqttSettings::new(
        "127.0.0.1",
        port().parse().unwrap(),
        &format!("dominet-test-{tag}-{}-pub", std::process::id()),
    ))
    .client;
    connected(&client).await;
    client
}
