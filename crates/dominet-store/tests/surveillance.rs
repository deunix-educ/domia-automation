// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depots de la surveillance (R6 lot 1, migration 0011). Chaque test a sa base isolee (`make dev-up`).

use chrono::{Duration, TimeZone, Utc};
use dominet_domain::RepoError;
use dominet_domain::camera::{
    CameraDraft, Detection, DetectionTrigger, DetectorBackend, NewRecording, NightApi, OnvifInfo,
    PtzPresetDraft, RecordingEvent, RecordingMode, RecordingStatus, Sealed,
};
use dominet_domain::location::LocationDraft;
use dominet_domain::repo::{
    CameraRepo, DetectorConfigRepo, EventRepo, LocationRepo, PtzPresetRepo, RecordingRepo,
    SnapshotRepo,
};
use dominet_store::PgStore;
use sqlx::PgPool;

fn draft(name: &str) -> CameraDraft {
    let mut d = CameraDraft::new(name);
    d.ip = "192.168.1.29".into();
    d.rtsp_user = "admin".into();
    d.rtsp_pass = Sealed("v1:chiffre".into());
    d
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_camera_round_trips_with_a_database_chosen_public_id(pool: PgPool) {
    let store = PgStore::new(pool);
    let mut d = draft("Entree");
    d.night_mode_api = NightApi::Thingino;
    d.recording_mode = RecordingMode::Motion;
    d.yolo_classes = vec!["person".into(), "car".into()];
    d.web_api_key = Sealed("v1:cle".into());
    let cam = store.cameras().create(None, &d).await.unwrap();
    assert_eq!(cam.config, d);
    assert_eq!(cam.uuid.to_string().len(), 32);
    assert_eq!(cam.last_seen, None);

    let by_uuid = store
        .cameras()
        .get_by_uuid(cam.uuid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_uuid, cam);
    let other = store
        .cameras()
        .create(None, &draft("Jardin"))
        .await
        .unwrap();
    assert_ne!(other.uuid, cam.uuid);

    let mut changed = d.clone();
    changed.name = "Porte".into();
    changed.priority = 3;
    let updated = store.cameras().update(cam.id, &changed).await.unwrap();
    assert_eq!(updated.config.name, "Porte");
    assert_eq!(updated.uuid, cam.uuid, "l'identifiant public ne change pas");
    assert!(matches!(
        store
            .cameras()
            .update(dominet_domain::camera::CameraId(0), &d)
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn cameras_are_listed_by_priority_then_name(pool: PgPool) {
    let store = PgStore::new(pool);
    for (name, prio) in [("Zeta", 1), ("Alpha", 2), ("Beta", 1)] {
        let mut d = draft(name);
        d.priority = prio;
        store.cameras().create(None, &d).await.unwrap();
    }
    let names: Vec<_> = store
        .cameras()
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.config.name)
        .collect();
    assert_eq!(names, ["Beta", "Zeta", "Alpha"]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_database_refuses_what_the_domain_refuses(pool: PgPool) {
    let store = PgStore::new(pool);
    let mut bad = draft("Mauvaise");
    bad.port = 70000;
    assert!(store.cameras().create(None, &bad).await.is_err());
    let mut bad = draft("Mauvaise");
    bad.yolo_min_confidence = 2.0;
    assert!(store.cameras().create(None, &bad).await.is_err());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn partial_updates_and_location_link(pool: PgPool) {
    let store = PgStore::new(pool);
    let loc = store
        .locations()
        .create(&LocationDraft {
            name: "Garage".into(),
            description: String::new(),
            active: true,
        })
        .await
        .unwrap();
    let mut d = draft("Garage");
    d.location_id = Some(loc.id.0);
    let cam = store.cameras().create(None, &d).await.unwrap();

    let at = Utc.with_ymd_and_hms(2026, 11, 7, 10, 0, 0).unwrap();
    store.cameras().touch_last_seen(cam.id, at).await.unwrap();
    store
        .cameras()
        .apply_onvif(
            cam.id,
            &OnvifInfo {
                manufacturer: "Longse".into(),
                model: "Q3R".into(),
                firmware: "1.2".into(),
                main_token: "profile_1".into(),
                ptz: true,
            },
        )
        .await
        .unwrap();
    store
        .cameras()
        .set_recording_mode(cam.id, RecordingMode::Continuous)
        .await
        .unwrap();
    store
        .cameras()
        .set_yolo_enabled(cam.id, true)
        .await
        .unwrap();
    let c = store.cameras().get(cam.id).await.unwrap().unwrap();
    assert_eq!(c.last_seen, Some(at));
    assert_eq!(
        (
            c.config.model.as_str(),
            c.config.main_token.as_str(),
            c.config.ptz
        ),
        ("Q3R", "profile_1", true)
    );
    assert_eq!(c.config.recording_mode, RecordingMode::Continuous);
    assert!(c.config.yolo_enabled);

    // Supprimer l'emplacement detache la camera.
    store.locations().delete(loc.id).await.unwrap();
    let c = store.cameras().get(cam.id).await.unwrap().unwrap();
    assert_eq!(c.config.location_id, None);
    assert!(matches!(
        store
            .cameras()
            .set_yolo_enabled(dominet_domain::camera::CameraId(0), true)
            .await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn ptz_presets_are_ordered_and_unique_per_camera(pool: PgPool) {
    let store = PgStore::new(pool);
    let cam = store.cameras().create(None, &draft("PTZ")).await.unwrap();
    let other = store.cameras().create(None, &draft("Autre")).await.unwrap();
    let preset = |name: &str, position| PtzPresetDraft {
        name: name.into(),
        token: String::new(),
        pan: 0.25,
        tilt: -0.5,
        zoom: 0.0,
        position,
    };
    store
        .ptz_presets()
        .create(cam.id, &preset("Porte", 2))
        .await
        .unwrap();
    let home = store
        .ptz_presets()
        .create(cam.id, &preset("Maison", 1))
        .await
        .unwrap();
    store
        .ptz_presets()
        .create(other.id, &preset("Porte", 0))
        .await
        .unwrap();
    assert!(matches!(
        store
            .ptz_presets()
            .create(cam.id, &preset("Porte", 5))
            .await,
        Err(RepoError::Conflict { .. })
    ));
    let names: Vec<_> = store
        .ptz_presets()
        .list(cam.id)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.draft.name)
        .collect();
    assert_eq!(names, ["Maison", "Porte"]);
    assert!((home.draft.tilt + 0.5).abs() < f64::EPSILON);
    store.ptz_presets().delete(home.id).await.unwrap();
    assert_eq!(store.ptz_presets().list(cam.id).await.unwrap().len(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_recording_goes_from_pending_to_done_or_error(pool: PgPool) {
    let store = PgStore::new(pool);
    let cam = store.cameras().create(None, &draft("Rec")).await.unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 7, 10, 0, 0).unwrap();
    let new = |path: &str, at| NewRecording {
        started_at: at,
        segment_path: path.into(),
        event_type: RecordingEvent::Continuous,
        size_bytes: Some(1024),
        duration_ms: Some(60_000),
    };
    let a = store
        .recordings()
        .create(cam.id, &new("a/1.mp4", t0))
        .await
        .unwrap();
    let b = store
        .recordings()
        .create(cam.id, &new("a/2.mp4", t0 + Duration::minutes(1)))
        .await
        .unwrap();
    assert_eq!(a.status, RecordingStatus::Pending);
    assert!(matches!(
        store.recordings().create(cam.id, &new("a/1.mp4", t0)).await,
        Err(RepoError::Conflict { .. })
    ));
    assert!(store.recordings().path_known("a/1.mp4").await.unwrap());
    assert!(!store.recordings().path_known("a/9.mp4").await.unwrap());

    store.recordings().mark_uploading(a.id).await.unwrap();
    store
        .recordings()
        .mark_error(b.id, "reduct injoignable")
        .await
        .unwrap();
    let todo = store.recordings().list_unarchived(10).await.unwrap();
    assert_eq!(todo.len(), 2, "uploading et error restent a traiter");
    assert_eq!(todo[0].id, a.id, "les plus anciens d'abord");
    assert_eq!(todo[1].error, "reduct injoignable");

    store
        .recordings()
        .mark_done(a.id, 1_762_509_600_000_000)
        .await
        .unwrap();
    let done = store.recordings().get(a.id).await.unwrap().unwrap();
    assert_eq!(
        (done.status, done.reduct_ts, done.error.as_str()),
        (RecordingStatus::Done, Some(1_762_509_600_000_000), "")
    );
    assert_eq!(
        store.recordings().list_unarchived(10).await.unwrap().len(),
        1
    );
    let listed = store.recordings().list(cam.id, 10).await.unwrap();
    assert_eq!(listed[0].id, b.id, "les plus recents d'abord");

    store.recordings().delete(a.id).await.unwrap();
    assert!(matches!(
        store.recordings().delete(a.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn snapshots_motion_and_detections(pool: PgPool) {
    let store = PgStore::new(pool);
    let cam = store.cameras().create(None, &draft("Evt")).await.unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 7, 10, 0, 0).unwrap();

    let s1 = store
        .snapshots()
        .create(cam.id, t0, "x/1.jpg", 100)
        .await
        .unwrap();
    store
        .snapshots()
        .create(cam.id, t0 + Duration::minutes(1), "x/2.jpg", 120)
        .await
        .unwrap();
    let listed = store.snapshots().list(cam.id, 10).await.unwrap();
    assert_eq!(listed[0].file, "x/2.jpg");
    assert_eq!(store.snapshots().list(cam.id, 1).await.unwrap().len(), 1);
    store.snapshots().delete(s1.id).await.unwrap();
    assert!(store.snapshots().get(s1.id).await.unwrap().is_none());

    store
        .camera_events()
        .record_motion(cam.id, t0, 0.04)
        .await
        .unwrap();
    let inside = t0 + Duration::seconds(30);
    assert!(
        store
            .camera_events()
            .motion_between(cam.id, t0 - Duration::seconds(5), inside)
            .await
            .unwrap()
    );
    assert!(
        !store
            .camera_events()
            .motion_between(cam.id, t0 + Duration::minutes(1), t0 + Duration::minutes(2))
            .await
            .unwrap()
    );
    assert!(
        store
            .camera_events()
            .record_motion(cam.id, t0, 1.5)
            .await
            .is_err()
    );

    let labels = vec![
        Detection {
            class: "person".into(),
            confidence: 0.87,
            bbox: [10.0, 20.0, 110.5, 220.0],
        },
        Detection {
            class: "car".into(),
            confidence: 0.55,
            bbox: [0.0, 0.0, 5.0, 5.0],
        },
    ];
    let ev = store
        .camera_events()
        .record_detection(cam.id, t0, DetectionTrigger::Motion, &labels, "d/1.jpg")
        .await
        .unwrap();
    let back = store
        .camera_events()
        .list_detections(cam.id, 5)
        .await
        .unwrap();
    assert_eq!(back, [ev]);
    assert_eq!(back[0].class_names(), ["car", "person"]);
    assert_eq!(
        store
            .camera_events()
            .list_motion(cam.id, 5)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deleting_a_camera_deletes_everything_it_owns(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let cam = store
        .cameras()
        .create(None, &draft("Cascade"))
        .await
        .unwrap();
    let t0 = Utc::now();
    store
        .snapshots()
        .create(cam.id, t0, "c/1.jpg", 1)
        .await
        .unwrap();
    store
        .camera_events()
        .record_motion(cam.id, t0, 0.1)
        .await
        .unwrap();
    store
        .camera_events()
        .record_detection(cam.id, t0, DetectionTrigger::Periodic, &[], "")
        .await
        .unwrap();
    store
        .ptz_presets()
        .create(
            cam.id,
            &PtzPresetDraft {
                name: "p".into(),
                token: String::new(),
                pan: 0.0,
                tilt: 0.0,
                zoom: 0.0,
                position: 0,
            },
        )
        .await
        .unwrap();
    store
        .recordings()
        .create(
            cam.id,
            &NewRecording {
                started_at: t0,
                segment_path: "c/1.mp4".into(),
                event_type: RecordingEvent::Manual,
                size_bytes: None,
                duration_ms: None,
            },
        )
        .await
        .unwrap();
    store.cameras().delete(cam.id).await.unwrap();
    for (table, sql) in [
        ("ptz_preset", "SELECT count(*) FROM ptz_preset"),
        ("recording", "SELECT count(*) FROM recording"),
        ("snapshot", "SELECT count(*) FROM snapshot"),
        ("motion_event", "SELECT count(*) FROM motion_event"),
        ("detection_event", "SELECT count(*) FROM detection_event"),
    ] {
        let n: i64 = sqlx::query_scalar(sql).fetch_one(&pool).await.unwrap();
        assert_eq!(n, 0, "{table}");
    }
    assert!(matches!(
        store.cameras().delete(cam.id).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_detector_config_is_a_singleton_that_exists_from_the_start(pool: PgPool) {
    let store = PgStore::new(pool);
    let mut c = store.detector_config().get().await.unwrap();
    assert_eq!(
        (c.backend, c.enabled, c.device.as_str()),
        (DetectorBackend::Yolov8n, false, "cpu")
    );
    c.enabled = true;
    c.save_frames = true;
    c.backend = DetectorBackend::Onnx;
    let saved = store.detector_config().update(&c).await.unwrap();
    assert!(saved.enabled && saved.save_frames);
    assert_eq!(saved.backend, DetectorBackend::Onnx);
    assert!(saved.updated_at >= c.updated_at);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_preset_of_the_same_name_is_replaced_not_duplicated(pool: PgPool) {
    let store = PgStore::new(pool);
    let cam = store.cameras().create(None, &draft("Rot")).await.unwrap();
    let other = store.cameras().create(None, &draft("Autre")).await.unwrap();
    let preset = |token: &str, pan| PtzPresetDraft {
        name: "Porte".into(),
        token: token.into(),
        pan,
        tilt: 0.0,
        zoom: 0.0,
        position: 0,
    };
    let first = store
        .ptz_presets()
        .upsert(cam.id, &preset("p1", 10.0))
        .await
        .unwrap();
    let again = store
        .ptz_presets()
        .upsert(cam.id, &preset("p2", 20.0))
        .await
        .unwrap();
    assert_eq!(first.id, again.id, "meme ligne");
    assert_eq!((again.draft.token.as_str(), again.draft.pan), ("p2", 20.0));
    assert_eq!(store.ptz_presets().list(cam.id).await.unwrap().len(), 1);
    // Le meme nom sur une autre camera est un autre preset.
    let elsewhere = store
        .ptz_presets()
        .upsert(other.id, &preset("x", 1.0))
        .await
        .unwrap();
    assert_ne!(elsewhere.id, again.id);
    assert_eq!(
        store
            .ptz_presets()
            .get(again.id)
            .await
            .unwrap()
            .unwrap()
            .camera,
        cam.id
    );
    assert!(
        store
            .ptz_presets()
            .get(dominet_domain::camera::PtzPresetId(0))
            .await
            .unwrap()
            .is_none()
    );
}
