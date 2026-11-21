// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service des cameras (R6 lot 2) : secrets chiffres, modification sans renvoyer les secrets, synchronisation MediaMTX contre
//! le faux serveur.

mod common;

use common::state;
use dominet_domain::Problem;
use dominet_domain::camera::{CameraDraft, RecordingMode};
use dominet_server::cameras::{CameraError, CameraInput, SecretInput, SyncError, SyncStatus};
use dominet_server::secrets::MasterKey;
use dominet_surveillance::testing::FakeMediaMtx;
use serde_json::json;
use sqlx::PgPool;

fn input(name: &str, pass: &str) -> CameraInput {
    let mut draft = CameraDraft::new(name);
    draft.ip = "192.168.1.29".into();
    draft.rtsp_user = "admin".into();
    draft.path = "stream1".into();
    CameraInput {
        draft,
        rtsp_pass: SecretInput::from_form(pass),
        onvif_pass: SecretInput::Keep,
        web_pass: SecretInput::Keep,
        web_api_key: SecretInput::Keep,
    }
}

async fn rig(pool: PgPool) -> (dominet_server::AppState, FakeMediaMtx) {
    let fake = FakeMediaMtx::start().await;
    let st = state(
        pool,
        &[
            ("DOMINET_MASTER_KEY", &MasterKey::generate()),
            ("MEDIAMTX_API_URL", &fake.url()),
        ],
    );
    (st, fake)
}

async fn stored(pool: &PgPool, column: &str) -> String {
    // Colonnes fixes (pas de saisie utilisateur) : requetes litterales.
    let sql = match column {
        "rtsp" => "SELECT rtsp_pass_enc FROM camera",
        "onvif" => "SELECT onvif_pass_enc FROM camera",
        other => panic!("{other}"),
    };
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_camera_is_saved_with_encrypted_secrets_and_a_mediamtx_path(pool: PgPool) {
    let (st, fake) = rig(pool.clone()).await;
    let (cam, sync) = st
        .cameras
        .save(None, input("Entree", "mot-de-passe"))
        .await
        .unwrap();
    assert_eq!(sync, SyncStatus::Synced);

    let raw = stored(&pool, "rtsp").await;
    assert!(
        raw.starts_with("v1:") && !raw.contains("mot-de-passe"),
        "{raw}"
    );
    assert_eq!(
        st.cameras
            .open(cam.uuid, "rtsp_pass", &cam.config.rtsp_pass)
            .unwrap(),
        "mot-de-passe"
    );
    // Le secret est lie a la camera et au champ : recopie ailleurs, il ne se dechiffre plus.
    assert!(
        st.cameras
            .open(cam.uuid, "onvif_pass", &cam.config.rtsp_pass)
            .is_err()
    );
    let other = dominet_domain::camera::CameraUuid(uuid::Uuid::from_u128(1));
    assert!(
        st.cameras
            .open(other, "rtsp_pass", &cam.config.rtsp_pass)
            .is_err()
    );

    let path = fake.path(&cam.uuid.to_string()).expect("path cree");
    assert_eq!(
        path["source"],
        "rtsp://admin:mot-de-passe@192.168.1.29:554/stream1"
    );
    assert_eq!(path["sourceOnDemand"], false);
    assert!(path.get("record").is_none());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_update_without_a_secret_keeps_it_and_follows_the_options(pool: PgPool) {
    let (st, fake) = rig(pool).await;
    let (cam, _) = st
        .cameras
        .save(None, input("Entree", "mot-de-passe"))
        .await
        .unwrap();

    let mut change = input("Porte", "");
    change.draft.recording_mode = RecordingMode::Continuous;
    change.draft.rtsp_transport_tcp = true;
    let (cam2, sync) = st.cameras.save(Some(cam.id), change).await.unwrap();
    assert_eq!(sync, SyncStatus::Synced);
    assert_eq!(cam2.uuid, cam.uuid);
    assert_eq!(
        cam2.config.rtsp_pass, cam.config.rtsp_pass,
        "secret inchange"
    );
    let path = fake.path(&cam.uuid.to_string()).unwrap();
    assert_eq!(
        path["source"],
        "rtsp://admin:mot-de-passe@192.168.1.29:554/stream1"
    );
    assert_eq!(
        (path["record"].clone(), path["sourceProtocol"].clone()),
        (json!(true), json!("tcp"))
    );

    // Un nouveau secret remplace l'ancien.
    let (cam3, _) = st
        .cameras
        .save(Some(cam.id), input("Porte", "autre"))
        .await
        .unwrap();
    assert_ne!(cam3.config.rtsp_pass, cam.config.rtsp_pass);
    assert!(
        fake.path(&cam.uuid.to_string()).unwrap()["source"]
            .as_str()
            .unwrap()
            .contains(":autre@")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_onvif_camera_without_its_own_credentials_reuses_the_stream_ones(pool: PgPool) {
    let (st, _fake) = rig(pool).await;
    let mut i = input("Q3R", "partage");
    i.draft.onvif = true;
    let (cam, _) = st.cameras.save(None, i).await.unwrap();
    assert_eq!(cam.config.onvif_user, "admin");
    assert_eq!(
        st.cameras
            .open(cam.uuid, "onvif_pass", &cam.config.onvif_pass)
            .unwrap(),
        "partage"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn what_onvif_learned_survives_a_form_save(pool: PgPool) {
    use dominet_domain::camera::OnvifInfo;
    use dominet_domain::repo::CameraRepo;
    let (st, _fake) = rig(pool).await;
    let (cam, _) = st.cameras.save(None, input("Q3R", "x")).await.unwrap();
    st.store
        .cameras()
        .apply_onvif(
            cam.id,
            &OnvifInfo {
                manufacturer: "Longse".into(),
                model: "Q3R".into(),
                firmware: "1.0".into(),
                main_token: "profile_1".into(),
                ptz: true,
            },
        )
        .await
        .unwrap();
    let (again, _) = st
        .cameras
        .save(Some(cam.id), input("Q3R bis", ""))
        .await
        .unwrap();
    assert_eq!(
        (
            again.config.model.as_str(),
            again.config.main_token.as_str()
        ),
        ("Q3R", "profile_1")
    );
    // Le PTZ, lui, est dans le formulaire : l'administrateur decide.
    assert!(!again.config.ptz);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn onvif_proposes_ptz_once_then_the_administrator_decides(pool: PgPool) {
    let fake = FakeOnvif::start(Family::Thingino, "admin", "mot-de-passe").await;
    let (st, _mtx) = rig(pool).await;
    let (cam, _) = st
        .cameras
        .save(None, onvif_input("Q3R", fake.port(), "mot-de-passe"))
        .await
        .unwrap();
    st.cameras.onvif_sync(cam.id).await.unwrap();
    assert!(st.cameras.get(cam.id).await.unwrap().unwrap().config.ptz);
    // L'administrateur decoche (camera sans moteur malgre le service PTZ annonce) : une nouvelle synchronisation ne le recoche pas.
    st.cameras
        .save(Some(cam.id), onvif_input("Q3R", fake.port(), ""))
        .await
        .unwrap();
    st.cameras.onvif_sync(cam.id).await.unwrap();
    assert!(!st.cameras.get(cam.id).await.unwrap().unwrap().config.ptz);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_input_and_a_missing_master_key_are_refused(pool: PgPool) {
    let (st, fake) = rig(pool.clone()).await;
    let mut bad = input("", "x");
    bad.draft.port = 0;
    let Err(CameraError::Invalid(problems)) = st.cameras.save(None, bad).await else {
        panic!("refus attendu")
    };
    assert_eq!(problems[0], ("name", Problem::Required));
    assert!(
        fake.names().iter().all(|n| n == "~.*"),
        "rien n'est envoye a MediaMTX"
    );

    // Sans cle maitresse, un secret ne s'enregistre pas (jamais en clair).
    let bare = state(pool, &[]);
    assert!(matches!(
        bare.cameras.save(None, input("Sans cle", "secret")).await,
        Err(CameraError::MasterKeyMissing)
    ));
    // ... mais une camera sans mot de passe passe.
    assert!(
        bare.cameras
            .save(None, input("Sans mot de passe", ""))
            .await
            .is_ok()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deleting_or_deactivating_a_camera_removes_its_path(pool: PgPool) {
    let (st, fake) = rig(pool).await;
    let (a, _) = st.cameras.save(None, input("A", "x")).await.unwrap();
    let (b, _) = st.cameras.save(None, input("B", "x")).await.unwrap();
    assert!(fake.path(&a.uuid.to_string()).is_some() && fake.path(&b.uuid.to_string()).is_some());

    let mut off = input("A", "");
    off.draft.active = false;
    let (_, sync) = st.cameras.save(Some(a.id), off).await.unwrap();
    assert_eq!(sync, SyncStatus::Synced);
    assert!(
        fake.path(&a.uuid.to_string()).is_none(),
        "camera inactive : pas de path"
    );

    assert_eq!(st.cameras.delete(b.id).await.unwrap(), SyncStatus::Synced);
    assert!(fake.path(&b.uuid.to_string()).is_none());
    assert!(matches!(
        st.cameras.delete(b.id).await,
        Err(CameraError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_mediamtx_outage_does_not_undo_the_save_and_sync_all_catches_up(pool: PgPool) {
    let (st, fake) = rig(pool).await;
    fake.fail_with(Some(503));
    let (cam, sync) = st.cameras.save(None, input("A", "x")).await.unwrap();
    assert!(matches!(sync, SyncStatus::Failed(_)), "{sync:?}");
    assert!(
        st.cameras.get(cam.id).await.unwrap().is_some(),
        "la base fait foi"
    );
    assert!(st.cameras.sync_all().await.is_err());

    fake.fail_with(None);
    fake.insert(
        "ffffffffffffffffffffffffffffffff",
        json!({"source": "rtsp://vieux"}),
    );
    let report = st.cameras.sync_all().await.unwrap();
    assert_eq!(report.added, [cam.uuid.to_string()]);
    assert_eq!(report.removed, ["ffffffffffffffffffffffffffffffff"]);
    assert!(fake.path("~.*").is_some());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_a_mediamtx_url_nothing_is_synchronised(pool: PgPool) {
    let st = state(pool, &[("DOMINET_MASTER_KEY", &MasterKey::generate())]);
    let (_, sync) = st.cameras.save(None, input("A", "x")).await.unwrap();
    assert_eq!(sync, SyncStatus::Disabled);
    assert!(matches!(
        st.cameras.sync_all().await,
        Err(SyncError::Disabled)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_unreadable_secret_is_reported_and_its_path_is_kept(pool: PgPool) {
    let (st, fake) = rig(pool.clone()).await;
    let (cam, _) = st.cameras.save(None, input("A", "x")).await.unwrap();
    // Autre cle maitresse : le secret n'est plus lisible.
    let other = state(
        pool,
        &[
            ("DOMINET_MASTER_KEY", &MasterKey::generate()),
            ("MEDIAMTX_API_URL", &fake.url()),
        ],
    );
    let report = other.cameras.sync_all().await.unwrap();
    assert_eq!(report.failed.len(), 1);
    assert!(
        fake.path(&cam.uuid.to_string()).is_some(),
        "le path existant n'est pas supprime"
    );
    assert!(
        report.failed[0].1.contains("illisible"),
        "{:?}",
        report.failed
    );
}

// ------------------------------------------------------------------ ONVIF (R6 lot 3)

use dominet_surveillance::testing::onvif::{FakeOnvif, Family};

fn onvif_input(name: &str, port: u16, pass: &str) -> CameraInput {
    let mut i = input(name, pass);
    i.draft.ip = "127.0.0.1".into();
    i.draft.onvif = true;
    i.draft.onvif_port = i32::from(port);
    i.draft.rtsp_user = "admin".into();
    i
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn onvif_sync_reads_the_camera_and_records_what_it_learned(pool: PgPool) {
    let fake = FakeOnvif::start(Family::Thingino, "admin", "mot-de-passe").await;
    let (st, _mtx) = rig(pool).await;
    let (cam, _) = st
        .cameras
        .save(None, onvif_input("Q3R", fake.port(), "mot-de-passe"))
        .await
        .unwrap();
    let r = st.cameras.onvif_sync(cam.id).await.unwrap();
    assert_eq!(
        (r.info.model.as_str(), r.main_token.as_str(), r.ptz),
        ("Q3R", "profile_1", true)
    );
    let now = st.cameras.get(cam.id).await.unwrap().unwrap();
    assert_eq!(
        (
            now.config.manufacturer.as_str(),
            now.config.model.as_str(),
            now.config.main_token.as_str(),
            now.config.ptz
        ),
        ("Thingino", "Q3R", "profile_1", true)
    );
    assert!(
        r.stream_uri.starts_with("rtsp://127.0.0.1:554/"),
        "{}",
        r.stream_uri
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn onvif_sync_uses_the_stream_credentials_when_onvif_has_none_of_its_own(pool: PgPool) {
    // Les identifiants ONVIF vides sont remplis a l'enregistrement par ceux du flux (comme la reference) : la camera repond.
    let fake = FakeOnvif::start(Family::Vatilon, "admin", "partage").await;
    let (st, _mtx) = rig(pool).await;
    let (cam, _) = st
        .cameras
        .save(None, onvif_input("Ascam", fake.port(), "partage"))
        .await
        .unwrap();
    assert!(st.cameras.onvif_sync(cam.id).await.is_ok());
    // Un mauvais mot de passe est reporte, sans toucher a la fiche.
    let (bad, _) = st
        .cameras
        .save(Some(cam.id), onvif_input("Ascam", fake.port(), "autre"))
        .await
        .unwrap();
    // Le mot de passe ONVIF deja copie reste l'ancien (donc bon) : on force un echec en changeant les deux.
    let mut wrong = onvif_input("Ascam", fake.port(), "");
    wrong.onvif_pass = SecretInput::Set("faux".into());
    st.cameras.save(Some(bad.id), wrong).await.unwrap();
    assert!(matches!(
        st.cameras.onvif_sync(cam.id).await,
        Err(CameraError::Onvif(
            dominet_surveillance::onvif::OnvifError::NotAuthorized
        ))
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn onvif_sync_needs_onvif_enabled(pool: PgPool) {
    let (st, _mtx) = rig(pool).await;
    let (cam, _) = st
        .cameras
        .save(None, input("Sans ONVIF", "x"))
        .await
        .unwrap();
    assert!(matches!(
        st.cameras.onvif_sync(cam.id).await,
        Err(CameraError::OnvifDisabled)
    ));
    assert!(matches!(
        st.cameras
            .onvif_sync(dominet_domain::camera::CameraId(0))
            .await,
        Err(CameraError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn add_by_ip_creates_an_onvif_camera_once_and_syncs_it_when_it_has_credentials(pool: PgPool) {
    let fake = FakeOnvif::start(Family::Nvt, "admin", "secret-nvt").await;
    let (st, mtx) = rig(pool).await;
    let (cam, created) = st
        .cameras
        .add_by_ip("127.0.0.1", Some(fake.port()), "admin", "secret-nvt", None)
        .await
        .unwrap();
    assert!(created);
    assert!(cam.config.name.starts_with("Cam") && cam.config.name.ends_with("127.0.0.1"));
    assert!(cam.config.onvif);
    assert_eq!(cam.config.onvif_port, i32::from(fake.port()));
    assert_eq!(
        cam.config.model, "IPC_NT98566",
        "synchronisee tout de suite"
    );
    assert!(
        mtx.path(&cam.uuid.to_string()).is_some(),
        "path MediaMTX cree"
    );

    // Deuxieme ajout de la meme adresse : la camera existante, rien de nouveau.
    let (again, created) = st
        .cameras
        .add_by_ip("127.0.0.1", Some(fake.port()), "", "", None)
        .await
        .unwrap();
    assert!(!created);
    assert_eq!(again.id, cam.id);
    assert_eq!(st.cameras.list().await.unwrap().len(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn add_by_ip_without_credentials_still_adds_and_refuses_bad_addresses(pool: PgPool) {
    let fake = FakeOnvif::start(Family::Thingino, "admin", "x").await;
    let (st, _mtx) = rig(pool).await;
    let (cam, created) = st
        .cameras
        .add_by_ip("127.0.0.1", Some(fake.port()), "", "", Some("Garage"))
        .await
        .unwrap();
    assert!(created && cam.config.name == "Garage" && cam.config.model.is_empty());
    for bad in ["", "cam; rm -rf /", "a b"] {
        assert!(
            matches!(
                st.cameras.add_by_ip(bad, Some(80), "", "", None).await,
                Err(CameraError::Invalid(_))
            ),
            "{bad:?}"
        );
    }
}

// ------------------------------------------------------------------ decouverte (role discovery <-> role web)

use std::time::Duration;

use dominet_proto::{DiscoveryEvent, Topic, WsServerMsg};
use dominet_server::discovery::{self, Probe};
use dominet_surveillance::testing::discovery::FakeResponders;

async fn next_event(
    rx: &mut tokio::sync::broadcast::Receiver<std::sync::Arc<WsServerMsg>>,
) -> DiscoveryEvent {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(8), rx.recv())
            .await
            .expect("evenement attendu")
            .unwrap();
        if let WsServerMsg::Discovery { event } = &*msg {
            return event.clone();
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_request_goes_through_the_discovery_role_and_reaches_the_browsers_topic(pool: PgPool) {
    let responders = FakeResponders::start(vec![(
        "NVT".into(),
        "IPC".into(),
        "http://192.168.1.29:8899/onvif/device_service".into(),
    )])
    .await;
    let (st, _mtx) = rig(pool.clone()).await;
    // Une camera deja enregistree a une autre adresse n'est pas « connue » ; celle de la reponse l'est.
    st.cameras
        .save(None, {
            let mut i = input("Deja la", "");
            i.draft.ip = "127.0.0.1".into();
            i
        })
        .await
        .unwrap();
    let mut topic = st.hub.subscribe(&Topic::Discovery);
    discovery::spawn_relay(&st);
    discovery::spawn_role_with(
        &st,
        Probe {
            target: responders.addr(),
            interface: None,
            probes: 2,
            listen: Duration::from_millis(1500),
        },
    );
    // Les ecoutes PostgreSQL s'etablissent en tache de fond : on redemande jusqu'a la premiere reponse.
    let mut first = None;
    for _ in 0..40 {
        discovery::request(&pool).await.unwrap();
        if let Ok(ev) =
            tokio::time::timeout(Duration::from_millis(300), next_event(&mut topic)).await
        {
            first = Some(ev);
            break;
        }
    }
    assert_eq!(first, Some(DiscoveryEvent::Scanning));
    let DiscoveryEvent::Found { device } = next_event(&mut topic).await else {
        panic!("appareil attendu")
    };
    // Le repondeur est en 127.0.0.1 : c'est l'adresse d'ou vient la reponse, et la camera de ce test l'a deja.
    assert_eq!(
        (device.ip.as_str(), device.port, device.name.as_str()),
        ("127.0.0.1", 8899, "NVT")
    );
    assert!(device.known, "une camera a deja cette adresse");
    assert_eq!(next_event(&mut topic).await, DiscoveryEvent::Done);
}

// ------------------------------------------------------------------ jour / nuit (R6 lot 5)

use dominet_domain::camera::NightApi;
use dominet_server::cameras::{NightAction, NightError};
use dominet_surveillance::testing::night::{FakeGoform, FakeThingino};

fn night_input(api: NightApi, port: u16) -> CameraInput {
    let mut i = input("Nuit", "flux-secret");
    i.draft.ip = "127.0.0.1".into();
    i.draft.night_mode_api = api;
    i.draft.web_port = i32::from(port);
    i.draft.onvif_port = i32::from(port);
    i
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn thingino_night_uses_the_decrypted_api_key_and_remembers_its_state(pool: PgPool) {
    let fake = FakeThingino::start("cle-api").await;
    let (st, _mtx) = rig(pool).await;
    let mut i = night_input(NightApi::Thingino, fake.port());
    i.web_api_key = SecretInput::Set("cle-api".into());
    let (cam, _) = st.cameras.save(None, i).await.unwrap();
    let first = st
        .cameras
        .night(cam.id, "daynight")
        .await
        .unwrap()
        .to_json();
    assert_eq!(first["value"], "night");
    let second = st
        .cameras
        .night(cam.id, "daynight")
        .await
        .unwrap()
        .to_json();
    assert_eq!(second["value"], "day", "la bascule se souvient de l'etat");
    assert_eq!(fake.bodies().len(), 2);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn goform_night_falls_back_to_the_stream_credentials(pool: PgPool) {
    let fake = FakeGoform::start("admin", "flux-secret").await;
    let (st, _mtx) = rig(pool).await;
    let (cam, _) = st
        .cameras
        .save(None, night_input(NightApi::Goform, fake.port()))
        .await
        .unwrap();
    // Pas d'identifiant web : ceux du flux (`admin` / `flux-secret`).
    let out = st.cameras.night(cam.id, "ircut").await.unwrap().to_json();
    assert_eq!(
        (out["mode"].as_i64(), out["was"].as_i64()),
        (Some(2), Some(1))
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn night_refuses_what_it_cannot_do_and_says_why(pool: PgPool) {
    let (st, _mtx) = rig(pool).await;
    let (none, _) = st
        .cameras
        .save(None, night_input(NightApi::None, 80))
        .await
        .unwrap();
    assert!(matches!(
        st.cameras.night(none.id, "daynight").await,
        Err(CameraError::Night(NightError::NotConfigured))
    ));
    assert!(matches!(
        st.cameras.night(none.id, "danse").await,
        Err(CameraError::Night(NightError::UnknownAction(_)))
    ));
    // Camera inactive ou inconnue : introuvable.
    let mut off = night_input(NightApi::Onvif, 80);
    off.draft.active = false;
    let (off, _) = st.cameras.save(None, off).await.unwrap();
    assert!(matches!(
        st.cameras.night(off.id, "light").await,
        Err(CameraError::NotFound)
    ));
    assert!(matches!(
        st.cameras
            .night(dominet_domain::camera::CameraId(0), "light")
            .await,
        Err(CameraError::NotFound)
    ));
    let _ = NightAction::Light;
}
