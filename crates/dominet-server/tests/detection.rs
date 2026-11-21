// SPDX-License-Identifier: AGPL-3.0-or-later
//! Mouvement et detection d'objets (R6 lot 8) : de vrais `ffmpeg` sur des sources `lavfi` / une image fixe, le vrai modele YOLO
//! (`make fetch-model`, sinon le test le dit), le Mosquitto de la stack de dev, un faux SMTP. Aucun envoi reel.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::broker::connected;
use common::state;
use dominet_domain::camera::{CameraDraft, DetectionTrigger, RecordingMode};
use dominet_domain::repo::{DetectorConfigRepo, EventRepo};
use dominet_mqtt::{Incoming, MqttSettings, Payload, start};
use dominet_notify::testing::FakeSmtp;
use dominet_server::cameras::{CameraInput, SecretInput};
use dominet_server::detection::MotionOutcome;
use dominet_server::frames::FrameHub;
use dominet_server::notify::Channels;
use dominet_surveillance::frames::{FrameConfig, Hwaccel};
use serde_json::Value;
use sqlx::PgPool;

fn have_ffmpeg() -> bool {
    let ok = std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("NOTE : ffmpeg absent, test non verifie ici");
    }
    ok
}

fn model() -> Option<PathBuf> {
    let path = std::env::var_os("YOLO_MODEL_PATH").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/yolov8n.onnx"),
        PathBuf::from,
    );
    if path.exists() {
        Some(path)
    } else {
        eprintln!("NOTE : modele YOLO absent, `make fetch-model` : detection non verifiee ici");
        None
    }
}

fn bus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dominet-surveillance/fixtures/yolo/bus.jpg")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dominet-det-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn input(name: &str, mode: RecordingMode, yolo: bool, classes: &[&str]) -> CameraInput {
    let mut draft = CameraDraft::new(name);
    draft.ip = "192.168.1.29".into();
    draft.recording_mode = mode;
    draft.yolo_enabled = yolo;
    draft.yolo_classes = classes.iter().map(|c| (*c).to_string()).collect();
    CameraInput {
        draft,
        rtsp_pass: SecretInput::Keep,
        onvif_pass: SecretInput::Keep,
        web_pass: SecretInput::Keep,
        web_api_key: SecretInput::Keep,
    }
}

/// Etat de test : MQTT sur le broker de dev, faux SMTP, sources d'images choisies par nom de camera.
struct Rig {
    st: dominet_server::AppState,
    smtp: FakeSmtp,
    mqtt: tokio::sync::mpsc::Receiver<Incoming>,
    _listener: dominet_mqtt::MqttClient,
    _dir: TempDir,
}

fn source_args(name: &str) -> Vec<String> {
    let args: Vec<&str> = match name {
        // Une luminosite qui oscille (0,8 Hz) : la premiere et la derniere image de la fenetre different presque toujours.
        "bouge" => vec![
            "-re",
            "-f",
            "lavfi",
            "-i",
            "mandelbrot=size=320x180:rate=10",
        ],
        // Une image qui ne bouge pas.
        "fixe" => vec![
            "-re",
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:size=320x180:rate=10",
        ],
        _ => vec![],
    };
    args.into_iter().map(String::from).collect()
}

async fn rig(pool: PgPool, tag: &str, bus_image: bool) -> Rig {
    let dir = TempDir::new();
    let id = format!("dominet-test-det-{tag}-{}", std::process::id());
    let port = common::broker::port();
    let mut st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &id),
            ("DETECTIONS_DIR", dir.path().to_str().unwrap()),
            ("MOTION_ALERT_COOLDOWN", "3600"),
            ("DETECTION_ALERT_COOLDOWN", "3600"),
        ],
    );
    let listener = start(&MqttSettings::new(
        "127.0.0.1",
        port.parse().unwrap(),
        &format!("{id}-listen"),
    ));
    connected(&listener.client).await;
    listener.client.set_subscriptions(BTreeMap::from([(
        "dominet/surveillance/+/+".to_string(),
        1,
    )]));
    tokio::time::sleep(Duration::from_millis(500)).await;
    st.settings
        .set("Z2M_BASE_TOPIC", &format!("det{tag}z2m"))
        .await
        .unwrap();
    st.mqtt = Some(dominet_server::mqtt::spawn(&st).await.unwrap());
    connected(st.mqtt.as_ref().unwrap()).await;

    let smtp = FakeSmtp::start().await;
    let mut config = (*st.config).clone();
    config.email = Some(dominet_server::config::EmailConfig {
        host: "127.0.0.1".into(),
        port: smtp.addr().port(),
        user: None,
        password: None,
        security: dominet_notify::Security::Plain,
        timeout_secs: 5,
    });
    st.channels = Channels::new(&config, st.settings.clone());
    st.settings
        .set("EMAIL_RECIPIENT", "alice@example.com")
        .await
        .unwrap();
    st.settings
        .set("DEFAULT_FROM_EMAIL", "maison@example.com")
        .await
        .unwrap();

    let bus_path = bus();
    st.frames = FrameHub::with_factory(
        move |camera| {
            let mut input = source_args(&camera.config.name);
            if input.is_empty() || bus_image {
                input = vec![
                    "-re".into(),
                    "-loop".into(),
                    "1".into(),
                    "-framerate".into(),
                    "10".into(),
                    "-i".into(),
                    bus_path.display().to_string(),
                ];
            }
            input.push("-nostdin".into());
            let mut cfg = FrameConfig::new(input);
            cfg.fps = 10.0;
            cfg.hwaccel = Hwaccel::Off;
            cfg.backoff_min = Duration::from_millis(100);
            Some(cfg)
        },
        dir.path(),
    );
    Rig {
        st,
        smtp,
        mqtt: listener.messages,
        _listener: listener.client,
        _dir: dir,
    }
}

async fn wait_frames(st: &dominet_server::AppState, camera: dominet_domain::camera::CameraUuid) {
    for _ in 0..150 {
        if st.frames.recent(camera).len() >= 20 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("pas d'images");
}

/// Evalue jusqu'a obtenir un mouvement : la luminosite oscillante de la source de test s'annule parfois entre deux images eloignees.
async fn until_moving(
    st: &dominet_server::AppState,
    camera: &dominet_domain::camera::Camera,
) -> MotionOutcome {
    let mut last = MotionOutcome::NoWindow;
    for _ in 0..30 {
        last = st.detector.check_motion(st, camera).await;
        if matches!(last, MotionOutcome::Moving(_)) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    last
}

async fn message_for(
    rx: &mut tokio::sync::mpsc::Receiver<Incoming>,
    uuid: &str,
    kind: &str,
) -> Value {
    let wanted = format!("dominet/surveillance/{uuid}/{kind}");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let m = rx.recv().await.unwrap();
            if m.topic == wanted
                && let Payload::Json(v) = m.payload
            {
                return v;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("message {wanted} non recu"))
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn motion_is_detected_published_notified_and_throttled_per_camera(pool: PgPool) {
    if !have_ffmpeg() {
        return;
    }
    let mut r = rig(pool, "motion", false).await;
    let (moving, _) =
        r.st.cameras
            .save(None, input("bouge", RecordingMode::Motion, false, &[]))
            .await
            .unwrap();
    let (still, _) =
        r.st.cameras
            .save(None, input("fixe", RecordingMode::Motion, false, &[]))
            .await
            .unwrap();
    let cameras = r.st.cameras.list().await.unwrap();
    r.st.frames.reconcile(&cameras);
    wait_frames(&r.st, moving.uuid).await;
    wait_frames(&r.st, still.uuid).await;

    let calm = r.st.detector.check_motion(&r.st, &still).await;
    assert!(
        matches!(calm, MotionOutcome::Still(x) if x < 0.001),
        "{calm:?}"
    );
    assert_eq!(
        r.st.store
            .camera_events()
            .list_motion(still.id, 10)
            .await
            .unwrap()
            .len(),
        0
    );

    let active = until_moving(&r.st, &moving).await;
    let MotionOutcome::Moving(ratio) = active else {
        panic!("{active:?}")
    };
    assert!(ratio > 0.01);
    let events =
        r.st.store
            .camera_events()
            .list_motion(moving.id, 10)
            .await
            .unwrap();
    assert_eq!(events.len(), 1);
    assert!((events[0].confidence - ratio).abs() < 1e-9);

    // MQTT : meme charge utile que reference.
    let msg = message_for(&mut r.mqtt, &moving.uuid.to_string(), "motion").await;
    assert_eq!(msg["camera_uuid"], moving.uuid.to_string());
    assert_eq!(msg["camera_name"], "bouge");
    assert!(msg["confidence"].as_f64().unwrap() > 0.01);
    assert!(msg["timestamp"].as_str().unwrap().contains('T'));

    // Notification : un e-mail, puis plus rien tant que le delai de la camera court ; l'evenement, lui, est enregistre.
    for _ in 0..50 {
        if !r.smtp.mails().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mails = r.smtp.mails();
    assert_eq!(mails.len(), 1);
    assert!(
        mails[0].data.contains("Motion - bouge"),
        "{}",
        mails[0].data
    );
    until_moving(&r.st, &moving).await;
    assert_eq!(
        r.st.store
            .camera_events()
            .list_motion(moving.id, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(r.smtp.mails().len(), 1, "delai par camera");
    r.st.frames.stop_all();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_enough_recent_frames_there_is_no_verdict(pool: PgPool) {
    let r = rig(pool, "nowin", false).await;
    let (cam, _) =
        r.st.cameras
            .save(None, input("rien", RecordingMode::Motion, false, &[]))
            .await
            .unwrap();
    // Aucune source : pas de fenetre.
    assert_eq!(
        r.st.detector.check_motion(&r.st, &cam).await,
        MotionOutcome::NoWindow
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn yolo_filters_by_camera_saves_the_annotated_frame_and_tells_everyone(pool: PgPool) {
    if !have_ffmpeg() {
        return;
    }
    let Some(model) = model() else { return };
    let mut r = rig(pool, "yolo", true).await;
    let mut cfg = r.st.store.detector_config().get().await.unwrap();
    cfg.model_path = model.display().to_string();
    cfg.enabled = false;
    cfg.save_frames = true;
    r.st.store.detector_config().update(&cfg).await.unwrap();

    let (wants_bus, _) =
        r.st.cameras
            .save(None, input("bus", RecordingMode::Motion, true, &["bus"]))
            .await
            .unwrap();
    let (wants_plane, _) =
        r.st.cameras
            .save(
                None,
                input("avion", RecordingMode::Motion, true, &["airplane"]),
            )
            .await
            .unwrap();
    let (yolo_off, _) =
        r.st.cameras
            .save(None, input("sans-yolo", RecordingMode::Motion, false, &[]))
            .await
            .unwrap();
    let cameras = r.st.cameras.list().await.unwrap();
    r.st.frames.reconcile(&cameras);
    wait_frames(&r.st, wants_bus.uuid).await;

    // Detecteur global coupe : rien, meme avec YOLO sur la camera.
    assert!(
        r.st.detector
            .detect(&r.st, &wants_bus, DetectionTrigger::Motion)
            .await
            .is_none()
    );
    cfg.enabled = true;
    r.st.store.detector_config().update(&cfg).await.unwrap();
    // YOLO coupe sur la camera : rien.
    assert!(
        r.st.detector
            .detect(&r.st, &yolo_off, DetectionTrigger::Motion)
            .await
            .is_none()
    );
    // Classe non voulue : aucun evenement.
    assert!(
        r.st.detector
            .detect(&r.st, &wants_plane, DetectionTrigger::Motion)
            .await
            .is_none()
    );

    let event =
        r.st.detector
            .detect(&r.st, &wants_bus, DetectionTrigger::Motion)
            .await
            .expect("le bus est reconnu");
    assert_eq!(event.class_names(), ["bus"]);
    assert_eq!(event.triggered_by, DetectionTrigger::Motion);
    assert!(event.labels[0].confidence > 0.5);
    // Image annotee, ecrite sous le nom de la reference (`<uuid>_<date>.jpg`) et lisible.
    assert!(
        event.frame_path.starts_with(&wants_bus.uuid.to_string()),
        "{}",
        event.frame_path
    );
    let bytes =
        std::fs::read(Path::new(&r.st.config.detections_dir).join(&event.frame_path)).unwrap();
    assert!(image::load_from_memory(&bytes).is_ok());

    let msg = message_for(&mut r.mqtt, &wants_bus.uuid.to_string(), "detection").await;
    assert_eq!(msg["detections"][0]["class"], "bus");
    assert_eq!(msg["camera_name"], "bus");
    for _ in 0..50 {
        if !r.smtp.mails().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(r.smtp.mails()[0].data.contains("bus"));
    // Sans « enregistrer les images », pas de fichier.
    cfg.save_frames = false;
    r.st.store.detector_config().update(&cfg).await.unwrap();
    let again =
        r.st.detector
            .detect(&r.st, &wants_bus, DetectionTrigger::Motion)
            .await
            .unwrap();
    assert_eq!(again.frame_path, "");
    r.st.frames.stop_all();
}
