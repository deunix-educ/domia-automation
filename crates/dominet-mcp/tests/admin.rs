// SPDX-License-Identifier: AGPL-3.0-or-later
//! The administration tools (R11 M5) through a real MCP client: settings, Rhai scripts, cameras. The criterion of the lot: **no
//! secret in any answer** (nor in the audit trail), and everything sensitive behind the `administration` scope.

#[macro_use]
mod common;

use common::{make_token, serve, state};
use dominet_domain::mcp::AuditKind;
use dominet_domain::repo::{CameraRepo, McpAuditRepo};
use dominet_server::AppState;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
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

fn refusal<T: std::fmt::Debug>(result: Result<T, rmcp::ServiceError>) -> String {
    format!("{:?}", result.expect_err("the call should be refused"))
}

/// A state with a master key: secrets can be written.
fn sealed(pool: PgPool) -> AppState {
    state(
        pool,
        &[
            ("MCP_ENABLED", "true"),
            (
                "DOMINET_MASTER_KEY",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            ),
        ],
    )
}

const ALL: [&str; 4] = ["lecture", "pilotage", "configuration", "administration"];

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn settings_are_validated_and_a_secret_is_written_and_never_shown(pool: PgPool) {
    let st = sealed(pool);
    let weak = make_token(&st, "configurateur", &["lecture", "configuration"]).await;
    let admin = make_token(&st, "administrateur", &ALL).await;
    let addr = serve(&st).await;

    // The administration tools are not even offered without the scope, and calling them is refused and recorded.
    let client = connect!(addr, weak);
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for tool in [
        "settings_list",
        "setting_set",
        "script_get",
        "script_check",
        "script_set",
        "cameras_sync",
    ] {
        assert!(!names.contains(&tool.to_string()), "{tool}");
    }
    let refused = call!(
        client,
        "setting_set",
        json!({"key": "EMAIL_HOST", "value": "x"})
    )
    .unwrap();
    assert_eq!(refused.is_error, Some(true));
    client.cancel().await.unwrap();

    let secret = "Mot-de-passe-SECRET-123";
    let client = connect!(addr, admin);
    let list = ok!(client, "settings_list", json!({}));
    let all = list["settings"].as_array().unwrap();
    let password = all
        .iter()
        .find(|s| s["key"] == "SMS_FREE_KEY")
        .expect("a secret setting");
    assert_eq!(password["is_secret"], true);
    assert_eq!(password["value"], "");

    // A value of the wrong type is refused, with or without a dry run.
    let bad = all
        .iter()
        .find(|s| s["value_type"] == "int")
        .expect("an integer setting");
    for dry_run in [true, false] {
        let message = refusal(call!(
            client,
            "setting_set",
            json!({"key": bad["key"], "value": "pas un nombre", "dry_run": dry_run})
        ));
        assert!(message.contains("refused"), "{message}");
    }
    assert!(
        refusal(call!(
            client,
            "setting_set",
            json!({"key": "N_EXISTE_PAS", "value": "1"})
        ))
        .contains("unknown")
    );

    // A dry run writes nothing; the real write stores the secret sealed, and nothing shows it.
    let dry = ok!(
        client,
        "setting_set",
        json!({"key": "SMS_FREE_KEY", "value": secret, "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert!(!dry.to_string().contains(secret), "{dry}");
    assert!(
        !ok!(client, "settings_list", json!({}))["settings"]
            .to_string()
            .contains("\"is_set\":true,\"key\":\"SMS_FREE_KEY\"")
    );
    let done = ok!(
        client,
        "setting_set",
        json!({"key": "SMS_FREE_KEY", "value": secret})
    );
    assert_eq!(done["done"], true, "{done}");
    assert!(!done.to_string().contains(secret), "{done}");
    let list = ok!(client, "settings_list", json!({}));
    assert!(!list.to_string().contains(secret), "{list}");
    let shown = list["settings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "SMS_FREE_KEY")
        .unwrap();
    assert_eq!(
        (shown["is_set"].as_bool(), shown["value"].as_str()),
        (Some(true), Some(""))
    );
    client.cancel().await.unwrap();

    // Neither the database nor the audit trail holds it in the clear.
    let stored: String =
        sqlx::query_scalar("SELECT value FROM app_setting WHERE key = 'SMS_FREE_KEY'")
            .fetch_one(&st.pool)
            .await
            .unwrap();
    assert!(!stored.contains(secret));
    let audit = st.store.mcp_audit().recent(None, 50).await.unwrap();
    assert!(!format!("{audit:?}").contains(secret));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn scripts_are_read_checked_tried_and_replaced_with_the_same_checks_as_config_upsert(
    pool: PgPool,
) {
    let st = sealed(pool);
    let admin = make_token(&st, "administrateur", &ALL).await;
    let addr = serve(&st).await;
    let client = connect!(addr, admin);

    // Check: compile only, then run on a sample; nothing is published.
    let good = "publish(\"maison/lumiere\", \"OFF\");";
    let compiled = ok!(client, "script_check", json!({"code": good}));
    assert_eq!(
        (compiled["ok"].as_bool(), compiled["ran"].as_bool()),
        (Some(true), Some(false))
    );
    let broken = ok!(client, "script_check", json!({"code": "if ("}));
    assert_eq!(broken["ok"], false);
    assert!(!broken["error"].as_str().unwrap().is_empty());
    let ran = ok!(
        client,
        "script_check",
        json!({"code": good, "topic": "maison/nuit", "payload": "{\"a\":1}"})
    );
    assert_eq!(
        (ran["ok"].as_bool(), ran["ran"].as_bool()),
        (Some(true), Some(true)),
        "{ran}"
    );
    assert!(
        ran["effects"].to_string().contains("maison/lumiere"),
        "{ran}"
    );

    // An automation with a script, then script_get / script_set.
    let created = ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "data": {"name": "Script nuit", "trigger": {"type": "mqtt_message", "config": {"topic": "maison/nuit"}},
                "action": {"type": "script"}, "script": good}})
    );
    assert_eq!(created["done"], true, "{created}");
    let got = ok!(
        client,
        "script_get",
        json!({"kind": "automation", "item": "script nuit"})
    );
    assert_eq!(got["code"], good);

    let new_code = "publish(\"maison/lumiere\", \"ON\");";
    assert!(
        refusal(call!(
            client,
            "script_set",
            json!({"kind": "automation", "item": "Script nuit", "code": "if ("})
        ))
        .contains("refus")
    );
    let dry = ok!(
        client,
        "script_set",
        json!({"kind": "automation", "item": "Script nuit", "code": new_code, "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(
        ok!(
            client,
            "script_get",
            json!({"kind": "automation", "item": "Script nuit"})
        )["code"],
        good,
        "a dry run changes nothing"
    );
    let done = ok!(
        client,
        "script_set",
        json!({"kind": "automation", "item": "Script nuit", "code": new_code})
    );
    assert_eq!(done["done"], true, "{done}");
    assert_eq!(
        ok!(
            client,
            "script_get",
            json!({"kind": "automation", "item": "Script nuit"})
        )["code"],
        new_code
    );

    // An automation whose action is not `script` has no script to replace.
    ok!(
        client,
        "config_upsert",
        json!({"kind": "automation", "data": {"name": "Sans script", "trigger": {"type": "mqtt_message", "config": {"topic": "a/b"}},
                "action": {"type": "notification", "config": {"message": "x"}}}})
    );
    assert!(
        refusal(call!(
            client,
            "script_set",
            json!({"kind": "automation", "item": "Sans script", "code": good})
        ))
        .contains("script")
    );

    // The processing code of an MQTT source.
    ok!(
        client,
        "config_upsert",
        json!({"kind": "mqtt_source", "data": {"name": "Maison", "topic_pattern": "maison/+/etat"}})
    );
    let done = ok!(
        client,
        "script_set",
        json!({"kind": "mqtt_source", "item": "Maison", "code": "let x = 1;"})
    );
    assert_eq!(done["done"], true, "{done}");
    assert_eq!(
        ok!(
            client,
            "script_get",
            json!({"kind": "mqtt_source", "item": "Maison"})
        )["code"],
        "let x = 1;"
    );
    assert!(
        refusal(call!(
            client,
            "script_get",
            json!({"kind": "scene", "item": "x"})
        ))
        .contains("automation")
    );
    client.cancel().await.unwrap();

    // Every change was recorded before it ran.
    let audit = st
        .store
        .mcp_audit()
        .recent(Some(AuditKind::ToolCall), 50)
        .await
        .unwrap();
    assert!(audit.iter().any(|l| l.tool == "script_set"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_camera_is_configured_with_write_only_secrets_and_only_by_administration(pool: PgPool) {
    let st = sealed(pool);
    let weak = make_token(&st, "configurateur", &["lecture", "configuration"]).await;
    let admin = make_token(&st, "administrateur", &ALL).await;
    let addr = serve(&st).await;
    let (rtsp, onvif, web, key) = (
        "RTSP-SECRET-1",
        "ONVIF-SECRET-2",
        "WEB-SECRET-3",
        "APIKEY-SECRET-4",
    );
    let secrets = [rtsp, onvif, web, key];
    let mut seen = String::new();

    // `configuration` alone neither reads, writes nor deletes a camera.
    let client = connect!(addr, weak);
    let data =
        json!({"name": "Entree", "ip": "192.168.1.60", "rtsp_user": "viewer", "rtsp_pass": rtsp});
    for dry_run in [true, false] {
        assert!(
            refusal(call!(
                client,
                "config_upsert",
                json!({"kind": "camera", "data": data.clone(), "dry_run": dry_run})
            ))
            .contains("administration")
        );
    }
    for (tool, args) in [
        ("config_list", json!({"kind": "camera"})),
        ("config_get", json!({"kind": "camera", "item": "Entree"})),
        (
            "config_delete",
            json!({"kind": "camera", "item": "Entree", "confirm": true}),
        ),
    ] {
        assert!(
            refusal(call!(client, tool, args)).contains("administration"),
            "{tool}"
        );
    }
    client.cancel().await.unwrap();
    assert!(st.store.cameras().list().await.unwrap().is_empty());

    let client = connect!(addr, admin);
    let full = json!({"name": "Entree", "ip": "192.168.1.60", "path": "stream1", "rtsp_user": "viewer", "rtsp_pass": rtsp,
        "recording_mode": "motion", "onvif": true, "onvif_user": "admin", "onvif_pass": onvif,
        "web_user": "root", "web_pass": web, "web_api_key": key, "yolo_enabled": true, "yolo_classes": ["person", " "]});
    for (bad, needle) in [
        (json!({"name": "X", "proto": "ftp"}), "proto"),
        (
            json!({"name": "X", "recording_mode": "toujours"}),
            "recording_mode",
        ),
        (
            json!({"name": "X", "yolo_min_confidence_pct": 150}),
            "yolo_min_confidence_pct",
        ),
        (
            json!({"name": "X", "location": "nulle part"}),
            "place not found",
        ),
    ] {
        assert!(
            refusal(call!(
                client,
                "config_upsert",
                json!({"kind": "camera", "data": bad})
            ))
            .contains(needle),
            "{needle}"
        );
    }
    let dry = ok!(
        client,
        "config_upsert",
        json!({"kind": "camera", "data": full.clone(), "dry_run": true})
    );
    seen += &dry.to_string();
    assert_eq!(dry["details"]["after"]["has_rtsp_pass"], true);
    let done = ok!(
        client,
        "config_upsert",
        json!({"kind": "camera", "data": full})
    );
    assert_eq!(done["done"], true, "{done}");
    seen += &done.to_string();
    let id = done["details"]["id"].as_i64().unwrap();

    let got = ok!(
        client,
        "config_get",
        json!({"kind": "camera", "item": "entree"})
    );
    seen += &got.to_string();
    let data = &got["data"];
    assert_eq!(
        (
            data["has_rtsp_pass"].as_bool(),
            data["has_onvif_pass"].as_bool(),
            data["has_web_pass"].as_bool(),
            data["has_web_api_key"].as_bool(),
            data["yolo_classes"].clone()
        ),
        (
            Some(true),
            Some(true),
            Some(true),
            Some(true),
            json!(["person"])
        )
    );
    assert!(data.get("rtsp_pass").is_none());

    // What is read is written back as it is, and the secrets stay (a field left out keeps them).
    let mut again = data.clone();
    again["notes"] = json!("porte d'entree");
    let changed = ok!(
        client,
        "config_upsert",
        json!({"kind": "camera", "id": id, "data": again})
    );
    seen += &changed.to_string();
    assert_eq!(changed["done"], true, "{changed}");
    let after = ok!(
        client,
        "config_get",
        json!({"kind": "camera", "item": id.to_string()})
    );
    seen += &after.to_string();
    assert_eq!(after["data"]["has_rtsp_pass"], true);
    assert_eq!(after["data"]["notes"], "porte d'entree");

    // Everything else that talks about cameras, and the context snapshot.
    seen += &ok!(client, "config_list", json!({"kind": "camera"})).to_string();
    seen += &ok!(client, "cameras_list", json!({})).to_string();
    seen += &ok!(client, "config_schema", json!({"kind": "camera"})).to_string();
    let context = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "dominet://context",
        ))
        .await
        .unwrap();
    seen += &format!("{context:?}");

    // cameras_sync: validated, dry run, and a MediaMTX that is not configured is said, not hidden.
    assert!(refusal(call!(client, "cameras_sync", json!({"target": "onvif"}))).contains("camera"));
    assert!(refusal(call!(client, "cameras_sync", json!({"target": "autre"}))).contains("unknown"));
    let dry = ok!(
        client,
        "cameras_sync",
        json!({"target": "mediamtx", "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    let sync = ok!(client, "cameras_sync", json!({"target": "mediamtx"}));
    assert_eq!(
        sync["done"], false,
        "MEDIAMTX_API_URL is not set in the test: {sync}"
    );
    seen += &sync.to_string();

    // Deleting needs confirm and says what goes.
    let dry = ok!(
        client,
        "config_delete",
        json!({"kind": "camera", "item": "Entree", "dry_run": true})
    );
    assert!(
        dry["details"]["impact"].to_string().contains("recordings"),
        "{dry}"
    );
    assert!(
        refusal(call!(
            client,
            "config_delete",
            json!({"kind": "camera", "item": "Entree"})
        ))
        .contains("confirm")
    );
    let gone = ok!(
        client,
        "config_delete",
        json!({"kind": "camera", "item": "Entree", "confirm": true})
    );
    assert_eq!(gone["done"], true, "{gone}");
    client.cancel().await.unwrap();

    // No secret in any answer, nor in the audit trail.
    seen += &format!(
        "{:?}",
        st.store.mcp_audit().recent(None, 100).await.unwrap()
    );
    for secret in secrets {
        assert!(!seen.contains(secret), "{secret} leaked");
    }
}
