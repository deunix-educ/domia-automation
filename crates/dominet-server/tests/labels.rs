// SPDX-License-Identifier: AGPL-3.0-or-later
//! `dominet seed-labels` : libelles manquants des cles d'etat des appareils.

// `assert!(x.is_empty())` : plus lisible que la comparaison a un tableau vide annote.
#![allow(clippy::assert_is_empty)]

mod common;

use common::state;
use dominet_domain::device::DeviceType;
use dominet_domain::repo::{DeviceLabelRepo, DeviceRepo};
use dominet_server::commands::seed_labels_data;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn missing_labels_come_from_the_known_table_and_are_never_overwritten(pool: PgPool) {
    let st = state(pool, &[]);
    st.store
        .devices()
        .upsert_auto(
            "a",
            "",
            DeviceType::Sensor,
            &json!({"temperature": 1, "co2_ppm": 400}),
        )
        .await
        .unwrap();
    // Simulation : rien n'est ecrit.
    let dry = seed_labels_data(&st.store, true).await.unwrap();
    assert_eq!(
        dry.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(),
        ["co2_ppm", "temperature"]
    );
    assert!(st.store.device_labels().list().await.unwrap().is_empty());
    // Ecriture, puis idempotence ; un libelle modifie par l'administrateur reste tel quel.
    assert_eq!(seed_labels_data(&st.store, false).await.unwrap().len(), 2);
    assert!(seed_labels_data(&st.store, false).await.unwrap().is_empty());
    let labels = st.store.device_labels().list().await.unwrap();
    let temperature = labels.iter().find(|l| l.key == "temperature").unwrap();
    assert_eq!(
        (temperature.name.as_str(), temperature.unit.as_str()),
        ("Température", "°C")
    );
    assert_eq!(
        labels
            .iter()
            .find(|l| l.key == "co2_ppm")
            .unwrap()
            .icon_name,
        "circle-dot"
    );
}
