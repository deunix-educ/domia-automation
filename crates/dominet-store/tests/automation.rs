// SPDX-License-Identifier: AGPL-3.0-or-later
//! Schema (migration 0009) and repositories of scenes, automations, alerts and the energy summary (R4). The scenarios come from the
//! `context.yaml` of the reference production (test data, no import). Each test has its isolated database (`make dev-up`).

use std::collections::BTreeMap;

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use dominet_domain::RepoError;
use dominet_domain::alert::{AlertRuleDraft, AlertStatus, Operator, Severity};
use dominet_domain::automation::{
    Action, AutomationDraft, AutomationId, AutomationRun, RunStatus, Trigger,
};
use dominet_domain::dashboard::{DashboardDraft, WidgetDraft, WidgetType};
use dominet_domain::device::{DeviceDraft, DeviceId};
use dominet_domain::energy::EnergyDay;
use dominet_domain::repo::{
    AlertRepo, AlertRuleRepo, AutomationRepo, AutomationRunRepo, DashboardRepo, DeviceRepo,
    EnergyRepo, NewAlert, SceneRepo, UserRepo, WidgetRepo,
};
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_domain::scene::{SceneActionDraft, SceneDraft, SceneId};
use dominet_domain::user::{NewUser, Role, Username};
use dominet_store::PgStore;
use serde_json::json;
use sqlx::PgPool;

async fn device(store: &PgStore, name: &str) -> DeviceId {
    store
        .devices()
        .create(&DeviceDraft::new(name))
        .await
        .unwrap()
        .id
}

fn scene(name: &str, actions: &[(DeviceId, serde_json::Value)]) -> SceneDraft {
    let mut s = SceneDraft::new(name);
    s.actions = actions
        .iter()
        .map(|(device, payload)| SceneActionDraft {
            device: *device,
            payload: payload.clone(),
        })
        .collect();
    s
}

// ------------------------------------------------------------------ scenes

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn scenes_keep_their_actions_in_order_and_replace_them_on_update(pool: PgPool) {
    let store = PgStore::new(pool);
    let (a, b, c) = (
        device(&store, "A").await,
        device(&store, "B").await,
        device(&store, "C").await,
    );
    let repo = store.scenes();
    let created = repo
        .create(&scene(
            "Soir",
            &[
                (b, json!({"state": "CLOSE"})),
                (a, json!({"state": "OFF"})),
                (c, json!({"state": "ON"})),
            ],
        ))
        .await
        .unwrap();
    // The order is that of the list, not that of the device ids.
    assert_eq!(
        created.actions.iter().map(|x| x.device).collect::<Vec<_>>(),
        [b, a, c]
    );
    assert_eq!(
        created
            .actions
            .iter()
            .map(|x| x.position)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(repo.get(created.id).await.unwrap().unwrap(), created);
    assert_eq!(
        repo.find_by_name("Soir").await.unwrap().unwrap().id,
        created.id
    );
    assert!(
        repo.find_by_name("soir").await.unwrap().is_none(),
        "sensible a la casse"
    );

    let mut d = scene("Soir", &[(c, json!({"state": "OFF"}))]);
    d.icon = "🌙".into();
    d.active = false;
    let updated = repo.update(created.id, &d).await.unwrap();
    assert_eq!(
        (updated.icon.as_str(), updated.active, updated.actions.len()),
        ("🌙", false, 1)
    );
    assert_eq!(updated.actions[0].device, c);
    assert!(matches!(
        repo.update(SceneId(999), &d).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn scene_names_are_unique_and_actions_need_existing_devices(pool: PgPool) {
    let store = PgStore::new(pool);
    let a = device(&store, "A").await;
    let repo = store.scenes();
    repo.create(&scene("Soir", &[(a, json!({"state": "ON"}))]))
        .await
        .unwrap();
    assert!(matches!(
        repo.create(&scene("Soir", &[])).await,
        Err(RepoError::Conflict { .. })
    ));
    let before = repo.list().await.unwrap().len();
    assert!(matches!(
        repo.create(&scene("Autre", &[(DeviceId(999), json!({"state": "ON"}))]))
            .await,
        Err(RepoError::Conflict { .. })
    ));
    assert_eq!(
        repo.list().await.unwrap().len(),
        before,
        "la transaction annulee ne laisse pas de scene a moitie creee"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deleting_a_device_removes_its_scene_actions_and_deleting_a_scene_unlinks_its_widgets(
    pool: PgPool,
) {
    let store = PgStore::new(pool);
    let (a, b) = (device(&store, "A").await, device(&store, "B").await);
    let s = store
        .scenes()
        .create(&scene(
            "S",
            &[(a, json!({"state": "ON"})), (b, json!({"state": "ON"}))],
        ))
        .await
        .unwrap();
    store.devices().delete(a).await.unwrap();
    assert_eq!(
        store
            .scenes()
            .get(s.id)
            .await
            .unwrap()
            .unwrap()
            .actions
            .len(),
        1
    );

    let owner = store
        .users()
        .create(&NewUser {
            username: Username::parse("alice").unwrap(),
            email: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role: Role::Admin,
            is_active: true,
            password_hash: "x".into(),
        })
        .await
        .unwrap();
    let board = store
        .dashboards()
        .create(
            owner.id,
            &DashboardDraft {
                name: "T".into(),
                is_default: false,
            },
        )
        .await
        .unwrap();
    let mut w = WidgetDraft::new(WidgetType::Scene);
    w.scene_id = Some(s.id.0);
    let widget = store.widgets().create(board.id, &w).await.unwrap();
    let mut orphan = WidgetDraft::new(WidgetType::Scene);
    orphan.scene_id = Some(424_242);
    orphan.rect.col = 6;
    assert!(
        matches!(
            store.widgets().create(board.id, &orphan).await,
            Err(RepoError::Conflict { .. })
        ),
        "plus de numero fictif"
    );
    store.scenes().delete(s.id).await.unwrap();
    assert_eq!(
        store
            .widgets()
            .get(widget.id)
            .await
            .unwrap()
            .unwrap()
            .scene_id,
        None
    );
    assert!(matches!(
        store.scenes().delete(s.id).await,
        Err(RepoError::NotFound)
    ));
}

// ------------------------------------------------------------------ automations

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_scenes_and_automations_are_stored_and_read_back_unchanged(pool: PgPool) {
    let store = PgStore::new(pool);
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let mut ids: BTreeMap<String, DeviceId> = BTreeMap::new();
    for d in &ctx.devices {
        let mut draft = DeviceDraft::new(&d.name);
        draft.device_type = d.device_type;
        draft.base_topic = d.base_topic.clone();
        ids.insert(
            d.name.clone(),
            store.devices().create(&draft).await.unwrap().id,
        );
    }
    let mut local: BTreeMap<i64, SceneId> = BTreeMap::new();
    for sc in &ctx.scenes {
        let mut draft = SceneDraft::new(&sc.name);
        draft.icon = sc.icon.clone();
        draft.actions = sc
            .actions
            .iter()
            .map(|(name, payload)| SceneActionDraft {
                device: ids[name],
                payload: payload.clone(),
            })
            .collect();
        assert!(draft.problems().is_empty(), "{}", sc.name);
        local.insert(sc.pk, store.scenes().create(&draft).await.unwrap().id);
    }
    // The production's three scenes keep their 6 / 4 / 4 actions, in order and towards the right command topics.
    let scenes = store.scenes().list().await.unwrap();
    assert_eq!(scenes.iter().map(|s| s.actions.len()).sum::<usize>(), 14);
    let evening = store
        .scenes()
        .find_by_name("Fermeture du soir")
        .await
        .unwrap()
        .unwrap();
    let topics: Vec<String> = {
        let mut out = vec![];
        for a in &evening.actions {
            out.push(
                store
                    .devices()
                    .get(a.device)
                    .await
                    .unwrap()
                    .unwrap()
                    .command_topic(),
            );
        }
        out
    };
    assert_eq!(topics[0], "zigbee2mqtt/Relais-lora-1/set");
    assert_eq!(topics[3], "zigbee2mqtt/Relais-SB-est/set");
    assert_eq!(evening.actions[4].payload, json!({"state": "OFF"}));

    let mut stored = vec![];
    for a in &ctx.automations {
        let draft = a.draft(&local).unwrap();
        let created = store.automations().create(&draft).await.unwrap();
        assert_eq!(created.draft, draft, "{}", a.name);
        assert_eq!(created.trigger_count, 0);
        stored.push(created);
    }
    let all = store.automations().list().await.unwrap();
    assert_eq!(all.len(), 7);
    // The two solar automations target the scenes of the test database, not the production's numbers.
    let solar: Vec<_> = all
        .iter()
        .filter(|a| a.draft.trigger.kind() == "solar")
        .collect();
    assert_eq!(solar.len(), 2);
    assert!(solar.iter().all(
        |a| matches!(&a.draft.action, Action::Scene { scene } if local.values().any(|l| l == scene))
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn automations_are_updated_counted_and_deleted_and_their_json_stays_reference_shaped(
    pool: PgPool,
) {
    let store = PgStore::new(pool.clone());
    let repo = store.automations();
    let trigger = Trigger::DeviceState {
        friendly_name: "Tele-commande".into(),
        key: "action".into(),
        value: json!("1_single"),
    };
    let action = Action::MqttPublish {
        topic: "zigbee2mqtt/Volet-salon/set".into(),
        payload: json!({"state": "OPEN"}),
    };
    let created = repo
        .create(&AutomationDraft::new("TC", trigger, action))
        .await
        .unwrap();
    let (trigger_type, trigger_config, condition): (String, serde_json::Value, serde_json::Value) =
        sqlx::query_as(
            "SELECT trigger_type, trigger_config, condition FROM automation WHERE id = $1",
        )
        .bind(created.id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(trigger_type, "device_state");
    assert_eq!(
        trigger_config,
        json!({"friendly_name": "Tele-commande", "key": "action", "value": "1_single"})
    );
    assert_eq!(condition, json!({}));

    let at = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
    repo.record_trigger(created.id, at).await.unwrap();
    repo.record_trigger(created.id, at + Duration::seconds(5))
        .await
        .unwrap();
    let counted = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(
        (counted.trigger_count, counted.last_triggered),
        (2, Some(at + Duration::seconds(5)))
    );

    let mut changed = counted.draft.clone();
    changed.enabled = false;
    changed.name = "TC bis".into();
    let updated = repo.update(created.id, &changed).await.unwrap();
    assert_eq!(
        (
            updated.draft.name.as_str(),
            updated.draft.enabled,
            updated.trigger_count
        ),
        ("TC bis", false, 2)
    );
    assert_eq!(repo.list_enabled().await.unwrap(), vec![]);
    assert!(matches!(
        repo.update(AutomationId(999), &changed).await,
        Err(RepoError::NotFound)
    ));
    repo.delete(created.id).await.unwrap();
    assert!(repo.get(created.id).await.unwrap().is_none());
    assert!(matches!(
        repo.delete(created.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_run_journal_is_recent_first_cascades_with_the_automation_and_is_purged_by_age(
    pool: PgPool,
) {
    let store = PgStore::new(pool);
    let a = store
        .automations()
        .create(&AutomationDraft::new(
            "A",
            Trigger::Schedule {
                cron: "* * * * *".into(),
            },
            Action::Scene { scene: SceneId(1) },
        ))
        .await
        .unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap();
    for (i, status) in [RunStatus::Ok, RunStatus::Error, RunStatus::Limited]
        .into_iter()
        .enumerate()
    {
        store
            .automation_runs()
            .record(&AutomationRun {
                automation: a.id,
                at: t0 + Duration::minutes(i64::try_from(i).unwrap()),
                trigger: "schedule".into(),
                status,
                duration_ms: 12,
                message: "x".repeat(1500),
            })
            .await
            .unwrap();
    }
    let recent = store.automation_runs().recent(a.id, 2).await.unwrap();
    assert_eq!(
        recent.iter().map(|r| r.status).collect::<Vec<_>>(),
        [RunStatus::Limited, RunStatus::Error]
    );
    assert_eq!(recent[0].message.chars().count(), 1000, "message borne");
    assert_eq!(
        store
            .automation_runs()
            .purge_before(t0 + Duration::minutes(1))
            .await
            .unwrap(),
        1
    );
    store.automations().delete(a.id).await.unwrap();
    assert_eq!(
        store.automation_runs().recent(a.id, 10).await.unwrap(),
        vec![]
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_scheduler_job_belongs_to_at_most_one_automation_and_all_nine_solar_events_are_allowed(
    pool: PgPool,
) {
    let store = PgStore::new(pool.clone());
    let a = store
        .automations()
        .create(&AutomationDraft::new(
            "A",
            Trigger::Schedule {
                cron: "* * * * *".into(),
            },
            Action::Scene { scene: SceneId(1) },
        ))
        .await
        .unwrap();
    for event in dominet_domain::automation::SolarEvent::ALL {
        sqlx::query("INSERT INTO schedule_job (name, task, kind, solar_event, latitude, longitude) VALUES ($1, 'run_automation', 'solar', $2, 43.3, 5.4)")
            .bind(format!("job-{}", event.as_str()))
            .bind(event.as_str())
            .execute(&pool)
            .await
            .unwrap();
    }
    assert!(sqlx::query("INSERT INTO schedule_job (name, task, kind, solar_event, latitude, longitude) VALUES ('x', 't', 'solar', 'midi', 1, 1)")
        .execute(&pool)
        .await
        .is_err());
    sqlx::query("UPDATE schedule_job SET automation_id = $1 WHERE name = 'job-sunset'")
        .bind(a.id.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE schedule_job SET automation_id = $1 WHERE name = 'job-sunrise'")
            .bind(a.id.0)
            .execute(&pool)
            .await
            .is_err()
    );
    store.automations().delete(a.id).await.unwrap();
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM schedule_job WHERE name = 'job-sunset'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        left, 0,
        "le travail du planificateur disparait avec son automation"
    );
}

// ------------------------------------------------------------------ alertes

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn alert_rules_round_trip_and_the_production_rule_is_valid(pool: PgPool) {
    let store = PgStore::new(pool);
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let meteo = device(&store, "meteo_locale").await;
    let draft = ctx.alert_rules[0].draft(meteo);
    let rule = store.alert_rules().create(&draft).await.unwrap();
    assert_eq!(rule.rule, draft);
    assert_eq!(
        (
            rule.rule.operator,
            rule.rule.severity,
            rule.rule.cooldown_minutes
        ),
        (Operator::Gt, Severity::Warning, 60)
    );
    let off = {
        let mut d = draft.clone();
        d.name = "autre".into();
        d.enabled = false;
        store.alert_rules().create(&d).await.unwrap()
    };
    assert_eq!(
        store
            .alert_rules()
            .list_enabled_for_device(meteo)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.alert_rules().list().await.unwrap().len(), 2);
    let mut changed = draft.clone();
    changed.threshold_num = Some(38.0);
    assert_eq!(
        store
            .alert_rules()
            .update(rule.id, &changed)
            .await
            .unwrap()
            .rule
            .threshold_num,
        Some(38.0)
    );
    let unknown = AlertRuleDraft::new("x", DeviceId(999), "temperature");
    assert!(matches!(
        store.alert_rules().create(&unknown).await,
        Err(RepoError::Conflict { .. })
    ));
    store.alert_rules().delete(off.id).await.unwrap();
    assert!(matches!(
        store.alert_rules().delete(off.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn alerts_fire_resolve_and_are_acknowledged_one_by_one_or_all(pool: PgPool) {
    let store = PgStore::new(pool);
    let dev = device(&store, "meteo_locale").await;
    let rule = store
        .alert_rules()
        .create(&AlertRuleDraft::new("Canicule", dev, "temperature"))
        .await
        .unwrap();
    let user = store
        .users()
        .create(&NewUser {
            username: Username::parse("bob").unwrap(),
            email: String::new(),
            first_name: String::new(),
            last_name: String::new(),
            mobile: String::new(),
            role: Role::User,
            is_active: true,
            password_hash: "x".into(),
        })
        .await
        .unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 7, 1, 14, 0, 0).unwrap();
    let new = |minutes: i64, value: &str| NewAlert {
        rule: rule.id,
        device: dev,
        severity: Severity::Warning,
        value_triggered: value.into(),
        fired_at: t0 + Duration::minutes(minutes),
    };
    let first = store.alerts().create(&new(0, "36.2")).await.unwrap();
    assert_eq!((first.status, first.notified), (AlertStatus::Firing, false));
    assert_eq!(
        store
            .alerts()
            .latest_firing(rule.id, dev)
            .await
            .unwrap()
            .unwrap()
            .id,
        first.id
    );
    assert_eq!(store.alerts().count_firing().await.unwrap(), 1);

    // Condition cleared: ongoing alerts are resolved; a resolved one is no longer "ongoing".
    assert_eq!(
        store
            .alerts()
            .resolve_firing(rule.id, dev, t0 + Duration::minutes(5))
            .await
            .unwrap(),
        1
    );
    assert!(
        store
            .alerts()
            .latest_firing(rule.id, dev)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .alerts()
            .resolve_firing(rule.id, dev, t0)
            .await
            .unwrap(),
        0
    );

    // Acknowledgement: only an ongoing alert, and an acknowledged one leaves "ongoing" (a quirk of the reference, ADR 0031).
    let second = store.alerts().create(&new(10, "37")).await.unwrap();
    let third = store.alerts().create(&new(11, "38")).await.unwrap();
    let acked = store
        .alerts()
        .acknowledge(second.id, user.id, t0 + Duration::minutes(12))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (acked.status, acked.acknowledged_by),
        (AlertStatus::Acknowledged, Some(user.id))
    );
    assert!(
        store
            .alerts()
            .acknowledge(second.id, user.id, t0)
            .await
            .unwrap()
            .is_none(),
        "deja acquittee"
    );
    assert!(
        store
            .alerts()
            .acknowledge(first.id, user.id, t0)
            .await
            .unwrap()
            .is_none(),
        "resolue : non acquittable"
    );
    assert_eq!(
        store
            .alerts()
            .latest_firing(rule.id, dev)
            .await
            .unwrap()
            .unwrap()
            .id,
        third.id
    );
    assert_eq!(
        store
            .alerts()
            .acknowledge_all(user.id, t0 + Duration::minutes(13))
            .await
            .unwrap(),
        1
    );
    assert_eq!(store.alerts().count_firing().await.unwrap(), 0);

    // Page filters: by status, most recent first; `notified` is set separately.
    let alerts = store.alerts();
    let by = |s| alerts.list(s, 10);
    assert_eq!(by(None).await.unwrap().len(), 3);
    assert_eq!(
        by(Some(AlertStatus::Acknowledged))
            .await
            .unwrap()
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>(),
        [third.id, second.id]
    );
    assert_eq!(by(Some(AlertStatus::Resolved)).await.unwrap().len(), 1);
    store.alerts().mark_notified(first.id).await.unwrap();
    assert!(by(Some(AlertStatus::Resolved)).await.unwrap()[0].notified);

    // Deleting the rule keeps the history, without a rule; deleting the device takes its alerts with it.
    store.alert_rules().delete(rule.id).await.unwrap();
    assert!(by(None).await.unwrap().iter().all(|a| a.rule.is_none()));
    store.devices().delete(dev).await.unwrap();
    assert_eq!(by(None).await.unwrap(), vec![]);
}

// ------------------------------------------------------------------ energie

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_daily_balance_is_replaced_per_day_and_read_back_oldest_first(pool: PgPool) {
    let store = PgStore::new(pool);
    let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
    for d in 1..=5 {
        store
            .energy()
            .upsert(&EnergyDay {
                day: day(d),
                linky_hp_wh: 1000 * i32::try_from(d).unwrap(),
                linky_hc_wh: 500,
                solar_wh: 2000,
                pac_wh: 100,
            })
            .await
            .unwrap();
    }
    store
        .energy()
        .upsert(&EnergyDay {
            day: day(3),
            linky_hp_wh: 1,
            linky_hc_wh: 2,
            solar_wh: 3,
            pac_wh: 4,
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .energy()
            .get(day(3))
            .await
            .unwrap()
            .unwrap()
            .linky_total_wh(),
        3
    );
    assert!(store.energy().get(day(9)).await.unwrap().is_none());
    // The last three recorded days, from oldest to most recent (the reference returned the oldest: a defect).
    let recent = store.energy().recent(3).await.unwrap();
    assert_eq!(
        recent.iter().map(|d| d.day).collect::<Vec<_>>(),
        [day(3), day(4), day(5)]
    );
    assert_eq!(store.energy().recent(100).await.unwrap().len(), 5);
}
