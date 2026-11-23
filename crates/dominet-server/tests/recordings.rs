// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recordings (R6 lot 7): segment indexing, archiving into the **real dev-stack ReductStore** (throw-away bucket),
//! deletion of the local file, reading from the archive, manual recording against the fake MediaMTX.
//!
//! `REDUCTSTORE_URL` (default `http://127.0.0.1:18383`) and `REDUCTSTORE_TOKEN` (default `dominet-dev-token`).

mod common;

use std::fs::{File, FileTimes};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{TimeZone, Utc};
use common::state;
use dominet_domain::camera::{
    CameraDraft, CameraUuid, RecordingEvent, RecordingMode, RecordingStatus,
};
use dominet_domain::repo::{EventRepo, RecordingRepo};
use dominet_server::cameras::{CameraInput, SecretInput};
use dominet_server::recordings::{RecordingError, SegmentBody};
use dominet_surveillance::testing::FakeMediaMtx;
use sqlx::PgPool;

/// Temporary folder specific to a test, deleted at the end.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dominet-rec-{}-{nanos}", std::process::id()));
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

fn reduct_url() -> String {
    std::env::var("REDUCTSTORE_URL").unwrap_or_else(|_| "http://127.0.0.1:18383".into())
}

fn reduct_token() -> String {
    std::env::var("REDUCTSTORE_TOKEN").unwrap_or_else(|_| "dominet-dev-token".into())
}

fn bucket(tag: &str) -> String {
    format!("test-video-{tag}-{}", std::process::id())
}

fn input(name: &str, mode: RecordingMode) -> CameraInput {
    let mut draft = CameraDraft::new(name);
    draft.ip = "192.168.1.29".into();
    draft.recording_mode = mode;
    CameraInput {
        draft,
        rtsp_pass: SecretInput::Keep,
        onvif_pass: SecretInput::Keep,
        web_pass: SecretInput::Keep,
        web_api_key: SecretInput::Keep,
    }
}

fn segment(root: &Path, camera: CameraUuid, name: &str, bytes: &[u8]) -> PathBuf {
    let dir = root.join(camera.to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    // Old: the segment is finished.
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(120)))
        .unwrap();
    path
}

fn test_state(
    pool: PgPool,
    dir: &Path,
    extra: &[(&str, &str)],
    bucket: &str,
    url: &str,
) -> dominet_server::AppState {
    let token = reduct_token();
    let mut vars = vec![
        ("RECORDINGS_DIR", dir.to_str().unwrap()),
        ("REDUCTSTORE_URL", url),
        ("REDUCTSTORE_TOKEN", token.as_str()),
        ("REDUCTSTORE_VIDEO_BUCKET", bucket),
    ];
    vars.extend_from_slice(extra);
    state(pool, &vars)
}

/// Direct access to the throw-away bucket of a test state (checks and cleanup).
async fn raw_bucket(st: &dominet_server::AppState) -> std::sync::Arc<reduct_rs::Bucket> {
    let cfg = st.config.reductstore.as_ref().unwrap();
    dominet_store::video::ReductVideo::new(
        &cfg.url,
        cfg.token
            .as_ref()
            .map(dominet_server::config::Secret::expose),
        &cfg.video_bucket,
        1 << 30,
    )
    .unwrap()
    .open()
    .await
    .unwrap()
}

async fn drop_bucket(st: &dominet_server::AppState) {
    // The throw-away bucket disappears with its segments.
    let _ = raw_bucket(st).await.remove().await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn segments_are_indexed_typed_archived_and_the_local_file_goes_away(pool: PgPool) {
    let tmp = TempDir::new();
    let st = test_state(pool, tmp.path(), &[], &bucket("a"), &reduct_url());
    let (cam, _) = st
        .cameras
        .save(None, input("Entr\u{e9}e", RecordingMode::Continuous))
        .await
        .unwrap();
    let (off, _) = st
        .cameras
        .save(None, input("Muette", RecordingMode::None))
        .await
        .unwrap();

    let t0 = Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
    // A motion in the second segment's window.
    st.store
        .camera_events()
        .record_motion(cam.id, t0 + chrono::Duration::seconds(75), 0.4)
        .await
        .unwrap();
    let first = segment(
        tmp.path(),
        cam.uuid,
        "2026-11-08_12-00-00-000000.mp4",
        b"segment-un",
    );
    let second = segment(
        tmp.path(),
        cam.uuid,
        "2026-11-08_12-01-00-000000.mp4",
        b"segment-deux",
    );
    // A camera that does not record: never indexed or touched.
    let ignored = segment(tmp.path(), off.uuid, "2026-11-08_12-00-00-000000.mp4", b"x");
    // Still being written: expected at the next pass.
    let fresh = tmp
        .path()
        .join(cam.uuid.to_string())
        .join("2026-11-08_12-02-00-000000.mp4");
    std::fs::write(&fresh, b"en-cours").unwrap();

    let report = st.recordings.scan().await.unwrap();
    assert_eq!((report.indexed, report.archived, report.failed), (2, 2, 0));

    let list = st
        .store
        .recordings()
        .list_between(cam.id, t0, t0 + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].event_type, RecordingEvent::Continuous);
    assert_eq!(list[1].event_type, RecordingEvent::Motion);
    assert!(
        list.iter()
            .all(|r| r.status == RecordingStatus::Done && r.reduct_ts.is_some())
    );
    assert_eq!(list[0].size_bytes, Some(10));

    // Safe archive: no more local file for the pushed segments; the others are intact.
    assert!(!first.exists() && !second.exists());
    assert!(ignored.exists() && fresh.exists());

    // The segment is read back from ReductStore, with the reference's labels.
    let body = st
        .recordings
        .open(cam.uuid, &list[0])
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&body, SegmentBody::Memory(d) if d == b"segment-un"));
    let record = raw_bucket(&st)
        .await
        .read_record(&format!("cam_{}", cam.uuid))
        .timestamp_us(u64::try_from(list[0].reduct_ts.unwrap()).unwrap())
        .send()
        .await
        .unwrap();
    let labels = record.labels();
    assert_eq!(
        labels.get("event_type").map(String::as_str),
        Some("continuous")
    );
    assert_eq!(
        labels.get("camera_name").map(String::as_str),
        Some("Entree")
    );
    assert_eq!(
        labels.get("camera_uuid").map(String::as_str),
        Some(cam.uuid.to_string().as_str())
    );

    // A second pass recreates nothing.
    assert_eq!(st.recordings.scan().await.unwrap().indexed, 0);

    // Deletion: archive, file and row.
    st.recordings.delete(cam.uuid, &list[0]).await.unwrap();
    assert!(
        st.store
            .recordings()
            .get(list[0].id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        st.recordings
            .open(cam.uuid, &list[0])
            .await
            .unwrap()
            .is_none()
    );
    drop_bucket(&st).await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_failed_push_keeps_the_file_and_is_retried(pool: PgPool) {
    let tmp = TempDir::new();
    // An unreachable ReductStore: the segment is indexed, the failure is noted, the file stays.
    let broken = test_state(
        pool.clone(),
        tmp.path(),
        &[],
        &bucket("b"),
        "http://127.0.0.1:1",
    );
    let (cam, _) = broken
        .cameras
        .save(None, input("Garage", RecordingMode::Continuous))
        .await
        .unwrap();
    let path = segment(
        tmp.path(),
        cam.uuid,
        "2026-11-08_13-00-00-000000.mp4",
        b"important",
    );
    let report = broken.recordings.scan().await.unwrap();
    assert_eq!((report.indexed, report.archived, report.failed), (1, 0, 1));
    assert!(path.exists());
    let rec = broken
        .store
        .recordings()
        .list(cam.id, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(rec.status, RecordingStatus::Error);
    assert!(rec.error.contains("ReductStore"), "{}", rec.error);
    // Served from the disk in the meantime.
    assert!(matches!(
        broken.recordings.open(cam.uuid, &rec).await.unwrap(),
        Some(SegmentBody::Local(_))
    ));

    // The server comes back: the same segment is picked up at the next pass.
    let good = test_state(pool, tmp.path(), &[], &bucket("b2"), &reduct_url());
    let report = good.recordings.scan().await.unwrap();
    assert_eq!((report.indexed, report.archived, report.failed), (0, 1, 0));
    assert!(!path.exists());
    let rec = good.store.recordings().get(rec.id).await.unwrap().unwrap();
    assert_eq!(rec.status, RecordingStatus::Done);
    assert_eq!(rec.error, "");
    drop_bucket(&good).await;
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_an_archive_segments_stay_on_disk_and_are_served_from_it(pool: PgPool) {
    let tmp = TempDir::new();
    let st = state(pool, &[("RECORDINGS_DIR", tmp.path().to_str().unwrap())]);
    assert!(!st.recordings.has_archive());
    let (cam, _) = st
        .cameras
        .save(None, input("Cave", RecordingMode::Manual))
        .await
        .unwrap();
    let path = segment(
        tmp.path(),
        cam.uuid,
        "2026-11-08_14-00-00-000000.mp4",
        b"local",
    );
    let report = st.recordings.scan().await.unwrap();
    assert_eq!((report.indexed, report.archived, report.failed), (1, 0, 0));
    assert!(path.exists());
    let rec = st
        .store
        .recordings()
        .list(cam.id, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(rec.event_type, RecordingEvent::Manual);
    assert_eq!(rec.status, RecordingStatus::Pending);
    assert!(matches!(
        st.recordings.open(cam.uuid, &rec).await.unwrap(),
        Some(SegmentBody::Local(p)) if p == path
    ));
    // A path outside the folder is never served, even if it is in the database.
    let mut outside = rec.clone();
    outside.segment_path = "/etc/passwd".into();
    assert!(
        st.recordings
            .open(cam.uuid, &outside)
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn manual_recording_toggles_the_mediamtx_record_flag_only_in_manual_mode(pool: PgPool) {
    let fake = FakeMediaMtx::start().await;
    let tmp = TempDir::new();
    let st = state(
        pool,
        &[
            ("RECORDINGS_DIR", tmp.path().to_str().unwrap()),
            ("MEDIAMTX_API_URL", &fake.url()),
            (
                "DOMINET_MASTER_KEY",
                &dominet_server::secrets::MasterKey::generate(),
            ),
        ],
    );
    let (manual, _) = st
        .cameras
        .save(None, input("Manuelle", RecordingMode::Manual))
        .await
        .unwrap();
    let (cont, _) = st
        .cameras
        .save(None, input("Continue", RecordingMode::Continuous))
        .await
        .unwrap();
    let name = manual.uuid.to_string();
    // A path in manual mode starts without recording.
    assert_ne!(fake.path(&name).unwrap()["record"].as_bool(), Some(true));
    assert_eq!(st.recordings.manual_state(manual.uuid).await, Some(false));

    st.recordings.set_manual(manual.id, true).await.unwrap();
    assert_eq!(fake.path(&name).unwrap()["record"], true);
    assert_eq!(st.recordings.manual_state(manual.uuid).await, Some(true));
    st.recordings.set_manual(manual.id, false).await.unwrap();
    assert_eq!(fake.path(&name).unwrap()["record"], false);

    assert!(matches!(
        st.recordings.set_manual(cont.id, true).await,
        Err(RecordingError::NotManual)
    ));
    assert!(matches!(
        st.recordings
            .set_manual(dominet_domain::camera::CameraId(0), true)
            .await,
        Err(RecordingError::NotFound)
    ));
}
