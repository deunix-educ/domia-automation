// SPDX-License-Identifier: AGPL-3.0-or-later
//! WebSocket de pilotage PTZ (R6 lot 5) : un vrai client WebSocket pilote une camera ONVIF simulee (faux serveur qui verifie l'authentification).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use dominet_domain::user::Role;
use dominet_server::auth::password::Hasher;
use dominet_server::cameras::{CameraInput, SecretInput};
use dominet_server::config::Config;
use dominet_server::secrets::MasterKey;
use dominet_server::{AppState, web};
use dominet_surveillance::testing::onvif::{FakeOnvif, Family};
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
type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Rig {
    app: Router,
    state: AppState,
    addr: SocketAddr,
    onvif: FakeOnvif,
    uuid: String,
}

async fn rig(pool: PgPool, dead_man: &'static str) -> Rig {
    let key = MasterKey::generate();
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        "DOMINET_MASTER_KEY" => Some(key.clone()),
        "PTZ_DEAD_MAN_SECS" => Some(dead_man.into()),
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

    let onvif = FakeOnvif::start(Family::Thingino, "admin", "mot-de-passe").await;
    let mut draft = dominet_domain::camera::CameraDraft::new("Salon");
    draft.ip = "127.0.0.1".into();
    draft.onvif = true;
    draft.onvif_port = i32::from(onvif.port());
    draft.rtsp_user = "admin".into();
    draft.ptz = true;
    let (camera, _) = state
        .cameras
        .save(
            None,
            CameraInput {
                draft,
                rtsp_pass: SecretInput::Set("mot-de-passe".into()),
                onvif_pass: SecretInput::Keep,
                web_pass: SecretInput::Keep,
                web_api_key: SecretInput::Keep,
            },
        )
        .await
        .unwrap();
    // `ptz` est une information lue par la synchronisation ONVIF : on la fixe comme elle le ferait.
    state.cameras.onvif_sync(camera.id).await.unwrap();
    Rig {
        app,
        state,
        addr,
        onvif,
        uuid: camera.uuid.to_string(),
    }
}

impl Rig {
    async fn user(&self, name: &str, role: Role) -> String {
        self.state
            .auth
            .create_user(name, "", "", role, PASSWORD)
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

    async fn connect(
        &self,
        uuid: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
    ) -> Result<Client, tokio_tungstenite::tungstenite::Error> {
        let mut req = format!("ws://{}/ws/surveillance/ptz/{uuid}/", self.addr)
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

    async fn admin(&self) -> Client {
        let cookie = self.user("alice", Role::Admin).await;
        let mut ws = self
            .connect(
                &self.uuid,
                Some(&cookie),
                Some(&format!("http://{}", self.addr)),
            )
            .await
            .unwrap();
        assert_eq!(next(&mut ws).await, json!({"type": "ready"}));
        ws
    }
}

async fn next(ws: &mut Client) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("delai depasse")
        {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(Message::Close(_)) | Err(_)) | None => panic!("connexion fermee"),
            Some(Ok(_)) => {}
        }
    }
}

async fn tell(ws: &mut Client, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// Attend que l'etat PTZ de la camera simulee satisfasse `ok` (les ordres arrivent de facon asynchrone).
async fn until(
    onvif: &FakeOnvif,
    ok: impl Fn(&dominet_surveillance::testing::onvif::PtzState) -> bool,
) -> bool {
    for _ in 0..100 {
        if ok(&onvif.ptz_state()) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_with_the_right_origin_reach_an_active_ptz_camera(pool: PgPool) {
    let r = rig(pool, "8").await;
    let user = r.user("bob", Role::User).await;
    let origin = format!("http://{}", r.addr);
    // Sans session, compte ordinaire, mauvaise origine, camera inconnue : refus avant l'ouverture du WebSocket.
    assert!(r.connect(&r.uuid, None, Some(&origin)).await.is_err());
    assert!(
        r.connect(&r.uuid, Some(&user), Some(&origin))
            .await
            .is_err()
    );
    let admin = r.user("alice", Role::Admin).await;
    assert!(
        r.connect(&r.uuid, Some(&admin), Some("http://evil.example"))
            .await
            .is_err()
    );
    assert!(
        r.connect(&"0".repeat(32), Some(&admin), Some(&origin))
            .await
            .is_err()
    );
    assert!(
        r.connect("pas-un-uuid", Some(&admin), Some(&origin))
            .await
            .is_err()
    );
    assert!(
        r.onvif.ptz_state().log.is_empty(),
        "aucun ordre n'est parti"
    );
    // Le bon compte entre.
    let mut ws = r
        .connect(&r.uuid, Some(&admin), Some(&origin))
        .await
        .unwrap();
    assert_eq!(next(&mut ws).await, json!({"type": "ready"}));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn move_zoom_and_stop_follow_the_reference_protocol_and_scale_by_speed(pool: PgPool) {
    let r = rig(pool, "8").await;
    let mut ws = r.admin().await;
    tell(
        &mut ws,
        json!({"action": "move", "pan": 1, "tilt": -0.5, "speed_pt": 0.5}),
    )
    .await;
    assert!(
        until(&r.onvif, |s| s.velocity == Some((0.5, -0.25, 0.0))).await,
        "{:?}",
        r.onvif.ptz_state()
    );
    // Vitesse hors limites ramenee dans 0.05..1 ; texte numerique accepte comme `float()` de la reference.
    tell(
        &mut ws,
        json!({"action": "move", "pan": "1", "tilt": 0, "speed_pt": 9}),
    )
    .await;
    assert!(until(&r.onvif, |s| s.velocity == Some((1.0, 0.0, 0.0))).await);
    tell(&mut ws, json!({"action": "zoom", "z": -1, "speed_z": 0.5})).await;
    assert!(until(&r.onvif, |s| s.velocity == Some((0.0, 0.0, -0.5))).await);
    tell(&mut ws, json!({"action": "stop"})).await;
    assert!(until(&r.onvif, |s| s.velocity.is_none()).await);
    // Un message illisible ou une action inconnue ne fait rien et ne ferme pas la connexion.
    ws.send(Message::Text("pas du json".into())).await.unwrap();
    tell(&mut ws, json!({"action": "danse"})).await;
    tell(&mut ws, json!({"action": "get_presets"})).await;
    assert_eq!(
        next(&mut ws).await,
        json!({"type": "presets", "presets": []})
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn closing_the_socket_and_a_forgotten_move_both_stop_the_camera(pool: PgPool) {
    // Arret automatique apres 1 s sans rafraichissement.
    let r = rig(pool, "1").await;
    let mut ws = r.admin().await;
    tell(&mut ws, json!({"action": "move", "pan": 1, "tilt": 0})).await;
    assert!(until(&r.onvif, |s| s.velocity.is_some()).await);
    assert!(
        until(&r.onvif, |s| s.velocity.is_none()).await,
        "arret automatique"
    );
    // Fermeture : un mouvement en cours est arrete.
    tell(&mut ws, json!({"action": "move", "pan": 1, "tilt": 0})).await;
    assert!(until(&r.onvif, |s| s.velocity.is_some()).await);
    ws.close(None).await.unwrap();
    assert!(
        until(&r.onvif, |s| s.velocity.is_none()).await,
        "arret a la fermeture"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn presets_live_in_the_database_and_in_the_camera(pool: PgPool) {
    let r = rig(pool, "8").await;
    let mut ws = r.admin().await;
    tell(&mut ws, json!({"action": "set_preset", "name": "Porte"})).await;
    let reply = next(&mut ws).await;
    assert_eq!(reply["type"], "presets");
    let list = reply["presets"].as_array().unwrap();
    assert_eq!(
        (list[0]["name"].as_str(), list[0]["token"].as_str()),
        (Some("Porte"), Some("preset_1"))
    );
    let id = list[0]["id"].as_i64().unwrap();
    assert_eq!(
        r.onvif.ptz_state().presets.len(),
        1,
        "le preset est aussi dans la camera"
    );

    // Le meme nom remplace (upsert) : un seul preset en base.
    tell(&mut ws, json!({"action": "set_preset", "name": "Porte"})).await;
    assert_eq!(next(&mut ws).await["presets"].as_array().unwrap().len(), 1);

    assert_eq!(
        r.onvif.ptz_state().presets.len(),
        1,
        "l'ancien jeton n'est pas reste orphelin dans la camera"
    );

    // Nom vide ou trop long : refuse, rien n'est enregistre.
    tell(&mut ws, json!({"action": "set_preset", "name": "  "})).await;
    assert_eq!(next(&mut ws).await["type"], "error");
    tell(
        &mut ws,
        json!({"action": "set_preset", "name": "x".repeat(200)}),
    )
    .await;
    assert_eq!(next(&mut ws).await["type"], "error");

    // Aller au preset (la camera repond MOVING puis IDLE : la commande attend la fin).
    tell(&mut ws, json!({"action": "goto_preset", "id": id})).await;
    assert!(
        until(&r.onvif, |s| s
            .log
            .iter()
            .any(|l| l.starts_with("GotoPreset ")))
        .await
    );

    // Position d'origine : `GotoHomePosition`.
    tell(&mut ws, json!({"action": "home"})).await;
    assert!(until(&r.onvif, |s| s.log.iter().any(|l| l == "GotoHomePosition")).await);

    // Un preset nomme « Home » dans la camera fait office d'origine.
    tell(&mut ws, json!({"action": "set_preset", "name": "Home"})).await;
    assert_eq!(next(&mut ws).await["type"], "presets");
    let before = r
        .onvif
        .ptz_state()
        .log
        .iter()
        .filter(|l| l.starts_with("GotoPreset "))
        .count();
    tell(&mut ws, json!({"action": "home"})).await;
    assert!(
        until(&r.onvif, |s| s
            .log
            .iter()
            .filter(|l| l.starts_with("GotoPreset "))
            .count()
            > before)
        .await
    );

    // Definir l'origine ecrase le preset « Home » de la camera au lieu d'en creer un second.
    let known = r.onvif.ptz_state().presets.len();
    tell(&mut ws, json!({"action": "set_home"})).await;
    assert_eq!(next(&mut ws).await["type"], "home_set");
    assert_eq!(r.onvif.ptz_state().presets.len(), known);

    // Un preset inconnu ou d'une autre camera : erreur, pas de mouvement.
    tell(&mut ws, json!({"action": "goto_preset", "id": 999_999})).await;
    assert_eq!(
        next(&mut ws).await,
        json!({"type": "error", "message": "preset not found"})
    );

    tell(&mut ws, json!({"action": "remove_preset", "id": id})).await;
    // Reste « Home », cree plus haut.
    assert_eq!(next(&mut ws).await["presets"].as_array().unwrap().len(), 1);
    assert_eq!(
        r.onvif.ptz_state().presets.len(),
        1,
        "retire aussi de la camera"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_preset_without_a_camera_token_is_reached_by_an_absolute_move_in_the_cameras_space(
    pool: PgPool,
) {
    use dominet_domain::camera::PtzPresetDraft;
    use dominet_domain::repo::{CameraRepo, PtzPresetRepo};
    let r = rig(pool, "8").await;
    let camera = r.state.store.cameras().list().await.unwrap().remove(0);
    let preset = r
        .state
        .store
        .ptz_presets()
        .create(
            camera.id,
            &PtzPresetDraft {
                name: "Jardin".into(),
                token: String::new(),
                pan: 46.4,
                tilt: -3.0,
                zoom: 0.0,
                position: 0,
            },
        )
        .await
        .unwrap();
    let mut ws = r.admin().await;
    tell(&mut ws, json!({"action": "goto_preset", "id": preset.id.0})).await;
    assert!(
        until(&r.onvif, |s| s
            .log
            .iter()
            .any(|l| l.starts_with("AbsoluteMove 46.4 -3 0")))
        .await,
        "{:?}",
        r.onvif.ptz_state().log
    );
    let log = r.onvif.ptz_state().log.join("\n");
    assert!(
        log.contains("SphericalPositionSpace"),
        "l'espace annonce par la camera est repris : {log}"
    );
    assert!(!log.contains("GotoPreset"));
}
