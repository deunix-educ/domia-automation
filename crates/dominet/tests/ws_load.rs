// SPDX-License-Identifier: AGPL-3.0-or-later
//! Load test of the `/ws/notifications/` WebSocket (R8 lot 3): many browsers subscribed to devices while the states arrive
//! continuously. A real TCP server, real WebSocket clients, real sessions.
//!
//! Adjustable through the environment: `WS_LOAD_USERS` (10), `WS_LOAD_PER_USER` (8, server maximum), `WS_LOAD_RATE` (messages per second,
//! 200), `WS_LOAD_MESSAGES` (1000), `WS_LOAD_P99_MS` (p99 bound in milliseconds, 3000). Heavy example, on the target machine:
//! `WS_LOAD_USERS=125 WS_LOAD_RATE=500 cargo test --release -p dominet --test ws_load -- --nocapture`.
//!
//! Guarantees checked: all the connections open; each client receives the **last state** of every device it is
//! subscribed to (a slow client may skip intermediate states, never the last one); the p99 of the latency (send into the hub
//! -> reception by the client) stays under the bound.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use dominet_domain::user::Role;
use dominet_proto::{DeviceStateDto, WsClientMsg, WsServerMsg};
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use futures_util::{SinkExt, StreamExt};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";
const DEVICES: i64 = 3;

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

fn state(device: i64, seq: u64, sent_us: u64) -> WsServerMsg {
    WsServerMsg::DeviceState(DeviceStateDto {
        device_id: device,
        friendly_name: format!("Appareil {device}"),
        state: serde_json::json!({ "seq": seq, "t": sent_us }),
        available: true,
    })
}

/// A client: opens the connection, subscribes to the devices, reads up to the last state of each; returns (latencies in microseconds, states received).
async fn client(
    addr: SocketAddr,
    cookie: String,
    t0: Instant,
    last_seq: Vec<u64>,
    ready: Arc<tokio::sync::Barrier>,
) -> Option<(Vec<u64>, usize)> {
    let mut req = format!("ws://{addr}/ws/notifications/")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    req.headers_mut()
        .insert(header::ORIGIN, format!("http://{addr}").parse().unwrap());
    let connected = tokio_tungstenite::connect_async(req).await.ok();
    // Everyone waits for everyone: publication only starts once the `N` connections are established.
    let ok = connected.is_some();
    ready.wait().await;
    let (mut ws, _) = connected?;
    for d in 1..=DEVICES {
        let sub = WsClientMsg::Subscribe {
            topic: format!("device:{d}"),
        };
        ws.send(Message::Text(serde_json::to_string(&sub).unwrap().into()))
            .await
            .ok()?;
    }
    let mut seen_last = vec![false; DEVICES as usize];
    let mut latencies = Vec::new();
    let mut received = 0_usize;
    let deadline = Instant::now() + Duration::from_secs(120);
    while seen_last.iter().any(|s| !s) && Instant::now() < deadline {
        let frame = tokio::time::timeout(Duration::from_secs(30), ws.next())
            .await
            .ok()??;
        match frame {
            Ok(Message::Text(t)) => {
                if let Ok(WsServerMsg::DeviceState(d)) = serde_json::from_str::<WsServerMsg>(&t) {
                    received += 1;
                    let seq = d.state["seq"].as_u64().unwrap_or(0);
                    let sent = d.state["t"].as_u64().unwrap_or(0);
                    let now = t0.elapsed().as_micros() as u64;
                    latencies.push(now.saturating_sub(sent));
                    let idx = (d.device_id - 1) as usize;
                    if seq == last_seq[idx] {
                        seen_last[idx] = true;
                    }
                }
            }
            Ok(Message::Ping(p)) => {
                ws.send(Message::Pong(p)).await.ok()?;
            }
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
    let _ = ok;
    seen_last
        .iter()
        .all(|s| *s)
        .then_some((latencies, received))
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn many_browsers_follow_a_stream_of_device_states(pool: PgPool) {
    let users = env_num("WS_LOAD_USERS", 10) as usize;
    let per_user = env_num("WS_LOAD_PER_USER", 8) as usize;
    let rate = env_num("WS_LOAD_RATE", 200);
    let total = env_num("WS_LOAD_MESSAGES", 1000);
    let p99_limit_ms = env_num("WS_LOAD_P99_MS", 3000);

    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    let state_app = AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    );
    let app = web::router(state_app.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });

    // Accounts and sessions: an account carries at most 8 connections (server limit).
    let rss_before = rss_mb();
    let mut cookies = Vec::new();
    for u in 0..users {
        let name = format!("charge{u}");
        state_app
            .auth
            .create_user(&name, "", "", Role::User, PASSWORD)
            .await
            .unwrap();
        let body = format!(r#"{{"username":"{name}","password":"{PASSWORD}"}}"#);
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::HOST, "dominet.test")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from((
            [192, 168, 1, 50 + (u % 150) as u8],
            40_000,
        ))));
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success(), "connexion de {name}");
        cookies.push(
            resp.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .split(';')
                .next()
                .unwrap()
                .to_string(),
        );
    }

    let clients = users * per_user;
    let t0 = Instant::now();
    // Last number sent to each device (round robin: device `d` receives the messages i such that i % DEVICES == d - 1).
    let last_seq: Vec<u64> = (1..=DEVICES as u64)
        .map(|d| {
            (0..total)
                .rev()
                .find(|i| i % DEVICES as u64 == d - 1)
                .unwrap()
        })
        .collect();
    let ready = Arc::new(tokio::sync::Barrier::new(clients + 1));
    let mut handles = Vec::new();
    for cookie in &cookies {
        for _ in 0..per_user {
            handles.push(tokio::spawn(client(
                addr,
                cookie.clone(),
                t0,
                last_seq.clone(),
                ready.clone(),
            )));
        }
    }
    ready.wait().await;
    // Let the subscriptions establish, then broadcast at a steady rate.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let connected = state_app.hub.connection_count();
    let rss_connected = rss_mb();
    let started = Instant::now();
    let tick = Duration::from_secs_f64(1.0 / rate as f64);
    for i in 0..total {
        let target = started + tick * u32::try_from(i).unwrap();
        tokio::time::sleep_until(tokio::time::Instant::from_std(target)).await;
        let device = (i % DEVICES as u64) as i64 + 1;
        let sent = t0.elapsed().as_micros() as u64;
        state_app.hub.publish_retained(
            &dominet_proto::Topic::Device(device),
            state(device, i, sent),
        );
    }
    let publish_secs = started.elapsed().as_secs_f64();

    let mut all = Vec::new();
    let mut complete = 0_usize;
    let mut received_total = 0_usize;
    for h in handles {
        if let Some((lat, n)) = h.await.unwrap() {
            complete += 1;
            received_total += n;
            all.extend(lat);
        }
    }
    all.sort_unstable();
    let pct = |p: f64| all[((all.len() as f64 - 1.0) * p) as usize] as f64 / 1000.0;
    let expected = clients as u64 * total;
    println!(
        "WS CHARGE : {clients} clients ({users} comptes x {per_user}), {total} etats a {rate}/s ({publish_secs:.1} s)\n  \
         connexions ouvertes : {connected}/{clients}\n  clients allant au dernier etat : {complete}/{clients}\n  \
         etats recus : {received_total}/{expected} ({:.1} %)\n  latence ms : p50 {:.1}  p95 {:.1}  p99 {:.1}  max {:.1}\n  \
         memoire du processus : {rss_before:.0} Mo -> {rss_connected:.0} Mo avec {clients} connexions -> {:.0} Mo",
        received_total as f64 * 100.0 / expected as f64,
        pct(0.5),
        pct(0.95),
        pct(0.99),
        pct(1.0),
        rss_mb(),
    );

    assert_eq!(connected, clients, "toutes les connexions doivent s'ouvrir");
    assert_eq!(
        complete, clients,
        "chaque client doit recevoir le dernier etat de chaque appareil"
    );
    assert!(
        pct(0.99) < p99_limit_ms as f64,
        "p99 {} ms au-dessus de la borne de {p99_limit_ms} ms",
        pct(0.99)
    );
}
