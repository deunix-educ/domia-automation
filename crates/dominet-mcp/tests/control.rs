// SPDX-License-Identifier: AGPL-3.0-or-later
//! The control tools (R11 M3) through a real MCP client and the dev Mosquitto: real publications, `dry_run` that changes nothing,
//! the audit line written **before** the action, and the action **refused** when the audit trail cannot be written.

#[macro_use]
mod common;

use chrono::Utc;
use common::{
    admin_account, make_owned_token, make_token, next_command, serve, silent, with_broker,
};
use dominet_domain::alert::{AlertRuleDraft, Severity};
use dominet_domain::device::{Device, DeviceDraft};
use dominet_domain::mcp::AuditKind;
use dominet_domain::repo::{
    AlertRepo, AlertRuleRepo, DeviceRepo, McpAuditRepo, NewAlert, SceneRepo,
};
use dominet_domain::scene::{SceneActionDraft, SceneDraft};
use dominet_server::AppState;
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

/// The structured output of a call that must have succeeded.
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

fn tag() -> String {
    format!("mcp{}", std::process::id())
}

async fn device(st: &AppState, name: &str) -> Device {
    st.store
        .devices()
        .create(&DeviceDraft::new(name))
        .await
        .unwrap()
}

async fn audit_lines(st: &AppState) -> Vec<dominet_domain::mcp::McpAuditEntry> {
    st.store
        .mcp_audit()
        .recent(Some(AuditKind::ToolCall), 20)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_command_is_published_for_real_and_audited_before_it_runs(pool: PgPool) {
    let (st, mut rx, _listener) = with_broker(pool, "cmd").await;
    let name = format!("{}-Volet", tag());
    device(&st, &name).await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let v = ok!(
        client,
        "device_command",
        json!({"device": name, "command": {"state": "OPEN"}})
    );
    assert_eq!(
        (v["dry_run"].as_bool(), v["done"].as_bool()),
        (Some(false), Some(true))
    );
    let (topic, payload) = next_command(&mut rx, &name).await;
    assert_eq!(topic, format!("zigbee2mqtt/{name}/set"));
    assert_eq!(payload, json!({"state": "OPEN"}));

    let lines = audit_lines(&st).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert_eq!(
        (
            line.tool.as_str(),
            line.actor.as_str(),
            line.outcome.as_str()
        ),
        ("device_command", "assistant", "ok")
    );
    assert_eq!(line.params["device"], name.as_str());
    assert_eq!(line.params["command"], json!({"state": "OPEN"}));
    assert!(line.detail.contains("command published"));
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_dry_run_says_what_it_would_do_and_changes_nothing(pool: PgPool) {
    let (st, mut rx, _listener) = with_broker(pool, "dry").await;
    let name = format!("{}-Lampe", tag());
    device(&st, &name).await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let v = ok!(
        client,
        "device_command",
        json!({"device": name, "command": {"state": "ON"}, "dry_run": true})
    );
    assert_eq!(
        (v["dry_run"].as_bool(), v["done"].as_bool()),
        (Some(true), Some(false))
    );
    assert!(v["summary"].as_str().unwrap().starts_with("Would send"));
    assert!(
        silent(&mut rx, &name, 1200).await,
        "a dry run must publish nothing"
    );
    assert!(
        audit_lines(&st).await.is_empty(),
        "a dry run changes nothing, so it leaves no line"
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn mistakes_are_refused_before_anything_is_sent_or_written(pool: PgPool) {
    let (st, mut rx, _listener) = with_broker(pool, "bad").await;
    let name = format!("{}-Prise", tag());
    let d = device(&st, &name).await;
    sqlx::query("UPDATE device SET active = false WHERE id = $1")
        .bind(d.id.0)
        .execute(&st.pool)
        .await
        .unwrap();
    let live = format!("{}-Vivante", tag());
    device(&st, &live).await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    for (what, args) in [
        (
            "inactive device",
            json!({"device": name, "command": {"state": "ON"}}),
        ),
        (
            "unknown device",
            json!({"device": "Unknown", "command": {"state": "ON"}}),
        ),
        ("empty command", json!({"device": live, "command": {}})),
        ("not an object", json!({"device": live, "command": "OPEN"})),
        (
            "too deep",
            json!({"device": live, "command": {"a": {"b": {"c": {"d": 1}}}}}),
        ),
    ] {
        assert!(call!(client, "device_command", args).is_err(), "{what}");
    }
    for needle in [&name, &live] {
        assert!(
            silent(&mut rx, needle, 600).await,
            "nothing may be published to {needle}"
        );
    }
    assert!(audit_lines(&st).await.is_empty());
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_the_control_scope_nothing_is_sent_and_the_refusal_is_audited(pool: PgPool) {
    let (st, mut rx, _listener) = with_broker(pool, "scope").await;
    let name = format!("{}-Store", tag());
    device(&st, &name).await;
    let token = make_token(&st, "lecteur", &["lecture", "configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    // The tool is not even offered, and calling it anyway is refused.
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    assert!(!names.contains(&"device_command".to_string()), "{names:?}");
    let refused = call!(
        client,
        "device_command",
        json!({"device": name, "command": {"state": "OPEN"}})
    )
    .unwrap();
    assert_eq!(refused.is_error, Some(true));
    assert!(silent(&mut rx, &name, 1000).await);
    let denied = st
        .store
        .mcp_audit()
        .recent(Some(AuditKind::Denied), 10)
        .await
        .unwrap();
    assert_eq!(
        (denied.len(), denied[0].tool.as_str()),
        (1, "device_command")
    );
    assert!(
        audit_lines(&st).await.is_empty(),
        "a refusal is not an action"
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn when_the_audit_trail_cannot_be_written_the_action_is_refused(pool: PgPool) {
    let (st, mut rx, _listener) = with_broker(pool, "failclosed").await;
    let name = format!("{}-Garage", tag());
    device(&st, &name).await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    // The trail is gone: the call must fail, and nothing may reach the house.
    sqlx::query("DROP TABLE mcp_audit")
        .execute(&st.pool)
        .await
        .unwrap();

    let result = call!(
        client,
        "device_command",
        json!({"device": name, "command": {"state": "OPEN"}})
    );
    let message = format!("{result:?}");
    assert!(result.is_err(), "{message}");
    assert!(message.contains("audit log unavailable"), "{message}");
    assert!(
        silent(&mut rx, &name, 1500).await,
        "nothing may be published without a trace"
    );
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_scene_sends_its_commands_in_order_and_a_dry_run_lists_them(pool: PgPool) {
    let (st, mut rx, _listener) = with_broker(pool, "scene").await;
    let (a, b) = (format!("{}-A", tag()), format!("{}-B", tag()));
    let (da, db) = (device(&st, &a).await, device(&st, &b).await);
    let mut scene = SceneDraft::new(&format!("{}-Soiree", tag()));
    scene.actions = vec![
        SceneActionDraft {
            device: da.id,
            payload: json!({"state": "CLOSE"}),
        },
        SceneActionDraft {
            device: db.id,
            payload: json!({"state": "OFF"}),
        },
    ];
    let scene_name = scene.name.clone();
    st.store.scenes().create(&scene).await.unwrap();
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let dry = ok!(
        client,
        "scene_run",
        json!({"scene": scene_name, "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["details"]["actions"][0]["device"], a.as_str());
    assert!(silent(&mut rx, &a, 800).await && silent(&mut rx, &b, 100).await);

    let done = ok!(
        client,
        "scene_run",
        json!({"scene": scene_name.to_lowercase()})
    );
    assert_eq!(done["done"], true);
    assert_eq!(done["details"]["sent"], 2);
    let first = next_command(&mut rx, &a).await;
    let second = next_command(&mut rx, &b).await;
    assert_eq!(
        (first.1, second.1),
        (json!({"state": "CLOSE"}), json!({"state": "OFF"}))
    );
    let lines = audit_lines(&st).await;
    assert_eq!(
        (
            lines.len(),
            lines[0].tool.as_str(),
            lines[0].outcome.as_str()
        ),
        (1, "scene_run", "ok")
    );

    // An inactive scene is refused.
    sqlx::query("UPDATE scene SET active = false")
        .execute(&st.pool)
        .await
        .unwrap();
    assert!(call!(client, "scene_run", json!({"scene": scene_name})).is_err());
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_alert_is_acknowledged_in_the_name_of_the_account_that_owns_the_token(pool: PgPool) {
    let st = common::enabled(pool);
    let d = device(&st, "Cave").await;
    let rule = st
        .store
        .alert_rules()
        .create(&AlertRuleDraft::new("Trop humide", d.id, "humidity"))
        .await
        .unwrap();
    let alerts = st.store.alerts();
    let new = || NewAlert {
        rule: rule.id,
        device: d.id,
        severity: Severity::Critical,
        value_triggered: "85".into(),
        fired_at: Utc::now(),
    };
    let first = alerts.create(&new()).await.unwrap();
    let owner = admin_account(&st, "alice").await;
    let owned = make_owned_token(&st, "assistant", &["pilotage"], Some(owner)).await;
    let orphan = make_token(&st, "orphelin", &["pilotage"]).await;
    let addr = serve(&st).await;

    // A token with no owner cannot acknowledge: an acknowledgement is attributed to an account.
    let client = connect!(addr, orphan);
    assert!(call!(client, "alert_ack", json!({"all": true})).is_err());
    client.cancel().await.unwrap();

    let client = connect!(addr, owned);
    // Both or neither of `alert` and `all` is a mistake, and so is an unknown alert.
    assert!(call!(client, "alert_ack", json!({})).is_err());
    assert!(
        call!(
            client,
            "alert_ack",
            json!({"alert": first.id.0, "all": true})
        )
        .is_err()
    );
    assert!(call!(client, "alert_ack", json!({"alert": 999_999})).is_err());
    let dry = ok!(
        client,
        "alert_ack",
        json!({"alert": first.id.0, "dry_run": true})
    );
    assert_eq!(
        (dry["dry_run"].as_bool(), dry["done"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(
        alerts.count_firing().await.unwrap(),
        1,
        "a dry run acknowledges nothing"
    );

    let done = ok!(client, "alert_ack", json!({"alert": first.id.0}));
    assert_eq!(done["done"], true);
    assert_eq!(alerts.count_firing().await.unwrap(), 0);
    let listed = alerts.list(None, 10).await.unwrap();
    assert_eq!(listed[0].acknowledged_by, Some(owner));
    // Already acknowledged: no longer a candidate.
    assert!(call!(client, "alert_ack", json!({"alert": first.id.0})).is_err());

    // `all` acknowledges what is still firing.
    alerts.create(&new()).await.unwrap();
    alerts.create(&new()).await.unwrap();
    let all = ok!(client, "alert_ack", json!({"all": true}));
    assert_eq!(all["summary"], "2 alert(s) acknowledged");
    assert_eq!(alerts.count_firing().await.unwrap(), 0);
    assert_eq!(audit_lines(&st).await.len(), 2);
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_banner_reaches_the_signed_in_accounts_and_the_text_is_bounded(pool: PgPool) {
    use dominet_proto::{Topic, WsServerMsg};

    let st = common::enabled(pool);
    let mut watcher = st.hub.subscribe(&Topic::Notifications);
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    for bad in ["", "   ", &"x".repeat(201)] {
        assert!(call!(client, "notify_broadcast", json!({"text": bad})).is_err());
    }
    let dry = ok!(
        client,
        "notify_broadcast",
        json!({"text": "Coupure ce soir", "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert!(watcher.try_recv().is_err(), "a dry run shows nothing");

    let done = ok!(
        client,
        "notify_broadcast",
        json!({"text": "  Coupure ce soir  "})
    );
    assert_eq!(done["details"]["receivers"], 1);
    match watcher.recv().await.unwrap().as_ref() {
        WsServerMsg::Notification { text, .. } => assert_eq!(text, "Coupure ce soir"),
        other => panic!("{other:?}"),
    }
    client.cancel().await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_test_email_needs_a_mail_server_and_never_shows_the_address(pool: PgPool) {
    use dominet_notify::testing::FakeSmtp;

    // Without a mail server: explained, nothing sent, nothing audited.
    let st = common::enabled(pool.clone());
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let result = call!(client, "notify_test", json!({}));
    assert!(result.is_err());
    assert!(format!("{result:?}").contains("mail server"));
    assert!(audit_lines(&st).await.is_empty());
    // The tool says it cannot send an SMS (the decision of 2026-11-13).
    let tools = client.list_all_tools().await.unwrap();
    let t = tools.iter().find(|t| t.name == "notify_test").unwrap();
    assert!(
        t.description
            .as_deref()
            .unwrap()
            .contains("SMS do not go through MCP")
    );
    client.cancel().await.unwrap();

    // With a (fake) mail server: a real send, the address stays private, and the line is audited.
    let smtp = FakeSmtp::start().await;
    let port = smtp.addr().port().to_string();
    let st = common::state(
        pool,
        &[
            ("MCP_ENABLED", "true"),
            ("EMAIL_HOST", "127.0.0.1"),
            ("EMAIL_PORT", &port),
            ("EMAIL_USE_TLS", "false"),
        ],
    );
    st.settings
        .set("EMAIL_RECIPIENT", "alice@example.com")
        .await
        .unwrap();
    st.settings
        .set("DEFAULT_FROM_EMAIL", "maison@example.com")
        .await
        .unwrap();
    let token = make_token(&st, "assistant-mail", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let dry = ok!(client, "notify_test", json!({"dry_run": true}));
    assert_eq!(dry["dry_run"], true);
    assert!(smtp.mails().is_empty(), "a dry run sends nothing");
    let done = ok!(client, "notify_test", json!({}));
    assert_eq!(done["done"], true);
    assert!(
        !done.to_string().contains("example.com"),
        "the address must stay private"
    );
    let mails = smtp.mails();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].to[0], "alice@example.com");
    assert!(mails[0].header("subject")[0].contains("Send test"));
    assert!(mails[0].body().contains("assistant-mail"));
    let lines = audit_lines(&st).await;
    assert_eq!(
        (
            lines.len(),
            lines[0].tool.as_str(),
            lines[0].outcome.as_str()
        ),
        (1, "notify_test", "ok")
    );
    assert!(
        !serde_json::to_string(&lines[0].params)
            .unwrap()
            .contains("example.com")
    );
    client.cancel().await.unwrap();
    let _: Value = json!(null);
}
