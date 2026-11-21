// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rejeu d'une session de **sources MQTT** (appareils automatiques, journal, marquage « vu ») : l'ingestion Rust doit produire les memes
//! appareils et lignes d'historique que l'implementation de reference.
//! Attendu fige dans `tests/fixtures/mqtt/` (ADR 0028).

mod common;

use std::sync::{Arc, Mutex};

use common::{Item, apply_op, assert_same, base, clock_at, dump, ingestor, items_of};
use dominet_domain::device::{DeviceDraft, DeviceType, MqttSource};
use dominet_domain::repo::{DeviceRepo, MqttSourceRepo};
use dominet_mqtt::topic_matches;
use serde_json::{Value, json};
use sqlx::PgPool;

const FIXTURE: &str = include_str!("../../../tests/fixtures/mqtt/sources-r3.jsonl");
const GOLDEN: &str = include_str!("../../../tests/fixtures/mqtt/sources-r3.golden.json");

fn source_of(raw: &Value) -> MqttSource {
    let text = |k: &str| raw[k].as_str().unwrap_or_default().to_string();
    let mut s = MqttSource::draft(&text("name"), &text("topic_pattern"));
    s.base_topic = text("base_topic");
    s.auto_device = raw["auto_device"].as_bool().unwrap_or(false);
    s.auto_device_type = Some(
        raw["auto_device_type"]
            .as_str()
            .map_or(DeviceType::Sensor, |t| t.parse().unwrap()),
    );
    s.log_messages = raw["log_messages"].as_bool().unwrap_or(false);
    s.active = raw["active"].as_bool().unwrap_or(true);
    s.qos = u8::try_from(raw["qos"].as_u64().unwrap_or(0)).unwrap();
    s
}

/// **Ecart connu et voulu avec reference** (ADR 0028) : un upsert dont les valeurs par defaut ne s'appliquent qu'a
/// la creation, donc le **premier message** d'un appareil automatique n'en ecrit pas l'etat (il n'est que diffuse). Un appareil qui n'a
/// recu qu'un seul message reste donc sans etat chez la reference ; ici l'etat est ecrit tout de suite. Le test ne gomme pas l'ecart a
/// l'aveugle : pour chaque appareil dont l'etat attendu est vide, il exige qu'une source **active, en appareil automatique, dont le filtre
/// correspond** ait vu un message objet pour lui, et attend alors le dernier de ces messages.
fn apply_known_deviation(golden: &mut Value) -> usize {
    let mut sources = vec![];
    let mut messages = vec![];
    for item in items_of(FIXTURE) {
        match item {
            Item::Op { op, raw, .. } if op == "source" => sources.push(source_of(&raw)),
            Item::Message { incoming, .. } => messages.push(incoming),
            Item::Op { .. } => {}
        }
    }
    let mut patched = 0;
    for d in golden["devices"].as_array_mut().unwrap() {
        if d["state"] != json!({}) {
            continue;
        }
        let name = d["friendly_name"].as_str().unwrap().to_string();
        let base_topic = d["base_topic"].as_str().unwrap().to_string();
        let full = if base_topic.is_empty() {
            name.clone()
        } else {
            format!("{base_topic}/{name}")
        };
        let seen = messages.iter().rfind(|m| {
            m.topic == full
                && m.payload.as_object().is_some()
                && sources
                    .iter()
                    .any(|s| s.active && s.auto_device && topic_matches(&s.topic_pattern, &m.topic))
        });
        if let Some(m) = seen {
            d["state"] = Value::Object(m.payload.as_object().unwrap().clone());
            patched += 1;
        }
    }
    patched
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn replaying_the_sources_session_gives_what_the_reference_code_gives(pool: PgPool) {
    let clock = Arc::new(Mutex::new(base()));
    let (ing, rec, store) = ingestor(pool.clone(), &clock);
    for item in items_of(FIXTURE) {
        match item {
            Item::Message { t, incoming } => {
                clock_at(&clock, t);
                ing.handle(incoming).await;
            }
            Item::Op {
                t,
                op,
                device,
                keys,
                raw,
            } => {
                clock_at(&clock, t);
                match op.as_str() {
                    "source" => {
                        store.mqtt_sources().create(&source_of(&raw)).await.unwrap();
                        ing.set_sources(store.mqtt_sources().list().await.unwrap());
                    }
                    "device" => {
                        let mut d = DeviceDraft::new(&device);
                        d.base_topic = raw["base_topic"].as_str().unwrap_or_default().to_string();
                        d.device_type = raw["device_type"]
                            .as_str()
                            .map_or(DeviceType::Generic, |t| t.parse().unwrap());
                        d.hidden_keys = vec![];
                        store.devices().create(&d).await.unwrap();
                    }
                    _ => apply_op(&store, &op, &device, &keys).await,
                }
            }
        }
    }
    let mut expected: Value = serde_json::from_str(GOLDEN).unwrap();
    let patched = apply_known_deviation(&mut expected);
    // Un seul appareil est dans ce cas : « 0xCD/prod » (un seul message). Les appareils crees a la main (`capteur`, `agrege`)
    // n'ont jamais d'etat ; ceux qui ont recu plusieurs messages ont l'etat de la reference.
    assert_eq!(patched, 1, "{expected}");
    let mut actual = dump(&pool).await;
    // `exposes` / `hidden_keys` des appareils crees a la main : la creation reference laisse `hidden_keys` vide aussi.
    for v in [&mut actual, &mut expected] {
        v["devices"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|d| d["friendly_name"].as_str().unwrap().to_string());
    }
    assert_same(&actual, &expected);
    // Le sink a vu les appareils automatiques changer (diffusion en direct).
    let changed = rec.changed.lock().unwrap();
    assert!(
        changed.iter().any(|(n, _)| n == "compteur")
            && changed.iter().any(|(n, _)| n == "0xCD/prod")
    );
    assert!(
        !changed
            .iter()
            .any(|(n, _)| n == "y" || n.starts_with("journal"))
    );
}
