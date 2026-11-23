// SPDX-License-Identifier: AGPL-3.0-or-later
//! Endpoint `/ws/android/device/<token>/` (R5 lot 3): a real TCP server and a client that behaves like the Kotlin application
//! (same JSON frames, `OkHttp` control frames). The subject `android:<id>` is followed by a simulated browser.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use chrono::Utc;
use dominet_domain::android::{
    AndroidDevice, AndroidDeviceDraft, OutboxOrigin, OutboxStatus, SmsDirection,
};
use dominet_domain::messaging::{Channel, DeliveryStatus, MessageStatus};
use dominet_domain::repo::{AndroidDeviceRepo, CallRepo, MessageRepo, SmsOutboxRepo, SmsRepo};
use dominet_domain::user::{Role, UserId};
use dominet_proto::{AndroidEvent, RefusalReason, Topic, WsClientMsg, WsServerMsg};
use dominet_server::android::{
    AndroidRelay, CLOSE_IDLE, CLOSE_REPLACED, CLOSE_UNKNOWN_TOKEN, SendError, Timing, expire_stale,
};
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";
type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Server {
    app: Router,
    state: AppState,
    addr: SocketAddr,
}

async fn start(pool: PgPool, timing: Timing) -> Server {
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    let mut state = AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    );
    state.android = AndroidRelay::new(timing);
    let app = web::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    Server { app, state, addr }
}

impl Server {
    async fn user(&self, name: &str, role: Role) -> UserId {
        self.state
            .auth
            .create_user(name, "", "", role, PASSWORD)
            .await
            .unwrap()
            .id
    }

    async fn device(&self, owner: UserId, name: &str) -> (AndroidDevice, String) {
        let draft = AndroidDeviceDraft {
            name: name.into(),
            phone_number: "+33612345678".into(),
        };
        self.state
            .store
            .android_devices()
            .create(owner, &draft)
            .await
            .unwrap()
    }

    /// What the application does: opens `/ws/android/device/<token>/` without cookie or `Origin`.
    async fn app(&self, token: &str) -> Ws {
        let req = format!("ws://{}/ws/android/device/{token}/", self.addr)
            .into_client_request()
            .unwrap();
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }

    async fn login(&self, name: &str) -> String {
        let body = format!(r#"{{"username":"{name}","password":"{PASSWORD}"}}"#);
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header(header::HOST, "dominet.test")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40_000))));
        let resp = self.app.clone().oneshot(req).await.unwrap();
        assert!(
            resp.status().is_success(),
            "{:?}",
            resp.into_body().collect().await
        );
        resp.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    async fn browser(&self, cookie: &str) -> Ws {
        let mut req = format!("ws://{}/ws/notifications/", self.addr)
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        req.headers_mut().insert(
            header::ORIGIN,
            format!("http://{}", self.addr).parse().unwrap(),
        );
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }

    async fn is_online(&self, d: &AndroidDevice) -> bool {
        self.state
            .store
            .android_devices()
            .get(d.id)
            .await
            .unwrap()
            .unwrap()
            .is_online
    }

    /// Waits (at most 5 s) for a condition on the database to be true.
    async fn until<F: AsyncFn() -> bool>(&self, what: &str, f: F) {
        for _ in 0..100 {
            if f().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition jamais remplie : {what}");
    }
}

async fn json_next(ws: &mut Ws) -> Option<Value> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("delai depasse")
        {
            Some(Ok(Message::Text(t))) => return Some(serde_json::from_str(&t).unwrap()),
            Some(Ok(Message::Close(_)) | Err(_)) | None => return None,
            Some(Ok(_)) => {}
        }
    }
}

async fn close_code(ws: &mut Ws) -> Option<u16> {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("delai depasse")
        {
            Some(Ok(Message::Close(frame))) => return frame.map(|f| u16::from(f.code)),
            Some(Ok(_)) => {}
            Some(Err(_)) | None => return None,
        }
    }
}

async fn say(ws: &mut Ws, v: &Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

fn slow() -> Timing {
    Timing::default()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_known_token_gets_auth_ok_and_the_device_goes_online_then_offline(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel 7").await;
    let mut seen = s.state.hub.subscribe(&Topic::Android(d.id.0));
    let mut ws = s.app(&token).await;
    assert_eq!(
        json_next(&mut ws).await,
        Some(json!({"type": "auth_ok", "device_name": "Pixel 7"}))
    );
    assert!(s.is_online(&d).await);
    assert!(s.state.android.is_connected(d.id));
    let ev = seen.recv().await.unwrap();
    assert_eq!(
        *ev,
        WsServerMsg::Android {
            device_id: d.id.0,
            event: AndroidEvent::DeviceStatus { is_online: true }
        }
    );

    ws.close(None).await.unwrap();
    s.until("hors ligne", async || !s.is_online(&d).await).await;
    assert!(!s.state.android.is_connected(d.id));
    let ev = seen.recv().await.unwrap();
    assert_eq!(
        *ev,
        WsServerMsg::Android {
            device_id: d.id.0,
            event: AndroidEvent::DeviceStatus { is_online: false }
        }
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn unknown_or_malformed_tokens_are_closed_with_4001(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let other = "0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b";
    for bad in [
        other,
        "pas-un-jeton",
        &token.to_uppercase(),
        &token.replace('-', ""),
    ] {
        let mut ws = s.app(bad).await;
        assert_eq!(
            close_code(&mut ws).await,
            Some(CLOSE_UNKNOWN_TOKEN),
            "{bad}"
        );
    }
    assert!(!s.is_online(&d).await);
    assert_eq!(s.state.android.connected().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_regenerated_token_cuts_off_the_old_one(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, old) = s.device(owner, "Pixel").await;
    let fresh = s
        .state
        .store
        .android_devices()
        .regenerate_token(d.id)
        .await
        .unwrap();
    let mut ws = s.app(&old).await;
    assert_eq!(close_code(&mut ws).await, Some(CLOSE_UNKNOWN_TOKEN));
    let mut ws = s.app(&fresh).await;
    assert_eq!(json_next(&mut ws).await.unwrap()["type"], "auth_ok");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn what_the_app_sends_is_stored_and_shown_to_browsers(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let mut seen = s.state.hub.subscribe(&Topic::Android(d.id.0));
    let mut ws = s.app(&token).await;
    json_next(&mut ws).await;
    seen.recv().await.unwrap(); // presence

    say(&mut ws, &json!({"type": "sms_received", "from": "+33699999999", "body": "Salut", "contact_name": "Bob", "timestamp": "2026-01-02T10:00:00+01:00"})).await;
    let ev = seen.recv().await.unwrap();
    let WsServerMsg::Android {
        event: AndroidEvent::SmsReceived { sms },
        ..
    } = &*ev
    else {
        panic!("{ev:?}")
    };
    assert_eq!(
        (sms.body.as_str(), sms.direction.as_str(), sms.is_read),
        ("Salut", "incoming", false)
    );
    assert_eq!(sms.sent_at, "2026-01-02T09:00:00+00:00");
    let stored = s
        .state
        .store
        .sms()
        .list(Some(owner), None, 10)
        .await
        .unwrap();
    assert_eq!(
        (
            stored[0].phone_number.as_str(),
            stored[0].contact_name.as_str(),
            stored[0].direction
        ),
        ("+33699999999", "Bob", SmsDirection::Incoming)
    );

    // Call: ringing created, ended closed with its duration.
    say(&mut ws, &json!({"type": "call_event", "state": "ringing", "number": "0612", "contact_name": "", "timestamp": "2026-01-02T10:00:00+01:00", "duration": 0})).await;
    let ev = seen.recv().await.unwrap();
    let WsServerMsg::Android {
        event: AndroidEvent::CallEvent { call, state },
        ..
    } = &*ev
    else {
        panic!("{ev:?}")
    };
    assert_eq!(
        (
            state.as_str(),
            call.direction.as_str(),
            call.ended_at.clone()
        ),
        ("ringing", "incoming", None)
    );
    say(&mut ws, &json!({"type": "call_event", "state": "ended", "number": "0612", "contact_name": "", "timestamp": "2026-01-02T10:05:00+01:00", "duration": 300})).await;
    let ev = seen.recv().await.unwrap();
    let WsServerMsg::Android {
        event: AndroidEvent::CallEvent { call: ended, .. },
        ..
    } = &*ev
    else {
        panic!("{ev:?}")
    };
    assert_eq!((ended.id, ended.duration), (call.id, 300));
    assert_eq!(
        s.state
            .store
            .calls()
            .list(Some(owner), None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn junk_frames_are_ignored_and_the_connection_survives(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let mut ws = s.app(&token).await;
    json_next(&mut ws).await;
    for junk in [
        "pas du json",
        "[1,2]",
        "42",
        r#"{"type":"inconnu"}"#,
        r#"{"sans":"type"}"#,
        r#"{"type":"sms_received","from":"","body":"x"}"#,
        r#"{"type":"call_event","number":""}"#,
        r#"{"type":"sms_sent_ack","message_id":987654,"success":true}"#,
    ] {
        ws.send(Message::Text(junk.into())).await.unwrap();
    }
    // A JSON `ping` receives `pong`: the connection always answers, nothing was recorded.
    say(&mut ws, &json!({"type": "ping"})).await;
    assert_eq!(json_next(&mut ws).await, Some(json!({"type": "pong"})));
    assert_eq!(
        s.state
            .store
            .sms()
            .list(Some(owner), None, 10)
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        s.state
            .store
            .calls()
            .list(Some(owner), None, 10)
            .await
            .unwrap()
            .len(),
        0
    );
    assert!(s.is_online(&d).await);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn very_long_fields_are_clipped_not_refused(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (_, token) = s.device(owner, "Pixel").await;
    let mut ws = s.app(&token).await;
    json_next(&mut ws).await;
    say(&mut ws, &json!({"type": "sms_received", "from": "9".repeat(80), "contact_name": "c".repeat(300), "body": "é".repeat(9000)})).await;
    s.until("sms enregistre", async || {
        !s.state
            .store
            .sms()
            .list(Some(owner), None, 10)
            .await
            .unwrap()
            .is_empty()
    })
    .await;
    let sms = &s
        .state
        .store
        .sms()
        .list(Some(owner), None, 10)
        .await
        .unwrap()[0];
    assert_eq!(
        (
            sms.phone_number.chars().count(),
            sms.contact_name.chars().count(),
            sms.body.chars().count()
        ),
        (30, 100, 5000)
    );

    // A frame larger than 64 KiB cuts the connection (no wasted memory).
    ws.send(Message::Text("x".repeat(70_000).into()))
        .await
        .unwrap();
    s.until("coupee", async || {
        !s.state.android.connected().contains(&sms.device)
    })
    .await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_sms_order_reaches_the_app_and_its_ack_settles_the_queue(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let origin = OutboxOrigin::Browser(
        s.state
            .store
            .sms()
            .insert(&dominet_domain::android::NewSms {
                device: d.id,
                direction: SmsDirection::Outgoing,
                phone_number: "+33600000000".into(),
                contact_name: String::new(),
                body: "Coucou".into(),
                is_read: true,
                sent_at: Utc::now(),
            })
            .await
            .unwrap()
            .id,
    );
    // Offline: refused, nothing is queued.
    assert!(matches!(
        s.state
            .android
            .send_sms(&s.state, d.id, "+33600000000", "Coucou", origin)
            .await,
        Err(SendError::Offline)
    ));
    assert_eq!(
        s.state
            .store
            .sms_outbox()
            .pending_for_device(d.id)
            .await
            .unwrap()
            .len(),
        0
    );

    let mut ws = s.app(&token).await;
    json_next(&mut ws).await;
    let mut seen = s.state.hub.subscribe(&Topic::Android(d.id.0));
    let item = s
        .state
        .android
        .send_sms(&s.state, d.id, "+33600000000", "Coucou", origin)
        .await
        .unwrap();
    assert_eq!(
        json_next(&mut ws).await,
        Some(
            json!({"type": "send_sms", "to": "+33600000000", "body": "Coucou", "message_id": item.id.0})
        )
    );
    say(
        &mut ws,
        &json!({"type": "sms_sent_ack", "message_id": item.id.0, "success": true, "error": ""}),
    )
    .await;
    let ev = seen.recv().await.unwrap();
    assert_eq!(
        *ev,
        WsServerMsg::Android {
            device_id: d.id.0,
            event: AndroidEvent::SmsSentAck {
                message_id: item.id.0,
                success: true,
                error: String::new()
            }
        }
    );
    assert_eq!(
        s.state
            .store
            .sms_outbox()
            .get(item.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        OutboxStatus::Sent
    );

    // A second acknowledgement with the same identifier is not rebroadcast.
    say(&mut ws, &json!({"type": "sms_sent_ack", "message_id": item.id.0, "success": false, "error": "tard"})).await;
    say(&mut ws, &json!({"type": "ping"})).await;
    json_next(&mut ws).await;
    assert!(seen.try_recv().is_err());
    assert_eq!(
        s.state
            .store
            .sms_outbox()
            .get(item.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        OutboxStatus::Sent
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_ack_from_another_device_changes_nothing(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (a, ta) = s.device(owner, "A").await;
    let (_, tb) = s.device(owner, "B").await;
    let sms = s
        .state
        .store
        .sms()
        .insert(&dominet_domain::android::NewSms {
            device: a.id,
            direction: SmsDirection::Outgoing,
            phone_number: "+33600000000".into(),
            contact_name: String::new(),
            body: "x".into(),
            is_read: true,
            sent_at: Utc::now(),
        })
        .await
        .unwrap();
    let mut wa = s.app(&ta).await;
    json_next(&mut wa).await;
    let mut wb = s.app(&tb).await;
    json_next(&mut wb).await;
    let item = s
        .state
        .android
        .send_sms(
            &s.state,
            a.id,
            "+33600000000",
            "x",
            OutboxOrigin::Browser(sms.id),
        )
        .await
        .unwrap();
    json_next(&mut wa).await;
    say(
        &mut wb,
        &json!({"type": "sms_sent_ack", "message_id": item.id.0, "success": true}),
    )
    .await;
    say(&mut wb, &json!({"type": "ping"})).await;
    json_next(&mut wb).await;
    assert_eq!(
        s.state
            .store
            .sms_outbox()
            .get(item.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        OutboxStatus::Pending
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn messaging_deliveries_follow_the_acks_of_their_sms(pool: PgPool) {
    let s = start(pool, slow()).await;
    let sender = s.user("alice", Role::Admin).await;
    let (d, token) = s.device(sender, "Pixel").await;
    let (msg, deliveries) = s
        .state
        .store
        .messages()
        .create(
            sender,
            "Sujet",
            "Corps",
            Channel::Sms,
            &[(
                sender,
                DeliveryStatus::NotApplicable,
                DeliveryStatus::Pending,
            )],
        )
        .await
        .unwrap();
    let mut ws = s.app(&token).await;
    json_next(&mut ws).await;
    let item = s
        .state
        .android
        .send_sms(
            &s.state,
            d.id,
            "+33600000000",
            "Corps",
            OutboxOrigin::Messaging(deliveries[0].id),
        )
        .await
        .unwrap();
    json_next(&mut ws).await;
    say(&mut ws, &json!({"type": "sms_sent_ack", "message_id": item.id.0, "success": false, "error": "pas de reseau"})).await;
    s.until("message termine", async || {
        s.state
            .store
            .messages()
            .get(msg.id)
            .await
            .unwrap()
            .unwrap()
            .status
            == MessageStatus::Sent
    })
    .await;
    let after = s
        .state
        .store
        .messages()
        .delivery(deliveries[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (after.sms, after.error.as_str()),
        (DeliveryStatus::Failed, "pas de reseau")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn sms_without_an_ack_expire_after_the_deadline(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::Admin).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let (msg, deliveries) = s
        .state
        .store
        .messages()
        .create(
            owner,
            "S",
            "C",
            Channel::Sms,
            &[(
                owner,
                DeliveryStatus::NotApplicable,
                DeliveryStatus::Pending,
            )],
        )
        .await
        .unwrap();
    let mut ws = s.app(&token).await;
    json_next(&mut ws).await;
    let item = s
        .state
        .android
        .send_sms(
            &s.state,
            d.id,
            "+33600000000",
            "C",
            OutboxOrigin::Messaging(deliveries[0].id),
        )
        .await
        .unwrap();
    json_next(&mut ws).await;
    assert_eq!(expire_stale(&s.state, Utc::now()).await, 0, "trop tot");
    assert_eq!(
        expire_stale(&s.state, Utc::now() + chrono::Duration::seconds(61)).await,
        1
    );
    let got = s
        .state
        .store
        .sms_outbox()
        .get(item.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (got.status, got.error.as_str()),
        (OutboxStatus::Failed, "no acknowledgement from the device")
    );
    assert_eq!(
        s.state
            .store
            .messages()
            .get(msg.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        MessageStatus::Sent
    );
    // A late acknowledgement has no effect.
    say(
        &mut ws,
        &json!({"type": "sms_sent_ack", "message_id": item.id.0, "success": true}),
    )
    .await;
    say(&mut ws, &json!({"type": "ping"})).await;
    json_next(&mut ws).await;
    assert_eq!(
        s.state
            .store
            .sms_outbox()
            .get(item.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        OutboxStatus::Failed
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_new_connection_replaces_the_old_one_and_the_device_stays_online(pool: PgPool) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let mut first = s.app(&token).await;
    json_next(&mut first).await;
    let mut second = s.app(&token).await;
    json_next(&mut second).await;
    assert_eq!(close_code(&mut first).await, Some(CLOSE_REPLACED));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        s.is_online(&d).await,
        "la fermeture de l'ancienne ne met pas hors ligne"
    );
    assert!(s.state.android.is_connected(d.id));
    second.close(None).await.unwrap();
    s.until("hors ligne", async || !s.is_online(&d).await).await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_silent_line_is_cut_after_the_idle_delay_and_a_live_one_is_kept(pool: PgPool) {
    let s = start(
        pool,
        Timing {
            beat: Duration::from_millis(100),
            idle: Duration::from_millis(600),
        },
    )
    .await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    // Live line: we read (so the client answers the control frames) well beyond the delay.
    let mut live = s.app(&token).await;
    json_next(&mut live).await;
    let _ = tokio::time::timeout(Duration::from_millis(1500), live.next()).await;
    assert!(
        s.is_online(&d).await,
        "des pongs arrivent : la ligne est gardee"
    );
    drop(live);
    s.until("hors ligne", async || !s.is_online(&d).await).await;

    // Silent line: the client no longer reads, no pong goes out.
    let mut silent = s.app(&token).await;
    json_next(&mut silent).await;
    s.until("coupee", async || !s.is_online(&d).await).await;
    // The client sees the closing 4408 or, if the line has already dropped, an end of connection: never a line still open.
    assert!(matches!(
        close_code(&mut silent).await,
        Some(CLOSE_IDLE) | None
    ));
    assert!(!s.state.android.is_connected(d.id));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn alerts_are_pushed_to_every_connected_device(pool: PgPool) {
    let s = start(pool, slow()).await;
    let a = s.user("alice", Role::User).await;
    let b = s.user("bob", Role::User).await;
    let (_, ta) = s.device(a, "A").await;
    let (_, tb) = s.device(b, "B").await;
    let (offline, _) = s.device(b, "C").await;
    let mut wa = s.app(&ta).await;
    json_next(&mut wa).await;
    let mut wb = s.app(&tb).await;
    json_next(&mut wb).await;
    assert_eq!(
        s.state
            .android
            .push_alert(3, "Canicule", "critical", "[DomiNet] Canicule"),
        2
    );
    let want = json!({"type": "alert_notification", "alert_id": 3, "rule_name": "Canicule", "severity": "critical", "message": "[DomiNet] Canicule"});
    assert_eq!(json_next(&mut wa).await, Some(want.clone()));
    assert_eq!(json_next(&mut wb).await, Some(want));
    assert!(!s.state.android.is_connected(offline.id));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_browser_follows_only_its_own_devices_unless_admin(pool: PgPool) {
    let s = start(pool, slow()).await;
    let alice = s.user("alice", Role::User).await;
    s.user("bob", Role::User).await;
    s.user("root", Role::Admin).await;
    let (d, token) = s.device(alice, "Pixel").await;
    let topic = format!("android:{}", d.id.0);

    let mut app = s.app(&token).await;
    json_next(&mut app).await;

    for (who, allowed) in [("alice", true), ("bob", false), ("root", true)] {
        let cookie = s.login(who).await;
        let mut ws = s.browser(&cookie).await;
        json_next(&mut ws).await; // hello
        ws.send(Message::Text(
            serde_json::to_string(&WsClientMsg::Subscribe {
                topic: topic.clone(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let reply: WsServerMsg = serde_json::from_value(json_next(&mut ws).await.unwrap()).unwrap();
        if allowed {
            assert_eq!(
                reply,
                WsServerMsg::Subscribed {
                    topic: topic.clone()
                },
                "{who}"
            );
            // The last presence state is kept: it arrives immediately.
            let retained: WsServerMsg =
                serde_json::from_value(json_next(&mut ws).await.unwrap()).unwrap();
            assert_eq!(
                retained,
                WsServerMsg::Android {
                    device_id: d.id.0,
                    event: AndroidEvent::DeviceStatus { is_online: true }
                }
            );
        } else {
            assert_eq!(
                reply,
                WsServerMsg::Refused {
                    topic: topic.clone(),
                    reason: RefusalReason::Forbidden
                },
                "{who}"
            );
        }
    }
    // A device that does not exist is refused the same way.
    let cookie = s.login("root").await;
    let mut ws = s.browser(&cookie).await;
    json_next(&mut ws).await;
    ws.send(Message::Text(
        serde_json::to_string(&WsClientMsg::Subscribe {
            topic: "android:99999".into(),
        })
        .unwrap()
        .into(),
    ))
    .await
    .unwrap();
    let reply: WsServerMsg = serde_json::from_value(json_next(&mut ws).await.unwrap()).unwrap();
    assert!(matches!(
        reply,
        WsServerMsg::Refused {
            reason: RefusalReason::Forbidden,
            ..
        }
    ));
}

/// Calls a server function of the interface (without network, on the same state as the TCP server) with the session `cookie`.
async fn server_fn(s: &Server, cookie: &str, path: &str, form: &str) -> Value {
    let ui = dominet_web::ssr::ui_service(
        s.state.clone(),
        "target/site",
        SocketAddr::from(([127, 0, 0, 1], 3000)),
    );
    let app = web::router_with_ui(s.state.clone(), ui);
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/sf/{path}"))
        .header(header::HOST, "dominet.test")
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form.to_string()))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40_000))));
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(
        status.is_success(),
        "{path}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_sms_written_in_the_browser_reaches_the_app_and_its_delivery_follows_the_ack(
    pool: PgPool,
) {
    let s = start(pool, slow()).await;
    let owner = s.user("alice", Role::User).await;
    let (d, token) = s.device(owner, "Pixel").await;
    let cookie = s.login("alice").await;
    let mut seen = s.state.hub.subscribe(&Topic::Android(d.id.0));
    let mut app = s.app(&token).await;
    json_next(&mut app).await;
    seen.recv().await.unwrap(); // presence

    let id = d.id.0;
    let queued = server_fn(
        &s,
        &cookie,
        "android/sms/send",
        &format!("device={id}&to=%2B33600000000&body=Bonjour"),
    )
    .await;
    let sms = &queued["Queued"];
    assert_eq!(
        (
            sms["delivery"].as_str(),
            sms["direction"].as_str(),
            sms["body"].as_str()
        ),
        (Some("pending"), Some("outgoing"), Some("Bonjour"))
    );
    // The application receives the order with the queue identifier.
    let order = json_next(&mut app).await.unwrap();
    assert_eq!(
        (
            order["type"].as_str(),
            order["to"].as_str(),
            order["body"].as_str()
        ),
        (Some("send_sms"), Some("+33600000000"), Some("Bonjour"))
    );
    assert_eq!(order["message_id"], sms["outbox_id"]);
    // The other tabs are notified.
    let ev = seen.recv().await.unwrap();
    assert!(
        matches!(
            &*ev,
            WsServerMsg::Android {
                event: AndroidEvent::SmsQueued { .. },
                ..
            }
        ),
        "{ev:?}"
    );
    let page = server_fn(&s, &cookie, "android/sms", &format!("device={id}")).await;
    assert_eq!(page["sms"][0]["delivery"], "pending");

    say(&mut app, &json!({"type": "sms_sent_ack", "message_id": order["message_id"], "success": true, "error": ""})).await;
    seen.recv().await.unwrap(); // the acknowledgement
    let page = server_fn(&s, &cookie, "android/sms", &format!("device={id}")).await;
    assert_eq!(page["sms"][0]["delivery"], "sent");

    // A second SMS, failed: the reason is shown.
    server_fn(
        &s,
        &cookie,
        "android/sms/send",
        &format!("device={id}&to=%2B33611111111&body=Encore"),
    )
    .await;
    let order = json_next(&mut app).await.unwrap();
    say(&mut app, &json!({"type": "sms_sent_ack", "message_id": order["message_id"], "success": false, "error": "pas de reseau"})).await;
    s.until("echec visible", async || {
        server_fn(&s, &cookie, "android/sms", &format!("device={id}")).await["sms"][0]["delivery"]
            == "failed"
    })
    .await;
    let page = server_fn(&s, &cookie, "android/sms", &format!("device={id}")).await;
    assert_eq!(page["sms"][0]["error"], "pas de reseau");
    assert_eq!(page["devices"][0]["is_online"], true);
}
