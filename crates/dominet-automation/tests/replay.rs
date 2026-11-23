// SPDX-License-Identifier: AGPL-3.0-or-later
//! Replay of a recorded MQTT session: Rust ingestion must produce **the same devices, states and history rows**
//! as the reference implementation's code (acceptance criterion of R3). The expected result (`*.golden.json`) comes from the reference implementation
//! itself, run once; the scripts that produced it are not kept, the file is (ADR 0027).
//!
//! Format of the message file: see `tests/common`.

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

/// **Known and intended difference with the reference implementation** (ADR 0027). When a device is created, the reference writes `ieee`, `vendor`,
/// `model_id`, `description` and `exposes` into `defaults`, but an upsert whose default values only apply
/// at creation: a newly discovered device stays without metadata until
/// the next republication of `bridge/devices`. Here they are written from creation.
///
/// The test does not blindly erase the difference: for each device whose metadata is empty in the reference's result,
/// it requires that it be exactly that of the last discovery of the message file, and replaces the expected values.
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
    // The sink saw the changes (broadcast) and `permit_join`: false (first bridge/info) then true.
    assert!(!rec.changed.lock().unwrap().is_empty());
    assert_eq!(*rec.permit.lock().unwrap(), [false, true]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn replaying_by_batches_of_the_same_second_gives_the_same_result(pool: PgPool) {
    // In a burst, the states of the same device received in the same second are coalesced: the final result is identical.
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
    // QoS 1 duplicates: replaying the whole session creates neither an extra device nor an extra history row.
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
    // 50 devices x 80 messages at once in the queue: ingestion processes them in packets (coalesced per device and per
    // second) and the database ends on the LAST state of each; the queue empties in a few seconds at most.
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
    // 4000 messages received, but the history is per second: at most one row per device and per second.
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
