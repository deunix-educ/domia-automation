// SPDX-License-Identifier: AGPL-3.0-or-later
//! Role `video`, derniere image et snapshots indexes (R6 lot 6) : de vrais `ffmpeg` sur des sources `lavfi` (ignore sans `ffmpeg`), un vrai
//! dossier temporaire, une vraie base.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::state;
use dominet_domain::camera::{CameraDraft, CameraId};
use dominet_server::cameras::{CameraInput, SecretInput};
use dominet_server::frames::{FrameHub, latest_dir};
use dominet_server::snapshots::SnapshotError;
use dominet_surveillance::frames::{FrameConfig, Hwaccel};
use sqlx::PgPool;

const JPEG: [u8; 8] = [0xFF, 0xD8, 1, 2, 3, 4, 0xFF, 0xD9];

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

/// Dossier temporaire propre a un test, supprime a la fin.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "dominet-test-{}-{}",
            std::process::id(),
            uuid::Uuid::from_u128(rand_u128())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

fn rand_u128() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn input(name: &str, active: bool) -> CameraInput {
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
}

fn test_state(pool: PgPool, dir: &std::path::Path, max: &str) -> dominet_server::AppState {
    let mut st = state(
        pool,
        &[
            ("SNAPSHOTS_DIR", dir.to_str().unwrap()),
            ("SNAPSHOTS_MAX_PER_CAMERA", max),
        ],
    );
    st.frames = FrameHub::with_factory(
        |_| {
            let mut cfg = FrameConfig::new(
                ["-re", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=10"]
                    .map(String::from)
                    .to_vec(),
            );
            cfg.hwaccel = Hwaccel::Off;
            cfg.backoff_min = Duration::from_millis(100);
            Some(cfg)
        },
        dir,
    );
    st
}

async fn wait_for(what: &str, ok: impl Fn() -> bool) {
    for _ in 0..150 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what}");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_video_role_follows_the_cameras_and_keeps_their_last_image_on_disk(pool: PgPool) {
    if !have_ffmpeg() {
        return;
    }
    let dir = TempDir::new();
    let st = test_state(pool, &dir.0, "500");
    dominet_server::frames::spawn_role(&st);
    // Une camera active : une source, et sa derniere image apparait.
    let (cam, _) = st.cameras.save(None, input("Salon", true)).await.unwrap();
    let latest = latest_dir(&dir.0).join(format!("{}.jpg", cam.uuid));
    wait_for("derniere image", || latest.exists()).await;
    let bytes = std::fs::read(&latest).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xD8]) && bytes.ends_with(&[0xFF, 0xD9]));
    assert!(st.frames.has(cam.uuid));
    // Une camera inactive n'a pas de source.
    let (off, _) = st.cameras.save(None, input("Cave", false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!st.frames.has(off.uuid));
    // Desactivee : la source s'arrete et la derniere image disparait.
    st.cameras
        .save(Some(cam.id), input("Salon", false))
        .await
        .unwrap();
    wait_for("source arretee", || !st.frames.has(cam.uuid)).await;
    wait_for("image retiree", || !latest.exists()).await;
    st.frames.stop_all();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_last_image_is_written_at_most_once_per_second_and_atomically(pool: PgPool) {
    if !have_ffmpeg() {
        return;
    }
    let dir = TempDir::new();
    let st = test_state(pool, &dir.0, "500");
    let (cam, _) = st.cameras.save(None, input("Salon", true)).await.unwrap();
    st.frames.reconcile(std::slice::from_ref(&cam));
    let latest = latest_dir(&dir.0).join(format!("{}.jpg", cam.uuid));
    wait_for("derniere image", || latest.exists()).await;
    // 10 images par seconde en entree (2 par defaut en production) : on ne reecrit pas plus d'une fois par seconde.
    let mut changes = 0;
    let mut last = std::fs::metadata(&latest).unwrap().modified().unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(3200) {
        let now = std::fs::metadata(&latest).unwrap().modified().unwrap();
        if now != last {
            changes += 1;
            last = now;
        }
        // A tout instant le fichier est un JPEG complet (renommage atomique).
        let bytes = std::fs::read(&latest).unwrap();
        assert!(bytes.ends_with(&[0xFF, 0xD9]), "fichier incomplet lu");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!((2..=4).contains(&changes), "{changes} ecritures en 3,2 s");
    st.frames.stop_all();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_latest_image_is_served_only_while_it_is_recent_and_fresh_waits_for_a_new_one(
    pool: PgPool,
) {
    let dir = TempDir::new();
    let st = test_state(pool, &dir.0, "500");
    let (cam, _) = st.cameras.save(None, input("Salon", true)).await.unwrap();
    let path = latest_dir(&dir.0).join(format!("{}.jpg", cam.uuid));
    assert!(
        st.snapshots.latest(cam.uuid).await.is_none(),
        "rien tant que le role video n'a rien ecrit"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, JPEG).unwrap();
    let image = st.snapshots.latest(cam.uuid).await.unwrap();
    assert_eq!(image.bytes, JPEG);
    assert!(image.age < Duration::from_secs(5));

    // Une image qui n'est pas un JPEG n'est jamais servie.
    std::fs::write(&path, b"<html>pas une image</html>").unwrap();
    assert!(st.snapshots.latest(cam.uuid).await.is_none());

    // Une image trop vieille : la camera n'envoie plus rien.
    std::fs::write(&path, JPEG).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(120);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(
        st.snapshots.latest(cam.uuid).await.is_none(),
        "image de plus d'une minute"
    );

    // `fresh` attend une image posterieure a l'appel.
    let writer = {
        let path = path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            std::fs::write(&path, [0xFF, 0xD8, 9, 9, 0xFF, 0xD9]).unwrap();
        })
    };
    let started = std::time::Instant::now();
    let fresh = st
        .snapshots
        .fresh(cam.uuid, Duration::from_secs(4))
        .await
        .unwrap();
    assert_eq!(
        fresh.bytes,
        [0xFF, 0xD8, 9, 9, 0xFF, 0xD9],
        "l'image posterieure, pas l'ancienne"
    );
    assert!(started.elapsed() >= Duration::from_millis(600));
    writer.await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn indexed_snapshots_are_files_and_rows_pruned_beyond_the_cap_and_removed_with_the_camera(
    pool: PgPool,
) {
    let dir = TempDir::new();
    let st = test_state(pool, &dir.0, "3");
    let (cam, _) = st.cameras.save(None, input("Salon", true)).await.unwrap();
    let (other, _) = st.cameras.save(None, input("Garage", true)).await.unwrap();
    let t0 = chrono::Utc::now();
    let mut made = vec![];
    for i in 0..5 {
        let s = st
            .snapshots
            .index(
                &cam,
                t0 + chrono::Duration::seconds(i),
                &[0xFF, 0xD8, u8::try_from(i).unwrap(), 0xFF, 0xD9],
            )
            .await
            .unwrap();
        made.push(s);
    }
    st.snapshots.index(&other, t0, &JPEG).await.unwrap();

    // Plafond de 3 par camera : les deux plus anciens sont partis, fichier compris ; l'autre camera n'est pas touchee.
    let kept = st
        .snapshots
        .list(Some(cam.id), None, None, 50)
        .await
        .unwrap();
    assert_eq!(
        kept.iter().map(|s| s.id).collect::<Vec<_>>(),
        [made[4].id, made[3].id, made[2].id]
    );
    for gone in &made[..2] {
        assert!(!dir.0.join(&gone.file).exists(), "{}", gone.file);
    }
    for stays in &made[2..] {
        assert!(dir.0.join(&stays.file).exists());
    }
    assert_eq!(
        st.snapshots.list(None, None, None, 50).await.unwrap().len(),
        4,
        "3 + 1 de l'autre camera"
    );

    // Lecture, filtre par jour, refus d'un non-JPEG.
    let (row, bytes) = st.snapshots.read(made[3].id).await.unwrap().unwrap();
    assert_eq!(
        (row.id, bytes.as_slice()),
        (made[3].id, [0xFF, 0xD8, 3, 0xFF, 0xD9].as_slice())
    );
    assert!(
        st.snapshots.read(made[0].id).await.unwrap().is_none(),
        "supprime"
    );
    assert!(matches!(
        st.snapshots.index(&cam, t0, b"<html>").await,
        Err(SnapshotError::Invalid)
    ));
    let tomorrow = t0 + chrono::Duration::days(1);
    assert_eq!(
        st.snapshots
            .list(None, Some(tomorrow), None, 50)
            .await
            .unwrap()
            .len(),
        0
    );

    // Suppression a la main : ligne et fichier.
    st.snapshots.delete(made[4].id).await.unwrap();
    assert!(!dir.0.join(&made[4].file).exists());
    assert!(st.snapshots.delete(made[4].id).await.is_err());

    // Supprimer la camera supprime son dossier et sa derniere image.
    let last = latest_dir(&dir.0).join(format!("{}.jpg", cam.uuid));
    std::fs::create_dir_all(last.parent().unwrap()).unwrap();
    std::fs::write(&last, JPEG).unwrap();
    st.cameras.delete(cam.id).await.unwrap();
    assert!(!dir.0.join(cam.uuid.to_string()).exists() && !last.exists());
    assert!(
        dir.0.join(other.uuid.to_string()).exists(),
        "l'autre camera est intacte"
    );
    let _ = CameraId(0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_capture_needs_a_recent_image(pool: PgPool) {
    let dir = TempDir::new();
    let st = test_state(pool, &dir.0, "500");
    let (cam, _) = st.cameras.save(None, input("Salon", true)).await.unwrap();
    // Aucune image : la capture le dit au lieu d'inventer.
    let started = std::time::Instant::now();
    assert!(matches!(
        st.snapshots.capture(&cam).await,
        Err(SnapshotError::NoImage)
    ));
    assert!(
        started.elapsed() >= Duration::from_secs(3),
        "elle a attendu une image fraiche"
    );
    // Avec une image recente, elle est indexee.
    let path = latest_dir(&dir.0).join(format!("{}.jpg", cam.uuid));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, JPEG).unwrap();
    let s = st.snapshots.capture(&cam).await.unwrap();
    assert_eq!(s.size_bytes, 8);
    assert!(s.file.starts_with(&format!("{}/", cam.uuid)));
}
