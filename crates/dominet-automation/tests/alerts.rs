// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alert cycle against the **reference implementation**: triggering, cooldown, resolution,
//! acknowledgement (which takes the alert out of the cooldown computation), absent or null values, text, quick toggling. Each series of states
//! replayed on a throw-away database with a controlled clock must leave the same alerts as the reference after each step.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use dominet_automation::alerts::{AlertEngine, AlertEvent, AlertNotice, Notifier};
use dominet_domain::alert::{AlertRuleDraft, Operator, Severity};
use dominet_domain::device::{DeviceDraft, DeviceId};
use dominet_domain::repo::{AlertRepo, AlertRuleRepo, DeviceRepo};
use dominet_domain::user::UserId;
use dominet_store::{PgAlertRules, PgAlerts, PgStore};
use serde_json::{Value, json};
use sqlx::PgPool;

const FIXTURE: &str = include_str!("../../../tests/fixtures/automation/alerts.json");

#[derive(Default)]
struct Rec {
    fired: Mutex<Vec<AlertNotice>>,
    changed: Mutex<u32>,
}

struct Recorder(Arc<Rec>);

impl Notifier for Recorder {
    fn fired(&self, notice: &AlertNotice) {
        self.0.fired.lock().unwrap().push(notice.clone());
    }

    fn changed(&self) {
        *self.0.changed.lock().unwrap() += 1;
    }
}

type Engine = AlertEngine<PgAlertRules, PgAlerts, Recorder>;

fn start() -> DateTime<Utc> {
    "2026-01-01T10:00:00Z".parse().unwrap()
}

async fn rig(
    pool: PgPool,
) -> (
    Arc<Engine>,
    PgStore,
    Arc<Rec>,
    Arc<Mutex<DateTime<Utc>>>,
    DeviceId,
) {
    let store = PgStore::new(pool);
    let device = store
        .devices()
        .create(&DeviceDraft::new("meteo_locale"))
        .await
        .unwrap()
        .id;
    let clock = Arc::new(Mutex::new(start()));
    let rec = Arc::new(Rec::default());
    let c = Arc::clone(&clock);
    let (engine, _handle, _rx) = AlertEngine::with_clock(
        store.alert_rules(),
        store.alerts(),
        Recorder(Arc::clone(&rec)),
        Arc::new(move || *c.lock().unwrap()),
    );
    (engine, store, rec, clock, device)
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
        .replace('Z', "+00:00")
}

async fn snapshot(store: &PgStore) -> Value {
    let mut all = store.alerts().list(None, 1000).await.unwrap();
    all.sort_by_key(|a| a.id.0);
    Value::Array(
        all.into_iter()
            .map(|a| {
                json!({
                    "status": a.status.as_str(),
                    "fired_at": iso(a.fired_at),
                    "resolved_at": a.resolved_at.map(iso),
                    "acknowledged_at": a.acknowledged_at.map(iso),
                    "value": a.value_triggered,
                    "severity": a.severity.as_str(),
                })
            })
            .collect(),
    )
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn every_alert_scenario_leaves_the_same_alerts_as_reference_after_each_step(pool: PgPool) {
    let all: Value = serde_json::from_str(FIXTURE).unwrap();
    let scenarios = all["cycle"].as_array().unwrap();
    assert_eq!(scenarios.len(), 8);
    sqlx::query("INSERT INTO users (username, password_hash) VALUES ('ack', 'x')")
        .execute(&pool)
        .await
        .unwrap();
    let user: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'ack'")
        .fetch_one(&pool)
        .await
        .unwrap();
    for case in scenarios {
        let name = case["name"].as_str().unwrap();
        let (engine, store, _rec, clock, device) = rig(pool.clone()).await;
        let cfg = &case["rule"];
        let mut draft = AlertRuleDraft::new("R", device, cfg["key"].as_str().unwrap());
        draft.operator = Operator::parse(cfg["operator"].as_str().unwrap()).unwrap();
        draft.threshold_num = cfg["threshold_num"].as_f64();
        draft.threshold_str = cfg["threshold_str"].as_str().unwrap().to_string();
        draft.severity = Severity::parse(cfg["severity"].as_str().unwrap()).unwrap();
        draft.cooldown_minutes = i16::try_from(cfg["cooldown_minutes"].as_i64().unwrap()).unwrap();
        store.alert_rules().create(&draft).await.unwrap();
        engine.reload().await;
        for (i, ev) in case["events"].as_array().unwrap().iter().enumerate() {
            let now = start() + Duration::seconds(ev["at"].as_i64().unwrap());
            *clock.lock().unwrap() = now;
            match ev["kind"].as_str().unwrap() {
                "state" => {
                    engine
                        .process(&AlertEvent {
                            device,
                            name: "meteo_locale".into(),
                            state: ev["state"].clone(),
                        })
                        .await;
                }
                "ack" => {
                    store
                        .alerts()
                        .acknowledge_all(UserId(user), now)
                        .await
                        .unwrap();
                }
                other => panic!("{other}"),
            }
            assert_eq!(
                snapshot(&store).await,
                case["snapshots"][i],
                "{name} : apres l'etape {i} ({ev})"
            );
        }
        // Isolation between scenarios (same database: the device starts with its rules and its alerts).
        sqlx::query("DELETE FROM device")
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_fired_alert_is_announced_with_the_reference_texts_and_the_count_change_is_signalled(
    pool: PgPool,
) {
    let (engine, store, rec, clock, device) = rig(pool).await;
    let mut draft = AlertRuleDraft::new("Canicule", device, "temperature");
    draft.threshold_num = Some(35.0);
    draft.severity = Severity::Critical;
    store.alert_rules().create(&draft).await.unwrap();
    engine.reload().await;
    let state = |t: f64| AlertEvent {
        device,
        name: "meteo_locale".into(),
        state: json!({"temperature": t}),
    };
    engine.process(&state(20.0)).await;
    assert_eq!(*rec.changed.lock().unwrap(), 0, "rien ne bouge");
    *clock.lock().unwrap() = "2026-07-14T15:04:09Z".parse().unwrap();
    engine.process(&state(36.2)).await;
    engine.process(&state(37.0)).await; // cooldown: no second alert
    {
        let fired = rec.fired.lock().unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].short, "[DomiNet] Canicule — meteo_locale : 36.2");
        assert_eq!(
            fired[0].long,
            "[DomiNet Critical] Canicule\nDevice: meteo_locale\nValue: 36.2\nTriggered at: 14/07/2026 15:04:09"
        );
        assert!(
            !fired[0].alert.notified,
            "notified reste faux tant qu'un canal reel (R5) ne l'a pas envoyee"
        );
    }
    assert_eq!(*rec.changed.lock().unwrap(), 1);
    engine.process(&state(20.0)).await;
    assert_eq!(
        *rec.changed.lock().unwrap(),
        2,
        "la resolution est signalee"
    );
    engine.process(&state(21.0)).await;
    assert_eq!(
        *rec.changed.lock().unwrap(),
        2,
        "plus rien a resoudre : pas d'ecriture ni de signal"
    );
    assert_eq!(store.alerts().count_firing().await.unwrap(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn disabled_rules_and_other_devices_are_ignored_and_a_firing_alert_survives_a_restart(
    pool: PgPool,
) {
    let (engine, store, rec, _clock, device) = rig(pool.clone()).await;
    let other = store
        .devices()
        .create(&DeviceDraft::new("autre"))
        .await
        .unwrap()
        .id;
    let mut on = AlertRuleDraft::new("Active", device, "temperature");
    on.threshold_num = Some(10.0);
    let mut off = AlertRuleDraft::new("Eteinte", device, "temperature");
    off.threshold_num = Some(10.0);
    off.enabled = false;
    store.alert_rules().create(&on).await.unwrap();
    store.alert_rules().create(&off).await.unwrap();
    engine.reload().await;
    engine
        .process(&AlertEvent {
            device: other,
            name: "autre".into(),
            state: json!({"temperature": 50}),
        })
        .await;
    assert!(rec.fired.lock().unwrap().is_empty(), "autre appareil");
    engine
        .process(&AlertEvent {
            device,
            name: "meteo_locale".into(),
            state: json!({"temperature": 50}),
        })
        .await;
    assert_eq!(
        rec.fired.lock().unwrap().len(),
        1,
        "la regle eteinte ne declenche rien"
    );

    // Restart: a new engine must see the ongoing alert to resolve it when the value becomes normal again.
    let rec2 = Arc::new(Rec::default());
    let (engine2, _h, _rx) = AlertEngine::new(
        store.alert_rules(),
        store.alerts(),
        Recorder(Arc::clone(&rec2)),
    );
    engine2.reload().await;
    engine2.load_firing().await;
    engine2
        .process(&AlertEvent {
            device,
            name: "meteo_locale".into(),
            state: json!({"temperature": 1}),
        })
        .await;
    assert_eq!(store.alerts().count_firing().await.unwrap(), 0);
    assert_eq!(*rec2.changed.lock().unwrap(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_handle_only_queues_states_a_rule_is_watching(pool: PgPool) {
    let store = PgStore::new(pool);
    let device = store
        .devices()
        .create(&DeviceDraft::new("meteo_locale"))
        .await
        .unwrap()
        .id;
    let (engine, handle, mut rx) = AlertEngine::new(
        store.alert_rules(),
        store.alerts(),
        Recorder(Arc::new(Rec::default())),
    );
    handle.state(device, "meteo_locale", &json!({"temperature": 50}));
    assert!(rx.try_recv().is_err(), "aucune regle");
    let mut draft = AlertRuleDraft::new("R", device, "temperature");
    draft.threshold_num = Some(1.0);
    store.alert_rules().create(&draft).await.unwrap();
    engine.reload().await;
    handle.state(device, "meteo_locale", &json!({"humidity": 50}));
    handle.state(device, "meteo_locale", &json!({"temperature": null}));
    assert!(rx.try_recv().is_err(), "cle absente ou nulle");
    handle.state(device, "meteo_locale", &json!({"temperature": 5}));
    assert!(rx.try_recv().is_ok());
}
