// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests du WebSocket `/ws/notifications/` : un vrai serveur TCP et un vrai client WebSocket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use dominet_domain::user::Role;
use dominet_proto::{
    DeviceStateDto, NotificationLevel, RefusalReason, Topic, WsClientMsg, WsServerMsg,
};
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Serveur de test ecoutant sur un port libre ; `heartbeat` en secondes.
struct Server {
    app: Router,
    state: AppState,
    addr: SocketAddr,
}

async fn start(pool: PgPool, heartbeat: u32) -> Server {
    let beat = heartbeat.to_string();
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        "WS_HEARTBEAT_SECS" => Some(beat.clone()),
        _ => None,
    })
    .unwrap();
    let state = AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    );
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
    async fn user(&self, name: &str, role: Role) {
        self.state
            .auth
            .create_user(name, "", "", role, PASSWORD)
            .await
            .unwrap();
    }

    /// Connexion HTTP (sans reseau) ; renvoie le cookie de session.
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

    /// Poignee de main WebSocket avec les en-tetes donnes.
    async fn connect(
        &self,
        cookie: Option<&str>,
        origin: Option<&str>,
    ) -> Result<Client, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}/ws/notifications/", self.addr)
            .into_client_request()
            .unwrap();
        if let Some(c) = cookie {
            req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
        }
        if let Some(o) = origin {
            req.headers_mut().insert(header::ORIGIN, o.parse().unwrap());
        }
        tokio_tungstenite::connect_async(req)
            .await
            .map(|(ws, _)| ws)
    }

    fn same_origin(&self) -> String {
        format!("http://{}", self.addr)
    }
}

/// Prochain message JSON du serveur (ignore les trames de controle), avec delai maximal.
async fn next(ws: &mut Client) -> Option<WsServerMsg> {
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

/// Code de fermeture annonce par le serveur, `None` si la connexion s'est terminee sans trame de fermeture.
async fn close_code(ws: &mut Client) -> Option<u16> {
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

fn http_status(err: &tokio_tungstenite::tungstenite::Error) -> u16 {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => resp.status().as_u16(),
        other => panic!("erreur inattendue : {other}"),
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_authenticated_browser_gets_hello_then_broadcasts(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let cookie = s.login("bob").await;
    let mut ws = s
        .connect(Some(&cookie), Some(&s.same_origin()))
        .await
        .unwrap();

    assert_eq!(
        next(&mut ws).await,
        Some(WsServerMsg::Hello {
            username: "bob".into()
        })
    );
    let note = WsServerMsg::Notification {
        level: NotificationLevel::Info,
        text: "Bonjour".into(),
    };
    assert_eq!(s.state.hub.publish(&Topic::Notifications, note.clone()), 1);
    assert_eq!(next(&mut ws).await, Some(note));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_handshake_is_refused_without_a_session(pool: PgPool) {
    let s = start(pool, 25).await;
    let anon = s.connect(None, Some(&s.same_origin())).await.unwrap_err();
    assert_eq!(http_status(&anon), 401);
    let bogus = s
        .connect(Some("id=session-inventee"), Some(&s.same_origin()))
        .await
        .unwrap_err();
    assert_eq!(http_status(&bogus), 401);
    assert_eq!(s.state.hub.connection_count(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_foreign_origin_cannot_hijack_the_connection(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let cookie = s.login("bob").await;
    for evil in ["https://evil.example", "http://127.0.0.1:1", "null"] {
        let err = s.connect(Some(&cookie), Some(evil)).await.unwrap_err();
        assert_eq!(http_status(&err), 403, "{evil}");
    }
    assert_eq!(
        s.state.hub.connection_count(),
        0,
        "aucune connexion ouverte"
    );
    // Sans en-tete Origin (client hors navigateur), la session suffit.
    assert!(s.connect(Some(&cookie), None).await.is_ok());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn connections_per_account_are_limited(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let cookie = s.login("bob").await;
    let mut open = vec![];
    for _ in 0..dominet_server::hub::MAX_CONNECTIONS_PER_USER {
        open.push(
            s.connect(Some(&cookie), Some(&s.same_origin()))
                .await
                .unwrap(),
        );
    }
    let err = s
        .connect(Some(&cookie), Some(&s.same_origin()))
        .await
        .unwrap_err();
    assert_eq!(http_status(&err), 429);
    drop(open.pop());
    // Le compteur redescend quand le serveur constate la fermeture.
    for _ in 0..50 {
        if s.state.hub.connection_count() < dominet_server::hub::MAX_CONNECTIONS_PER_USER {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        s.connect(Some(&cookie), Some(&s.same_origin()))
            .await
            .is_ok()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_client_that_answers_pings_stays_connected_and_a_silent_one_is_dropped(pool: PgPool) {
    let s = start(pool, 1).await;
    s.user("bob", Role::User).await;
    let cookie = s.login("bob").await;
    let mut good = s
        .connect(Some(&cookie), Some(&s.same_origin()))
        .await
        .unwrap();
    let mut silent = s
        .connect(Some(&cookie), Some(&s.same_origin()))
        .await
        .unwrap();
    assert!(matches!(
        next(&mut good).await,
        Some(WsServerMsg::Hello { .. })
    ));

    // 4 battements : le bon client repond a chaque ping, l'autre jamais.
    let mut pings = 0;
    while pings < 4 {
        match next(&mut good).await {
            Some(WsServerMsg::Ping) => {
                good.send(Message::Text(r#"{"type":"pong"}"#.into()))
                    .await
                    .unwrap();
                pings += 1;
            }
            other => panic!("attendu un ping : {other:?}"),
        }
    }
    assert!(matches!(
        next(&mut silent).await,
        Some(WsServerMsg::Hello { .. })
    ));
    assert_eq!(
        close_code(&mut silent).await,
        Some(dominet_server::web::CLOSE_NO_HEARTBEAT),
        "silencieux : ferme"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn losing_the_session_cuts_the_realtime_connection(pool: PgPool) {
    let s = start(pool, 1).await;
    s.user("bob", Role::User).await;
    s.user("carol", Role::User).await;
    let (bob, carol) = (s.login("bob").await, s.login("carol").await);
    let mut ws_bob = s.connect(Some(&bob), Some(&s.same_origin())).await.unwrap();
    let mut ws_carol = s
        .connect(Some(&carol), Some(&s.same_origin()))
        .await
        .unwrap();

    // Bob se deconnecte (session supprimee) ; la connexion de Carol n'est pas touchee.
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/auth/logout")
        .header(header::HOST, "dominet.test")
        .header(header::COOKIE, bob.clone())
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40_000))));
    assert!(
        s.app
            .clone()
            .oneshot(req)
            .await
            .unwrap()
            .status()
            .is_success()
    );

    assert!(matches!(
        next(&mut ws_bob).await,
        Some(WsServerMsg::Hello { .. })
    ));
    assert_eq!(
        close_code(&mut ws_bob).await,
        Some(dominet_server::web::CLOSE_SESSION_ENDED)
    );
    assert!(matches!(
        next(&mut ws_carol).await,
        Some(WsServerMsg::Hello { .. })
    ));
    assert!(
        matches!(next(&mut ws_carol).await, Some(WsServerMsg::Ping)),
        "Carol continue de recevoir ses battements"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deactivating_or_resetting_the_account_cuts_the_connection(pool: PgPool) {
    use dominet_domain::repo::UserRepo;
    use dominet_domain::user::UserProfile;
    let s = start(pool, 1).await;
    s.user("bob", Role::User).await;
    let cookie = s.login("bob").await;
    let mut ws = s
        .connect(Some(&cookie), Some(&s.same_origin()))
        .await
        .unwrap();
    assert!(matches!(
        next(&mut ws).await,
        Some(WsServerMsg::Hello { .. })
    ));

    let bob = s
        .state
        .store
        .users()
        .find_by_username("bob")
        .await
        .unwrap()
        .unwrap();
    let profile = UserProfile {
        email: String::new(),
        first_name: String::new(),
        last_name: String::new(),
        mobile: String::new(),
        role: Role::User,
        is_active: false,
    };
    s.state
        .store
        .users()
        .update_profile(bob.id, &profile)
        .await
        .unwrap();
    assert_eq!(
        close_code(&mut ws).await,
        Some(dominet_server::web::CLOSE_SESSION_ENDED)
    );
}

// ---------------------------------------------------------------- sujets, abonnements, dernier etat retenu

fn device(id: i64, v: i64) -> WsServerMsg {
    WsServerMsg::DeviceState(DeviceStateDto {
        device_id: id,
        friendly_name: format!("Appareil {id}"),
        state: serde_json::json!({ "v": v }),
        available: true,
    })
}

async fn tell(ws: &mut Client, msg: &WsClientMsg) {
    ws.send(Message::Text(serde_json::to_string(msg).unwrap().into()))
        .await
        .unwrap();
}

async fn subscribe(ws: &mut Client, topic: &str) {
    tell(
        ws,
        &WsClientMsg::Subscribe {
            topic: topic.into(),
        },
    )
    .await;
}

// Renvoient un `Option` pour se comparer directement au resultat de `next`.
#[allow(clippy::unnecessary_wraps)]
fn subscribed(topic: &str) -> Option<WsServerMsg> {
    Some(WsServerMsg::Subscribed {
        topic: topic.into(),
    })
}

#[allow(clippy::unnecessary_wraps)]
fn refused(topic: &str, reason: RefusalReason) -> Option<WsServerMsg> {
    Some(WsServerMsg::Refused {
        topic: topic.into(),
        reason,
    })
}

/// Connexion authentifiee, `Hello` deja lu.
async fn open(s: &Server, user: &str) -> Client {
    let cookie = s.login(user).await;
    let mut ws = s
        .connect(Some(&cookie), Some(&s.same_origin()))
        .await
        .unwrap();
    assert!(matches!(
        next(&mut ws).await,
        Some(WsServerMsg::Hello { .. })
    ));
    ws
}

/// Un message temoin sur les notifications : s'il arrive en premier, rien d'autre n'etait en attente avant lui.
async fn assert_nothing_pending(s: &Server, ws: &mut Client) {
    let marker = WsServerMsg::Notification {
        level: NotificationLevel::Info,
        text: "temoin".into(),
    };
    assert!(s.state.hub.publish(&Topic::Notifications, marker.clone()) >= 1);
    assert_eq!(
        next(ws).await,
        Some(marker),
        "un message non attendu est arrive avant le temoin"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn subscribing_gives_an_ack_then_the_last_retained_state_then_live_updates(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let t = Topic::Device(7);
    s.state.hub.publish_retained(&t, device(7, 1)); // publie AVANT l'abonnement
    let mut ws = open(&s, "bob").await;

    subscribe(&mut ws, "device:7").await;
    assert_eq!(next(&mut ws).await, subscribed("device:7"));
    assert_eq!(
        next(&mut ws).await,
        Some(device(7, 1)),
        "dernier etat connu des l'abonnement"
    );
    assert_eq!(s.state.hub.publish_retained(&t, device(7, 2)), 1);
    assert_eq!(next(&mut ws).await, Some(device(7, 2)));

    // Un autre appareil, non demande, n'arrive pas.
    s.state
        .hub
        .publish_retained(&Topic::Device(8), device(8, 1));
    assert_nothing_pending(&s, &mut ws).await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn system_stats_reach_subscribers_with_the_last_state_first(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let mut stats = dominet_server::system_stats::sample_once()
        .await
        .expect("mesure");
    stats.cpu.percent = 12.3;
    s.state
        .hub
        .publish_retained(&Topic::System, WsServerMsg::SystemStats(stats));
    let mut ws = open(&s, "bob").await;
    subscribe(&mut ws, "system").await;
    assert_eq!(next(&mut ws).await, subscribed("system"));
    assert_eq!(next(&mut ws).await, Some(WsServerMsg::SystemStats(stats)));
    stats.cpu.percent = 45.6;
    assert_eq!(
        s.state
            .hub
            .publish_retained(&Topic::System, WsServerMsg::SystemStats(stats)),
        1
    );
    assert_eq!(next(&mut ws).await, Some(WsServerMsg::SystemStats(stats)));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_subscription_with_no_retained_state_just_acknowledges(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let mut ws = open(&s, "bob").await;
    subscribe(&mut ws, "device:99").await;
    assert_eq!(next(&mut ws).await, subscribed("device:99"));
    assert_nothing_pending(&s, &mut ws).await;
    subscribe(&mut ws, "system").await;
    assert_eq!(next(&mut ws).await, subscribed("system"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn unsubscribing_stops_delivery_and_resubscribing_is_idempotent(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let t = Topic::Device(7);
    let mut ws = open(&s, "bob").await;
    subscribe(&mut ws, "device:7").await;
    assert_eq!(next(&mut ws).await, subscribed("device:7"));
    // Deux fois le meme abonnement : un seul abonnement, donc une seule copie de chaque message.
    subscribe(&mut ws, "device:7").await;
    assert_eq!(next(&mut ws).await, subscribed("device:7"));
    assert_eq!(
        s.state.hub.publish_retained(&t, device(7, 1)),
        1,
        "un seul abonne cote hub"
    );
    assert_eq!(next(&mut ws).await, Some(device(7, 1)));
    assert_nothing_pending(&s, &mut ws).await;

    tell(
        &mut ws,
        &WsClientMsg::Unsubscribe {
            topic: "device:7".into(),
        },
    )
    .await;
    // Barriere : une demande suivante, traitee apres la precedente, prouve que le desabonnement est pris en compte.
    subscribe(&mut ws, "system").await;
    assert_eq!(next(&mut ws).await, subscribed("system"));
    s.state.hub.publish_retained(&t, device(7, 2));
    assert_nothing_pending(&s, &mut ws).await;
    // Se reabonner redonne le dernier etat.
    subscribe(&mut ws, "device:7").await;
    assert_eq!(next(&mut ws).await, subscribed("device:7"));
    assert_eq!(next(&mut ws).await, Some(device(7, 2)));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn notifications_cannot_be_unsubscribed_and_junk_is_ignored(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let mut ws = open(&s, "bob").await;
    tell(
        &mut ws,
        &WsClientMsg::Unsubscribe {
            topic: "notifications".into(),
        },
    )
    .await;
    ws.send(Message::Text("pas du json".into())).await.unwrap();
    ws.send(Message::Text(r#"{"type":"inconnu"}"#.into()))
        .await
        .unwrap();
    ws.send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    assert_nothing_pending(&s, &mut ws).await; // les notifications arrivent toujours, la connexion vit
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_forbidden_and_excessive_subscriptions_are_refused(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let mut ws = open(&s, "bob").await;
    for bad in ["device:07", "device:0", "nope", "", "Device:7", "device:7 "] {
        subscribe(&mut ws, bad).await;
        assert_eq!(
            next(&mut ws).await,
            refused(bad, RefusalReason::Invalid),
            "{bad:?}"
        );
    }
    let cam = "camera:123e4567-e89b-12d3-a456-426614174000";
    subscribe(&mut ws, cam).await;
    assert_eq!(
        next(&mut ws).await,
        refused(cam, RefusalReason::Forbidden),
        "camera inconnue : refus, comme une camera inactive"
    );
    // Texte tres long : tronque dans la reponse.
    let long = "x".repeat(500);
    subscribe(&mut ws, &long).await;
    let Some(WsServerMsg::Refused { topic, .. }) = next(&mut ws).await else {
        panic!("refus attendu")
    };
    assert!(topic.len() <= 80);

    // 128 abonnements au plus, notifications comprises : 127 appareils passent, le suivant est refuse.
    for id in 1..=127 {
        subscribe(&mut ws, &format!("device:{id}")).await;
        assert_eq!(
            next(&mut ws).await,
            subscribed(&format!("device:{id}")),
            "appareil {id}"
        );
    }
    subscribe(&mut ws, "device:128").await;
    assert_eq!(
        next(&mut ws).await,
        refused("device:128", RefusalReason::Limit)
    );
    // Se desabonner libere de la place.
    tell(
        &mut ws,
        &WsClientMsg::Unsubscribe {
            topic: "device:1".into(),
        },
    )
    .await;
    subscribe(&mut ws, "device:128").await;
    assert_eq!(next(&mut ws).await, subscribed("device:128"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn connections_have_independent_subscriptions(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    s.user("carol", Role::User).await;
    let (mut a, mut b) = (open(&s, "bob").await, open(&s, "carol").await);
    subscribe(&mut a, "device:7").await;
    assert_eq!(next(&mut a).await, subscribed("device:7"));
    assert_eq!(
        s.state
            .hub
            .publish_retained(&Topic::Device(7), device(7, 1)),
        1,
        "seule la connexion abonnee recoit"
    );
    assert_eq!(next(&mut a).await, Some(device(7, 1)));
    assert_nothing_pending(&s, &mut b).await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn closing_a_connection_releases_its_hub_subscriptions(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let mut ws = open(&s, "bob").await;
    subscribe(&mut ws, "device:7").await;
    assert_eq!(next(&mut ws).await, subscribed("device:7"));
    assert_eq!(s.state.hub.publish(&Topic::Device(7), device(7, 1)), 1);
    drop(ws);
    // Le serveur s'en apercoit, ses taches d'abonnement s'arretent : plus aucun abonne, ni sur l'appareil ni aux notifications.
    for _ in 0..100 {
        if s.state.hub.publish(&Topic::Device(7), device(7, 2)) == 0
            && s.state
                .hub
                .publish(&Topic::Notifications, WsServerMsg::Ping)
                == 0
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("des abonnements sont restes ouverts apres la fermeture de la connexion");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_discovery_topic_is_for_administrators_only(pool: PgPool) {
    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    s.user("alice", Role::Admin).await;
    let mut user = open(&s, "bob").await;
    subscribe(&mut user, "discovery").await;
    assert_eq!(
        next(&mut user).await,
        refused("discovery", RefusalReason::Forbidden)
    );
    let mut admin = open(&s, "alice").await;
    subscribe(&mut admin, "discovery").await;
    assert_eq!(next(&mut admin).await, subscribed("discovery"));
    let event = dominet_proto::DiscoveryEvent::Done;
    s.state.hub.publish(
        &Topic::Discovery,
        WsServerMsg::Discovery {
            event: event.clone(),
        },
    );
    assert_eq!(
        next(&mut admin).await,
        Some(WsServerMsg::Discovery { event })
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_active_camera_can_be_followed_by_any_account_and_an_inactive_one_cannot(pool: PgPool) {
    use dominet_domain::camera::CameraDraft;
    use dominet_server::cameras::{CameraInput, SecretInput};

    let s = start(pool, 25).await;
    s.user("bob", Role::User).await;
    let input = |name: &str, active: bool| {
        let mut draft = CameraDraft::new(name);
        draft.ip = "192.168.1.29".into();
        draft.active = active;
        CameraInput {
            draft,
            rtsp_pass: SecretInput::Keep,
            onvif_pass: SecretInput::Keep,
            web_pass: SecretInput::Keep,
            web_api_key: SecretInput::Keep,
        }
    };
    let (on, _) = s
        .state
        .cameras
        .save(None, input("Entree", true))
        .await
        .unwrap();
    let (off, _) = s
        .state
        .cameras
        .save(None, input("Cave", false))
        .await
        .unwrap();
    let topic = |c: &dominet_domain::camera::Camera| format!("camera:{}", c.uuid.0.hyphenated());

    let mut ws = open(&s, "bob").await;
    subscribe(&mut ws, &topic(&on)).await;
    assert_eq!(next(&mut ws).await, subscribed(&topic(&on)));
    for refused_topic in [
        topic(&off),
        "camera:123e4567-e89b-12d3-a456-426614174000".to_string(),
    ] {
        subscribe(&mut ws, &refused_topic).await;
        assert_eq!(
            next(&mut ws).await,
            refused(&refused_topic, RefusalReason::Forbidden),
            "{refused_topic}"
        );
    }
}
