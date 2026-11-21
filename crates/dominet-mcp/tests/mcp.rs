// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end tests of the MCP server (R11 M1): a real HTTP listener, a real MCP client (`rmcp`), the test database, and access
//! tokens created the way the administration page creates them.

#[macro_use]
mod common;

use chrono::Utc;
use common::{enabled, list_request, make_token, post, serve, state};
use dominet_domain::mcp::AuditKind;
use dominet_domain::repo::{McpAuditRepo, McpTokenRepo};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_real_client_with_a_read_token_lists_and_calls_system_status(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "Claude Code", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token.clone());

    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    // A read token sees exactly the tools the scope table declares for `lecture`.
    let mut declared: Vec<_> = dominet_mcp::SCOPES
        .iter()
        .filter(|(_, scope)| *scope == dominet_domain::mcp::Scope::Read)
        .map(|(n, _)| (*n).to_string())
        .collect();
    declared.sort();
    assert_eq!(names, declared);
    assert!(names.contains(&"system_status".to_string()));
    // The tool carries a description and an output schema: the assistant needs no outside documentation.
    let status = tools
        .iter()
        .find(|t| t.name == "system_status")
        .expect("system_status");
    assert!(
        status
            .description
            .as_deref()
            .is_some_and(|d| d.contains("Overall state"))
    );
    assert!(status.output_schema.is_some());

    let result = client
        .call_tool(CallToolRequestParams::new("system_status"))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let report = result.structured_content.expect("structured output");
    assert_eq!(report["database"], "ok");
    assert_eq!(report["devices_active"], 0);
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    client.cancel().await.unwrap();

    // The use was noted, and a read call leaves no audit line (only changes and refusals are audited).
    let listed = st.store.mcp_tokens().list().await.unwrap();
    assert!(listed[0].last_used_at.is_some());
    assert!(
        st.store
            .mcp_audit()
            .recent(None, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_missing_unknown_or_malformed_token_is_refused_with_a_challenge(pool: PgPool) {
    let st = enabled(pool);
    make_token(&st, "valide", &["lecture"]).await;
    let addr = serve(&st).await;
    let http = reqwest::Client::new();
    let body = list_request();
    for auth in [
        None,
        Some("Bearer faux"),
        Some("Bearer dnm_inconnu0123456789inconnu0123456789inconnu01"),
        Some("Basic abc"),
        Some("Bearer "),
    ] {
        let mut req = http
            .post(format!("http://{addr}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 401, "{auth:?}");
        assert_eq!(resp.headers()["www-authenticate"], "Bearer", "{auth:?}");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_revoked_token_is_refused_on_the_very_next_call(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "a revoquer", &["lecture"]).await;
    let addr = serve(&st).await;
    assert_eq!(
        post(addr, Some(&token), &list_request()).await.status(),
        200
    );

    let id = st.store.mcp_tokens().list().await.unwrap()[0].id;
    assert!(st.store.mcp_tokens().revoke(id, Utc::now()).await.unwrap());
    assert_eq!(
        post(addr, Some(&token), &list_request()).await.status(),
        401
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_expired_token_is_refused(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "perime", &["lecture"]).await;
    let addr = serve(&st).await;
    assert_eq!(
        post(addr, Some(&token), &list_request()).await.status(),
        200
    );
    sqlx::query("UPDATE mcp_token SET expires_at = now() - interval '1 minute'")
        .execute(&st.pool)
        .await
        .unwrap();
    assert_eq!(
        post(addr, Some(&token), &list_request()).await.status(),
        401
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_token_without_the_scope_sees_no_tool_and_is_refused_and_audited(pool: PgPool) {
    let st = enabled(pool);
    // `configuration` alone does not give `lecture`: scopes are independent, so it sees its own tools and none of the read ones.
    let token = make_token(&st, "configurateur", &["configuration"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token.clone());

    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    assert_eq!(
        names
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["config_delete", "config_upsert", "zigbee_pairing"]
            .into_iter()
            .collect(),
        "{names:?}"
    );
    let refused = client
        .call_tool(CallToolRequestParams::new("system_status"))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true));
    let text = serde_json::to_string(&refused.content).unwrap();
    assert!(
        text.contains("Insufficient scope") && text.contains("lecture"),
        "{text}"
    );
    assert!(refused.structured_content.is_none());

    // An unknown tool is refused too, but only a known tool is worth an audit line.
    let unknown = client
        .call_tool(CallToolRequestParams::new("n_existe_pas"))
        .await
        .unwrap();
    assert_eq!(unknown.is_error, Some(true));
    client.cancel().await.unwrap();

    let audit = st.store.mcp_audit().recent(None, 10).await.unwrap();
    assert_eq!(audit.len(), 1, "{audit:?}");
    let line = &audit[0];
    assert_eq!(line.kind, AuditKind::Denied.as_str());
    assert_eq!(
        (line.actor.as_str(), line.actor_kind.as_str()),
        ("configurateur", "token")
    );
    assert_eq!(
        (line.tool.as_str(), line.outcome.as_str()),
        ("system_status", "denied")
    );
    assert!(line.token_id.is_some());
    // The token value never reaches the audit trail.
    assert!(
        !serde_json::to_string(&audit[0].params)
            .unwrap()
            .contains(&token)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_rate_limit_applies_per_token_with_a_retry_after(pool: PgPool) {
    let st = state(
        pool,
        &[("MCP_ENABLED", "true"), ("MCP_RATE_LIMIT_PER_MIN", "3")],
    );
    let busy = make_token(&st, "bavard", &["lecture"]).await;
    let calm = make_token(&st, "calme", &["lecture"]).await;
    let addr = serve(&st).await;
    for _ in 0..3 {
        assert_eq!(post(addr, Some(&busy), &list_request()).await.status(), 200);
    }
    let limited = post(addr, Some(&busy), &list_request()).await;
    assert_eq!(limited.status(), 429);
    let wait: u64 = limited.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&wait), "{wait}");
    // Another token has its own minute.
    assert_eq!(post(addr, Some(&calm), &list_request()).await.status(), 200);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_foreign_host_header_is_refused_against_dns_rebinding(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "a", &["lecture"]).await;
    let addr = serve(&st).await;
    let http = reqwest::Client::new();
    let post_with_host = |host: &str| {
        http.post(format!("http://{addr}/mcp"))
            .header("host", host)
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "application/json, text/event-stream")
            .json(&list_request())
            .send()
    };
    assert_eq!(post_with_host("evil.example").await.unwrap().status(), 403);
    // The configured name is accepted (any status but a host refusal).
    assert_ne!(post_with_host("dominet.local").await.unwrap().status(), 403);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn nothing_is_mounted_unless_enabled(pool: PgPool) {
    // Default: off. Even with a valid token in the database, `MCP_ENABLED` decides.
    let st = state(pool, &[]);
    let token = make_token(&st, "a", &["lecture"]).await;
    assert!(dominet_mcp::routes(&st).is_none());
    let addr = serve(&st).await;
    assert_eq!(
        post(addr, Some(&token), &list_request()).await.status(),
        404
    );
}

/// A client that still speaks the previous revision of the protocol (`initialize`, then calls) must work against the stateless
/// server too, with the same token.
#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_client_on_the_previous_protocol_revision_works_without_a_session(pool: PgPool) {
    let st = enabled(pool);
    let token = make_token(&st, "ancien", &["lecture"]).await;
    let addr = serve(&st).await;
    let send = |body: Value| {
        reqwest::Client::new()
            .post(format!("http://{addr}/mcp"))
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .json(&body)
            .send()
    };
    let init = send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "legacy-test", "version": "0"}}
    }))
    .await
    .unwrap();
    assert_eq!(init.status(), 200, "{:?}", init.text().await);
    let note = send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await
        .unwrap();
    assert!(note.status().is_success(), "{}", note.status());
    let call = send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "system_status", "arguments": {}}
    }))
    .await
    .unwrap();
    assert_eq!(call.status(), 200);
    let text = call.text().await.unwrap();
    assert!(text.contains("\"database\""), "{text}");
}
