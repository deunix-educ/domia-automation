// SPDX-License-Identifier: AGPL-3.0-or-later
//! `dominet dump-context`: the document built from the database, then its YAML.

mod common;

use common::state;
use dominet_domain::alert::AlertRuleDraft;
use dominet_domain::automation::{Action, AutomationDraft, Trigger};
use dominet_domain::context::to_yaml;
use dominet_domain::device::{DeviceDraft, DeviceType};
use dominet_domain::location::LocationDraft;
use dominet_domain::repo::{AlertRuleRepo, AutomationRepo, DeviceRepo, LocationRepo, SceneRepo};
use dominet_domain::scene::{SceneActionDraft, SceneDraft};
use dominet_server::commands::context_document;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_context_lists_active_devices_by_type_then_name_with_their_state_schema(pool: PgPool) {
    let st = state(pool, &[]);
    let salon = st
        .store
        .locations()
        .create(&LocationDraft {
            name: "Salon".into(),
            description: String::new(),
            active: true,
        })
        .await
        .unwrap();
    let mut sonde = DeviceDraft::new("Sonde salon");
    sonde.device_type = DeviceType::Sensor;
    sonde.location = Some(salon.id);
    let mut lampe = DeviceDraft::new("Lampe");
    lampe.device_type = DeviceType::Light;
    let mut eteinte = DeviceDraft::new("Eteinte");
    eteinte.active = false;
    let mut abc = DeviceDraft::new("Abc");
    abc.device_type = DeviceType::Sensor;
    for d in [&sonde, &lampe, &eteinte, &abc] {
        st.store.devices().create(d).await.unwrap();
    }
    st.store
        .devices()
        .apply_state("Sonde salon", &json!({"temperature": 21.5, "humidity": 40, "linkquality": 80, "update": {"state": "idle"}}))
        .await
        .unwrap();
    st.store
        .devices()
        .apply_state(
            "Lampe",
            &json!({"state": "ON", "brightness": 200, "color_temp": null}),
        )
        .await
        .unwrap();

    let doc = context_document(&st.store).await.unwrap();
    let names: Vec<&str> = doc["devices"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    // Sorted by type then name; the inactive device is absent.
    assert_eq!(names, ["Lampe", "Abc", "Sonde salon"]);
    let sonde = &doc["devices"]["Sonde salon"];
    assert_eq!(sonde["type"], "sensor");
    assert_eq!(sonde["location"], "Salon");
    assert_eq!(sonde["command_topic"], "zigbee2mqtt/Sonde salon/set");
    assert_eq!(sonde["numeric_keys"].as_array().unwrap().len(), 2);
    let schema = sonde["state_schema"].as_object().unwrap();
    assert!(schema.get("update").is_none() && schema.get("linkquality").is_none());
    assert_eq!(schema["temperature"], "float");
    assert_eq!(schema["humidity"], "int");
    let lampe = &doc["devices"]["Lampe"];
    assert_eq!(lampe["state_schema"]["color_temp"], "float|null");
    assert!(
        doc["devices"]["Abc"].get("numeric_keys").is_none(),
        "pas de cle numerique : pas de rubrique"
    );
    assert!(doc["devices"]["Abc"].get("location").is_none());
    for empty in ["scenes", "automations", "alert_rules"] {
        assert_eq!(
            doc[empty],
            json!([]),
            "{empty}: rien n'est configure dans cette base"
        );
    }
    // No Grafana rule in this database: the section is empty (the content is tested with the rules, in `rules.rs`).
    assert_eq!(doc["grafana_rules"], json!([]));
    let yaml = to_yaml(&doc);
    assert!(yaml.starts_with("devices:\n"));
    assert!(yaml.contains("\n  \"Sonde salon\":\n    type: \"sensor\"\n"));
    assert!(yaml.contains("scenes: []\n") && yaml.contains("\nschemas:\n  trigger_configs:\n"));
}

#[test]
fn healthcheck_is_true_only_for_a_200_on_healthz() {
    use std::io::{Read, Write};
    let serve = |status: &'static str| {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0_u8; 256];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\n\r\n").as_bytes(),
                );
            }
        });
        addr
    };
    assert!(dominet_server::commands::healthcheck(Some(&serve(
        "200 OK"
    ))));
    assert!(!dominet_server::commands::healthcheck(Some(&serve(
        "503 Service Unavailable"
    ))));
    assert!(!dominet_server::commands::healthcheck(Some("127.0.0.1:1")));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_context_lists_active_scenes_automations_and_alert_rules(pool: PgPool) {
    let st = state(pool, &[]);
    let mut d = DeviceDraft::new("Lampe");
    d.device_type = DeviceType::Light;
    let lampe = st.store.devices().create(&d).await.unwrap();
    let mut z = DeviceDraft::new("Cave");
    z.device_type = DeviceType::Sensor;
    let cave = st.store.devices().create(&z).await.unwrap();

    // Scenes: one active with two commands, one inactive (absent).
    let mut soir = SceneDraft::new("Soir");
    soir.icon = "moon".into();
    soir.actions = vec![
        SceneActionDraft {
            device: lampe.id,
            payload: json!({"state": "ON"}),
        },
        SceneActionDraft {
            device: lampe.id,
            payload: json!({"brightness": 50}),
        },
    ];
    st.store.scenes().create(&soir).await.unwrap();
    let mut off = SceneDraft::new("Off");
    off.active = false;
    st.store.scenes().create(&off).await.unwrap();

    // Automations: sorted by trigger type then name; the deactivated one is absent.
    let notify = |m: &str| Action::Notification {
        message: m.into(),
        sms: false,
    };
    let cron = |c: &str| Trigger::Schedule { cron: c.into() };
    for (name, trigger, enabled) in [
        ("B planifiee", cron("0 7 * * *"), true),
        ("A planifiee", cron("0 8 * * *"), true),
        ("Eteinte", cron("0 9 * * *"), false),
        (
            "Message",
            Trigger::MqttMessage {
                topic: "a/#".into(),
                payload_filter: serde_json::Map::new(),
            },
            true,
        ),
    ] {
        let mut a = AutomationDraft::new(name, trigger, notify("salut"));
        a.enabled = enabled;
        st.store.automations().create(&a).await.unwrap();
    }

    // Alert rules: sorted by device; the deactivated one is absent.
    let mut r1 = AlertRuleDraft::new("Trop chaud", lampe.id, "temperature");
    r1.threshold_num = Some(30.0);
    r1.notify_sms = true;
    let mut r2 = AlertRuleDraft::new("Humide", cave.id, "humidity");
    r2.threshold_num = Some(80.0);
    let mut r3 = AlertRuleDraft::new("Muette", cave.id, "x");
    r3.enabled = false;
    for r in [&r1, &r2, &r3] {
        st.store.alert_rules().create(r).await.unwrap();
    }

    let doc = context_document(&st.store).await.unwrap();
    let scenes = doc["scenes"].as_array().unwrap();
    assert_eq!(scenes.len(), 1);
    assert_eq!(scenes[0]["name"], "Soir");
    assert_eq!(scenes[0]["icon"], "moon");
    assert_eq!(
        scenes[0]["actions"],
        json!([{"device": "Lampe", "payload": {"state": "ON"}}, {"device": "Lampe", "payload": {"brightness": 50}}])
    );
    let autos: Vec<(&str, &str)> = doc["automations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["trigger_type"].as_str().unwrap(),
                a["name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        autos,
        [
            ("mqtt_message", "Message"),
            ("schedule", "A planifiee"),
            ("schedule", "B planifiee")
        ]
    );
    let first = &doc["automations"][1];
    assert_eq!(first["trigger_config"]["cron"], "0 8 * * *");
    assert_eq!(first["action_type"], "notification");
    assert_eq!(first["action_config"]["message"], "salut");
    assert!(first.get("condition").is_none() && first.get("script").is_none());
    let rules = doc["alert_rules"].as_array().unwrap();
    let names: Vec<&str> = rules.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["Humide", "Trop chaud"],
        "tri par appareil (Cave puis Lampe), regle desactivee absente"
    );
    assert_eq!(rules[1]["device"], "Lampe");
    assert_eq!(rules[1]["operator"], "gt");
    assert_eq!(rules[1]["threshold_num"], 30.0);
    assert_eq!(rules[1]["threshold_str"], json!(null));
    assert_eq!(
        rules[1]["notify"],
        json!({"email": true, "sms": true, "android": true})
    );
    let yaml = to_yaml(&doc);
    assert!(
        yaml.contains("\nscenes:\n  - pk: ")
            && yaml.contains("\nalert_rules:\n  - name: \"Humide\"")
    );
}
