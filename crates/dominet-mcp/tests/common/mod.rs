// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared by the MCP integration tests: a state on the test database, tokens created the way the administration creates them,
//! a real HTTP listener with the MCP routes mounted as the binary does, and a real MCP client.
#![allow(dead_code)] // each test file uses only part of this

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use chrono::Utc;
use dominet_domain::mcp::NewMcpToken;
use dominet_domain::repo::McpTokenRepo;
use dominet_domain::user::UserId;
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, mcp_tokens, web};
use serde_json::{Value, json};
use sqlx::PgPool;

pub fn state(pool: PgPool, extra: &[(&str, &str)]) -> AppState {
    let mut vars = vec![
        ("DATABASE_URL", "postgres://u:p@127.0.0.1:1/none"),
        ("COOKIE_SECURE", "false"),
        ("ALLOWED_HOSTS", "dominet.local"),
    ];
    vars.extend_from_slice(extra);
    let config = Config::from_lookup(|k| {
        vars.iter()
            .rev()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| (*v).to_string())
    })
    .unwrap();
    AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    )
}

pub fn enabled(pool: PgPool) -> AppState {
    state(pool, &[("MCP_ENABLED", "true")])
}

/// A token as the administration creates it; returns its value (the only copy).
pub async fn make_token(state: &AppState, name: &str, scopes: &[&str]) -> String {
    make_owned_token(state, name, scopes, None).await
}

/// The same, attributed to the account `owner` (needed to acknowledge alerts).
pub async fn make_owned_token(
    state: &AppState,
    name: &str,
    scopes: &[&str],
    owner: Option<UserId>,
) -> String {
    let scopes: Vec<String> = scopes.iter().map(ToString::to_string).collect();
    let new = NewMcpToken::new(name, &scopes, 30).unwrap();
    let generated = mcp_tokens::generate();
    state
        .store
        .mcp_tokens()
        .create(&new, &generated.hash, &generated.prefix, Utc::now(), owner)
        .await
        .unwrap();
    generated.value
}

/// An administrator account (no way to sign in: the hash is not a real one).
pub async fn admin_account(state: &AppState, username: &str) -> UserId {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, role) VALUES ($1, 'x', 'admin') RETURNING id",
    )
    .bind(username)
    .fetch_one(&state.pool)
    .await
    .unwrap();
    UserId(id)
}

/// A state wired to the dev Mosquitto (`make dev-up`), and a listener on the commands the tools publish (`zigbee2mqtt/+/set`).
/// The Zigbee2MQTT base topic is moved aside so the server never reads the retained messages of the real bridge.
pub async fn with_broker(
    pool: PgPool,
    tag: &str,
) -> (
    AppState,
    tokio::sync::mpsc::Receiver<dominet_mqtt::Incoming>,
    dominet_mqtt::MqttClient,
) {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use dominet_mqtt::{MqttSettings, Status, start};

    async fn connected(c: &dominet_mqtt::MqttClient) {
        let mut rx = c.watch_status();
        tokio::time::timeout(Duration::from_secs(10), async {
            while *rx.borrow_and_update() != Status::Connected {
                rx.changed().await.unwrap();
            }
        })
        .await
        .expect("connexion au broker de dev");
    }
    let port = std::env::var("MQTT_TEST_PORT").unwrap_or_else(|_| "11883".into());
    let id = format!("dominet-mcp-test-{tag}-{}", std::process::id());
    let mut st = state(
        pool,
        &[
            ("MCP_ENABLED", "true"),
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
        ],
    );
    let listener = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("{id}-listen"),
    ));
    connected(&listener.client).await;
    listener
        .client
        .set_subscriptions(BTreeMap::from([("zigbee2mqtt/+/set".to_string(), 1)]));
    tokio::time::sleep(Duration::from_millis(500)).await;
    st.settings
        .set(
            "Z2M_BASE_TOPIC",
            &format!("{tag}-{}-z2m", std::process::id()),
        )
        .await
        .unwrap();
    st.mqtt = Some(dominet_server::mqtt::spawn(&st).await.unwrap());
    connected(st.mqtt.as_ref().unwrap()).await;
    (st, listener.messages, listener.client)
}

/// The next command published for a device whose name contains `needle` (others, from tests running in parallel, are skipped).
pub async fn next_command(
    rx: &mut tokio::sync::mpsc::Receiver<dominet_mqtt::Incoming>,
    needle: &str,
) -> (String, Value) {
    loop {
        let m = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("commande recue")
            .unwrap();
        if m.topic.contains(needle) {
            let dominet_mqtt::Payload::Json(v) = m.payload else {
                panic!("payload non JSON : {:?}", m.payload)
            };
            return (m.topic, v);
        }
    }
}

/// `true` if no command for a device whose name contains `needle` arrives within `ms`.
pub async fn silent(
    rx: &mut tokio::sync::mpsc::Receiver<dominet_mqtt::Incoming>,
    needle: &str,
    ms: u64,
) -> bool {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
    while let Ok(Some(m)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if m.topic.contains(needle) {
            return false;
        }
    }
    true
}

/// The application on a free local port, with the MCP routes mounted as the binary does.
pub async fn serve(state: &AppState) -> SocketAddr {
    let router: Router =
        web::router_with_extra(state.clone(), Router::new(), dominet_mcp::routes(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

/// A connected MCP client (`rmcp`), authenticated with `$token`.
macro_rules! connect {
    ($addr:expr, $token:expr) => {{
        use rmcp::ServiceExt;
        let transport = rmcp::transport::StreamableHttpClientTransport::from_config(
            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                format!("http://{}/mcp", $addr),
            )
            .auth_header($token),
        );
        rmcp::model::ClientConfig::default()
            .serve(transport)
            .await
            .unwrap()
    }};
}

/// One JSON-RPC request over plain HTTP.
pub async fn post(addr: SocketAddr, token: Option<&str>, body: &Value) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .json(body);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    req.send().await.unwrap()
}

pub fn list_request() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
}
