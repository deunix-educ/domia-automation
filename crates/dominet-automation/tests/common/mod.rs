// SPDX-License-Identifier: AGPL-3.0-or-later
//! Outils communs des rejeux : lecture du fichier de messages, ingestion sur une base jetable avec horloge virtuelle, instantane
//! et comparaison.
#![allow(dead_code)] // chaque fichier de test n'utilise qu'une partie de ces outils

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, TimeZone, Utc};
use dominet_automation::ingest::{Ingestor, Sink};
use dominet_domain::device::{Device, DeviceDraft};
use dominet_domain::repo::{DeviceFilter, DeviceRepo};
use dominet_mqtt::{Incoming, decode_payload};
use dominet_store::{PgDevices, PgStateLog, PgStore};
use serde_json::{Value, json};
use sqlx::PgPool;

pub fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
}

#[derive(Default)]
pub struct Recorder {
    pub changed: Mutex<Vec<(String, bool)>>,
    pub permit: Mutex<Vec<bool>>,
}

pub struct Shared(pub Arc<Recorder>);

impl Sink for Shared {
    fn device_changed(&self, device: &Device) {
        self.0
            .changed
            .lock()
            .unwrap()
            .push((device.friendly_name.clone(), device.available));
    }

    fn permit_join(&self, enabled: bool) {
        self.0.permit.lock().unwrap().push(enabled);
    }
}

pub enum Item {
    Message {
        t: f64,
        incoming: Incoming,
    },
    Op {
        t: f64,
        op: String,
        device: String,
        keys: Vec<String>,
        /// La ligne complete du fichier (pour les operations a parametres : `source`, `device`).
        raw: Value,
    },
}

pub fn items_of(text: &str) -> Vec<Item> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            let t = v["t"].as_f64().unwrap_or(0.0);
            if let Some(op) = v.get("op").and_then(Value::as_str) {
                return Item::Op {
                    t,
                    raw: v.clone(),
                    op: op.to_string(),
                    device: v["device"].as_str().unwrap_or_default().to_string(),
                    keys: v
                        .get("keys")
                        .and_then(Value::as_array)
                        .map_or_else(Vec::new, |k| {
                            k.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        }),
                };
            }
            let raw = match &v["payload"] {
                Value::String(s) => s.clone().into_bytes(),
                other => serde_json::to_vec(other).unwrap(),
            };
            Item::Message {
                t,
                incoming: Incoming {
                    topic: v["topic"].as_str().unwrap().to_string(),
                    payload: decode_payload(&raw),
                    retain: v["retain"].as_bool().unwrap_or(false),
                    qos: 1,
                },
            }
        })
        .collect()
}

pub type TestIngestor = Ingestor<PgDevices, PgStateLog, Shared>;
pub type LogRow = (String, String, DateTime<Utc>, Option<f64>, String);

pub fn ingestor(
    pool: PgPool,
    clock: &Arc<Mutex<DateTime<Utc>>>,
) -> (TestIngestor, Arc<Recorder>, PgStore) {
    let store = PgStore::new(pool);
    let rec = Arc::new(Recorder::default());
    let c = Arc::clone(clock);
    let ing = Ingestor::with_clock(
        store.devices(),
        store.state_log(),
        Shared(Arc::clone(&rec)),
        "zigbee2mqtt",
        Arc::new(move || *c.lock().unwrap()),
    );
    (ing, rec, store)
}

pub async fn apply_op(store: &PgStore, op: &str, device: &str, keys: &[String]) {
    let repo = store.devices();
    let d = repo
        .find_by_name(device)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("appareil {device} absent"));
    let mut draft = DeviceDraft {
        friendly_name: d.friendly_name.clone(),
        description: d.description.clone(),
        device_type: d.device_type,
        group: d.group,
        location: d.location,
        base_topic: d.base_topic.clone(),
        active: d.active,
        icon: d.icon.clone(),
        image_url: d.image_url.clone(),
        tracked_keys: d.tracked_keys.clone(),
        hidden_keys: d.hidden_keys.clone(),
        grafana_dashboard: d.grafana_dashboard,
    };
    match op {
        "track" => draft.tracked_keys = keys.to_vec(),
        "activate" => draft.active = true,
        "deactivate" => draft.active = false,
        other => panic!("operation inconnue : {other}"),
    }
    repo.update(d.id, &draft).await.unwrap();
}

/// Structure du fichier de messages rejoues.
pub async fn dump(pool: &PgPool) -> Value {
    let store = PgStore::new(pool.clone());
    let mut devices: Vec<Value> = store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .unwrap()
        .into_iter()
        .map(|d| {
            json!({
                "friendly_name": d.friendly_name, "ieee": d.ieee, "vendor": d.vendor, "model_id": d.model_id,
                "description": d.description, "device_type": d.device_type.as_str(), "base_topic": d.base_topic,
                "state": d.state, "available": d.available, "active": d.active, "exposes": d.exposes,
                "hidden_keys": d.hidden_keys, "tracked_keys": d.tracked_keys, "has_last_seen": d.last_seen.is_some(),
            })
        })
        .collect();
    devices.sort_by(|a, b| {
        a["friendly_name"]
            .as_str()
            .cmp(&b["friendly_name"].as_str())
    });
    let rows: Vec<LogRow> = sqlx::query_as(
        "SELECT d.friendly_name, l.key, l.ts, l.value_num, l.value_str FROM device_state_log l JOIN device d ON d.id = l.device_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut log: Vec<Value> = rows
        .into_iter()
        .map(|(device, key, ts, value_num, value_str)| {
            json!({"device": device, "key": key, "ts": (ts - base()).num_seconds(), "value_num": value_num, "value_str": value_str})
        })
        .collect();
    log.sort_by_key(|r| {
        (
            r["device"].as_str().unwrap().to_string(),
            r["key"].as_str().unwrap().to_string(),
            r["ts"].as_i64(),
        )
    });
    json!({"devices": devices, "state_log": log})
}

/// Compare appareil par appareil pour que l'ecart soit lisible.
pub fn assert_same(actual: &Value, expected: &Value) {
    let names = |v: &Value| {
        v["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["friendly_name"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(actual), names(expected), "memes appareils");
    for (a, e) in actual["devices"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected["devices"].as_array().unwrap())
    {
        assert_eq!(a, e, "appareil {}", e["friendly_name"]);
    }
    assert_eq!(
        actual["state_log"], expected["state_log"],
        "memes lignes d'historique"
    );
}

pub fn clock_at(clock: &Arc<Mutex<DateTime<Utc>>>, t: f64) {
    #[allow(clippy::cast_possible_truncation)]
    let millis = (t * 1000.0).round() as i64;
    *clock.lock().unwrap() = base() + Duration::milliseconds(millis);
}
