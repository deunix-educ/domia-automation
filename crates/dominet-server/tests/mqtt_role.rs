// SPDX-License-Identifier: AGPL-3.0-or-later
//! Role `mqtt` de bout en bout sur le Mosquitto de dev (`make dev-up`) : le client demarre depuis la configuration, un message
//! d'etat publie sur le broker arrive en base et sur le hub, puis le role s'arrete proprement.

mod common;

use std::time::Duration;

use common::state;
use dominet_domain::device::{DeviceDraft, DeviceType};
use dominet_domain::repo::{DeviceRepo, StateLogRepo};
use dominet_mqtt::{MqttSettings, Status, start};
use dominet_proto::{Topic, WsServerMsg};
use dominet_server::mqtt;
use serde_json::json;
use sqlx::PgPool;

fn port() -> String {
    std::env::var("MQTT_TEST_PORT").unwrap_or_else(|_| "11883".into())
}

async fn connected(c: &dominet_mqtt::MqttClient) {
    let mut rx = c.watch_status();
    tokio::time::timeout(Duration::from_secs(10), async {
        while *rx.borrow_and_update() != Status::Connected {
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("connexion au broker de dev");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_state_published_on_the_broker_reaches_the_database_the_history_and_the_hub(
    pool: PgPool,
) {
    let id = format!("dominet-test-role-{}", std::process::id());
    let port = port();
    let st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
        ],
    );
    // L'appareil est amorce comme le ferait `bridge/devices`.
    let name = format!("test/{id}");
    let mut draft = DeviceDraft::new(&name);
    draft.device_type = DeviceType::Sensor;
    draft.tracked_keys = vec!["temperature".into()];
    let device = st.store.devices().create(&draft).await.unwrap();

    let client = mqtt::spawn(&st).await.unwrap();
    connected(&client).await;
    let publisher = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("{id}-pub"),
    ))
    .client;
    connected(&publisher).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    publisher
        .publish(
            &format!("zigbee2mqtt/{name}"),
            r#"{"temperature": 21.5, "linkquality": 90}"#,
            1,
            false,
        )
        .unwrap();

    // Le hub a retenu l'etat (diffusion), la base aussi, et la cle suivie est dans l'historique.
    let topic = Topic::Device(device.id.0);
    let retained = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(m) = st.hub.retained(&topic) {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("etat diffuse");
    let WsServerMsg::DeviceState(dto) = &*retained else {
        panic!("{retained:?}")
    };
    assert_eq!(
        (dto.friendly_name.as_str(), dto.available),
        (name.as_str(), true)
    );
    assert_eq!(dto.state, json!({"temperature": 21.5, "linkquality": 90}));
    let stored = st.store.devices().get(device.id).await.unwrap().unwrap();
    assert_eq!(stored.state, dto.state);
    // L'historique s'ecrit apres la diffusion : on lui laisse un instant plutot que de lire une seule fois.
    let mut points = Vec::new();
    for _ in 0..100 {
        points = st
            .store
            .state_log()
            .points(
                device.id,
                "temperature",
                chrono::Utc::now() - chrono::Duration::minutes(5),
                10,
            )
            .await
            .unwrap();
        if !points.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        points.iter().map(|p| p.value_num).collect::<Vec<_>>(),
        [Some(21.5)]
    );

    client.shutdown();
    publisher.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn known_states_are_republished_to_the_hub_at_startup(pool: PgPool) {
    let id = format!("dominet-test-boot-{}", std::process::id());
    let port = port();
    let st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
        ],
    );
    let d = st
        .store
        .devices()
        .upsert_auto(
            &format!("boot/{id}"),
            "",
            DeviceType::Sensor,
            &json!({"temperature": 19}),
        )
        .await
        .unwrap()
        .0;
    assert!(st.hub.retained(&Topic::Device(d.id.0)).is_none());
    let client = mqtt::spawn(&st).await.unwrap();
    assert!(
        st.hub.retained(&Topic::Device(d.id.0)).is_some(),
        "etat connu republie des le demarrage"
    );
    client.shutdown();
}

/// Attend que `check` devienne vrai (10 s au plus).
async fn until<F: std::future::Future<Output = bool>>(what: &str, mut check: impl FnMut() -> F) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("delai depasse : {what}"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_source_added_at_runtime_is_subscribed_creates_its_device_and_feeds_the_history_and_reductstore(
    pool: PgPool,
) {
    use dominet_domain::repo::MqttSourceRepo;
    use dominet_store::reduct::{ReductIot, entry_name};

    let id = format!("dominet-test-src-{}", std::process::id());
    let port = port();
    let bucket = format!("test-iot-role-{}", std::process::id());
    let reduct_url =
        std::env::var("REDUCTSTORE_URL").unwrap_or_else(|_| "http://127.0.0.1:18383".into());
    let token = std::env::var("REDUCTSTORE_TOKEN").unwrap_or_else(|_| "dominet-dev-token".into());
    let st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
            ("REDUCTSTORE_URL", &reduct_url),
            ("REDUCTSTORE_TOKEN", &token),
            ("REDUCTSTORE_IOT_BUCKET", &bucket),
        ],
    );
    let client = mqtt::spawn(&st).await.unwrap();
    connected(&client).await;
    let publisher = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("{id}-pub"),
    ))
    .client;
    connected(&publisher).await;

    // Aucune source : le message est ignore. On l'ajoute « depuis l'administration » (base puis signal).
    let prefix = format!("src{}", std::process::id());
    let topic = format!("{prefix}/compteur/prod");
    let mut source = dominet_domain::device::MqttSource::draft("Compteur", &format!("{prefix}/#"));
    source.qos = 1;
    source.auto_device = true;
    source.base_topic = prefix.clone();
    st.store.mqtt_sources().create(&source).await.unwrap();
    st.sources_changed.notify_one();
    tokio::time::sleep(Duration::from_millis(800)).await; // abonnement pris en compte par le broker

    publisher
        .publish(&topic, r#"{"power": 1200, "unit": "W"}"#, 1, false)
        .unwrap();
    let name = "compteur/prod";
    let devices = st.store.devices();
    until("appareil cree par la source", || async {
        devices.find_by_name(name).await.unwrap().is_some()
    })
    .await;
    let device = devices.find_by_name(name).await.unwrap().unwrap();
    assert_eq!(device.base_topic, prefix);
    assert_eq!(device.state["power"], 1200);
    assert!(st.hub.retained(&Topic::Device(device.id.0)).is_some());

    // Cle suivie ajoutee : le message suivant va a l'historique et a ReductStore.
    let mut draft = DeviceDraft::new(name);
    draft.base_topic = prefix.clone();
    draft.tracked_keys = vec!["power".into()];
    devices.update(device.id, &draft).await.unwrap();
    publisher
        .publish(&topic, r#"{"power": 1300, "unit": "W"}"#, 1, false)
        .unwrap();
    let log = st.store.state_log();
    until("historique", || async {
        !log.points(
            device.id,
            "power",
            chrono::Utc::now() - chrono::Duration::minutes(5),
            10,
        )
        .await
        .unwrap()
        .is_empty()
    })
    .await;
    let reader = ReductIot::with_bucket(&reduct_url, Some(&token), &bucket, 1_000_000).unwrap();
    let entry = entry_name(device.id);
    until("archive ReductStore", || async {
        match reader.open().await {
            Ok(b) => b
                .entries()
                .await
                .unwrap()
                .iter()
                .any(|e| e.name == entry && e.record_count >= 1),
            Err(_) => false,
        }
    })
    .await;
    let record = reader
        .open()
        .await
        .unwrap()
        .read_record(&entry)
        .send()
        .await
        .unwrap();
    assert_eq!(
        record.labels().get("power").map(String::as_str),
        Some("1300")
    );

    // Source supprimee : plus de traitement des messages de son filtre.
    let sources = st.store.mqtt_sources().list().await.unwrap();
    st.store.mqtt_sources().delete(sources[0].id).await.unwrap();
    st.sources_changed.notify_one();
    tokio::time::sleep(Duration::from_millis(800)).await;
    publisher
        .publish(&topic, r#"{"power": 1, "unit": "W"}"#, 1, false)
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let after = devices.get(device.id).await.unwrap().unwrap();
    assert_eq!(
        after.state["power"], 1300,
        "message ignore apres suppression"
    );

    reader.open().await.unwrap().remove().await.unwrap();
    client.shutdown();
    publisher.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn commands_and_pairing_requests_are_published_on_the_broker(pool: PgPool) {
    use dominet_domain::device::DeviceId;
    use dominet_proto::CommandOutcome;
    use dominet_server::control::{send_command, set_permit_join};

    let id = format!("dominet-test-cmd-{}", std::process::id());
    let port = port();
    let mut st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
        ],
    );
    // Appareil sur un prefixe a lui : la commande part sur `<prefixe>/<nom>/set`.
    let prefix = format!("cmd{}", std::process::id());
    let mut draft = DeviceDraft::new("Lampe test");
    draft.base_topic = prefix.clone();
    let device = st.store.devices().create(&draft).await.unwrap();
    let mut inactive = DeviceDraft::new("Lampe eteinte");
    inactive.base_topic = prefix.clone();
    inactive.active = false;
    let inactive = st.store.devices().create(&inactive).await.unwrap();

    // Sans client MQTT dans l'etat : refus immediat.
    let command = json!({"state": "ON", "brightness": 128});
    assert_eq!(
        send_command(&st, device.id, &command).await,
        CommandOutcome::Unavailable
    );
    assert_eq!(
        set_permit_join(&st, true).await,
        CommandOutcome::Unavailable
    );

    // Un second client ecoute ce que le serveur publie.
    let listener = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("{id}-listen"),
    ));
    let mut seen = listener.messages;
    connected(&listener.client).await;
    listener
        .client
        .set_subscriptions(std::collections::BTreeMap::from([
            (format!("{prefix}/#"), 1),
            ("zigbee2mqtt/bridge/request/#".to_string(), 1),
        ]));
    tokio::time::sleep(Duration::from_millis(500)).await;

    st.mqtt = Some(mqtt::spawn(&st).await.unwrap());
    connected(st.mqtt.as_ref().unwrap()).await;
    assert_eq!(
        send_command(&st, device.id, &command).await,
        CommandOutcome::Sent
    );
    assert_eq!(
        send_command(&st, inactive.id, &command).await,
        CommandOutcome::NotFound
    );
    assert_eq!(
        send_command(&st, DeviceId(device.id.0 + 1000), &command).await,
        CommandOutcome::NotFound
    );
    let got = tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("commande recue")
        .unwrap();
    assert_eq!(got.topic, format!("{prefix}/Lampe test/set"));
    assert_eq!(
        got.payload,
        dominet_mqtt::decode_payload(command.to_string().as_bytes())
    );

    assert!(!st.permit_join.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(set_permit_join(&st, true).await, CommandOutcome::Sent);
    assert!(st.permit_join.load(std::sync::atomic::Ordering::Relaxed));
    let got = tokio::time::timeout(Duration::from_secs(10), seen.recv())
        .await
        .expect("permit_join recu")
        .unwrap();
    assert_eq!(got.topic, "zigbee2mqtt/bridge/request/permit_join");
    assert_eq!(
        got.payload,
        dominet_mqtt::decode_payload(br#"{"time":254}"#)
    );

    st.mqtt.as_ref().unwrap().shutdown();
    listener.client.shutdown();
}
