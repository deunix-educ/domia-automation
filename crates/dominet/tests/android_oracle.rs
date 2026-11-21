// SPDX-License-Identifier: AGPL-3.0-or-later
//! Le relais Rust rejoue les sessions de l'application executees par le **vrai** consommateur reference
//! (`tests/fixtures/android/oracle.json`) et doit laisser la meme base, repondre les memes trames a
//! l'application et diffuser les memes evenements aux navigateurs.
//!
//! Ecarts voulus, absents de ces sessions : appel ouvert le plus **recent** ferme (Reference : le plus ancien), numero vide ignore,
//! JSON non objet ignore (le consommateur reference plante), accuses relies a la file d'envoi (identifiants propres), jeton hache.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dominet_domain::android::AndroidDeviceDraft;
use dominet_domain::repo::{AndroidDeviceRepo, CallRepo, SmsRepo};
use dominet_domain::user::Role;
use dominet_proto::{AndroidEvent, Topic, WsServerMsg};
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;

/// Entier d'un JSON de la reference : une duree recue en `75.0` est rediffusee telle quelle (flottant), l'enregistrement la tronque.
#[allow(clippy::cast_possible_truncation)]
fn whole(v: &Value) -> i64 {
    v.as_i64().unwrap_or_else(|| v.as_f64().unwrap() as i64)
}

fn micros(d: DateTime<Utc>) -> i64 {
    d.timestamp() * 1_000_000 + i64::from(d.timestamp_subsec_micros())
}

fn rfc3339_micros(text: &str) -> i64 {
    micros(DateTime::parse_from_rfc3339(text).unwrap().to_utc())
}

/// Instant attendu : `"NOW"` quand la trame n'avait pas de date lisible (reception, a deux minutes pres), sinon identique.
fn same_instant(got: i64, want: &Value, what: &str) {
    if want == "NOW" {
        assert!(
            (micros(Utc::now()) - got).abs() < 120_000_000,
            "{what}: attendu maintenant, obtenu {got}"
        );
    } else {
        assert_eq!(got, want.as_i64().unwrap(), "{what}");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_rust_relay_agrees_with_the_reference_consumer(pool: PgPool) {
    let oracle: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/android/oracle.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let config = Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://u:p@127.0.0.1:1/none".to_string())
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
    let serve = app.into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    let owner = state
        .auth
        .create_user("alice", "", "", Role::Admin, "correct horse battery")
        .await
        .unwrap()
        .id;

    // Jeton inconnu : meme code de fermeture (4001).
    assert_eq!(
        oracle["unknown_token"],
        json!({"connected": false, "close_code": 4001})
    );
    let req = format!("ws://{addr}/ws/android/device/0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b/")
        .into_client_request()
        .unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let Some(Ok(Message::Close(Some(frame)))) = ws.next().await else {
        panic!("fermeture attendue")
    };
    assert_eq!(u16::from(frame.code), 4001);

    let scenarios = oracle["scenarios"].as_array().unwrap();
    assert!(scenarios.len() >= 20);
    for (n, sc) in scenarios.iter().enumerate() {
        let name = sc["name"].as_str().unwrap();
        let (device, token) = state
            .store
            .android_devices()
            .create(
                owner,
                &AndroidDeviceDraft {
                    name: format!("Pixel {n}"),
                    phone_number: "+33612345678".into(),
                },
            )
            .await
            .unwrap();
        let mut events = state.hub.subscribe(&Topic::Android(device.id.0));
        let req = format!("ws://{addr}/ws/android/device/{token}/")
            .into_client_request()
            .unwrap();
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
        let hello = next_json(&mut ws, Duration::from_secs(5)).await.unwrap();
        assert_eq!(
            hello,
            json!({"type": "auth_ok", "device_name": format!("Pixel {n}")}),
            "{name}"
        );
        assert!(
            state
                .store
                .android_devices()
                .get(device.id)
                .await
                .unwrap()
                .unwrap()
                .is_online,
            "{name}"
        );

        for f in sc["frames"].as_array().unwrap() {
            ws.send(Message::Text(f.as_str().unwrap().into()))
                .await
                .unwrap();
        }
        // Rien de plus a attendre que ce que reference a repondu a l'application.
        let mut app_frames = vec![];
        while let Some(v) = next_json(&mut ws, Duration::from_millis(400)).await {
            app_frames.push(v);
        }
        assert_eq!(
            Value::Array(app_frames),
            sc["app_frames"],
            "{name} : trames pour l'application"
        );
        ws.close(None).await.unwrap();
        // Up to 12 s: the server notices the closed socket on its own schedule, slower when the machine is busy.
        for _ in 0..400 {
            if !state
                .store
                .android_devices()
                .get(device.id)
                .await
                .unwrap()
                .unwrap()
                .is_online
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert!(
            !state
                .store
                .android_devices()
                .get(device.id)
                .await
                .unwrap()
                .unwrap()
                .is_online,
            "{name}"
        );

        // Base.
        let want = &sc["state"];
        let sms = state
            .store
            .sms()
            .list(Some(owner), Some(device.id), 100)
            .await
            .unwrap();
        let mut sms: Vec<_> = sms.into_iter().rev().collect();
        let want_sms = want["sms"].as_array().unwrap();
        assert_eq!(sms.len(), want_sms.len(), "{name} : nombre de SMS");
        for (got, w) in sms.drain(..).zip(want_sms) {
            assert_eq!(got.direction.as_str(), w["direction"], "{name}");
            assert_eq!(
                (
                    got.phone_number.as_str(),
                    got.contact_name.as_str(),
                    got.body.as_str()
                ),
                (
                    w["phone_number"].as_str().unwrap(),
                    w["contact_name"].as_str().unwrap(),
                    w["body"].as_str().unwrap()
                ),
                "{name}"
            );
            assert_eq!(got.is_read, w["is_read"].as_bool().unwrap(), "{name}");
            same_instant(micros(got.sent_at), &w["sent_at"], name);
        }
        let calls: Vec<_> = state
            .store
            .calls()
            .list(Some(owner), Some(device.id), 100)
            .await
            .unwrap()
            .into_iter()
            .rev()
            .collect();
        let want_calls = want["calls"].as_array().unwrap();
        assert_eq!(calls.len(), want_calls.len(), "{name} : nombre d'appels");
        for (got, w) in calls.iter().zip(want_calls) {
            assert_eq!(got.direction.as_str(), w["direction"], "{name}");
            assert_eq!(
                (got.phone_number.as_str(), got.contact_name.as_str()),
                (
                    w["phone_number"].as_str().unwrap(),
                    w["contact_name"].as_str().unwrap()
                ),
                "{name}"
            );
            assert_eq!(
                i64::from(got.duration),
                w["duration"].as_i64().unwrap(),
                "{name} : duree"
            );
            same_instant(micros(got.started_at), &w["started_at"], name);
            assert_eq!(
                got.ended_at.map(micros).is_some(),
                !w["ended_at"].is_null(),
                "{name} : fin"
            );
            if let (Some(e), false) = (got.ended_at, w["ended_at"].is_null()) {
                same_instant(micros(e), &w["ended_at"], name);
            }
        }

        // Evenements diffuses aux navigateurs (hors presence).
        let mut got_events = vec![];
        while let Ok(ev) = events.try_recv() {
            if let WsServerMsg::Android { event, .. } = &*ev
                && !matches!(event, AndroidEvent::DeviceStatus { .. })
            {
                got_events.push(event.clone());
            }
        }
        let want_events = sc["events"].as_array().unwrap();
        assert_eq!(
            got_events.len(),
            want_events.len(),
            "{name} : nombre d'evenements"
        );
        for (got, w) in got_events.iter().zip(want_events) {
            match got {
                AndroidEvent::SmsReceived { sms } => {
                    assert_eq!(w["kind"], "sms_received", "{name}");
                    assert_eq!(
                        (
                            sms.direction.as_str(),
                            sms.phone_number.as_str(),
                            sms.contact_name.as_str(),
                            sms.body.as_str(),
                            sms.is_read
                        ),
                        (
                            w["direction"].as_str().unwrap(),
                            w["phone_number"].as_str().unwrap(),
                            w["contact_name"].as_str().unwrap(),
                            w["body"].as_str().unwrap(),
                            w["is_read"].as_bool().unwrap()
                        ),
                        "{name}"
                    );
                    let sent = DateTime::parse_from_rfc3339(&sms.sent_at).unwrap().to_utc();
                    same_instant(micros(sent), &w["sent_at"], name);
                }
                AndroidEvent::CallEvent { call, state } => {
                    assert_eq!(w["kind"], "call_event", "{name}");
                    assert_eq!(state, w["state"].as_str().unwrap(), "{name}");
                    assert_eq!(
                        (
                            call.direction.as_str(),
                            call.phone_number.as_str(),
                            call.contact_name.as_str(),
                            i64::from(call.duration)
                        ),
                        (
                            w["direction"].as_str().unwrap(),
                            w["phone_number"].as_str().unwrap(),
                            w["contact_name"].as_str().unwrap(),
                            whole(&w["duration"])
                        ),
                        "{name}"
                    );
                    let started = rfc3339_micros(&call.started_at);
                    same_instant(started, &w["started_at"], name);
                    assert_eq!(call.ended_at.is_some(), !w["ended_at"].is_null(), "{name}");
                }
                other => panic!("{name} : evenement inattendu {other:?}"),
            }
        }
    }
}

async fn next_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    wait: Duration,
) -> Option<Value> {
    loop {
        match tokio::time::timeout(wait, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return Some(serde_json::from_str(&t).unwrap()),
            Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => return None,
            Ok(Some(Ok(_))) => {}
        }
    }
}
