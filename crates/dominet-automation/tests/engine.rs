// SPDX-License-Identifier: AGPL-3.0-or-later
//! Automation engine on a throw-away database with a simulated executor: `mqtt_message` / `device_state` triggers, condition,
//! actions, log and counters, anti-loop safeguard, and the **production automations** (`context.yaml`).
#![allow(clippy::unused_async_trait_impl)] // the simulated executor waits for nothing

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use dominet_automation::engine::{Engine, EngineHandle, Event, Executor, MAX_PER_MINUTE, QUEUE};
use dominet_automation::script::DeviceView;
use dominet_domain::automation::{
    Action, AutomationDraft, AutomationId, Condition, RunStatus, Trigger,
};
use dominet_domain::repo::{AutomationRepo, AutomationRunRepo};
use dominet_domain::scenarios::{PROD_CONTEXT_JSON, ProdContext};
use dominet_domain::scene::SceneId;
use dominet_mqtt::decode_payload;
use dominet_store::{PgAutomationRuns, PgAutomations, PgStore};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use tokio::sync::mpsc;

#[derive(Default)]
struct Calls {
    published: Vec<(String, Value)>,
    scenes: Vec<i64>,
    scenes_by_name: Vec<String>,
    logged: Vec<(i64, String, Value)>,
    notified: Vec<(String, bool)>,
    /// Age of the last message of the shutter relay (`None`: it never spoke).
    relay_age: Option<f64>,
}

/// Simulated executor; `fail` makes all publications fail.
struct Fake {
    calls: Arc<Mutex<Calls>>,
    fail: bool,
}

impl Executor for Fake {
    async fn publish(&self, topic: &str, payload: &str) -> Result<String, String> {
        if self.fail {
            return Err("source indisponible".into());
        }
        self.calls.lock().unwrap().published.push((
            topic.to_string(),
            serde_json::from_str(payload).unwrap_or_else(|_| Value::String(payload.into())),
        ));
        Ok(format!("published on {topic}"))
    }

    async fn scene(&self, id: SceneId) -> Result<String, String> {
        self.calls.lock().unwrap().scenes.push(id.0);
        Ok(format!("scene {} activee", id.0))
    }

    async fn scene_by_name(&self, name: &str) -> Result<String, String> {
        self.calls
            .lock()
            .unwrap()
            .scenes_by_name
            .push(name.to_string());
        if name == "Inconnue" {
            Err("scene introuvable".into())
        } else {
            Ok(format!("scene {name} activee"))
        }
    }

    async fn log_value(&self, device: i64, key: &str, value: &Value) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .logged
            .push((device, key.to_string(), value.clone()));
        Ok(())
    }

    async fn devices(&self) -> Result<Vec<DeviceView>, String> {
        Ok(vec![
            DeviceView {
                id: 7,
                name: "Volet".into(),
                device_type: "cover".into(),
                state: json!({"state": "OPEN", "position": 100}),
                available: true,
                topic: "zigbee2mqtt/Volet".into(),
                age_secs: Some(0.4),
            },
            DeviceView {
                id: 8,
                name: "Relais-SB-est".into(),
                device_type: "cover".into(),
                state: json!({"state": "OPEN", "position": 40}),
                available: true,
                topic: "zigbee2mqtt/Relais-SB-est".into(),
                age_secs: self.calls.lock().unwrap().relay_age,
            },
        ])
    }

    fn notify(&self, message: &str, sms: bool) {
        self.calls
            .lock()
            .unwrap()
            .notified
            .push((message.to_string(), sms));
    }
}

type TestEngine = Engine<PgAutomations, PgAutomationRuns, Fake>;

struct Rig {
    engine: Arc<TestEngine>,
    handle: EngineHandle,
    rx: mpsc::Receiver<Event>,
    calls: Arc<Mutex<Calls>>,
    store: PgStore,
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn rig(pool: PgPool, fail: bool) -> Rig {
    let store = PgStore::new(pool);
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (engine, handle, rx) = Engine::with_clock(
        store.automations(),
        store.automation_runs(),
        Fake {
            calls: Arc::clone(&calls),
            fail,
        },
        Arc::new(|| at(0)),
    );
    Rig {
        engine,
        handle,
        rx,
        calls,
        store,
    }
}

fn state(name: &str, state: &Value) -> Event {
    Event::State {
        name: name.into(),
        topic: format!("zigbee2mqtt/{name}"),
        state: state.clone(),
    }
}

fn message(topic: &str, payload: &str) -> Event {
    Event::Message {
        topic: topic.into(),
        payload: decode_payload(payload.as_bytes()),
    }
}

fn publish_action(topic: &str, payload: Value) -> Action {
    Action::MqttPublish {
        topic: topic.into(),
        payload,
    }
}

async fn add(rig: &Rig, d: AutomationDraft) -> AutomationId {
    let id = rig.store.automations().create(&d).await.unwrap().id;
    rig.engine.reload().await;
    id
}

async fn no_runs(rig: &Rig, id: AutomationId) -> bool {
    rig.store
        .automation_runs()
        .recent(id, 1)
        .await
        .unwrap()
        .is_empty()
}

async fn runs(rig: &Rig, id: AutomationId) -> Vec<(RunStatus, String, String)> {
    let mut all = rig.store.automation_runs().recent(id, 100).await.unwrap();
    all.reverse();
    all.into_iter()
        .map(|r| (r.status, r.trigger, r.message))
        .collect()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_device_state_automation_fires_on_every_matching_state_and_is_counted_and_journaled(
    pool: PgPool,
) {
    let rig = rig(pool, false);
    let id = add(
        &rig,
        AutomationDraft::new(
            "TC ouvrir",
            Trigger::DeviceState {
                friendly_name: "Tele-commande".into(),
                key: "action".into(),
                value: json!("1_single"),
            },
            publish_action("zigbee2mqtt/Volet-salon/set", json!({"state": "OPEN"})),
        ),
    )
    .await;
    for ev in [
        state("Tele-commande", &json!({"action": "1_single"})),
        state("Tele-commande", &json!({"action": "2_single"})), // other key
        state("Autre", &json!({"action": "1_single"})),         // other device
        state("Tele-commande", &json!({"battery": 90})),        // no key
        state(
            "Tele-commande",
            &json!({"action": "1_single", "battery": 80}),
        ),
    ] {
        rig.engine.process(&ev).await;
    }
    assert_eq!(
        rig.calls.lock().unwrap().published,
        vec![
            (
                "zigbee2mqtt/Volet-salon/set".to_string(),
                json!({"state": "OPEN"})
            ),
            (
                "zigbee2mqtt/Volet-salon/set".to_string(),
                json!({"state": "OPEN"})
            ),
        ]
    );
    let stored = rig.store.automations().get(id).await.unwrap().unwrap();
    assert_eq!(stored.trigger_count, 2);
    assert_eq!(stored.last_triggered, Some(at(0)));
    let journal = runs(&rig, id).await;
    assert_eq!(journal.len(), 2);
    assert_eq!(journal[0].0, RunStatus::Ok);
    assert_eq!(journal[0].1, "state Tele-commande: action = 1_single");
    assert_eq!(journal[0].2, "published on zigbee2mqtt/Volet-salon/set");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_mqtt_message_automation_uses_wildcards_the_payload_filter_and_the_condition(
    pool: PgPool,
) {
    let rig = rig(pool, false);
    let mut filter = Map::new();
    filter.insert("action".into(), json!("single"));
    let mut d = AutomationDraft::new(
        "Interrupteur",
        Trigger::MqttMessage {
            topic: "maison/+/inter".into(),
            payload_filter: filter,
        },
        Action::Scene { scene: SceneId(7) },
    );
    d.condition = Some(Condition {
        key: "battery".into(),
        value: json!(100),
    });
    let id = add(&rig, d).await;
    for (topic, payload) in [
        ("maison/salon/inter", r#"{"action":"single","battery":100}"#), // yes
        ("maison/salon/inter", r#"{"action":"single"}"#), // condition: absent key = true
        ("maison/salon/inter", r#"{"action":"single","battery":50}"#), // false condition
        ("maison/salon/inter", r#"{"action":"double","battery":100}"#), // filter
        ("maison/salon/autre", r#"{"action":"single"}"#), // topic
        ("maison/a/b/inter", r#"{"action":"single"}"#),   // extra levels
        ("maison/salon/inter", "texte"),                  // text payload: non-empty filter
    ] {
        rig.engine.process(&message(topic, payload)).await;
    }
    assert_eq!(rig.calls.lock().unwrap().scenes, [7, 7]);
    let journal = runs(&rig, id).await;
    assert_eq!(journal.len(), 2);
    assert_eq!(journal[0].1, "message maison/salon/inter");
    assert_eq!(journal[0].2, "scene 7 activee");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn notification_failure_script_and_disabled_automations(pool: PgPool) {
    let rig = rig(pool, true);
    let trigger = || Trigger::MqttMessage {
        topic: "t/#".into(),
        payload_filter: Map::new(),
    };
    let notify = add(
        &rig,
        AutomationDraft::new(
            "A notifier",
            trigger(),
            Action::Notification {
                message: String::new(),
                sms: true,
            },
        ),
    )
    .await;
    let publish = add(
        &rig,
        AutomationDraft::new(
            "A publier",
            trigger(),
            publish_action("x/set", json!({"a": 1})),
        ),
    )
    .await;
    let mut script = AutomationDraft::new("A scripter", trigger(), Action::Script);
    script.script = "publish(\"a\", 1)".into();
    let script = add(&rig, script).await;
    let mut off = AutomationDraft::new("Eteinte", trigger(), Action::Scene { scene: SceneId(1) });
    off.enabled = false;
    let off = add(&rig, off).await;

    rig.engine.process(&message("t/x", r#"{"a":1}"#)).await;

    let n = runs(&rig, notify).await;
    assert_eq!(n[0].0, RunStatus::Ok);
    assert_eq!(
        n[0].2, "notification : Automation A notifier fired",
        "message par defaut comme la reference"
    );
    assert_eq!(
        rig.calls.lock().unwrap().notified,
        [("Automation A notifier fired".to_string(), true)]
    );
    let p = runs(&rig, publish).await;
    assert_eq!(
        (p[0].0, p[0].2.as_str()),
        (RunStatus::Error, "source indisponible")
    );
    let s = runs(&rig, script).await;
    assert_eq!(
        s[0].0,
        RunStatus::Error,
        "le script a tourne, sa publication a ete refusee"
    );
    assert!(s[0].2.contains("source indisponible"), "{}", s[0].2);
    assert!(
        no_runs(&rig, off).await,
        "une automation desactivee ne se declenche pas"
    );
    // A failure counts as a trigger (the automation did trigger), like an action with no effect.
    for id in [notify, publish, script] {
        assert_eq!(
            rig.store
                .automations()
                .get(id)
                .await
                .unwrap()
                .unwrap()
                .trigger_count,
            1
        );
    }
    assert_eq!(
        rig.store
            .automations()
            .get(off)
            .await
            .unwrap()
            .unwrap()
            .trigger_count,
        0
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_loop_guard_lets_thirty_firings_per_minute_through_and_journals_the_excess_once(
    pool: PgPool,
) {
    let rig = rig(pool, false);
    let id = add(
        &rig,
        AutomationDraft::new(
            "Boucle",
            Trigger::MqttMessage {
                topic: "boucle/#".into(),
                payload_filter: Map::new(),
            },
            publish_action("boucle/x", json!({"v": 1})),
        ),
    )
    .await;
    for _ in 0..(MAX_PER_MINUTE + 25) {
        rig.engine.process(&message("boucle/x", "{}")).await;
    }
    assert_eq!(rig.calls.lock().unwrap().published.len(), MAX_PER_MINUTE);
    let stored = rig.store.automations().get(id).await.unwrap().unwrap();
    assert_eq!(
        usize::try_from(stored.trigger_count).unwrap(),
        MAX_PER_MINUTE
    );
    let journal = runs(&rig, id).await;
    assert_eq!(journal.len(), MAX_PER_MINUTE + 1);
    let limited: Vec<_> = journal
        .iter()
        .filter(|r| r.0 == RunStatus::Limited)
        .collect();
    assert_eq!(limited.len(), 1, "le depassement n'est inscrit qu'une fois");
    assert!(limited[0].2.contains("30 firings per minute"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_handle_only_queues_what_an_automation_waits_for(pool: PgPool) {
    let mut rig = rig(pool, false);
    // Nothing active: everything is dropped at the source.
    rig.handle.message("a/b", &decode_payload(b"{}"));
    rig.handle
        .state("Lampe", "z/Lampe", &json!({"state": "ON"}));
    assert!(rig.rx.try_recv().is_err());
    let with_script = |mut d: AutomationDraft| {
        d.script = "1".into();
        d
    };
    add(
        &rig,
        with_script(AutomationDraft::new(
            "M",
            Trigger::MqttMessage {
                topic: "a/#".into(),
                payload_filter: Map::new(),
            },
            Action::Script,
        )),
    )
    .await;
    add(
        &rig,
        with_script(AutomationDraft::new(
            "S",
            Trigger::DeviceState {
                friendly_name: "Lampe".into(),
                key: "state".into(),
                value: json!("ON"),
            },
            Action::Script,
        )),
    )
    .await;
    rig.handle.message("autre/b", &decode_payload(b"{}"));
    rig.handle.state("Autre", "z/Autre", &json!({}));
    assert!(rig.rx.try_recv().is_err());
    rig.handle.message("a/b", &decode_payload(b"{}"));
    rig.handle
        .state("Lampe", "z/Lampe", &json!({"state": "OFF"}));
    assert!(matches!(rig.rx.try_recv(), Ok(Event::Message { .. })));
    assert!(matches!(rig.rx.try_recv(), Ok(Event::State { .. })));
    assert_eq!(rig.handle.message_filters(), ["a/#"]);
    // Queue full: the event is dropped and counted, ingestion never waits.
    for _ in 0..(QUEUE + 3) {
        rig.handle.message("a/b", &decode_payload(b"{}"));
    }
    assert_eq!(rig.handle.dropped(), 3);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_run_loop_processes_queued_events_in_order(pool: PgPool) {
    let Rig {
        engine,
        handle,
        rx,
        calls,
        store,
    } = rig(pool, false);
    let id = store
        .automations()
        .create(&AutomationDraft::new(
            "Ordre",
            Trigger::MqttMessage {
                topic: "o/#".into(),
                payload_filter: Map::new(),
            },
            publish_action("o/out", json!({"v": 1})),
        ))
        .await
        .unwrap()
        .id;
    engine.reload().await;
    let task = tokio::spawn(Arc::clone(&engine).run(rx));
    for i in 0..5 {
        handle.message(&format!("o/{i}"), &decode_payload(b"{}"));
    }
    for _ in 0..100 {
        if store
            .automations()
            .get(id)
            .await
            .unwrap()
            .unwrap()
            .trigger_count
            == 5
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let journal = store.automation_runs().recent(id, 10).await.unwrap();
    assert_eq!(
        journal
            .iter()
            .rev()
            .map(|r| r.trigger.clone())
            .collect::<Vec<_>>(),
        [
            "message o/0",
            "message o/1",
            "message o/2",
            "message o/3",
            "message o/4"
        ]
    );
    assert_eq!(calls.lock().unwrap().published.len(), 5);
    drop(handle);
    task.abort();
}

// ---------------------------------------------------------------- production automations

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_production_remote_control_automations_move_the_shutter(pool: PgPool) {
    let rig = rig(pool, false);
    let ctx = ProdContext::parse(PROD_CONTEXT_JSON).unwrap();
    let local: BTreeMap<i64, SceneId> = BTreeMap::new();
    let mut by_name = BTreeMap::new();
    for a in &ctx.automations {
        let id = add(&rig, a.draft(&local).unwrap()).await;
        by_name.insert(a.name.clone(), id);
    }
    // The remote control sends its keys as states (`action`), followed by an empty state.
    for (action, name) in [
        ("1_single", "Ouvrir"),
        ("2_single", "Stopper"),
        ("3_single", "Fermer"),
    ] {
        rig.engine
            .process(&state(
                "Tele-commande",
                &json!({"action": action, "battery": 100}),
            ))
            .await;
        rig.engine
            .process(&state(
                "Tele-commande",
                &json!({"action": "", "battery": 100}),
            ))
            .await;
        let calls = rig.calls.lock().unwrap();
        let last = calls.published.last().unwrap();
        let expected = match name {
            "Ouvrir" => "OPEN",
            "Stopper" => "STOP",
            _ => "CLOSE",
        };
        assert_eq!(
            last,
            &(
                "zigbee2mqtt/Volet-salon/set".to_string(),
                json!({"state": expected})
            ),
            "{name}"
        );
        assert_eq!(
            calls.published.len(),
            match name {
                "Ouvrir" => 1,
                "Stopper" => 2,
                _ => 3,
            }
        );
    }
    // The production's two plugins, ported to Rhai: the left button opens, the right one closes; the relay has no `moving`
    // key, it is "moving" if its last message is less than 2 s old, and a press then stops it.
    let plugin = &by_name["Volet-SB-est ← gauche (open/stop)"];
    let other = &by_name["Volet-SB-est → droite (close/stop)"];
    for (action, relay_age, expected) in [
        ("single_left", Some(300.0), Some("OPEN")),
        ("single_left", Some(0.8), Some("STOP")),
        ("single_right", Some(1.9), Some("STOP")),
        ("single_right", None, Some("CLOSE")),
        ("double_left", Some(0.1), None),
    ] {
        let before = {
            let mut calls = rig.calls.lock().unwrap();
            calls.relay_age = relay_age;
            calls.published.len()
        };
        rig.engine
            .process(&state("Volet-SB-est", &json!({"action": action})))
            .await;
        let calls = rig.calls.lock().unwrap();
        match expected {
            Some(want) => assert_eq!(
                calls.published[before..],
                [(
                    "zigbee2mqtt/Relais-SB-est/set".to_string(),
                    json!({"state": want})
                )],
                "{action} {relay_age:?}"
            ),
            None => assert_eq!(calls.published.len(), before, "{action}"),
        }
    }
    assert_eq!(runs(&rig, *plugin).await.len(), 2);
    assert!(
        runs(&rig, *plugin)
            .await
            .iter()
            .all(|r| r.0 == RunStatus::Ok)
    );
    assert_eq!(runs(&rig, *other).await.len(), 2);
    for solar in ["Volets — coucher du soleil", "Volets — lever du soleil"] {
        assert!(no_runs(&rig, by_name[solar]).await, "{solar}");
    }
}

// ---------------------------------------------------------------- scripts

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_script_sees_the_trigger_the_devices_and_applies_its_effects_in_order(pool: PgPool) {
    let rig = rig(pool, false);
    let mut d = AutomationDraft::new(
        "Script complet",
        Trigger::DeviceState {
            friendly_name: "Volet".into(),
            key: "action".into(),
            value: json!("go"),
        },
        Action::Script,
    );
    d.script = r#"
        let v = get_device("Volet");
        publish("maison/" + device.name, #{ position: v.state.position, vu: payload.action });
        log_value("essai", 42);
        scene("Soir");
        print("fait pour " + topic);
    "#
    .into();
    let id = add(&rig, d).await;
    rig.engine
        .process(&state("Volet", &json!({"action": "go"})))
        .await;
    {
        let calls = rig.calls.lock().unwrap();
        assert_eq!(
            calls.published,
            [(
                "maison/Volet".to_string(),
                json!({"position": 100, "vu": "go"})
            )]
        );
        assert_eq!(calls.logged, [(7, "essai".to_string(), json!(42))]);
        assert_eq!(calls.scenes_by_name, ["Soir"]);
    }
    let journal = runs(&rig, id).await;
    assert_eq!(journal[0].0, RunStatus::Ok);
    assert!(
        journal[0].2.starts_with("script: 3 effect(s) applied"),
        "{}",
        journal[0].2
    );
    assert!(
        journal[0]
            .2
            .ends_with("journal : fait pour zigbee2mqtt/Volet"),
        "{}",
        journal[0].2
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_failing_or_hostile_script_applies_nothing_is_journaled_and_the_engine_carries_on(
    pool: PgPool,
) {
    let rig = rig(pool, false);
    let trigger = |k: &str| Trigger::MqttMessage {
        topic: format!("s/{k}"),
        payload_filter: Map::new(),
    };
    let make = |name: &str, topic: &str, code: &str| {
        let mut d = AutomationDraft::new(name, trigger(topic), Action::Script);
        d.script = code.into();
        d
    };
    let broken = add(
        &rig,
        make(
            "Erreur",
            "erreur",
            r#"publish("a/b", 1); let x = inconnue + 1;"#,
        ),
    )
    .await;
    let endless = add(
        &rig,
        make("Sans fin", "boucle", r#"publish("a/b", 1); loop { }"#),
    )
    .await;
    let fine = add(&rig, make("Sage", "sage", r#"publish("ok/x", 2);"#)).await;
    let started = std::time::Instant::now();
    for topic in ["s/erreur", "s/boucle", "s/sage"] {
        rig.engine.process(&message(topic, "{}")).await;
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    // The effects of a script in error do not go out; the next one's do.
    assert_eq!(
        rig.calls.lock().unwrap().published,
        [("ok/x".to_string(), json!(2))]
    );
    let e = runs(&rig, broken).await;
    assert_eq!(e[0].0, RunStatus::Error);
    assert!(e[0].2.starts_with("script failed"), "{}", e[0].2);
    let l = runs(&rig, endless).await;
    assert_eq!(l[0].0, RunStatus::Error);
    assert!(
        l[0].2.contains("too many operations") || l[0].2.contains("maximum duration"),
        "{}",
        l[0].2
    );
    assert_eq!(runs(&rig, fine).await[0].0, RunStatus::Ok);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_source_handler_runs_in_the_same_host_and_a_failing_one_is_muted_for_a_while(
    pool: PgPool,
) {
    let rig = rig(pool, false);
    let handler = |code: &str, topic: &str| Event::Handler {
        source: "Linky".into(),
        code: code.into(),
        topic: topic.into(),
        payload: decode_payload(br#"{"papp": 420}"#),
    };
    rig.engine
        .process(&handler(
            r#"publish("maison/papp", payload.papp);"#,
            "linky/compteur",
        ))
        .await;
    assert_eq!(
        rig.calls.lock().unwrap().published,
        [("maison/papp".to_string(), json!(420))]
    );
    // On failure: muted, the next message (same source) is ignored, even with correct code.
    rig.engine
        .process(&handler("inconnue()", "linky/compteur"))
        .await;
    rig.engine
        .process(&handler(r#"publish("maison/papp", 1);"#, "linky/compteur"))
        .await;
    assert_eq!(rig.calls.lock().unwrap().published.len(), 1);
}
