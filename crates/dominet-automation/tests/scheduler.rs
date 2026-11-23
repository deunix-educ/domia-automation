// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scheduler on a throw-away database with a manual clock (`Scheduler::tick`): synchronisation with the automations,
//! on-time runs, no catch-up, failures, and the two solar automations of the **production**.

#![allow(clippy::unused_async_trait_impl)] // the simulated executor waits for nothing

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Europe::Paris;
use dominet_automation::scheduler::{JOB_DAILY_ENERGY, Scheduler, TASK_RUN_AUTOMATION, TaskRunner};
use dominet_domain::automation::{Action, AutomationDraft, AutomationId, SolarEvent, Trigger};
use dominet_domain::repo::{AutomationRepo, ScheduleRepo};
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_domain::scene::SceneId;
use dominet_domain::schedule::{Job, JobSchedule};
use dominet_store::{PgAutomations, PgSchedule, PgStore};
use serde_json::{Map, json};
use sqlx::PgPool;

const SCHEDULE_FIXTURE: &str = include_str!("../../../tests/fixtures/automation/schedule.json");

#[derive(Default)]
struct Ran(Mutex<Vec<(String, i64)>>);

struct Runner {
    ran: Arc<Ran>,
    fail: bool,
}

impl TaskRunner for Runner {
    async fn run(&self, job: &Job) -> Result<String, String> {
        let id = job.draft.args["automation_id"].as_i64().unwrap_or(0);
        self.ran
            .0
            .lock()
            .unwrap()
            .push((job.draft.task.clone(), id));
        if self.fail {
            Err("boum".into())
        } else {
            Ok("fait".into())
        }
    }
}

type Sched = Scheduler<PgSchedule, PgAutomations, Runner>;

fn rig(pool: PgPool, fail: bool) -> (Sched, Arc<Ran>, PgStore) {
    let store = PgStore::new(pool);
    let ran = Arc::new(Ran::default());
    let s = Scheduler::new(
        store.schedule(),
        store.automations(),
        Runner {
            ran: Arc::clone(&ran),
            fail,
        },
        Paris,
    );
    (s, ran, store)
}

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
}

fn cron_automation(name: &str, cron: &str) -> AutomationDraft {
    let mut d = AutomationDraft::new(
        name,
        Trigger::Schedule { cron: cron.into() },
        Action::Scene { scene: SceneId(1) },
    );
    d.enabled = true;
    d
}

async fn job_names(store: &PgStore) -> Vec<String> {
    store
        .schedule()
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|j| j.draft.name)
        .collect()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn jobs_follow_the_scheduled_automations_and_the_system_job_exists(pool: PgPool) {
    let (s, _ran, store) = rig(pool, false);
    s.sync().await;
    assert_eq!(job_names(&store).await, [JOB_DAILY_ENERGY]);
    let system = &store.schedule().list().await.unwrap()[0];
    assert_eq!(system.draft.schedule, JobSchedule::Cron("5 0 * * *".into()));
    assert_eq!(system.draft.timezone, "Europe/Paris");

    let cron = store
        .automations()
        .create(&cron_automation("Matin", "0 7 * * *"))
        .await
        .unwrap();
    let solar = store
        .automations()
        .create(&AutomationDraft::new(
            "Soir",
            Trigger::Solar {
                event: SolarEvent::Sunset,
                latitude: 43.3,
                longitude: 5.4,
            },
            Action::Scene { scene: SceneId(1) },
        ))
        .await
        .unwrap();
    let message = store
        .automations()
        .create(&AutomationDraft::new(
            "Message",
            Trigger::MqttMessage {
                topic: "a/#".into(),
                payload_filter: Map::new(),
            },
            Action::Scene { scene: SceneId(1) },
        ))
        .await
        .unwrap();
    s.sync().await;
    let mut names = job_names(&store).await;
    names.sort();
    let mut expected = vec![
        format!("automation-{}", cron.id.0),
        format!("automation-{}", solar.id.0),
        JOB_DAILY_ENERGY.to_string(),
    ];
    expected.sort();
    assert_eq!(
        names, expected,
        "pas de travail pour l'automation sur message {}",
        message.id.0
    );
    let job = store
        .schedule()
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.draft.automation == Some(cron.id))
        .unwrap();
    assert_eq!(job.draft.task, TASK_RUN_AUTOMATION);
    assert_eq!(job.draft.args, json!({"automation_id": cron.id.0}));

    // A changed definition forgets the next run; unchanged, it keeps it.
    s.tick(utc(2026, 11, 2, 12, 0, 0)).await;
    let next = |id: AutomationId| {
        let store = store.clone();
        async move {
            store
                .schedule()
                .automation_next_runs()
                .await
                .unwrap()
                .into_iter()
                .find(|(a, _)| *a == id)
                .and_then(|(_, t)| t)
        }
    };
    assert_eq!(next(cron.id).await, Some(utc(2026, 11, 3, 6, 0, 0)));
    s.sync().await;
    assert_eq!(
        next(cron.id).await,
        Some(utc(2026, 11, 3, 6, 0, 0)),
        "inchange : conserve"
    );
    store
        .automations()
        .update(cron.id, &cron_automation("Matin", "30 8 * * *"))
        .await
        .unwrap();
    s.sync().await;
    assert_eq!(next(cron.id).await, None, "horaire change : a recalculer");
    s.tick(utc(2026, 11, 2, 12, 0, 0)).await;
    assert_eq!(next(cron.id).await, Some(utc(2026, 11, 3, 7, 30, 0)));

    // Deactivated or deleted: the job disappears.
    let mut off = cron_automation("Matin", "30 8 * * *");
    off.enabled = false;
    store.automations().update(cron.id, &off).await.unwrap();
    store.automations().delete(solar.id).await.unwrap();
    s.sync().await;
    assert_eq!(job_names(&store).await, [JOB_DAILY_ENERGY]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_job_runs_once_at_its_time_not_before_and_not_again(pool: PgPool) {
    let (s, ran, store) = rig(pool, false);
    let a = store
        .automations()
        .create(&cron_automation("Matin", "0 7 * * *"))
        .await
        .unwrap();
    s.sync().await;
    // 12:00 UTC on 2 November 2026 = 13:00 in Paris: the next 07:00 is the next day (06:00 UTC).
    let t0 = utc(2026, 11, 2, 12, 0, 0);
    // The system job (00:05 local = 23:05 UTC) is the nearest.
    assert_eq!(s.tick(t0).await, Some(utc(2026, 11, 2, 23, 5, 0)));
    s.tick(utc(2026, 11, 3, 5, 59, 59)).await;
    assert!(ran.0.lock().unwrap().is_empty(), "pas avant l'heure");
    s.tick(utc(2026, 11, 3, 6, 0, 2)).await;
    assert_eq!(
        *ran.0.lock().unwrap(),
        [(TASK_RUN_AUTOMATION.to_string(), a.id.0)]
    );
    s.tick(utc(2026, 11, 3, 6, 0, 30)).await;
    assert_eq!(ran.0.lock().unwrap().len(), 1, "pas deux fois");
    let job = store
        .schedule()
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.draft.automation == Some(a.id))
        .unwrap();
    assert_eq!(job.last_run_at, Some(utc(2026, 11, 3, 6, 0, 2)));
    assert_eq!(job.last_status, "ok : fait");
    assert_eq!(job.next_run_at, Some(utc(2026, 11, 4, 6, 0, 0)));
    // The next day, once more.
    s.tick(utc(2026, 11, 4, 6, 0, 1)).await;
    assert_eq!(ran.0.lock().unwrap().len(), 2);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_missed_run_is_not_replayed_and_a_failure_is_recorded(pool: PgPool) {
    let (s, ran, store) = rig(pool, false);
    let a = store
        .automations()
        .create(&cron_automation("Matin", "0 7 * * *"))
        .await
        .unwrap();
    s.sync().await;
    s.tick(utc(2026, 11, 2, 12, 0, 0)).await;
    // The server was stopped: we come back 3 hours after the scheduled time.
    s.tick(utc(2026, 11, 3, 9, 0, 0)).await;
    assert!(ran.0.lock().unwrap().is_empty(), "pas de rattrapage");
    let job = store
        .schedule()
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.draft.automation == Some(a.id))
        .unwrap();
    assert!(job.last_status.starts_with("missed"), "{}", job.last_status);
    assert_eq!(job.last_run_at, None);
    assert_eq!(job.next_run_at, Some(utc(2026, 11, 4, 6, 0, 0)));

    // A task failure is recorded, and the following run stays scheduled.
    let (s, _ran, store2) = rig(store.pool().clone(), true);
    s.tick(utc(2026, 11, 4, 6, 0, 5)).await;
    let job = store2
        .schedule()
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.draft.automation == Some(a.id))
        .unwrap();
    assert_eq!(job.last_status, "failure: boum");
    assert_eq!(job.next_run_at, Some(utc(2026, 11, 5, 6, 0, 0)));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_solar_automations_fire_at_sunset_and_sunrise(pool: PgPool) {
    let (s, ran, store) = rig(pool, false);
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let scenes: BTreeMap<i64, SceneId> = BTreeMap::new();
    let mut ids = BTreeMap::new();
    for a in ctx.automations.iter().filter(|a| a.trigger_type == "solar") {
        ids.insert(
            a.name.clone(),
            store
                .automations()
                .create(&a.draft(&scenes).unwrap())
                .await
                .unwrap()
                .id,
        );
    }
    assert_eq!(ids.len(), 2);
    s.sync().await;
    // `ephem` values for Marseille on 1 March 2026.
    let start = utc(2026, 3, 1, 12, 0, 0);
    let soonest = s.tick(start).await.unwrap();
    let sunset = ids["Volets — coucher du soleil"];
    let sunrise = ids["Volets — lever du soleil"];
    let at = |id: AutomationId| {
        let store = store.clone();
        async move {
            store
                .schedule()
                .automation_next_runs()
                .await
                .unwrap()
                .into_iter()
                .find(|(a, _)| *a == id)
                .and_then(|(_, t)| t)
                .unwrap()
        }
    };
    let (set_at, rise_at) = (at(sunset).await, at(sunrise).await);
    assert_eq!(soonest, set_at.min(rise_at));
    // The `ephem` instants for Marseille from 1 March 2026 at 12:00 UTC: to within 90 s.
    let oracle: serde_json::Value = serde_json::from_str(SCHEDULE_FIXTURE).unwrap();
    let expected = |event: &str| {
        let case = oracle["solar"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["place"] == "Marseille"
                    && c["event"] == event
                    && c["start"].as_str().unwrap().starts_with("2026-03-01T12:00")
            })
            .unwrap();
        DateTime::parse_from_rfc3339(case["next"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc)
    };
    for (got, event) in [(set_at, "sunset"), (rise_at, "sunrise")] {
        assert!(
            (got - expected(event)).num_seconds().abs() <= 90,
            "{event} : {got} au lieu de {}",
            expected(event)
        );
    }
    // On time, the right automation goes off, once.
    s.tick(set_at + Duration::seconds(1)).await;
    assert_eq!(
        *ran.0.lock().unwrap(),
        [(TASK_RUN_AUTOMATION.to_string(), sunset.0)]
    );
    s.tick(rise_at + Duration::seconds(1)).await;
    assert_eq!(ran.0.lock().unwrap().len(), 2);
    assert_eq!(ran.0.lock().unwrap()[1].1, sunrise.0);
    // The next day: the next sunset is about 24 h later (to within two minutes).
    let next_set = at(sunset).await;
    let day = (next_set - set_at).num_seconds() - 86_400;
    assert!(day.abs() < 180, "{day}");
}
