// SPDX-License-Identifier: AGPL-3.0-or-later
//! The configuration tools (R11 M4) through a real MCP client: the lot's acceptance (the sunset automation created end to end,
//! the snapshot differing by that automation only), the same refusals as the interface, scripts only for `administration`, what is
//! read written back, deletions that need `confirm`.

#[macro_use]
mod common;

use common::{enabled, make_token, serve};
use dominet_domain::automation::{Action, AutomationId};
use dominet_domain::device::{Device, DeviceDraft};
use dominet_domain::mcp::AuditKind;
use dominet_domain::repo::{AutomationRepo, DeviceRepo, McpAuditRepo};
use dominet_server::AppState;
use dominet_server::commands::context_document;
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use sqlx::PgPool;

macro_rules! call {
    ($client:expr, $tool:expr, $args:expr) => {{
        let args = $args.as_object().cloned().unwrap_or_default();
        $client
            .call_tool(CallToolRequestParams::new($tool).with_arguments(args))
            .await
    }};
}

macro_rules! ok {
    ($client:expr, $tool:expr, $args:expr) => {{
        let result = call!($client, $tool, $args).unwrap();
        assert_ne!(
            result.is_error,
            Some(true),
            "{}: {:?}",
            $tool,
            result.content
        );
        result.structured_content.expect("structured output")
    }};
}

/// The text of the error of a refused call.
fn refusal<T: std::fmt::Debug>(result: Result<T, rmcp::ServiceError>) -> String {
    format!("{:?}", result.expect_err("the call should be refused"))
}

async fn device(st: &AppState, name: &str) -> Device {
    st.store
        .devices()
        .create(&DeviceDraft::new(name))
        .await
        .unwrap()
}

async fn lines(st: &AppState, kind: AuditKind) -> Vec<dominet_domain::mcp::McpAuditEntry> {
    st.store.mcp_audit().recent(Some(kind), 20).await.unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_sunset_automation_is_created_end_to_end_and_only_it_changes(pool: PgPool) {
    let st = enabled(pool);
    device(&st, "Volet-salon").await;
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let before = context_document(&st.store).await.unwrap();

    // The assistant discovers what exists, then reads the schema and uses the example it is given.
    let kinds = ok!(client, "config_kinds", json!({}));
    assert_eq!(kinds["kinds"].as_array().unwrap().len(), 16);
    let schema = ok!(client, "config_schema", json!({"kind": "automation"}));
    assert!(schema["schema"]["properties"]["trigger"].is_object());
    assert!(schema["notes"].to_string().contains("sunset"));
    let data = schema["example"].clone();
    assert_eq!(data["name"], "Volets au coucher du soleil");

    // A dry run says what it would create and writes nothing.
    let dry = ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "data": data, "dry_run": true})
    );
    assert_eq!(
        (dry["dry_run"].as_bool(), dry["done"].as_bool()),
        (Some(true), Some(false))
    );
    assert!(
        dry["summary"]
            .as_str()
            .unwrap()
            .starts_with("Would create automation")
    );
    assert!(st.store.automations().list().await.unwrap().is_empty());
    assert!(lines(&st, AuditKind::ToolCall).await.is_empty());

    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "data": data})
    );
    assert_eq!(done["done"], true, "{done}");
    let id = done["details"]["id"].as_i64().unwrap();
    let saved = st
        .store
        .automations()
        .get(AutomationId(id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.draft.name, "Volets au coucher du soleil");
    assert!(saved.draft.enabled);

    // Acceptance: the snapshot before and after differs by the automation, and by nothing else.
    let after = context_document(&st.store).await.unwrap();
    let (b, a) = (before.as_object().unwrap(), after.as_object().unwrap());
    assert_eq!(b.keys().collect::<Vec<_>>(), a.keys().collect::<Vec<_>>());
    let changed: Vec<_> = a.keys().filter(|k| a[*k] != b[*k]).collect();
    assert_eq!(
        changed,
        ["automations"],
        "only the automations section may change"
    );
    let (was, now) = (
        b["automations"].as_array().unwrap(),
        a["automations"].as_array().unwrap(),
    );
    assert_eq!(now.len(), was.len() + 1);
    let added = now
        .iter()
        .find(|x| x["name"] == "Volets au coucher du soleil")
        .unwrap();
    assert_eq!(
        (
            added["trigger_type"].as_str(),
            added["action_type"].as_str()
        ),
        (Some("solar"), Some("mqtt_publish"))
    );
    assert_eq!(added["trigger_config"]["event"], "sunset");

    // One audit line, written before and closed, with the payload (the state key stays readable).
    let audit = lines(&st, AuditKind::ToolCall).await;
    assert_eq!(
        (
            audit.len(),
            audit[0].tool.as_str(),
            audit[0].outcome.as_str()
        ),
        (1, "config_upsert", "ok")
    );
    assert_eq!(
        audit[0].params["data"]["name"],
        "Volets au coucher du soleil"
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_payload_is_refused_for_the_same_reasons_as_in_the_interface_and_a_dry_run_agrees(
    pool: PgPool,
) {
    let st = enabled(pool);
    device(&st, "Volet-salon").await;
    let token = make_token(
        &st,
        "admin",
        &["lecture", "configuration", "administration"],
    )
    .await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "typo in a field",
            json!({"name": "A", "enabeld": true, "trigger": {"type": "schedule", "config": {"cron": "0 7 * * *"}}, "action": {"type": "script"}}),
            "enabeld",
        ),
        (
            "unknown trigger type",
            json!({"name": "A", "trigger": {"type": "lune"}, "action": {"type": "script"}}),
            "trigger_type",
        ),
        (
            "missing name",
            json!({"name": "  ", "trigger": {"type": "schedule", "config": {"cron": "0 7 * * *"}}, "action": {"type": "notification", "config": {"message": "x"}}}),
            "name",
        ),
        (
            "latitude out of range",
            json!({"name": "A", "trigger": {"type": "solar", "config": {"event": "sunset", "latitude": 143.0, "longitude": 5.4}}, "action": {"type": "notification", "config": {"message": "x"}}}),
            "latitude",
        ),
        (
            "scene that does not exist",
            json!({"name": "A", "trigger": {"type": "schedule", "config": {"cron": "0 7 * * *"}}, "action": {"type": "scene", "config": {"scene_id": 999}}}),
            "scene_id",
        ),
        (
            "script that does not compile",
            json!({"name": "A", "trigger": {"type": "schedule", "config": {"cron": "0 7 * * *"}}, "action": {"type": "script"}, "script": "if ("}),
            "syntax",
        ),
        (
            "command that is not an object",
            json!({"name": "A", "trigger": {"type": "schedule", "config": {"cron": "0 7 * * *"}}, "action": {"type": "mqtt_publish", "config": {"topic": "a/b", "payload": "OPEN"}}}),
            "action_payload",
        ),
    ];
    for (what, data, expected) in cases {
        for dry_run in [true, false] {
            let message = refusal(call!(
                client,
                "config_upsert",
                json!({"kind": "automation", "data": data.clone(), "dry_run": dry_run})
            ));
            assert!(
                message.contains(expected),
                "{what} (dry_run={dry_run}): {message}"
            );
        }
    }
    assert!(
        st.store.automations().list().await.unwrap().is_empty(),
        "nothing is written by a refused call"
    );
    assert!(
        lines(&st, AuditKind::ToolCall).await.is_empty(),
        "a refusal is not an action"
    );
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "frigo", "data": {}})
        ))
        .contains("unknown type")
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn writing_a_script_needs_administration_and_an_update_never_erases_the_code(pool: PgPool) {
    let st = enabled(pool);
    let writer = make_token(&st, "configurateur", &["lecture", "configuration"]).await;
    let admin = make_token(
        &st,
        "administrateur",
        &["lecture", "configuration", "administration"],
    )
    .await;
    let addr = serve(&st).await;
    let code = "// MARQUEUR-SECRET-DU-SCRIPT\npublish(\"maison/lumiere\", \"OFF\");";
    let data = json!({
        "name": "Script nuit",
        "trigger": {"type": "mqtt_message", "config": {"topic": "maison/nuit"}},
        "action": {"type": "script"},
        "script": code,
    });

    // `configuration` alone cannot write code, whether it asks for a dry run or not, and the refusal is recorded.
    let client = connect!(addr, writer.clone());
    for dry_run in [true, false] {
        let message = refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "automation", "data": data.clone(), "dry_run": dry_run})
        ));
        assert!(message.contains("administration"), "{message}");
    }
    assert!(st.store.automations().list().await.unwrap().is_empty());
    assert_eq!(lines(&st, AuditKind::Denied).await.len(), 2);
    client.cancel().await.unwrap();

    // `administration` can; the code is stored but never returned.
    let client = connect!(addr, admin.clone());
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "data": data})
    );
    assert_eq!(done["done"], true, "{done}");
    let id = done["details"]["id"].as_i64().unwrap();
    assert!(
        !done.to_string().contains("MARQUEUR-SECRET"),
        "the answer must not carry the code"
    );
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "automation", "item": "script nuit"})
    );
    assert_eq!(
        (
            got["data"]["script"].is_null(),
            got["data"]["has_script"].as_bool()
        ),
        (true, Some(true))
    );
    assert!(!got.to_string().contains("MARQUEUR-SECRET"));
    client.cancel().await.unwrap();

    // What is read is written back with `configuration` alone: the code is kept (the payload does not mention it).
    let client = connect!(addr, writer);
    let mut back = got["data"].clone();
    back.as_object_mut().unwrap().remove("script");
    back.as_object_mut().unwrap().remove("has_script");
    back["enabled"] = json!(false);
    let dry = ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "id": id, "data": back.clone(), "dry_run": true})
    );
    let fields: Vec<_> = dry["details"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(fields, ["enabled"], "only the field that changed: {dry}");
    ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "id": id, "data": back})
    );
    let kept = st
        .store
        .automations()
        .get(AutomationId(id))
        .await
        .unwrap()
        .unwrap();
    assert!(!kept.draft.enabled);
    assert!(matches!(kept.draft.action, Action::Script));
    assert_eq!(
        kept.draft.script, code,
        "the code must survive an update that does not mention it"
    );

    // Removing the code (another action) is also a change of code: refused to `configuration` alone.
    let drop = json!({
        "name": "Script nuit",
        "trigger": {"type": "mqtt_message", "config": {"topic": "maison/nuit"}},
        "action": {"type": "notification", "config": {"message": "x"}},
    });
    let message = refusal(call!(
        client,
        "config_upsert",
        json!({"kind": "automation", "id": id, "data": drop})
    ));
    assert!(message.contains("administration"), "{message}");
    assert_eq!(
        st.store
            .automations()
            .get(AutomationId(id))
            .await
            .unwrap()
            .unwrap()
            .draft
            .script,
        code
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn places_scenes_and_alert_rules_are_written_listed_and_replaced(pool: PgPool) {
    let st = enabled(pool);
    device(&st, "Volet-salon").await;
    device(&st, "Cave").await;
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    ok!(
        client,
        "config_upsert",
        json!({"kind": "location", "data": {"name": "Salon", "description": "RDC"}})
    );
    let scene = ok!(
        client,
        "config_upsert",
        json!({"kind": "scene", "data": {
            "name": "Fermeture du soir", "location": "salon",
            "actions": [{"device": "Volet-salon", "command": {"state": "CLOSE"}}],
        }})
    );
    assert_eq!(scene["done"], true, "{scene}");
    let rule = ok!(
        client,
        "config_upsert",
        json!({"kind": "alert_rule", "data": {
            "name": "Trop humide", "device": "cave", "key": "humidity", "operator": "gt", "threshold_num": 80, "severity": "critical",
        }})
    );
    assert_eq!(rule["done"], true, "{rule}");

    // Listed in the shape that can be written back; a scene shows its place and its devices by name.
    let scenes = ok!(client, "config_list", json!({"kind": "scene"}));
    assert_eq!(scenes["items"][0]["data"]["location"], "Salon");
    assert_eq!(
        scenes["items"][0]["data"]["actions"][0]["device"],
        "Volet-salon"
    );
    let rules = ok!(
        client,
        "config_get",
        json!({"kind": "alert_rule", "item": "trop humide"})
    );
    assert_eq!(
        (
            rules["data"]["device"].as_str(),
            rules["data"]["operator"].as_str(),
            rules["data"]["severity"].as_str()
        ),
        (Some("Cave"), Some("gt"), Some("critical"))
    );

    // Replaced by id: read, change one field, write back.
    let mut data = rules["data"].clone();
    data["threshold_num"] = json!(75);
    let id = rules["id"].as_i64().unwrap();
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "alert_rule", "id": id, "data": data})
    );
    assert_eq!(done["done"], true);
    let again = ok!(
        client,
        "config_get",
        json!({"kind": "alert_rule", "item": id.to_string()})
    );
    assert_eq!(again["data"]["threshold_num"], 75.0);

    // Refusals: a scene name is unique, an unknown device and an unknown id are explained.
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "scene", "data": {"name": "FERMETURE DU SOIR"}})
        ))
        .contains("already taken")
    );
    assert!(refusal(call!(client, "config_upsert", json!({"kind": "scene", "data": {"name": "X", "actions": [{"device": "Fantome", "command": {"state": "ON"}}]}}))).contains("not found"));
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "location", "id": 999_999, "data": {"name": "X"}})
        ))
        .contains("not found")
    );
    assert!(refusal(call!(client, "config_upsert", json!({"kind": "alert_rule", "data": {"name": "X", "device": "Cave", "key": "t", "operator": "pareil"}}))).contains("operator"));
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_deletion_needs_confirm_and_a_dry_run_says_what_depends_on_it(pool: PgPool) {
    let st = enabled(pool);
    device(&st, "Volet-salon").await;
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let scene = ok!(
        client,
        "config_upsert",
        json!({"kind": "scene", "data": {"name": "Soiree", "actions": [{"device": "Volet-salon", "command": {"state": "CLOSE"}}]}})
    );
    let scene_id = scene["details"]["id"].as_i64().unwrap();
    ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "data": {
            "name": "Lancer la soiree", "trigger": {"type": "schedule", "config": {"cron": "0 19 * * *"}},
            "action": {"type": "scene", "config": {"scene_id": scene_id}},
        }})
    );

    // Said first: the automation that runs this scene would fail afterwards.
    let dry = ok!(
        client,
        "config_delete",
        json!({"kind": "scene", "item": "soiree", "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert!(
        dry["details"]["impact"]
            .to_string()
            .contains("Lancer la soiree"),
        "{dry}"
    );
    // Without `confirm` the call is refused, and the scene is still there.
    let message = refusal(call!(
        client,
        "config_delete",
        json!({"kind": "scene", "item": "soiree"})
    ));
    assert!(message.contains("confirm"), "{message}");
    assert_eq!(
        ok!(client, "config_list", json!({"kind": "scene"}))["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        lines(&st, AuditKind::ToolCall)
            .await
            .iter()
            .all(|l| l.tool != "config_delete")
    );

    let done = ok!(
        client,
        "config_delete",
        json!({"kind": "scene", "item": "soiree", "confirm": true})
    );
    assert_eq!(done["done"], true, "{done}");
    assert!(
        ok!(client, "config_list", json!({"kind": "scene"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let audit = lines(&st, AuditKind::ToolCall).await;
    assert_eq!(
        (audit[0].tool.as_str(), audit[0].outcome.as_str()),
        ("config_delete", "ok")
    );
    // An unknown item is explained.
    assert!(
        refusal(call!(
            client,
            "config_delete",
            json!({"kind": "scene", "item": "soiree", "confirm": true})
        ))
        .contains("not found")
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_read_token_reads_the_configuration_but_cannot_write_it(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "lecteur", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for read in ["config_kinds", "config_schema", "config_list", "config_get"] {
        assert!(names.contains(&read.to_string()), "{read}");
    }
    for write in ["config_upsert", "config_delete", "zigbee_pairing"] {
        assert!(!names.contains(&write.to_string()), "{write}");
    }
    let refused = call!(
        client,
        "config_upsert",
        json!({"kind": "location", "data": {"name": "X"}})
    )
    .unwrap();
    assert_eq!(refused.is_error, Some(true));
    assert!(
        ok!(client, "config_list", json!({"kind": "location"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let denied = lines(&st, AuditKind::Denied).await;
    assert_eq!(
        (denied.len(), denied[0].tool.as_str()),
        (1, "config_upsert")
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn zigbee_pairing_is_a_configuration_action_with_a_dry_run(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "assistant", &["configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let dry = ok!(
        client,
        "zigbee_pairing",
        json!({"enabled": true, "dry_run": true})
    );
    assert!(dry["summary"].as_str().unwrap().contains("254"));
    assert!(lines(&st, AuditKind::ToolCall).await.is_empty());
    // No broker in this server: said, and recorded as an error rather than hidden.
    let done = ok!(client, "zigbee_pairing", json!({"enabled": true}));
    assert_eq!(done["done"], false);
    assert!(done["summary"].as_str().unwrap().contains("broker"));
    assert_eq!(lines(&st, AuditKind::ToolCall).await[0].outcome, "error");
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn groups_labels_sources_and_stations_go_through_the_same_checks(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    ok!(
        client,
        "config_upsert",
        json!({"kind": "location", "data": {"name": "Salon"}})
    );
    let group = ok!(
        client,
        "config_upsert",
        json!({"kind": "device_group", "data": {"name": "Volets", "location": "salon"}})
    );
    assert_eq!(group["done"], true, "{group}");
    assert_eq!(
        ok!(
            client,
            "config_get",
            json!({"kind": "device_group", "item": "volets"})
        )["data"]["location"],
        "Salon"
    );
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "device_group", "data": {"name": "VOLETS"}})
        ))
        .contains("already taken")
    );

    let label = ok!(
        client,
        "config_upsert",
        json!({"kind": "device_label", "data": {"key": "temperature", "name": "Temperature", "icon_name": "thermometer", "unit": "C"}})
    );
    assert_eq!(label["done"], true, "{label}");
    assert!(refusal(call!(client, "config_upsert", json!({"kind": "device_label", "data": {"key": "temperature", "name": "Autre", "icon_name": "x"}}))).contains("already taken"));
    assert!(refusal(call!(client, "config_upsert", json!({"kind": "device_label", "data": {"key": "Pas Valide", "name": "x", "icon_name": "x"}}))).contains("key"));

    let station = ok!(
        client,
        "config_upsert",
        json!({"kind": "weather_station", "data": {"name": "Maison", "latitude": 43.3, "longitude": 5.4}})
    );
    assert_eq!(station["done"], true, "{station}");
    assert!(refusal(call!(client, "config_upsert", json!({"kind": "weather_station", "data": {"name": "Ailleurs", "latitude": 143.0, "longitude": 5.4}}))).contains("latitude"));

    let source = ok!(
        client,
        "config_upsert",
        json!({"kind": "mqtt_source", "data": {"name": "Maison", "topic_pattern": "maison/+/etat", "auto_device": true}})
    );
    assert_eq!(source["done"], true, "{source}");
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "mqtt_source", "data": {"name": "X", "topic_pattern": "maison/#/x"}})
        ))
        .contains("topic_pattern")
    );
    assert!(refusal(call!(client, "config_upsert", json!({"kind": "mqtt_source", "data": {"name": "X", "topic_pattern": "a/b", "auto_device_type": "licorne"}}))).contains("auto_device_type"));

    for kind in [
        "device_group",
        "device_label",
        "mqtt_source",
        "weather_station",
    ] {
        assert_eq!(
            ok!(client, "config_list", json!({"kind": kind}))["items"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "{kind}"
        );
    }
    let done = ok!(
        client,
        "config_delete",
        json!({"kind": "device_group", "item": "volets", "confirm": true})
    );
    assert_eq!(done["done"], true, "{done}");
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_handler_code_of_a_source_needs_administration_and_is_never_shown(pool: PgPool) {
    let st = enabled(pool);
    let writer = make_token(&st, "configurateur", &["lecture", "configuration"]).await;
    let admin = make_token(
        &st,
        "administrateur",
        &["lecture", "configuration", "administration"],
    )
    .await;
    let addr = serve(&st).await;
    let code = "// MARQUEUR-HANDLER\nlet x = 1;";
    let data = json!({"name": "Avec code", "topic_pattern": "a/b", "handler_code": code});

    let client = connect!(addr, writer.clone());
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "mqtt_source", "data": data.clone()})
        ))
        .contains("administration")
    );
    client.cancel().await.unwrap();

    let client = connect!(addr, admin);
    let bad = json!({"name": "Casse", "topic_pattern": "a/c", "handler_code": "if ("});
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "mqtt_source", "data": bad})
        ))
        .contains("handler_code")
    );
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "mqtt_source", "data": data})
    );
    assert_eq!(done["done"], true, "{done}");
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "mqtt_source", "item": "avec code"})
    );
    assert_eq!(
        (
            got["data"]["handler_code"].is_null(),
            got["data"]["has_handler_code"].as_bool()
        ),
        (true, Some(true))
    );
    assert!(!got.to_string().contains("MARQUEUR-HANDLER"));
    client.cancel().await.unwrap();

    // Read, change a field, write back with `configuration` alone: the code is kept.
    let client = connect!(addr, writer);
    let mut back = got["data"].clone();
    for k in ["handler_code", "has_handler_code"] {
        back.as_object_mut().unwrap().remove(k);
    }
    back["description"] = json!("modifie");
    let id = got["id"].as_i64().unwrap();
    ok!(
        client,
        "config_upsert",
        json!({"kind": "mqtt_source", "id": id, "data": back})
    );
    use dominet_domain::device::MqttSourceId;
    use dominet_domain::repo::MqttSourceRepo;
    let kept = st
        .store
        .mqtt_sources()
        .get(MqttSourceId(id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (kept.description.as_str(), kept.handler_code.as_str()),
        ("modifie", code.replace("\\n", "\n").as_str())
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn prometheus_and_grafana_rules_are_written_with_the_domain_checks(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let prom =
        json!({"name": "CaveTropHumide", "group": "maison", "expr": "up == 0", "summary": "Cave"});
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "prometheus_rule", "data": prom.clone()})
    );
    assert_eq!(done["done"], true, "{done}");
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "prometheus_rule", "item": "cavetrophumide"})
    );
    assert_eq!(
        (
            got["data"]["severity"].as_str(),
            got["data"]["for_duration"].as_str()
        ),
        (Some("warning"), Some("5m"))
    );
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "prometheus_rule", "data": prom.clone()})
        ))
        .contains("already taken")
    );
    let mut bad = prom.clone();
    bad["name"] = json!("Autre");
    bad["for_duration"] = json!("cinq minutes");
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "prometheus_rule", "data": bad.clone(), "dry_run": true})
        ))
        .contains("for_duration")
    );
    bad["for_duration"] = json!("5m");
    bad["severity"] = json!("grave");
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "prometheus_rule", "data": bad})
        ))
        .contains("severity")
    );

    let graf = json!({"title": "Cave trop humide", "folder": "Maison", "group": "cave", "expr": "up == 0", "summary": "Cave"});
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "grafana_rule", "data": graf.clone()})
    );
    assert_eq!(done["done"], true, "{done}");
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "grafana_rule", "item": "Cave trop humide"})
    );
    assert_eq!(
        (
            got["data"]["datasource_uid"].as_str(),
            got["data"]["uid"].as_str()
        ),
        (Some("prometheus"), Some("cave-trop-humide"))
    );
    let mut bad = graf.clone();
    bad["title"] = json!("Autre");
    bad["query_window"] = json!(0);
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "grafana_rule", "data": bad})
        ))
        .contains("query_window")
    );

    let done = ok!(
        client,
        "config_delete",
        json!({"kind": "prometheus_rule", "item": "CaveTropHumide", "confirm": true})
    );
    assert_eq!(done["done"], true);
    assert!(
        ok!(client, "config_list", json!({"kind": "prometheus_rule"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_grafana_rule_that_runs_sql_needs_administration(pool: PgPool) {
    let st = enabled(pool);
    let writer = make_token(&st, "configurateur", &["lecture", "configuration"]).await;
    let admin = make_token(
        &st,
        "administrateur",
        &["lecture", "configuration", "administration"],
    )
    .await;
    let addr = serve(&st).await;
    let sql = json!({"title": "Requete SQL", "folder": "Maison", "group": "sql", "expr": "SELECT 1", "summary": "x", "datasource_uid": "postgres"});

    let client = connect!(addr, writer.clone());
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "grafana_rule", "data": sql.clone()})
        ))
        .contains("administration")
    );
    client.cancel().await.unwrap();

    let client = connect!(addr, admin);
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "grafana_rule", "data": sql.clone()})
    );
    assert_eq!(done["done"], true, "{done}");
    let id = done["details"]["id"].as_i64().unwrap();
    client.cancel().await.unwrap();

    // `configuration` alone may change anything but the query of a SQL rule.
    let client = connect!(addr, writer);
    let mut same = sql.clone();
    same["summary"] = json!("autre resume");
    ok!(
        client,
        "config_upsert",
        json!({"kind": "grafana_rule", "id": id, "data": same})
    );
    let mut other = sql.clone();
    other["expr"] = json!("SELECT 2");
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "grafana_rule", "id": id, "data": other})
        ))
        .contains("administration")
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn dashboards_and_their_widgets_are_placed_on_the_grid_without_overlap(pool: PgPool) {
    let st = enabled(pool);
    common::admin_account(&st, "alice").await;
    device(&st, "Lampe-salon").await;
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    // The owner is required, must exist, and never changes.
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "dashboard", "data": {"name": "Maison"}})
        ))
        .contains("owner")
    );
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "dashboard", "data": {"name": "Maison", "owner": "personne"}})
        ))
        .contains("not found")
    );
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "dashboard", "data": {"name": "Maison", "owner": "alice"}})
    );
    assert_eq!(done["done"], true, "{done}");
    let id = done["details"]["id"].as_i64().unwrap();
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "dashboard", "item": "alice/maison"})
    );
    assert_eq!(
        (
            got["data"]["owner"].as_str(),
            got["data"]["is_default"].as_bool()
        ),
        (Some("alice"), Some(true))
    );
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "dashboard", "id": id, "data": {"name": "Maison", "owner": "bob"}})
        ))
        .contains("owner")
    );

    let widget = json!({"dashboard": "alice/Maison", "type": "device", "title": "Salon", "device": "lampe-salon", "col": 0, "row": 0, "width": 2, "height": 2});
    let dry = ok!(
        client,
        "config_upsert",
        json!({"kind": "widget", "data": widget.clone(), "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert!(
        ok!(client, "config_list", json!({"kind": "widget"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "widget", "data": widget.clone()})
    );
    assert_eq!(done["done"], true, "{done}");
    let wid = done["details"]["id"].as_i64().unwrap();

    // Overlap, outside the grid, unknown device, a widget that would move to another dashboard: all said before writing.
    let mut over = widget.clone();
    over["col"] = json!(1);
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "widget", "data": over, "dry_run": true})
        ))
        .contains("overlaps")
    );
    let mut out = widget.clone();
    out["col"] = json!(11);
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "widget", "data": out})
        ))
        .contains("width")
    );
    let mut ghost = widget.clone();
    ghost["col"] = json!(4);
    ghost["device"] = json!("fantome");
    assert!(
        refusal(call!(
            client,
            "config_upsert",
            json!({"kind": "widget", "data": ghost})
        ))
        .contains("not found")
    );

    // Read, move it, write it back: it does not overlap itself.
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "widget", "item": wid.to_string()})
    );
    assert_eq!(got["data"]["device"], "Lampe-salon");
    let mut moved = got["data"].clone();
    moved["col"] = json!(1);
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "widget", "id": wid, "data": moved})
    );
    assert_eq!(done["done"], true, "{done}");

    // A text widget is sanitised.
    let text = json!({"dashboard": "alice/Maison", "type": "text", "col": 6, "row": 0,
                      "content": "<p>bonjour</p><script>alert(1)</script>"});
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "widget", "data": text})
    );
    let tid = done["details"]["id"].as_i64().unwrap();
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "widget", "item": tid.to_string()})
    );
    let content = got["data"]["content"].as_str().unwrap();
    assert!(
        content.contains("bonjour") && !content.contains("script"),
        "{content}"
    );

    // Deleting the dashboard says what goes with it, and needs confirm.
    let dry = ok!(
        client,
        "config_delete",
        json!({"kind": "dashboard", "item": "alice/Maison", "dry_run": true})
    );
    assert!(
        dry["details"]["impact"].to_string().contains("2 widget"),
        "{dry}"
    );
    assert!(
        refusal(call!(
            client,
            "config_delete",
            json!({"kind": "dashboard", "item": "alice/Maison"})
        ))
        .contains("confirm")
    );
    let done = ok!(
        client,
        "config_delete",
        json!({"kind": "dashboard", "item": "alice/Maison", "confirm": true})
    );
    assert_eq!(done["done"], true);
    assert!(
        ok!(client, "config_list", json!({"kind": "widget"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn devices_navigation_and_the_detector_are_configured_like_the_interface(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "assistant", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    // A device: the group and the place are named, the type is checked, a taken name is said at the dry run.
    ok!(
        client,
        "config_upsert",
        json!({"kind": "location", "data": {"name": "Salon"}})
    );
    ok!(
        client,
        "config_upsert",
        json!({"kind": "device_group", "data": {"name": "Volets", "location": "Salon"}})
    );
    let payload = json!({"friendly_name": "Volet-salon", "device_type": "cover", "group": "volets",
        "location": "salon", "base_topic": "zigbee2mqtt/", "tracked_keys": ["position", " "]});
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "device", "data": payload.clone()})
    );
    assert_eq!(done["done"], true, "{done}");
    let id = done["details"]["id"].as_i64().unwrap();
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "device", "item": "VOLET-SALON"})
    );
    assert_eq!(
        got["data"],
        json!({"friendly_name": "Volet-salon", "description": "", "device_type": "cover", "group": "Volets",
               "location": "Salon", "base_topic": "zigbee2mqtt", "active": true, "icon": "", "image_url": "",
               "tracked_keys": ["position"], "hidden_keys": [], "grafana_dashboard": false})
    );
    // What is read is what can be written back.
    let mut again = got["data"].clone();
    again["description"] = json!("Volet roulant");
    let changed = ok!(
        client,
        "config_upsert",
        json!({"kind": "device", "id": id, "data": again, "dry_run": true})
    );
    assert_eq!(changed["details"]["changes"][0]["field"], "description");
    for (data, needle) in [
        (
            json!({"friendly_name": "X", "device_type": "frigo"}),
            "device_type",
        ),
        (
            json!({"friendly_name": "X", "device_type": "cover", "group": "nulle part"}),
            "group not found",
        ),
        (
            json!({"friendly_name": "Volet-salon", "device_type": "cover"}),
            "already taken",
        ),
    ] {
        assert!(
            refusal(call!(
                client,
                "config_upsert",
                json!({"kind": "device", "data": data})
            ))
            .contains(needle),
            "{needle}"
        );
    }

    // Navigation: an icon that Lucide does not know is refused, as in the interface.
    for (data, needle) in [
        (
            json!({"icon_name": "n-existe-pas-du-tout", "label": "A", "url": "/a"}),
            "icon_name",
        ),
        (
            json!({"icon_name": "home", "label": "A", "url": "javascript:alert(1)"}),
            "url",
        ),
        (
            json!({"icon_name": "home", "label": "A", "url": "/a", "position": "left"}),
            "position",
        ),
    ] {
        assert!(
            refusal(call!(
                client,
                "config_upsert",
                json!({"kind": "navbar", "data": data})
            ))
            .contains(needle),
            "{needle}"
        );
    }
    let nav = ok!(
        client,
        "config_upsert",
        json!({"kind": "navbar", "data": {"icon_name": "house", "label": "Accueil 2", "url": "/home", "position": "bottom", "sort_order": 3}})
    );
    assert_eq!(nav["done"], true, "{nav}");
    let nav_id = nav["details"]["id"].as_i64().unwrap();
    let got = ok!(
        client,
        "config_get",
        json!({"kind": "navbar", "item": "accueil 2"})
    );
    assert_eq!(
        got["data"],
        json!({"icon_name": "house", "label": "Accueil 2", "url": "/home", "url_target": "_self",
               "admin_only": false, "position": "bottom", "sort_order": 3})
    );

    // The detector is one item: changed, never created or deleted.
    let before = ok!(
        client,
        "config_get",
        json!({"kind": "detector", "item": "detector"})
    );
    let was = before["data"]["enabled"].as_bool().unwrap();
    let flipped = ok!(
        client,
        "config_upsert",
        json!({"kind": "detector", "data": {"enabled": !was, "save_frames": true}})
    );
    assert_eq!(flipped["done"], true, "{flipped}");
    let after = ok!(
        client,
        "config_get",
        json!({"kind": "detector", "item": "1"})
    );
    assert_eq!(after["data"], json!({"enabled": !was, "save_frames": true}));
    assert!(
        refusal(call!(
            client,
            "config_delete",
            json!({"kind": "detector", "item": "detector", "confirm": true})
        ))
        .contains("cannot be deleted")
    );

    // A device deletion says what goes with it, and needs confirm.
    let dry = ok!(
        client,
        "config_delete",
        json!({"kind": "device", "item": "Volet-salon", "dry_run": true})
    );
    assert!(
        dry["details"]["impact"].to_string().contains("history"),
        "{dry}"
    );
    assert!(
        refusal(call!(
            client,
            "config_delete",
            json!({"kind": "device", "item": "Volet-salon"})
        ))
        .contains("confirm")
    );
    for (kind, item) in [("device", id.to_string()), ("navbar", nav_id.to_string())] {
        let done = ok!(
            client,
            "config_delete",
            json!({"kind": kind, "item": item, "confirm": true})
        );
        assert_eq!(done["done"], true, "{done}");
    }
    client.cancel().await.unwrap();
}
