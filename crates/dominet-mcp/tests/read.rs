// SPDX-License-Identifier: AGPL-3.0-or-later
//! The read tools (R11 M2) through a real MCP client, on real data: the two questions the lot is accepted on ("which devices have
//! said nothing for 24 hours?", "why did this automation fail?"), and what must never show (script code, credentials).

#[macro_use]
mod common;

use chrono::{Duration, Utc};
use common::{enabled, make_token, serve};
use dominet_domain::automation::{AutomationDraft, AutomationRun, RunStatus};
use dominet_domain::device::{Device, DeviceDraft, LogEntry};
use dominet_domain::repo::{
    AutomationRepo, AutomationRunRepo, DeviceRepo, SceneRepo, StateLogRepo,
};
use dominet_domain::scene::{SceneActionDraft, SceneDraft};
use dominet_server::AppState;
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams};
use serde_json::{Value, json};
use sqlx::PgPool;

/// Calls a tool with JSON arguments and returns its structured output (the call must succeed).
macro_rules! call {
    ($client:expr, $tool:expr, $args:expr) => {{
        let args = $args.as_object().cloned().unwrap_or_default();
        let result = $client
            .call_tool(CallToolRequestParams::new($tool).with_arguments(args))
            .await
            .unwrap();
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

/// A device last heard `hours_ago` hours ago (`None`: never), reachable or not.
async fn device(st: &AppState, name: &str, hours_ago: Option<i32>, available: bool) -> Device {
    let d = st
        .store
        .devices()
        .create(&DeviceDraft::new(name))
        .await
        .unwrap();
    sqlx::query("UPDATE device SET available = $2 WHERE id = $1")
        .bind(d.id.0)
        .bind(available)
        .execute(&st.pool)
        .await
        .unwrap();
    if let Some(h) = hours_ago {
        sqlx::query(
            "UPDATE device SET last_seen = now() - make_interval(hours => $2) WHERE id = $1",
        )
        .bind(d.id.0)
        .bind(h)
        .execute(&st.pool)
        .await
        .unwrap();
    }
    d
}

fn automation(
    name: &str,
    trigger: (&str, Value),
    action: (&str, Value),
    script: &str,
) -> AutomationDraft {
    AutomationDraft::from_parts(
        name,
        true,
        "",
        trigger.0,
        &trigger.1,
        &Value::Null,
        action.0,
        &action.1,
        script,
    )
    .unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_read_token_sees_every_read_tool_with_a_description_and_an_output_schema(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "diagnostic", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    // A `lecture` token sees exactly the read tools; the control tools are for `pilotage`.
    let mut declared: Vec<String> = dominet_mcp::SCOPES
        .iter()
        .filter(|(_, scope)| *scope == dominet_domain::mcp::Scope::Read)
        .map(|(n, _)| (*n).to_string())
        .collect();
    declared.sort();
    assert_eq!(names, declared);
    assert!(names.len() >= 11, "{names:?}");
    for t in &tools {
        assert!(
            t.description.as_deref().is_some_and(|d| d.len() > 30),
            "{}: description",
            t.name
        );
        // Every tool describes its answer, except the one that returns a picture (an image has no JSON shape).
        assert!(
            t.output_schema.is_some() || t.name == "snapshot_get",
            "{}: output schema",
            t.name
        );
    }
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_token_without_the_read_scope_sees_none_of_them(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "configurateur", &["configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let tools = client.list_all_tools().await.unwrap();
    assert!(
        tools
            .iter()
            .all(|t| t.name.starts_with("config_") || t.name == "zigbee_pairing"),
        "only configuration tools: {:?}",
        tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>()
    );
    assert!(
        !tools
            .iter()
            .any(|t| t.name == "devices_list" || t.name == "config_list")
    );
    let refused = client
        .call_tool(CallToolRequestParams::new("devices_list"))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true));
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn which_devices_have_said_nothing_for_24_hours(pool: PgPool) {
    let st = enabled(pool);
    device(&st, "Salon", Some(1), true).await;
    device(&st, "Cave", Some(30), true).await;
    device(&st, "Garage", None, false).await;
    device(&st, "Bureau", Some(25), false).await;
    let token = make_token(&st, "diagnostic", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let v = call!(client, "devices_list", json!({"silent_for_hours": 24}));
    assert_eq!(
        (v["total"].as_u64(), v["truncated"].as_bool()),
        (Some(3), Some(false))
    );
    let names: Vec<_> = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    // Never heard first, then the longest silence; Salon (1 h) is not silent.
    assert_eq!(names, ["Garage", "Cave", "Bureau"]);
    assert!(v["devices"][0]["silent_hours"].is_null());
    assert!(v["devices"][0]["last_seen"].is_null());
    let cave = v["devices"][1]["silent_hours"].as_f64().unwrap();
    assert!((29.9..=30.2).contains(&cave), "{cave}");

    // Filters combine; the notice reminds the assistant that names are data.
    let v = call!(
        client,
        "devices_list",
        json!({"available": false, "name_contains": "bur"})
    );
    assert_eq!(v["total"], 1);
    assert_eq!(v["devices"][0]["name"], "Bureau");
    assert!(v["notice"].as_str().unwrap().contains("data"));
    let v = call!(client, "devices_list", json!({"limit": 2}));
    assert_eq!(
        (
            v["total"].as_u64(),
            v["shown"].as_u64(),
            v["truncated"].as_bool()
        ),
        (Some(4), Some(2), Some(true))
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_device_in_full_and_its_history(pool: PgPool) {
    let st = enabled(pool);
    let mut draft = DeviceDraft::new("Bureau-temp");
    draft.tracked_keys = vec!["temperature".into()];
    let d = st.store.devices().create(&draft).await.unwrap();
    st.store
        .devices()
        .apply_state("Bureau-temp", &json!({"temperature": 21.5, "battery": 87}))
        .await
        .unwrap();
    for i in 0..5 {
        st.store
            .state_log()
            .record(
                d.id,
                Utc::now() - Duration::minutes(10 * i),
                &[LogEntry {
                    key: "temperature".into(),
                    value_num: Some(20.0 + f64::from(i32::try_from(i).unwrap())),
                    value_str: String::new(),
                }],
            )
            .await
            .unwrap();
    }
    let token = make_token(&st, "diagnostic", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let by_name = call!(client, "device_get", json!({"device": "Bureau-temp"}));
    assert_eq!(by_name["state"]["temperature"], 21.5);
    assert_eq!(by_name["topic"], "zigbee2mqtt/Bureau-temp");
    assert_eq!(by_name["command_topic"], "zigbee2mqtt/Bureau-temp/set");
    assert_eq!(by_name["tracked_keys"], json!(["temperature"]));
    let by_id = call!(client, "device_get", json!({"device": d.id.0.to_string()}));
    assert_eq!(by_id["name"], "Bureau-temp");

    let h = call!(
        client,
        "device_history",
        json!({"device": "Bureau-temp", "key": "temperature", "hours": 2})
    );
    assert_eq!(h["total_points"], 5);
    assert_eq!(
        (h["min"].as_f64(), h["max"].as_f64(), h["average"].as_f64()),
        (Some(20.0), Some(24.0), Some(22.0))
    );
    assert_eq!(h["points"].as_array().unwrap().len(), 5);
    // Thinning keeps the statistics of the whole window.
    let h = call!(
        client,
        "device_history",
        json!({"device": "Bureau-temp", "key": "temperature", "hours": 2, "max_points": 2})
    );
    assert_eq!(
        (
            h["downsampled"].as_bool(),
            h["points"].as_array().unwrap().len()
        ),
        (Some(true), 2)
    );
    assert_eq!(h["average"], 22.0);
    // An unrecorded key says which ones are.
    let none = call!(
        client,
        "device_history",
        json!({"device": "Bureau-temp", "key": "battery"})
    );
    assert_eq!(none["total_points"], 0);
    assert_eq!(none["tracked_keys"], json!(["temperature"]));

    // Mistakes are explained, not crashes.
    for (tool, args) in [
        ("device_get", json!({"device": "Unknown"})),
        (
            "device_history",
            json!({"device": "Bureau-temp", "key": "  "}),
        ),
    ] {
        let args = args.as_object().cloned().unwrap();
        let r = client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
            .await;
        assert!(r.is_err(), "{tool}");
    }
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn why_did_this_automation_fail_and_a_script_is_never_shown(pool: PgPool) {
    let st = enabled(pool);
    let open = st
        .store
        .automations()
        .create(&automation(
            "Ouvrir le volet",
            (
                "device_state",
                json!({"friendly_name": "Tele", "key": "action", "value": "1_single"}),
            ),
            (
                "mqtt_publish",
                json!({"topic": "zigbee2mqtt/Volet/set", "payload": {"state": "OPEN"}}),
            ),
            "",
        ))
        .await
        .unwrap();
    st.store
        .automations()
        .create(&automation(
            "Script nuit",
            ("mqtt_message", json!({"topic": "maison/nuit"})),
            ("script", json!({})),
            "// MARQUEUR-SECRET-DU-SCRIPT\npublish(\"maison/lumiere\", \"OFF\");",
        ))
        .await
        .unwrap();
    let runs = st.store.automation_runs();
    for (minutes, status, message) in [
        (30, RunStatus::Ok, "publie sur zigbee2mqtt/Volet/set"),
        (
            20,
            RunStatus::Error,
            "publication refusee : broker injoignable",
        ),
        (10, RunStatus::Ok, "publie sur zigbee2mqtt/Volet/set"),
    ] {
        runs.record(&AutomationRun {
            automation: open.id,
            at: Utc::now() - Duration::minutes(minutes),
            trigger: "device_state Tele".into(),
            status,
            duration_ms: 12,
            message: message.into(),
        })
        .await
        .unwrap();
    }
    let token = make_token(&st, "diagnostic", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let list = call!(client, "automations_list", json!({}));
    let all = list["automations"].as_array().unwrap();
    assert_eq!(all.len(), 2);
    let script = all.iter().find(|a| a["name"] == "Script nuit").unwrap();
    assert_eq!(
        (script["has_script"].as_bool(), script["action"].as_str()),
        (Some(true), Some("script"))
    );
    assert_eq!(
        all.iter().find(|a| a["name"] == "Ouvrir le volet").unwrap()["trigger"],
        "device_state Tele : action = 1_single"
    );

    // The script code is never returned, by any read tool.
    let detail = call!(
        client,
        "automation_get",
        json!({"automation": "script nuit"})
    );
    assert!(
        detail["script_note"]
            .as_str()
            .unwrap()
            .contains("administration")
    );
    assert!(!detail.to_string().contains("MARQUEUR-SECRET"));
    assert!(!list.to_string().contains("MARQUEUR-SECRET"));
    let open_detail = call!(
        client,
        "automation_get",
        json!({"automation": open.id.0.to_string()})
    );
    assert_eq!(
        open_detail["action_config"]["topic"],
        "zigbee2mqtt/Volet/set"
    );
    assert!(open_detail["script_note"].is_null());

    // The question of the lot: the failure and its cause.
    let failed = call!(
        client,
        "automation_runs",
        json!({"automation": "Ouvrir le volet", "status": "error"})
    );
    assert_eq!(failed["shown"], 1);
    assert_eq!(failed["runs"][0]["status"], "error");
    assert_eq!(failed["runs"][0]["automation"], "Ouvrir le volet");
    assert!(
        failed["runs"][0]["message"]
            .as_str()
            .unwrap()
            .contains("broker injoignable")
    );
    let latest = call!(client, "automation_runs", json!({"limit": 2}));
    let statuses: Vec<_> = latest["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["status"].as_str().unwrap())
        .collect();
    // Most recent first: the success of 10 minutes ago, then the failure of 20.
    assert_eq!(statuses, ["ok", "error"]);
    let bad = client
        .call_tool(
            CallToolRequestParams::new("automation_runs")
                .with_arguments(json!({"status": "nimporte"}).as_object().cloned().unwrap()),
        )
        .await;
    assert!(bad.is_err());
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn scenes_name_their_devices_and_alerts_and_cameras_start_empty(pool: PgPool) {
    let st = enabled(pool);
    let lamp = device(&st, "Lampe-salon", Some(1), true).await;
    let mut scene = SceneDraft::new("Soiree");
    scene.actions = vec![SceneActionDraft {
        device: lamp.id,
        payload: json!({"state": "OFF"}),
    }];
    st.store.scenes().create(&scene).await.unwrap();
    let token = make_token(&st, "diagnostic", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let v = call!(client, "scenes_list", json!({}));
    assert_eq!(v["scenes"][0]["name"], "Soiree");
    assert_eq!(v["scenes"][0]["actions"][0]["device"], "Lampe-salon");
    assert_eq!(
        v["scenes"][0]["actions"][0]["payload"],
        json!({"state": "OFF"})
    );
    let alerts = call!(client, "alerts_list", json!({}));
    assert_eq!(alerts["shown"], 0);
    let cameras = call!(client, "cameras_list", json!({}));
    assert!(cameras["cameras"].as_array().unwrap().is_empty());
    let bad = client
        .call_tool(
            CallToolRequestParams::new("alerts_list")
                .with_arguments(json!({"status": "bof"}).as_object().cloned().unwrap()),
        )
        .await;
    assert!(bad.is_err());
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_application_log_is_searched_by_level_module_and_text(pool: PgPool) {
    use dominet_server::logbuf::{self, LogEntry as Line};

    let st = enabled(pool);
    let marker = format!("essai-mcp-{}", std::process::id());
    for (rank, target, message) in [
        (3, "dominet_test::alpha", format!("information {marker}")),
        (2, "dominet_test::beta", format!("avertissement {marker}")),
        (1, "dominet_test::alpha", format!("erreur {marker}")),
    ] {
        logbuf::global().push(Line {
            at_ms: logbuf::now_ms(),
            rank,
            target: target.into(),
            message,
        });
    }
    let token = make_token(&st, "diagnostic", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    // By default warnings and errors only, most recent first.
    let v = call!(client, "logs_query", json!({"text": marker}));
    assert_eq!(v["matched"], 2);
    assert!(
        v["lines"][0]["message"]
            .as_str()
            .unwrap()
            .starts_with("erreur")
    );
    assert_eq!(v["lines"][0]["level"], "ERROR");
    assert!(v["notice"].as_str().unwrap().contains("data"));
    let v = call!(
        client,
        "logs_query",
        json!({"text": marker, "level": "info"})
    );
    assert_eq!(v["matched"], 3);
    let v = call!(
        client,
        "logs_query",
        json!({"text": marker, "level": "info", "module": "BETA"})
    );
    assert_eq!(v["matched"], 1);
    let v = call!(
        client,
        "logs_query",
        json!({"text": marker, "level": "error", "limit": 1})
    );
    assert_eq!(
        (v["matched"].as_u64(), v["shown"].as_u64()),
        (Some(1), Some(1))
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_configuration_snapshot_is_a_resource_without_secrets_or_script_code(pool: PgPool) {
    use dominet_domain::mcp::AuditKind;
    use dominet_domain::repo::McpAuditRepo;

    let st = enabled(pool);
    device(&st, "Volet-salon", Some(1), true).await;
    st.store
        .automations()
        .create(&automation(
            "Script nuit",
            ("mqtt_message", json!({"topic": "maison/nuit"})),
            ("script", json!({})),
            "// MARQUEUR-SECRET-DU-SCRIPT\npublish(\"maison/lumiere\", \"OFF\");",
        ))
        .await
        .unwrap();
    let addr = serve(&st).await;

    let reader = make_token(&st, "diagnostic", &["lecture"]).await;
    let client = connect!(addr, reader);
    let listed = client.list_all_resources().await.unwrap();
    assert!(listed.iter().any(|r| r.uri == "dominet://context"));
    let read = client
        .read_resource(ReadResourceRequestParams::new("dominet://context"))
        .await
        .unwrap();
    let text = serde_json::to_string(&read).unwrap();
    assert!(
        text.contains("Script nuit"),
        "the automation is in the snapshot"
    );
    assert!(
        text.contains("Volet-salon"),
        "the device is in the snapshot"
    );
    assert!(
        !text.contains("MARQUEUR-SECRET"),
        "script code must be masked"
    );
    assert!(text.contains("code hidden"), "{text}");
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new("dominet://inconnue"))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();

    // Without the `lecture` scope: nothing listed, reading refused and written to the audit trail.
    let pilot = make_token(&st, "pilote", &["pilotage"]).await;
    let client = connect!(addr, pilot);
    assert!(client.list_all_resources().await.unwrap().is_empty());
    assert!(
        client
            .read_resource(ReadResourceRequestParams::new("dominet://context"))
            .await
            .is_err()
    );
    client.cancel().await.unwrap();
    let audit = st
        .store
        .mcp_audit()
        .recent(Some(AuditKind::Denied), 10)
        .await
        .unwrap();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(
        (audit[0].actor.as_str(), audit[0].tool.as_str()),
        ("pilote", "resource:context")
    );
}
