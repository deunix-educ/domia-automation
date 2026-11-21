// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rejeu d'une session MQTT enregistree : l'ingestion Rust doit produire **les memes appareils, etats et lignes d'historique**
//! que le code de l'implementation de reference (critere d'acceptation de R3). Le resultat attendu (`*.golden.json`) vient de l'implementation de reference
//! lui-meme, execute une fois ; les scripts qui l'ont produit ne sont pas conserves, le fichier l'est (ADR 0027).
//!
//! Format du fichier de messages : voir `tests/common`.

mod common;

use std::sync::{Arc, Mutex};

use common::{
    Item, Recorder, Shared, apply_op, assert_same, base, clock_at, dump, ingestor, items_of,
};
use dominet_automation::ingest::Ingestor;
use dominet_domain::device::DeviceDraft;
use dominet_domain::repo::DeviceRepo;
use dominet_mqtt::{Incoming, decode_payload};
use dominet_store::PgStore;
use serde_json::{Value, json};
use sqlx::PgPool;

const FIXTURE: &str = include_str!("../../../tests/fixtures/z2m/session-r3.jsonl");
const GOLDEN: &str = include_str!("../../../tests/fixtures/z2m/session-r3.golden.json");

fn items() -> Vec<Item> {
    items_of(FIXTURE)
}

fn golden() -> Value {
    let mut g: Value = serde_json::from_str(GOLDEN).unwrap();
    g["devices"].as_array_mut().unwrap().sort_by(|a, b| {
        a["friendly_name"]
            .as_str()
            .cmp(&b["friendly_name"].as_str())
    });
    g["state_log"].as_array_mut().unwrap().sort_by_key(|r| {
        (
            r["device"].as_str().unwrap().to_string(),
            r["key"].as_str().unwrap().to_string(),
            r["ts"].as_i64(),
        )
    });
    apply_known_deviation(&mut g);
    g
}

/// **Ecart connu et voulu avec l'implementation de reference** (ADR 0027). A la creation d'un appareil, reference ecrit `ieee`, `vendor`,
/// `model_id`, `description` et `exposes` dans `defaults`, mais un upsert dont les valeurs par defaut ne s'appliquent
/// qu'a la creation : un appareil nouvellement decouvert reste sans metadonnees jusqu'a
/// la republication suivante de `bridge/devices`. Ici elles sont ecrites des la creation.
///
/// Le test ne gomme pas l'ecart a l'aveugle : pour chaque appareil dont les metadonnees sont vides dans le resultat de la reference,
/// il exige qu'elles soient exactement celles de la derniere decouverte du fichier de messages, et remplace les valeurs attendues.
fn apply_known_deviation(golden: &mut Value) {
    let last: Vec<Value> = items()
        .into_iter()
        .filter_map(|i| match i {
            Item::Message { incoming, .. } if incoming.topic.ends_with("/bridge/devices") => {
                match incoming.payload {
                    dominet_mqtt::Payload::Json(Value::Array(a)) => Some(a),
                    _ => None,
                }
            }
            _ => None,
        })
        .next_back()
        .expect("une decouverte dans la session");
    let mut patched = 0;
    for d in golden["devices"].as_array_mut().unwrap() {
        let empty = d["ieee"] == ""
            && d["vendor"] == ""
            && d["model_id"] == ""
            && d["description"] == ""
            && d["exposes"] == json!([]);
        let name = d["friendly_name"].as_str().unwrap().to_string();
        let discovered = last.iter().find_map(|e| {
            dominet_domain::device::Discovered::from_bridge_entry(e)
                .filter(|x| x.friendly_name == name)
        });
        if let (true, Some(x)) = (empty, discovered)
            && !(x.ieee.is_empty() && x.exposes.is_empty())
        {
            d["ieee"] = json!(x.ieee);
            d["vendor"] = json!(x.vendor);
            d["model_id"] = json!(x.model_id);
            d["description"] = json!(x.description);
            d["exposes"] = json!(x.exposes);
            patched += 1;
        }
    }
    assert_eq!(
        patched, 1,
        "un seul appareil (« Nouveau ») est dans ce cas : {patched}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn replaying_the_session_one_message_at_a_time_gives_what_the_reference_code_gives(
    pool: PgPool,
) {
    let clock = Arc::new(Mutex::new(base()));
    let (ing, rec, store) = ingestor(pool.clone(), &clock);
    for item in items() {
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
                ..
            } => {
                clock_at(&clock, t);
                apply_op(&store, &op, &device, &keys).await;
            }
        }
    }
    assert_same(&dump(&pool).await, &golden());
    // Le sink a vu les changements (diffusion) et `permit_join` : false (premier bridge/info) puis true.
    assert!(!rec.changed.lock().unwrap().is_empty());
    assert_eq!(*rec.permit.lock().unwrap(), [false, true]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn replaying_by_batches_of_the_same_second_gives_the_same_result(pool: PgPool) {
    // En rafale, les etats d'un meme appareil recus dans la meme seconde sont regroupes : le resultat final est identique.
    let clock = Arc::new(Mutex::new(base()));
    let (ing, _rec, store) = ingestor(pool.clone(), &clock);
    let mut batch: Vec<Incoming> = vec![];
    let mut batch_second: Option<i64> = None;
    for item in items() {
        match item {
            Item::Message { t, incoming } => {
                #[allow(clippy::cast_possible_truncation)]
                let second = t.floor() as i64;
                if batch_second.is_some_and(|s| s != second) {
                    ing.handle_batch(std::mem::take(&mut batch)).await;
                }
                batch_second = Some(second);
                clock_at(&clock, f64::from(i32::try_from(second).unwrap()));
                batch.push(incoming);
            }
            Item::Op {
                t,
                op,
                device,
                keys,
                ..
            } => {
                ing.handle_batch(std::mem::take(&mut batch)).await;
                batch_second = None;
                clock_at(&clock, t);
                apply_op(&store, &op, &device, &keys).await;
            }
        }
    }
    ing.handle_batch(batch).await;
    assert_same(&dump(&pool).await, &golden());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn replaying_the_same_session_twice_changes_nothing_more(pool: PgPool) {
    // Doublons QoS 1 : rejouer toute la session ne cree ni appareil ni ligne d'historique de plus.
    let clock = Arc::new(Mutex::new(base()));
    let (ing, _rec, store) = ingestor(pool.clone(), &clock);
    for round in 0..2 {
        for item in items() {
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
                    ..
                } => {
                    clock_at(&clock, t);
                    apply_op(&store, &op, &device, &keys).await;
                }
            }
        }
        if round == 0 {
            assert_same(&dump(&pool).await, &golden());
        }
    }
    assert_same(&dump(&pool).await, &golden());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_burst_through_the_channel_ends_with_the_last_state_of_every_device(pool: PgPool) {
    // 50 appareils x 80 messages d'un coup dans la file : l'ingestion les traite par paquets (regroupes par appareil et par
    // seconde) et la base finit sur le DERNIER etat de chacun ; la file se vide en quelques secondes au plus.
    let store = PgStore::new(pool.clone());
    let rec = Arc::new(Recorder::default());
    let ing = Arc::new(Ingestor::new(
        store.devices(),
        store.state_log(),
        Shared(Arc::clone(&rec)),
        "zigbee2mqtt",
    ));
    for n in 0..50 {
        let mut d = DeviceDraft::new(&format!("burst/{n}"));
        d.tracked_keys = vec!["seq".into()];
        store.devices().create(&d).await.unwrap();
    }
    let (tx, rx) = tokio::sync::mpsc::channel(8192);
    let started = std::time::Instant::now();
    let worker = tokio::spawn(Arc::clone(&ing).run(rx));
    for seq in 0..80 {
        for n in 0..50 {
            let raw = serde_json::to_vec(
                &json!({"seq": seq, "temperature": 20.0 + f64::from(seq) / 10.0}),
            )
            .unwrap();
            tx.send(Incoming {
                topic: format!("zigbee2mqtt/burst/{n}"),
                payload: decode_payload(&raw),
                retain: false,
                qos: 1,
            })
            .await
            .unwrap();
        }
    }
    drop(tx);
    worker.await.unwrap();
    let elapsed = started.elapsed();
    let seqs: Vec<i64> = sqlx::query_scalar(
        "SELECT (state->>'seq')::bigint FROM device WHERE friendly_name LIKE 'burst/%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(seqs.len(), 50);
    assert!(
        seqs.iter().all(|s| *s == 79),
        "chaque appareil finit sur son dernier etat : {seqs:?}"
    );
    // 4000 messages recus, mais l'historique est a la seconde : une ligne par appareil et par seconde au plus.
    let lines: i64 = sqlx::query_scalar("SELECT count(*) FROM device_state_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        lines >= 50 && lines <= 50 * (elapsed.as_secs().cast_signed() + 2),
        "{lines} lignes en {elapsed:?}"
    );
    assert!(elapsed.as_secs() < 20, "4000 messages en {elapsed:?}");
    eprintln!(
        "rafale : 4000 messages en {elapsed:?}, {} diffusions, {lines} lignes d'historique",
        rec.changed.lock().unwrap().len()
    );
}
