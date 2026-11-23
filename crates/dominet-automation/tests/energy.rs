// SPDX-License-Identifier: AGPL-3.0-or-later
//! Energy summary against the **reference implementation**: the real `compute_daily_energy` run on a
//! 327-row history (Linky and heat pump meters, SolarMax's `qdy`, text values, reset, clock-change nights,
//! local-day boundaries, a day without data) must give, day by day, the same Wh.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Europe::Paris;
use dominet_automation::energy::{
    EnergyDevices, compute, compute_and_store, day_window, yesterday,
};
use dominet_domain::device::{DeviceDraft, LogEntry};
use dominet_domain::repo::{DeviceRepo, EnergyRepo, StateLogRepo};
use dominet_store::PgStore;
use serde_json::Value;
use sqlx::PgPool;

const SESSION: &str = include_str!("../../../tests/fixtures/energy/session.json");
const GOLDEN: &str = include_str!("../../../tests/fixtures/energy/session.golden.json");

async fn load(store: &PgStore) {
    let doc: Value = serde_json::from_str(SESSION).unwrap();
    let mut ids = std::collections::HashMap::new();
    for name in doc["devices"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        ids.insert(
            name.to_string(),
            store
                .devices()
                .create(&DeviceDraft::new(name))
                .await
                .unwrap()
                .id,
        );
    }
    for r in doc["logs"].as_array().unwrap() {
        let ts: DateTime<Utc> = r["ts"].as_str().unwrap().parse().unwrap();
        store
            .state_log()
            .record(
                ids[r["device"].as_str().unwrap()],
                ts,
                &[LogEntry {
                    key: r["key"].as_str().unwrap().to_string(),
                    value_num: r["value_num"].as_f64(),
                    value_str: r["value_str"].as_str().unwrap().to_string(),
                }],
            )
            .await
            .unwrap();
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn every_day_of_the_history_gives_the_same_watt_hours_as_reference(pool: PgPool) {
    let store = PgStore::new(pool);
    load(&store).await;
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let days = golden["days"].as_array().unwrap();
    assert_eq!(days.len(), 13);
    for d in days {
        let day: NaiveDate = d["date"].as_str().unwrap().parse().unwrap();
        let got = compute_and_store(
            &store.devices(),
            &store.state_log(),
            &store.energy(),
            &EnergyDevices::default(),
            Paris,
            day,
        )
        .await
        .unwrap();
        let want = |k: &str| i32::try_from(d[k].as_i64().unwrap()).unwrap();
        assert_eq!(
            (
                got.linky_hp_wh,
                got.linky_hc_wh,
                got.solar_wh,
                got.pac_wh,
                got.net_wh()
            ),
            (
                want("linky_hp_wh"),
                want("linky_hc_wh"),
                want("solar_wh"),
                want("pac_wh"),
                want("net_wh")
            ),
            "{day}"
        );
        assert_eq!(
            store.energy().get(day).await.unwrap(),
            Some(got),
            "enregistre"
        );
    }
    // Recomputing overwrites (upsert): still thirteen days.
    assert_eq!(store.energy().recent(365).await.unwrap().len(), 13);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_missing_device_counts_zero_and_the_local_day_has_the_right_length(pool: PgPool) {
    let store = PgStore::new(pool);
    let day = "2026-03-29".parse().unwrap();
    let got = compute(
        &store.devices(),
        &store.state_log(),
        &EnergyDevices::default(),
        Paris,
        day,
    )
    .await
    .unwrap();
    assert_eq!(
        (got.linky_hp_wh, got.linky_hc_wh, got.solar_wh, got.pac_wh),
        (0, 0, 0, 0)
    );
    // 29 March 2026: 23 h (switch to summer time); 25 October: 25 h; an ordinary day: 24 h.
    for (day, hours) in [("2026-03-29", 23), ("2026-10-25", 25), ("2026-11-03", 24)] {
        let (from, to) = day_window(Paris, day.parse().unwrap()).unwrap();
        assert_eq!((to - from).num_hours(), hours, "{day}");
    }
    // "Yesterday" is yesterday's local day, not the UTC one.
    let now: DateTime<Utc> = "2026-11-03T23:30:00Z".parse().unwrap(); // 00:30 on the 4th in Paris
    assert_eq!(yesterday(Paris, now).to_string(), "2026-11-03");
}
