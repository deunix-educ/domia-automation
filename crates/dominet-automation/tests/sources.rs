// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sources MQTT generiques (hors Zigbee2MQTT) : appareils automatiques, historique, marquage « vu » (base jetable).

use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use dominet_automation::ingest::{Ingestor, Sink};
use dominet_domain::device::{Device, DeviceDraft, DeviceType, LogEntry, MqttSource};
use dominet_domain::repo::{DeviceFilter, DeviceRepo, MqttSourceRepo, StateLogRepo};
use dominet_mqtt::{Incoming, decode_payload};
use dominet_store::{PgDevices, PgStateLog, PgStore};
use sqlx::PgPool;

#[derive(Default)]
struct Recorder {
    changed: Mutex<Vec<String>>,
    archived: Mutex<Vec<(String, Vec<String>)>>,
}

struct Shared(Arc<Recorder>);

impl Sink for Shared {
    fn device_changed(&self, device: &Device) {
        self.0
            .changed
            .lock()
            .unwrap()
            .push(device.friendly_name.clone());
    }

    fn permit_join(&self, _enabled: bool) {}

    fn archive(&self, device: &Device, _at: DateTime<Utc>, entries: &[LogEntry]) {
        self.0.archived.lock().unwrap().push((
            device.friendly_name.clone(),
            entries.iter().map(|e| e.key.clone()).collect(),
        ));
    }
}

type TestIngestor = Ingestor<PgDevices, PgStateLog, Shared>;

fn setup(pool: PgPool) -> (TestIngestor, Arc<Recorder>, PgStore) {
    let store = PgStore::new(pool);
    let rec = Arc::new(Recorder::default());
    let ing = Ingestor::with_clock(
        store.devices(),
        store.state_log(),
        Shared(Arc::clone(&rec)),
        "zigbee2mqtt",
        Arc::new(|| Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()),
    );
    (ing, rec, store)
}

fn msg(topic: &str, payload: &str) -> Incoming {
    Incoming {
        topic: topic.to_string(),
        payload: decode_payload(payload.as_bytes()),
        retain: false,
        qos: 0,
    }
}

fn source(pattern: &str) -> MqttSource {
    let mut s = MqttSource::draft("Source", pattern);
    s.auto_device = true;
    s.auto_device_type = Some(DeviceType::Sensor);
    s.base_topic = "solarmax".to_string();
    s
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_auto_device_source_creates_the_device_named_after_the_topic_without_the_prefix(
    pool: PgPool,
) {
    let (ing, rec, store) = setup(pool);
    ing.set_sources(vec![source("solarmax/+/prod")]);
    ing.handle(msg("solarmax/0xAB12/prod", r#"{"power": 1500}"#))
        .await;
    let device = store
        .devices()
        .find_by_name("0xAB12/prod")
        .await
        .unwrap()
        .expect("appareil cree");
    assert_eq!(device.base_topic, "solarmax");
    assert_eq!(device.device_type, DeviceType::Sensor);
    assert_eq!(device.state["power"], 1500);
    assert!(device.available && device.last_seen.is_some());
    assert_eq!(device.state_topic(), "solarmax/0xAB12/prod");
    assert_eq!(*rec.changed.lock().unwrap(), ["0xAB12/prod"]);

    // Le message suivant remplace l'etat.
    ing.handle(msg("solarmax/0xAB12/prod", r#"{"energy": 3}"#))
        .await;
    let again = store.devices().get(device.id).await.unwrap().unwrap();
    assert_eq!(again.state, serde_json::json!({"energy": 3}));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_a_matching_prefix_the_whole_topic_is_the_name_and_the_type_is_the_sources(
    pool: PgPool,
) {
    let (ing, _rec, store) = setup(pool);
    let mut s = source("linky/#");
    s.base_topic = String::new();
    s.auto_device_type = Some(DeviceType::BinarySensor);
    ing.set_sources(vec![s]);
    ing.handle(msg("linky/compteur", r#"{"papp": 420}"#)).await;
    let d = store
        .devices()
        .find_by_name("linky/compteur")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (d.base_topic.as_str(), d.device_type),
        ("", DeviceType::BinarySensor)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn tracked_keys_of_an_auto_device_feed_the_history_and_the_archive(pool: PgPool) {
    let (ing, rec, store) = setup(pool);
    ing.set_sources(vec![source("solarmax/#")]);
    ing.handle(msg("solarmax/a", r#"{"power": 100, "other": 1}"#))
        .await;
    let d = store.devices().find_by_name("a").await.unwrap().unwrap();
    // Aucune cle suivie au depart : pas d'historique (comme en reference).
    assert!(rec.archived.lock().unwrap().is_empty());
    let mut draft = DeviceDraft::new("a");
    draft.base_topic = "solarmax".into();
    draft.tracked_keys = vec!["power".into(), "absent".into()];
    store.devices().update(d.id, &draft).await.unwrap();

    ing.handle(msg("solarmax/a", r#"{"power": 250, "other": 2}"#))
        .await;
    let points = store
        .state_log()
        .points(
            d.id,
            "power",
            Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            10,
        )
        .await
        .unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].value_num, Some(250.0));
    assert_eq!(
        *rec.archived.lock().unwrap(),
        [("a".to_string(), vec!["power".to_string()])]
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn inactive_sources_non_object_payloads_and_other_topics_do_nothing(pool: PgPool) {
    let (ing, rec, store) = setup(pool);
    let mut off = source("off/#");
    off.active = false;
    ing.set_sources(vec![off, source("solarmax/#")]);
    ing.handle(msg("off/x", r#"{"a":1}"#)).await;
    ing.handle(msg("solarmax/texte", "pas du json")).await;
    ing.handle(msg("solarmax/liste", "[1,2]")).await;
    ing.handle(msg("ailleurs/x", r#"{"a":1}"#)).await;
    let all = store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 0);
    assert!(rec.changed.lock().unwrap().is_empty());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_auto_device_message_reactivates_a_deactivated_device_like_reference(pool: PgPool) {
    let (ing, _rec, store) = setup(pool);
    ing.set_sources(vec![source("solarmax/#")]);
    ing.handle(msg("solarmax/b", r#"{"v":1}"#)).await;
    let d = store.devices().find_by_name("b").await.unwrap().unwrap();
    let mut draft = DeviceDraft::new("b");
    draft.base_topic = "solarmax".into();
    draft.active = false;
    store.devices().update(d.id, &draft).await.unwrap();
    assert!(!store.devices().get(d.id).await.unwrap().unwrap().active);
    ing.handle(msg("solarmax/b", r#"{"v":2}"#)).await;
    let back = store.devices().get(d.id).await.unwrap().unwrap();
    assert!(back.active);
    assert_eq!(back.state["v"], 2);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_matching_source_marks_devices_with_that_topic_as_seen_even_without_auto_device(
    pool: PgPool,
) {
    let (ing, _rec, store) = setup(pool);
    let mut draft = DeviceDraft::new("compteur");
    draft.base_topic = "maison".into();
    let d = store.devices().create(&draft).await.unwrap();
    assert!(d.last_seen.is_none());
    let mut s = MqttSource::draft("Maison", "maison/#");
    s.log_messages = true; // exerce le journal (texte volumineux tronque)
    ing.set_sources(vec![s]);
    ing.handle(msg("maison/compteur", &"x".repeat(5000))).await;
    let seen = store.devices().get(d.id).await.unwrap().unwrap();
    assert!(seen.last_seen.is_some() && seen.available);
    // Un topic qui ne correspond a aucune source ne marque rien.
    ing.set_sources(vec![]);
    let before = seen.last_seen;
    ing.handle(msg("maison/compteur", "{}")).await;
    assert_eq!(
        store.devices().get(d.id).await.unwrap().unwrap().last_seen,
        before
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn sources_saved_through_the_repository_drive_the_ingestor(pool: PgPool) {
    let (ing, _rec, store) = setup(pool);
    let repo = store.mqtt_sources();
    let mut a = source("solarmax/#");
    a.name = "Actif".into();
    let mut b = source("autre/#");
    b.name = "Inactif".into();
    b.active = false;
    repo.create(&a).await.unwrap();
    repo.create(&b).await.unwrap();
    ing.set_sources(repo.list().await.unwrap());
    ing.handle(msg("autre/x", r#"{"a":1}"#)).await;
    ing.handle(msg("solarmax/y", r#"{"a":1}"#)).await;
    let names: Vec<String> = store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.friendly_name)
        .collect();
    assert_eq!(names, ["y"]);
}
