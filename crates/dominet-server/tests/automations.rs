// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end automation engine on the dev Mosquitto (R4 lot 3, `make dev-up`): a state or a message published on
//! the broker goes through ingestion, triggers the automation, and its command comes out on the broker. The three
//! production remote-control automations move the shutter; an `mqtt_message` automation on a topic without a source subscribes
//! by itself; an administration change is taken into account live.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::broker::{device, next, publisher, setup};
use dominet_domain::automation::{Action, AutomationDraft, RunStatus, Trigger};
use dominet_domain::repo::{AutomationRepo, AutomationRunRepo, SceneRepo};
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_domain::scene::{SceneActionDraft, SceneDraft};
use serde_json::{Map, json};
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_remote_control_moves_the_shutter_through_the_broker(pool: PgPool) {
    let prefix = format!("auto{}", std::process::id());
    let z2m = format!("{prefix}z2m"); // Zigbee2MQTT base chosen by `setup`
    let (st, mut seen, listener) = setup(pool, "auto-prod", &prefix).await;
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    for name in ["Tele-commande", "Volet-salon"] {
        device(&st, name, &prefix, true).await;
    }
    // The production's three "TC Salon" automations, whose command topic is brought under the test's prefix.
    let mut created = 0;
    for a in ctx
        .automations
        .iter()
        .filter(|a| a.name.starts_with("TC Salon"))
    {
        let mut draft = a.draft(&BTreeMap::new()).unwrap();
        if let Action::MqttPublish { topic, .. } = &mut draft.action {
            *topic = topic.replace("zigbee2mqtt/", &format!("{prefix}/"));
        }
        st.store.automations().create(&draft).await.unwrap();
        created += 1;
    }
    assert_eq!(created, 3);
    st.automations_changed.notify_one();

    let remote = publisher("auto-prod").await;
    let state_topic = format!("{z2m}/Tele-commande");
    let press = |action: &str| {
        remote
            .publish(
                &state_topic,
                json!({"action": action, "battery": 97}).to_string(),
                1,
                false,
            )
            .unwrap();
        // Z2M republishes an empty action state right after: it triggers nothing.
        remote
            .publish(
                &state_topic,
                json!({"action": "", "battery": 97}).to_string(),
                1,
                false,
            )
            .unwrap();
    };
    // The engine reloads its automations in the background: we press until the first reaction (the presses that precede the
    // reload are dropped at the source), then we discard any duplicates.
    let store = st.store.clone();
    let mut loaded = false;
    for _ in 0..30 {
        press("1_single");
        tokio::time::sleep(Duration::from_millis(700)).await;
        let total: i32 = store
            .automations()
            .list()
            .await
            .unwrap()
            .iter()
            .map(|a| a.trigger_count)
            .sum();
        if total >= 1 {
            loaded = true;
            break;
        }
    }
    assert!(loaded, "le moteur n'a jamais reagi");
    let open = (
        format!("{prefix}/Volet-salon/set"),
        json!({"state": "OPEN"}),
    );
    assert_eq!(next(&mut seen).await, open);
    tokio::time::sleep(Duration::from_millis(1100)).await; // one second apart (burst coalescing)
    while tokio::time::timeout(Duration::from_millis(400), seen.recv())
        .await
        .is_ok()
    {}
    let opened: i32 = store
        .automations()
        .list()
        .await
        .unwrap()
        .iter()
        .map(|a| a.trigger_count)
        .sum();
    for (action, state) in [("2_single", "STOP"), ("3_single", "CLOSE")] {
        press(action);
        assert_eq!(
            next(&mut seen).await,
            (format!("{prefix}/Volet-salon/set"), json!({"state": state}))
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
    // Counters and log. The counter is written after the publication: we wait for it to catch up, otherwise a loaded machine
    // (the push hook runs at low priority) reads a late total.
    let count = |all: &[dominet_domain::automation::Automation]| -> i32 {
        all.iter().map(|a| a.trigger_count).sum()
    };
    let mut all = st.store.automations().list().await.unwrap();
    for _ in 0..50 {
        if count(&all) == opened + 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        all = st.store.automations().list().await.unwrap();
    }
    assert_eq!(count(&all), opened + 2);
    for a in &all {
        let runs = st.store.automation_runs().recent(a.id, 10).await.unwrap();
        assert!(!runs.is_empty(), "{}", a.draft.name);
        assert!(runs.iter().all(|r| r.status == RunStatus::Ok));
    }
    st.mqtt.as_ref().unwrap().shutdown();
    remote.shutdown();
    listener.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_mqtt_message_automation_subscribes_by_itself_follows_changes_and_activates_scenes(
    pool: PgPool,
) {
    let prefix = format!("autm{}", std::process::id());
    let (st, mut seen, listener) = setup(pool, "auto-msg", &prefix).await;
    let lamp = device(&st, "Lampe", &prefix, true).await;
    let blind = device(&st, "Volet", &prefix, true).await;
    let scene = st
        .store
        .scenes()
        .create(&{
            let mut s = SceneDraft::new("Cinema");
            s.actions = vec![
                SceneActionDraft {
                    device: lamp,
                    payload: json!({"state": "OFF"}),
                },
                SceneActionDraft {
                    device: blind,
                    payload: json!({"state": "CLOSE"}),
                },
            ];
            s
        })
        .await
        .unwrap();
    let watched = format!("{prefix}in/+/bouton");
    let mut filter = Map::new();
    filter.insert("action".into(), json!("single"));
    let created = st
        .store
        .automations()
        .create(&AutomationDraft::new(
            "Bouton cinema",
            Trigger::MqttMessage {
                topic: watched.clone(),
                payload_filter: filter,
            },
            Action::Scene { scene: scene.id },
        ))
        .await
        .unwrap();
    st.automations_changed.notify_one();
    tokio::time::sleep(Duration::from_millis(800)).await;

    let remote = publisher("auto-msg").await;
    let press = |action: &str| {
        remote
            .publish(
                &format!("{prefix}in/salon/bouton"),
                json!({"action": action}).to_string(),
                1,
                false,
            )
            .unwrap();
    };
    press("double"); // filter: nothing
    // Repeated presses until the first reaction (the engine and the subscription set themselves up in the background).
    let store = st.store.clone();
    let mut fired = false;
    for _ in 0..30 {
        press("single");
        tokio::time::sleep(Duration::from_millis(700)).await;
        if store
            .automations()
            .get(created.id)
            .await
            .unwrap()
            .unwrap()
            .trigger_count
            >= 1
        {
            fired = true;
            break;
        }
    }
    assert!(fired, "l'automation ne s'est jamais declenchee");
    assert_eq!(
        next(&mut seen).await,
        (format!("{prefix}/Lampe/set"), json!({"state": "OFF"}))
    );
    assert_eq!(
        next(&mut seen).await,
        (format!("{prefix}/Volet/set"), json!({"state": "CLOSE"}))
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    while tokio::time::timeout(Duration::from_millis(400), seen.recv())
        .await
        .is_ok()
    {}
    let count = store
        .automations()
        .get(created.id)
        .await
        .unwrap()
        .unwrap()
        .trigger_count;

    // Deactivated from the administration: taken into account live, nothing goes out any more.
    let mut off = created.draft.clone();
    off.enabled = false;
    st.store
        .automations()
        .update(created.id, &off)
        .await
        .unwrap();
    st.automations_changed.notify_one();
    tokio::time::sleep(Duration::from_millis(2000)).await;
    press("single");
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(500), seen.recv())
            .await
            .is_err(),
        "une automation desactivee ne declenche plus"
    );
    assert_eq!(
        st.store
            .automations()
            .get(created.id)
            .await
            .unwrap()
            .unwrap()
            .trigger_count,
        count
    );
    st.mqtt.as_ref().unwrap().shutdown();
    remote.shutdown();
    listener.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_solar_automations_run_their_scenes_through_the_scheduler(pool: PgPool) {
    use chrono::{Duration as Span, TimeZone, Utc};
    use dominet_automation::scheduler::Scheduler;
    use dominet_domain::repo::ScheduleRepo;
    use dominet_server::scheduler::ServerRunner;

    let prefix = format!("sol{}", std::process::id());
    let (st, mut seen, listener) = setup(pool, "auto-solar", &prefix).await;
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let mut devices = BTreeMap::new();
    for d in &ctx.devices {
        devices.insert(d.name.clone(), device(&st, &d.name, &prefix, true).await);
    }
    let mut local = BTreeMap::new();
    for sc in &ctx.scenes {
        let mut draft = SceneDraft::new(&sc.name);
        draft.actions = sc
            .actions
            .iter()
            .map(|(name, payload)| SceneActionDraft {
                device: devices[name],
                payload: payload.clone(),
            })
            .collect();
        local.insert(sc.pk, st.store.scenes().create(&draft).await.unwrap().id);
    }
    for a in ctx.automations.iter().filter(|a| a.trigger_type == "solar") {
        st.store
            .automations()
            .create(&a.draft(&local).unwrap())
            .await
            .unwrap();
    }
    let scheduler = Scheduler::new(
        st.store.schedule(),
        st.store.automations(),
        ServerRunner::new(&st),
        chrono_tz::Europe::Paris,
    );
    scheduler.sync().await;
    let start = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
    scheduler.tick(start).await;
    let due = |name: &str| {
        let store = st.store.clone();
        let name = name.to_string();
        async move {
            let autos = store.automations().list().await.unwrap();
            let id = autos.iter().find(|a| a.draft.name == name).unwrap().id;
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
    let sunset = due("Volets — coucher du soleil").await;
    let sunrise = due("Volets — lever du soleil").await;
    // At sunset: "Evening closing" (6 commands, in order); at sunrise: "Morning opening" (4).
    for (when, scene) in [
        (sunset, "Fermeture du soir"),
        (sunrise, "Ouverture du matin"),
    ] {
        scheduler.tick(when + Span::seconds(1)).await;
        let sc = ctx.scenes.iter().find(|s| s.name == scene).unwrap();
        for (name, payload) in &sc.actions {
            assert_eq!(
                next(&mut seen).await,
                (format!("{prefix}/{name}/set"), payload.clone()),
                "{scene}"
            );
        }
    }
    let autos = st.store.automations().list().await.unwrap();
    for a in autos.iter().filter(|a| a.draft.name.starts_with("Volets")) {
        assert_eq!(a.trigger_count, 1, "{}", a.draft.name);
        let runs = st.store.automation_runs().recent(a.id, 5).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, RunStatus::Ok);
        assert!(runs[0].trigger.starts_with("sun: "), "{}", runs[0].trigger);
    }
    st.mqtt.as_ref().unwrap().shutdown();
    listener.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_plugins_and_a_source_handler_run_as_rhai_through_the_broker(pool: PgPool) {
    use dominet_domain::device::MqttSource;
    use dominet_domain::repo::MqttSourceRepo;

    let prefix = format!("rhai{}", std::process::id());
    let z2m = format!("{prefix}z2m");
    let (st, mut seen, listener) = setup(pool, "auto-rhai", &prefix).await;
    device(&st, "Volet-SB-est", &prefix, true).await;
    // The relay commanded by the scripts: it has never spoken, so it is not moving.
    device(&st, "Relais-SB-est", &prefix, true).await;
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    for a in ctx
        .automations
        .iter()
        .filter(|a| a.trigger_type == "device_state" && a.action_type == "script")
    {
        let draft = a.draft(&BTreeMap::new()).unwrap();
        assert!(dominet_automation::script::check(&draft.script).is_ok());
        st.store.automations().create(&draft).await.unwrap();
    }
    // Source with processing code: republishes the power read on another topic.
    let mut source = MqttSource::draft("Compteur", &format!("{prefix}in/#"));
    source.handler_code = format!(r#"publish("{prefix}/out/set", #{{ w: payload.papp }});"#);
    st.store.mqtt_sources().create(&source).await.unwrap();
    st.sources_changed.notify_one();
    st.automations_updated();

    let remote = publisher("auto-rhai").await;
    let state_topic = format!("{z2m}/Volet-SB-est");
    // Repeated presses until the first reaction (reload in the background).
    let mut got = None;
    for _ in 0..30 {
        remote
            .publish(
                &state_topic,
                json!({"action": "single_left"}).to_string(),
                1,
                false,
            )
            .unwrap();
        remote
            .publish(
                &format!("{prefix}in/compteur"),
                json!({"papp": 420}).to_string(),
                1,
                false,
            )
            .unwrap();
        if let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(900), seen.recv()).await {
            got = Some(m);
            break;
        }
    }
    let mut messages = vec![got.expect("aucune commande recue")];
    // The source's code: same active wait (its subscription sets itself up in the background).
    for _ in 0..30 {
        remote
            .publish(
                &format!("{prefix}in/compteur"),
                json!({"papp": 420}).to_string(),
                1,
                false,
            )
            .unwrap();
        while let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(600), seen.recv()).await
        {
            messages.push(m);
        }
        if messages
            .iter()
            .any(|m| m.topic == format!("{prefix}/out/set"))
        {
            break;
        }
    }
    let texts: Vec<(String, serde_json::Value)> = messages
        .into_iter()
        .filter_map(|m| match m.payload {
            dominet_mqtt::Payload::Json(v) => Some((m.topic, v)),
            dominet_mqtt::Payload::Text(_) => None,
        })
        .collect();
    assert!(
        texts.contains(&(
            format!("{prefix}/Relais-SB-est/set"),
            json!({"state": "OPEN"})
        )),
        "le bouton gauche ouvre le volet : {texts:?}"
    );
    assert!(
        texts.contains(&(format!("{prefix}/out/set"), json!({"w": 420}))),
        "le code de la source republie : {texts:?}"
    );
    st.mqtt.as_ref().unwrap().shutdown();
    remote.shutdown();
    listener.shutdown();
}
