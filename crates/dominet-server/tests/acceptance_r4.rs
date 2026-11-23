// SPDX-License-Identifier: AGPL-3.0-or-later
//! R4 acceptance: **the whole production `context.yaml` at once** (29 devices, 3 scenes, 7 automations including the two
//! plugins ported to Rhai, the heatwave rule) runs the engine, the alert evaluation and the scheduler together on the
//! dev Mosquitto. The same scenarios are checked one by one, in detail, by `scenes.rs`, `automations.rs`, `alerts.rs`;
//! here we check that they do not get in each other's way.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use common::broker::{device, publisher, setup};
use dominet_automation::scheduler::Scheduler;
use dominet_domain::automation::Action;
use dominet_domain::repo::{AlertRepo, AlertRuleRepo, AutomationRepo, SceneRepo, ScheduleRepo};
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_domain::scene::{SceneActionDraft, SceneDraft};
use dominet_mqtt::Payload;
use dominet_proto::{Topic, WsServerMsg};
use dominet_server::scheduler::ServerRunner;
use serde_json::{Value, json};
use sqlx::PgPool;

/// The elements of `wanted` appear in `seen`, in that order (other messages may be interleaved).
fn in_order(seen: &[(String, Value)], wanted: &[(String, Value)]) -> bool {
    let mut it = seen.iter();
    wanted.iter().all(|w| it.any(|s| s == w))
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_whole_production_context_runs_together_without_getting_in_each_others_way(
    pool: PgPool,
) {
    let prefix = format!("acc{}", std::process::id());
    let z2m = format!("{prefix}z2m");
    let (st, mut seen, listener) = setup(pool, "accept", &prefix).await;
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    // The production installation, under the test's prefix.
    let mut devices = BTreeMap::new();
    for d in &ctx.devices {
        devices.insert(d.name.clone(), device(&st, &d.name, &prefix, true).await);
    }
    let mut scene_of = BTreeMap::new();
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
        scene_of.insert(sc.pk, st.store.scenes().create(&draft).await.unwrap().id);
    }
    for a in &ctx.automations {
        let mut draft = a.draft(&scene_of).unwrap();
        if let Action::MqttPublish { topic, .. } = &mut draft.action {
            *topic = topic.replace("zigbee2mqtt/", &format!("{prefix}/"));
        }
        st.store.automations().create(&draft).await.unwrap();
    }
    st.store
        .alert_rules()
        .create(&ctx.alert_rules[0].draft(devices["meteo_locale"]))
        .await
        .unwrap();
    st.automations_updated();
    st.alert_rules_changed.notify_one();
    let mut toasts = st.hub.subscribe(&Topic::Notifications);

    // 1. The remote control, the shutter's Rhai plugin, the heatwave: we repeat until each one's first reaction (the engine and
    //    the evaluation re-read their lists in the background), never exceeding 30 triggers per minute.
    let remote = publisher("accept").await;
    let mut collected: Vec<(String, Value)> = vec![];
    let mut drain = |collected: &mut Vec<(String, Value)>| {
        while let Ok(m) = seen.try_recv() {
            if let Payload::Json(v) = m.payload {
                collected.push((m.topic, v));
            }
        }
    };
    let open = (
        format!("{prefix}/Volet-salon/set"),
        json!({"state": "OPEN"}),
    );
    let stop = (
        format!("{prefix}/Relais-SB-est/set"),
        json!({"state": "STOP"}),
    );
    for _ in 0..40 {
        remote
            .publish(
                &format!("{z2m}/Tele-commande"),
                json!({"action": "1_single"}).to_string(),
                1,
                false,
            )
            .unwrap();
        // The shutter relay has just spoken (it publishes its position while it moves): a press stops it.
        remote
            .publish(
                &format!("{z2m}/Relais-SB-est"),
                json!({"state": "OPEN", "position": 40}).to_string(),
                1,
                false,
            )
            .unwrap();
        remote
            .publish(
                &format!("{z2m}/Volet-SB-est"),
                json!({"action": "single_left"}).to_string(),
                1,
                false,
            )
            .unwrap();
        remote
            .publish(
                &format!("{z2m}/meteo_locale"),
                json!({"temperature": 36.5}).to_string(),
                1,
                false,
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        drain(&mut collected);
        if collected.contains(&open)
            && collected.contains(&stop)
            && st.store.alerts().count_firing().await.unwrap() > 0
        {
            break;
        }
    }
    assert!(
        collected.contains(&open),
        "la telecommande ouvre le volet du salon : {collected:?}"
    );
    assert!(
        collected.contains(&stop),
        "le plugin gauche arrete le volet en mouvement : {collected:?}"
    );
    assert_eq!(
        st.store.alerts().count_firing().await.unwrap(),
        1,
        "une seule alerte canicule malgre les etats repetes (cooldown 60 min)"
    );
    let toast = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(m) = toasts.recv().await
                && let WsServerMsg::Notification { text, .. } = &*m
                && text.contains("Heat wave")
            {
                return text.clone();
            }
        }
    })
    .await
    .expect("alert banner");
    assert!(toast.contains("meteo_locale : 36.5"), "{toast}");

    // 2. The production's sunset (scheduler): "Evening closing", six commands in order.
    let scheduler = Scheduler::new(
        st.store.schedule(),
        st.store.automations(),
        ServerRunner::new(&st),
        chrono_tz::Europe::Paris,
    );
    scheduler.sync().await;
    scheduler
        .tick(Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap())
        .await;
    let autos = st.store.automations().list().await.unwrap();
    let sunset = autos
        .iter()
        .find(|a| a.draft.name == "Volets — coucher du soleil")
        .unwrap();
    let due = st
        .store
        .schedule()
        .automation_next_runs()
        .await
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == sunset.id)
        .and_then(|(_, t)| t)
        .expect("prochain coucher du soleil");
    scheduler.tick(due + chrono::Duration::seconds(1)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    drain(&mut collected);
    let evening = ctx
        .scenes
        .iter()
        .find(|s| s.name == "Fermeture du soir")
        .unwrap();
    let wanted: Vec<(String, Value)> = evening
        .actions
        .iter()
        .map(|(name, payload)| (format!("{prefix}/{name}/set"), payload.clone()))
        .collect();
    assert_eq!(wanted.len(), 6);
    assert!(
        in_order(&collected, &wanted),
        "les six commandes de la scene, dans l'ordre : {collected:?}"
    );
    let runs = st
        .store
        .automations()
        .get(sunset.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(runs.trigger_count, 1);

    st.mqtt.as_ref().unwrap().shutdown();
    remote.shutdown();
    listener.shutdown();
}
