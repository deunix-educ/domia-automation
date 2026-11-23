// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end alerts on the dev Mosquitto (R4 lot 6): a state published on the broker triggers the production rule's alert (heatwave:
//! `meteo_locale.temperature > 35`, cooldown 60 min), the banner and the number of ongoing alerts are
//! broadcast, the cooldown holds back the next one, and the return to normal resolves the alert.

mod common;

use std::time::Duration;

use common::broker::{device, publisher, setup};
use dominet_domain::repo::{AlertRepo, AlertRuleRepo};
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_proto::{NotificationLevel, Topic, WsServerMsg};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_heatwave_rule_fires_through_the_broker_then_cools_down_and_resolves(
    pool: PgPool,
) {
    let prefix = format!("alr{}", std::process::id());
    let z2m = format!("{prefix}z2m");
    let (st, _seen, listener) = setup(pool, "alerts", &prefix).await;
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let rule = &ctx.alert_rules[0];
    assert_eq!(
        (rule.device.as_str(), rule.key.as_str()),
        ("meteo_locale", "temperature")
    );
    let id = device(&st, &rule.device, &prefix, true).await;
    st.store
        .alert_rules()
        .create(&rule.draft(id))
        .await
        .unwrap();
    st.alert_rules_changed.notify_one();
    let mut toasts = st.hub.subscribe(&Topic::Notifications);

    let remote = publisher("alerts").await;
    let topic = format!("{z2m}/meteo_locale");
    // Normal states, then the heatwave: the evaluation sets itself up in the background, we repeat the hot state until the alert.
    remote
        .publish(&topic, json!({"temperature": 22.5}).to_string(), 1, false)
        .unwrap();
    let mut fired = false;
    for _ in 0..30 {
        remote
            .publish(&topic, json!({"temperature": 36.5}).to_string(), 1, false)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
        if st.store.alerts().count_firing().await.unwrap() > 0 {
            fired = true;
            break;
        }
    }
    assert!(fired, "aucune alerte declenchee");
    // The banner and the number of ongoing alerts.
    let toast = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(m) = toasts.recv().await
                && let WsServerMsg::Notification { level, text } = &*m
            {
                return (*level, text.clone());
            }
        }
    })
    .await
    .expect("bandeau");
    assert_eq!(toast.0, NotificationLevel::Warning);
    assert!(toast.1.starts_with("[DomiNet] Heat wave"), "{}", toast.1);
    assert!(toast.1.ends_with("meteo_locale : 36.5"), "{}", toast.1);
    let retained = st.hub.retained(&Topic::Alerts).expect("compte diffuse");
    assert!(
        matches!(&*retained, WsServerMsg::AlertCount { firing: 1 }),
        "{retained:?}"
    );

    // 60 min cooldown: more hot states create nothing.
    for t in [37.0, 38.2] {
        remote
            .publish(&topic, json!({"temperature": t}).to_string(), 1, false)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
    assert_eq!(st.store.alerts().list(None, 10).await.unwrap().len(), 1);

    // Return to normal: resolved, and the count falls back to zero.
    remote
        .publish(&topic, json!({"temperature": 24.0}).to_string(), 1, false)
        .unwrap();
    let mut resolved = false;
    for _ in 0..40 {
        if st.store.alerts().count_firing().await.unwrap() == 0 {
            resolved = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(resolved, "l'alerte n'a pas ete resolue");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let retained = st.hub.retained(&Topic::Alerts).unwrap();
    assert!(
        matches!(&*retained, WsServerMsg::AlertCount { firing: 0 }),
        "{retained:?}"
    );
    st.mqtt.as_ref().unwrap().shutdown();
    remote.shutdown();
    listener.shutdown();
}
