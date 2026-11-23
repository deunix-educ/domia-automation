// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side energy (R4 lot 7): `seed-energy`, the 00:05 job (`compute_daily_energy`), the `/api/energy/` JSON API and the page.

mod common;

use chrono::{DateTime, Utc};
use common::state;
use dominet_automation::energy::EnergyDevices;
use dominet_automation::scheduler::{TASK_DAILY_ENERGY, TaskRunner};
use dominet_domain::device::{DeviceDraft, DeviceType, LogEntry};
use dominet_domain::energy::EnergyDay;
use dominet_domain::repo::{DeviceRepo, EnergyRepo, StateLogRepo};
use dominet_domain::schedule::{Job, JobDraft, JobId, JobSchedule};
use dominet_server::commands::seed_energy_data;
use dominet_server::scheduler::ServerRunner;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn seed_energy_adds_the_missing_keys_without_removing_any_and_reports_missing_devices(
    pool: PgPool,
) {
    let st = state(pool, &[]);
    let mut linky = DeviceDraft::new("consommation");
    linky.device_type = DeviceType::Sensor;
    linky.tracked_keys = vec!["PAPP".into(), "HCHP".into()];
    st.store.devices().create(&linky).await.unwrap();
    st.store
        .devices()
        .create(&DeviceDraft::new("PAC-CE"))
        .await
        .unwrap();
    let names = EnergyDevices::default();
    // Simulation: nothing is written.
    let dry = seed_energy_data(&st.store, &names, true).await.unwrap();
    assert_eq!(
        dry,
        [
            ("consommation".to_string(), Some(vec!["HCHC".to_string()])),
            ("prod".to_string(), None),
            (
                "PAC-CE".to_string(),
                Some(vec![
                    "energy_a".to_string(),
                    "energy_b".to_string(),
                    "power_ab".to_string()
                ])
            ),
        ]
    );
    let linky = st
        .store
        .devices()
        .find_by_name("consommation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(linky.tracked_keys, ["PAPP", "HCHP"]);
    // Write: existing keys stay, missing ones are added; then idempotent.
    seed_energy_data(&st.store, &names, false).await.unwrap();
    let linky = st
        .store
        .devices()
        .find_by_name("consommation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(linky.tracked_keys, ["PAPP", "HCHP", "HCHC"]);
    let again = seed_energy_data(&st.store, &names, false).await.unwrap();
    assert_eq!(again[0], ("consommation".to_string(), Some(vec![])));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_nightly_job_computes_yesterday_or_the_requested_day_and_replaces_it(pool: PgPool) {
    let st = state(pool, &[]);
    let id = st
        .store
        .devices()
        .create(&DeviceDraft::new("prod"))
        .await
        .unwrap()
        .id;
    let ts: DateTime<Utc> = "2026-06-10T10:00:00Z".parse().unwrap();
    for (hour, qdy) in [(8, 1.5), (14, 9.25)] {
        st.store
            .state_log()
            .record(
                id,
                ts.with_timezone(&Utc) + chrono::Duration::hours(hour - 10),
                &[LogEntry {
                    key: "qdy".into(),
                    value_num: Some(qdy),
                    value_str: String::new(),
                }],
            )
            .await
            .unwrap();
    }
    let runner = ServerRunner::new(&st);
    let job = |args: serde_json::Value| Job {
        id: JobId(1),
        draft: JobDraft {
            name: "compute_daily_energy".into(),
            task: TASK_DAILY_ENERGY.into(),
            args,
            schedule: JobSchedule::Cron("5 0 * * *".into()),
            timezone: "Europe/Paris".into(),
            enabled: true,
            automation: None,
        },
        last_run_at: None,
        next_run_at: None,
        last_status: String::new(),
    };
    let done = runner
        .run(&job(json!({"date": "2026-06-10"})))
        .await
        .unwrap();
    assert!(
        done.starts_with("balance of 2026-06-10: solar 9250 Wh"),
        "{done}"
    );
    let day = "2026-06-10".parse().unwrap();
    assert_eq!(
        st.store.energy().get(day).await.unwrap(),
        Some(EnergyDay {
            day,
            linky_hp_wh: 0,
            linky_hc_wh: 0,
            solar_wh: 9250,
            pac_wh: 0
        })
    );
    // Without a date: yesterday (local day); an unreadable date is a clear error.
    assert!(
        runner
            .run(&job(json!({})))
            .await
            .unwrap()
            .starts_with("balance of ")
    );
    assert!(
        runner
            .run(&job(json!({"date": "hier"})))
            .await
            .unwrap_err()
            .contains("invalid date")
    );
    // An unknown job fails cleanly.
    let mut other = job(json!({}));
    other.draft.task = "inconnue".into();
    assert!(
        runner
            .run(&other)
            .await
            .unwrap_err()
            .contains("unknown task")
    );
}
