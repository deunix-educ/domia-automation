// SPDX-License-Identifier: AGPL-3.0-or-later
//! Asynchronous messaging (R5 lot 5): e-mails to a fake SMTP, SMS handed to a fake phone (WebSocket client that speaks like
//! the Kotlin application), per-recipient statuses, `message:<id>` events and subscription rights. Nothing is really sent.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use chrono::Utc;
use dominet_domain::android::AndroidDeviceDraft;
use dominet_domain::messaging::{
    Channel, Delivery, DeliveryStatus, Message, MessageDraft, MessageId, MessageStatus,
};
use dominet_domain::repo::{AndroidDeviceRepo, MessageRepo, UserRepo};
use dominet_domain::user::{Role, User, UserId, UserProfile};
use dominet_notify::Security;
use dominet_notify::testing::{FakeSmtp, SmtpBehavior};
use dominet_proto::{RefusalReason, Topic, WsServerMsg};
use dominet_server::auth::password::Hasher;
use dominet_server::config::{Config, EmailConfig};
use dominet_server::messaging;
use dominet_server::{AppState, web};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";
type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Server {
    app: Router,
    state: AppState,
    addr: SocketAddr,
    smtp: FakeSmtp,
}

async fn start(pool: PgPool) -> Server {
    let smtp = FakeSmtp::start().await;
    let mut config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        _ => None,
    })
    .unwrap();
    config.email = Some(EmailConfig {
        host: "127.0.0.1".into(),
        port: smtp.addr().port(),
        user: None,
        password: None,
        security: Security::Plain,
        timeout_secs: 5,
    });
    let state = AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    );
    state
        .settings
        .set("DEFAULT_FROM_EMAIL", "maison@example.com")
        .await
        .unwrap();
    let app = web::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    Server {
        app,
        state,
        addr,
        smtp,
    }
}

impl Server {
    async fn user(&self, name: &str, role: Role, email: &str, mobile: &str) -> User {
        let u = self
            .state
            .auth
            .create_user(name, "", "", role, PASSWORD)
            .await
            .unwrap();
        self.state
            .store
            .users()
            .update_profile(
                u.id,
                &UserProfile {
                    email: email.into(),
                    first_name: String::new(),
                    last_name: String::new(),
                    mobile: mobile.into(),
                    role,
                    is_active: true,
                },
            )
            .await
            .unwrap()
    }

    /// A connected phone, like the Kotlin application.
    async fn phone(&self, owner: UserId) -> Ws {
        let (_, token) = self
            .state
            .store
            .android_devices()
            .create(
                owner,
                &AndroidDeviceDraft {
                    name: "Pixel".into(),
                    phone_number: "+33612345678".into(),
                },
            )
            .await
            .unwrap();
        let req = format!("ws://{}/ws/android/device/{token}/", self.addr)
            .into_client_request()
            .unwrap();
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        assert_eq!(next_json(&mut ws).await["type"], "auth_ok");
        ws
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

    async fn browser(&self, name: &str) -> Ws {
        let cookie = self.login(name).await;
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

    async fn compose(
        &self,
        sender: &User,
        channel: Channel,
        to: &[&User],
    ) -> (Message, Vec<Delivery>) {
        let draft = MessageDraft {
            subject: "Coupure d'eau".into(),
            body: "Demain de 9h a 12h.".into(),
            channel,
            recipients: to.iter().map(|u| u.id).collect(),
        };
        let m = messaging::send(&self.state, sender.id, &draft, to)
            .await
            .unwrap();
        let d = self.state.store.messages().deliveries(m.id).await.unwrap();
        (m, d)
    }

    async fn message(&self, id: MessageId) -> Message {
        self.state.store.messages().get(id).await.unwrap().unwrap()
    }

    async fn deliveries(&self, id: MessageId) -> Vec<Delivery> {
        self.state.store.messages().deliveries(id).await.unwrap()
    }

    async fn until<F: AsyncFn() -> bool>(&self, what: &str, f: F) {
        for _ in 0..200 {
            if f().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition jamais remplie : {what}");
    }
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("delai depasse")
        {
            Some(Ok(WsMessage::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(WsMessage::Close(_)) | Err(_)) | None => panic!("connexion fermee"),
            Some(Ok(_)) => {}
        }
    }
}

async fn next_msg(ws: &mut Ws) -> WsServerMsg {
    serde_json::from_value(next_json(ws).await).unwrap()
}

async fn say(ws: &mut Ws, v: &Value) {
    ws.send(WsMessage::Text(v.to_string().into()))
        .await
        .unwrap();
}

fn status(d: &[Delivery], who: UserId) -> (DeliveryStatus, DeliveryStatus, String) {
    let d = d.iter().find(|d| d.recipient == who).unwrap();
    (d.email, d.sms, d.error.clone())
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn emails_and_sms_are_sent_and_statuses_follow_the_phone_ack(pool: PgPool) {
    use DeliveryStatus::{NotApplicable as Na, Pending, Sent};
    let s = start(pool).await;
    let admin = s.user("root", Role::Admin, "root@example.com", "").await;
    let alice = s
        .user("alice", Role::User, "alice@example.com", "+33611111111")
        .await;
    let bob = s.user("bob", Role::User, "bob@example.com", "").await;
    let carl = s.user("carl", Role::User, "", "").await;
    let mut phone = s.phone(admin.id).await;

    let (m, initial) = s
        .compose(&admin, Channel::Both, &[&alice, &bob, &carl])
        .await;
    assert_eq!(m.status, MessageStatus::Sending);
    assert_eq!(m.recipients_count, 3);
    assert_eq!(status(&initial, alice.id).0, Pending);
    assert_eq!(status(&initial, carl.id), (Na, Na, String::new()));

    // The phone receives the send order, with the recipient's number and an identifier of its own.
    let order = next_json(&mut phone).await;
    assert_eq!(order["type"], "send_sms");
    assert_eq!(order["to"], "+33611111111");
    assert_eq!(order["body"], "Demain de 9h a 12h.");
    let id = order["message_id"].as_i64().unwrap();

    // The e-mails went out before the acknowledgement; the message is not finished as long as the SMS is waiting.
    s.until("deux e-mails", async || s.smtp.mails().len() == 2)
        .await;
    let mails = s.smtp.mails();
    let mut to: Vec<_> = mails.iter().map(|m| m.to[0].clone()).collect();
    to.sort();
    assert_eq!(to, ["alice@example.com", "bob@example.com"]);
    assert_eq!(mails[0].from, "maison@example.com");
    assert_eq!(mails[0].header("subject"), ["Coupure d'eau"]);
    assert!(mails[0].body().contains("Demain de 9h a 12h."));
    s.until("e-mails notes", async || {
        let d = s.deliveries(m.id).await;
        status(&d, alice.id).0 == Sent && status(&d, bob.id).0 == Sent
    })
    .await;
    assert_eq!(s.message(m.id).await.status, MessageStatus::Sending);
    assert_eq!(status(&s.deliveries(m.id).await, alice.id).1, Pending);

    say(
        &mut phone,
        &json!({"type": "sms_sent_ack", "message_id": id, "success": true}),
    )
    .await;
    s.until("message termine", async || {
        s.message(m.id).await.status == MessageStatus::Sent
    })
    .await;
    let done = s.deliveries(m.id).await;
    assert_eq!(status(&done, alice.id), (Sent, Sent, String::new()));
    assert_eq!(status(&done, bob.id), (Sent, Na, String::new()));
    assert_eq!(status(&done, carl.id), (Na, Na, String::new()));
    assert!(s.message(m.id).await.sent_at.is_some());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn failures_are_recorded_per_recipient_and_the_message_still_finishes(pool: PgPool) {
    use DeliveryStatus::Failed;
    let s = start(pool).await;
    let admin = s.user("root", Role::Admin, "root@example.com", "").await;
    let alice = s
        .user("alice", Role::User, "alice@example.com", "+33611111111")
        .await;
    s.smtp.set_behavior(SmtpBehavior::RejectRecipients);

    // SMTP server that refuses + no phone connected: everything fails, with the reason.
    let (m, _) = s.compose(&admin, Channel::Both, &[&alice]).await;
    s.until("message termine", async || {
        s.message(m.id).await.status == MessageStatus::Sent
    })
    .await;
    let (email, sms, error) = status(&s.deliveries(m.id).await, alice.id);
    assert_eq!((email, sms), (Failed, Failed));
    assert!(error.contains("email: "), "{error}");
    assert!(error.contains(messaging::ERR_NO_DEVICE), "{error}");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_sms_without_ack_fails_after_the_delay(pool: PgPool) {
    let s = start(pool).await;
    let admin = s.user("root", Role::Admin, "", "").await;
    let alice = s.user("alice", Role::User, "", "+33611111111").await;
    let mut phone = s.phone(admin.id).await;
    let (m, _) = s.compose(&admin, Channel::Sms, &[&alice]).await;
    assert_eq!(next_json(&mut phone).await["type"], "send_sms");
    // The phone never answers: a minute later the sweep declares the failure.
    s.until("ordre en file", async || {
        s.deliveries(m.id).await[0].sms == DeliveryStatus::Pending
    })
    .await;
    let later = Utc::now() + chrono::Duration::seconds(61);
    assert_eq!(
        dominet_server::android::expire_stale(&s.state, later).await,
        1
    );
    let (_, sms, error) = status(&s.deliveries(m.id).await, alice.id);
    assert_eq!(sms, DeliveryStatus::Failed);
    assert!(error.contains("no acknowledgement"), "{error}");
    s.until("message termine", async || {
        s.message(m.id).await.status == MessageStatus::Sent
    })
    .await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn browsers_follow_the_progress_of_their_own_messages_only(pool: PgPool) {
    let s = start(pool).await;
    let admin = s.user("root", Role::Admin, "root@example.com", "").await;
    let alice = s.user("alice", Role::User, "alice@example.com", "").await;
    let _eve = s.user("eve", Role::User, "eve@example.com", "").await;
    let (m, _) = s.compose(&admin, Channel::Email, &[&alice]).await;
    let topic = Topic::Message(m.id.0).to_string();
    let follow = json!({"type": "subscribe", "topic": topic});

    for (name, allowed) in [("alice", true), ("root", true), ("eve", false)] {
        let mut ws = s.browser(name).await;
        assert_eq!(
            next_msg(&mut ws).await,
            WsServerMsg::Hello {
                username: name.into()
            }
        );
        say(&mut ws, &follow).await;
        loop {
            match next_msg(&mut ws).await {
                WsServerMsg::Subscribed { topic: t } => {
                    assert!(allowed, "{name} ne devrait pas pouvoir suivre {t}");
                    break;
                }
                WsServerMsg::Refused { reason, .. } => {
                    assert!(!allowed, "{name} devrait pouvoir suivre");
                    assert_eq!(reason, RefusalReason::Forbidden);
                    break;
                }
                _ => {}
            }
        }
    }
    // An unknown message is refused like another person's.
    let mut ws = s.browser("alice").await;
    next_msg(&mut ws).await;
    say(
        &mut ws,
        &json!({"type": "subscribe", "topic": "message:99999"}),
    )
    .await;
    loop {
        if let WsServerMsg::Refused { reason, .. } = next_msg(&mut ws).await {
            assert_eq!(reason, RefusalReason::Forbidden);
            break;
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn progress_events_reach_a_subscribed_browser_until_complete(pool: PgPool) {
    let s = start(pool).await;
    let admin = s.user("root", Role::Admin, "root@example.com", "").await;
    let alice = s.user("alice", Role::User, "alice@example.com", "").await;
    let mut hub = {
        // Fresh database: the first message will have the identifier 1 (we subscribe before sending so as not to miss anything).
        let next = s
            .state
            .store
            .messages()
            .list(1, 0)
            .await
            .unwrap()
            .first()
            .map_or(1, |m| m.id.0 + 1);
        s.state.hub.subscribe(&Topic::Message(next))
    };
    let (m, _) = s.compose(&admin, Channel::Email, &[&alice]).await;
    let mut complete = false;
    while !complete {
        let ev = tokio::time::timeout(Duration::from_secs(10), hub.recv())
            .await
            .expect("pas d'evenement")
            .unwrap();
        let WsServerMsg::MessageProgress {
            message_id,
            complete: c,
        } = &*ev
        else {
            panic!("{ev:?}")
        };
        assert_eq!(*message_id, m.id.0);
        complete = *c;
    }
    assert_eq!(s.message(m.id).await.status, MessageStatus::Sent);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_message_interrupted_by_a_restart_is_closed_with_its_pending_sends_failed(pool: PgPool) {
    let s = start(pool).await;
    let admin = s.user("root", Role::Admin, "root@example.com", "").await;
    let alice = s.user("alice", Role::User, "alice@example.com", "").await;
    // A `sending` message that nobody is handling (the server stopped before sending).
    let (m, _) = s
        .state
        .store
        .messages()
        .create(
            admin.id,
            "Orphelin",
            "corps",
            Channel::Email,
            &[(
                alice.id,
                DeliveryStatus::Pending,
                DeliveryStatus::NotApplicable,
            )],
        )
        .await
        .unwrap();
    assert_eq!(messaging::recover_interrupted(&s.state).await, 1);
    let (email, _, error) = status(&s.deliveries(m.id).await, alice.id);
    assert_eq!(email, DeliveryStatus::Failed);
    assert!(error.contains("restart"), "{error}");
    assert_eq!(s.message(m.id).await.status, MessageStatus::Sent);
    assert_eq!(messaging::recover_interrupted(&s.state).await, 0);
}
