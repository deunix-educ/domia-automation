// SPDX-License-Identifier: AGPL-3.0-or-later
//! Activation des scenes sur le Mosquitto de dev (`make dev-up`, R4 lot 2) : les commandes partent **dans l'ordre**, une
//! commande en echec n'arrete pas les suivantes, et les trois scenes de la production publient exactement leurs 6 / 4 / 4
//! commandes sur les bons topics. Chaque test utilise un prefixe de topics a lui (le broker garde des messages Zigbee2MQTT
//! retenus et les tests tournent en parallele).

mod common;

use std::collections::BTreeMap;

use common::broker::{device, next, setup};
use common::state;
use dominet_domain::device::DeviceId;
use dominet_domain::repo::SceneRepo;
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_domain::scene::{SceneActionDraft, SceneDraft};
use dominet_proto::{CommandOutcome, SceneOutcome};
use dominet_server::scenes;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn commands_are_published_in_order_and_a_failure_does_not_stop_the_next(pool: PgPool) {
    let prefix = format!("scn{}", std::process::id());
    let (st, mut seen, listener) = setup(pool, "scene-order", &prefix).await;
    let a = device(&st, "Lampe A", &prefix, true).await;
    let off = device(&st, "Lampe eteinte", &prefix, false).await;
    let b = device(&st, "Lampe B", &prefix, true).await;
    let mut draft = SceneDraft::new("Soiree");
    draft.actions = [
        (a, json!({"state": "ON"})),
        (off, json!({"state": "ON"})),
        (b, json!({"brightness": 128})),
        (a, json!({"state": "OFF"})),
    ]
    .into_iter()
    .map(|(device, payload)| SceneActionDraft { device, payload })
    .collect();
    let scene = st.store.scenes().create(&draft).await.unwrap();

    let outcome = scenes::activate_by_id(&st, scene.id).await;
    let SceneOutcome::Activated { sent, failed } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(sent, 3);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].device, "Lampe eteinte");
    assert_eq!(failed[0].outcome, CommandOutcome::NotFound);
    for (topic, payload) in [
        (format!("{prefix}/Lampe A/set"), json!({"state": "ON"})),
        (format!("{prefix}/Lampe B/set"), json!({"brightness": 128})),
        (format!("{prefix}/Lampe A/set"), json!({"state": "OFF"})),
    ] {
        assert_eq!(next(&mut seen).await, (topic, payload));
    }
    // Par nom (API de script) : meme effet ; une scene desactivee ou inconnue ne part pas.
    assert!(matches!(
        scenes::activate_by_name(&st, "Soiree").await,
        SceneOutcome::Activated { sent: 3, .. }
    ));
    assert_eq!(
        scenes::activate_by_name(&st, "Inconnue").await,
        SceneOutcome::NotFound
    );
    draft.active = false;
    st.store.scenes().update(scene.id, &draft).await.unwrap();
    assert_eq!(
        scenes::activate_by_id(&st, scene.id).await,
        SceneOutcome::NotFound
    );
    assert_eq!(
        scenes::activate_by_name(&st, "Soiree").await,
        SceneOutcome::NotFound
    );

    st.mqtt.as_ref().unwrap().shutdown();
    listener.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_a_broker_every_command_is_reported_unavailable(pool: PgPool) {
    let st = state(pool, &[]);
    let a = device(&st, "Lampe A", "zigbee2mqtt", true).await;
    let mut draft = SceneDraft::new("Seule");
    draft.actions = vec![
        SceneActionDraft {
            device: a,
            payload: json!({"state": "ON"}),
        },
        SceneActionDraft {
            device: a,
            payload: json!({"state": "OFF"}),
        },
    ];
    let scene = st.store.scenes().create(&draft).await.unwrap();
    let SceneOutcome::Activated { sent, failed } = scenes::activate_by_id(&st, scene.id).await
    else {
        panic!()
    };
    assert_eq!(sent, 0);
    assert_eq!(
        failed.len(),
        2,
        "une commande en echec n'arrete pas la suivante"
    );
    assert!(
        failed
            .iter()
            .all(|f| f.outcome == CommandOutcome::Unavailable)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_three_production_scenes_publish_their_six_four_and_four_commands(pool: PgPool) {
    let prefix = format!("prod{}", std::process::id());
    let (st, mut seen, listener) = setup(pool, "scene-prod", &prefix).await;
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let mut ids: BTreeMap<String, DeviceId> = BTreeMap::new();
    for d in &ctx.devices {
        ids.insert(d.name.clone(), device(&st, &d.name, &prefix, true).await);
    }
    for sc in &ctx.scenes {
        let mut draft = SceneDraft::new(&sc.name);
        draft.actions = sc
            .actions
            .iter()
            .map(|(name, payload)| SceneActionDraft {
                device: ids[name],
                payload: payload.clone(),
            })
            .collect();
        st.store.scenes().create(&draft).await.unwrap();
    }
    let mut counts: Vec<usize> = ctx.scenes.iter().map(|s| s.actions.len()).collect();
    counts.sort_unstable();
    assert_eq!(counts, [4, 4, 6], "trois scenes : 6, 4 et 4 commandes");
    for sc in &ctx.scenes {
        let outcome = scenes::activate_by_name(&st, &sc.name).await;
        assert!(
            matches!(&outcome, SceneOutcome::Activated { sent, failed } if *sent as usize == sc.actions.len() && failed.is_empty()),
            "{}: {outcome:?}",
            sc.name
        );
        for (device, payload) in &sc.actions {
            assert_eq!(
                next(&mut seen).await,
                (format!("{prefix}/{device}/set"), payload.clone()),
                "{}",
                sc.name
            );
        }
    }
    st.mqtt.as_ref().unwrap().shutdown();
    listener.shutdown();
}
