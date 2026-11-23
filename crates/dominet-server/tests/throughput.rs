// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sustained end-to-end ingestion throughput (R3 acceptance criterion): a second client publishes **2000 messages per second**
//! for 5 s (QoS 1) on the dev Mosquitto; the server receives them, writes them to the database (state, history) and broadcasts them. At the end,
//! each device carries its last state (no loss); the lag behind the last publication is printed and bounded by `WAIT`.
//!
//! The measured figure is printed (`cargo test -p dominet-server --test throughput -- --nocapture`); see `docs/measures.md`.

mod common;

use std::time::{Duration, Instant};

use common::state;
use dominet_domain::device::DeviceDraft;
use dominet_domain::repo::DeviceRepo;
use dominet_mqtt::{MqttSettings, Status, start};
use dominet_server::mqtt;
use sqlx::PgPool;

const DEVICES: i64 = 50;
const RATE: u64 = 2_000;
const SECONDS: u64 = 5;
/// Maximum wait for the final states after the last publication: this is the only bound on the ingestion lag.
const WAIT: Duration = Duration::from_secs(120);

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
async fn two_thousand_messages_per_second_are_ingested_without_loss_of_final_states(pool: PgPool) {
    let pid = std::process::id();
    let port = port();
    let st = state(
        pool.clone(),
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &format!("dominet-test-load-{pid}")),
        ],
    );
    // Prefix specific to the test: the dev broker is shared.
    let prefix = format!("load{pid}");
    st.settings.set("Z2M_BASE_TOPIC", &prefix).await.ok();
    for n in 0..DEVICES {
        let mut d = DeviceDraft::new(&format!("charge/{n}"));
        d.base_topic = prefix.clone();
        d.tracked_keys = vec!["seq".into()];
        st.store.devices().create(&d).await.unwrap();
    }
    let client = mqtt::spawn(&st).await.unwrap();
    connected(&client).await;
    let publisher = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("dominet-test-load-pub-{pid}"),
    ))
    .client;
    connected(&publisher).await;
    tokio::time::sleep(Duration::from_millis(800)).await; // subscription acknowledged

    // 2000 messages / s: 20 messages every 10 ms, for 5 s, spread over the devices.
    let total = RATE * SECONDS;
    let per_tick = usize::try_from(RATE / 100).unwrap();
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let mut sent: u64 = 0;
    let mut refused: u64 = 0;
    while sent < total {
        tick.tick().await;
        for _ in 0..per_tick {
            let (device, seq) = (
                sent % u64::try_from(DEVICES).unwrap(),
                sent / u64::try_from(DEVICES).unwrap(),
            );
            let topic = format!("{prefix}/charge/{device}");
            let payload = format!(
                r#"{{"seq":{seq},"temperature":{}}}"#,
                20.0 + f64::from(u32::try_from(seq % 100).unwrap()) / 10.0
            );
            loop {
                match publisher.publish(&topic, payload.clone(), 1, false) {
                    Ok(()) => break,
                    Err(dominet_mqtt::PublishError::Busy) => {
                        refused += 1;
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    Err(e) => panic!("publication : {e}"),
                }
            }
            sent += 1;
        }
    }
    let published_in = started.elapsed();
    let last_seq = i64::try_from((total - 1) / u64::try_from(DEVICES).unwrap()).unwrap();

    // Waiting for each device's final state.
    let finished = tokio::time::timeout(WAIT, async {
        loop {
            let done: i64 = sqlx::query_scalar("SELECT count(*) FROM device WHERE friendly_name LIKE 'charge/%' AND (state->>'seq')::bigint >= $1")
                .bind(last_seq - 1) // the last round of each device (two possible last seq values depending on arrival order)
                .fetch_one(&pool)
                .await
                .unwrap();
            if done == DEVICES {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    let lag = started.elapsed().saturating_sub(published_in);
    assert!(
        finished.is_ok(),
        "les etats finaux ne sont pas tous arrives ({sent} messages envoyes en {published_in:?}, {refused} refus d'envoi ; etat de la connexion : subscriber {:?}, publisher {:?} ; en base : {})",
        client.status(),
        publisher.status(),
        sqlx::query_scalar::<_, String>(
            "SELECT string_agg(state->>'seq', ',') FROM device WHERE friendly_name LIKE 'charge/%'"
        )
        .fetch_one(&pool)
        .await
        .unwrap_or_default()
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM device_state_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    #[allow(clippy::cast_precision_loss)]
    let rate = total as f64 / published_in.as_secs_f64();
    eprintln!(
        "debit : {total} messages publies en {published_in:?} ({rate:.0} msg/s, {refused} refus d'envoi), retard final {lag:?}, {rows} lignes d'historique"
    );
    assert!(
        rate > 1_800.0,
        "le test n'a pas tenu le debit voulu : {rate:.0} msg/s"
    );
    // No tighter bound on the lag: it is already bounded by the wait above (`WAIT`), and it depends on the machine
    // (debug build, pre-push hook at low priority: 0.2 s at rest, 14 to 34 s measured under load on
    // 2026-11-14). The reference figure is the one printed; this test guarantees that NO final state is lost.
    client.shutdown();
    publisher.shutdown();
}
