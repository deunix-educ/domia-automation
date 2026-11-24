// SPDX-License-Identifier: AGPL-3.0-or-later
//! The production MediaMTX configuration (`deploy/mediamtx/mediamtx.yml`, ADR 0064) against a **real MediaMTX 1.21.3** that asks a
//! **real DomiNet router** to authorise every read. Ignored by default (needs Docker, the image `bluenviron/mediamtx:1.21.3`, `ffmpeg`,
//! `ffprobe` and `curl`; ports 28554, 28889, 28189, 29997 and 29998 free): to be run by hand,
//! `cargo test -p dominet-server --test real_mediamtx_auth -- --ignored`.
//! Only the ports and the address of DomiNet are rewritten in the configuration: the authentication section is the shipped one.

mod common;

use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::Utc;
use common::{add_user, state};
use dominet_domain::repo::{StreamAccessRepo, UserRepo};
use dominet_domain::stream_access::NewStreamCredential;
use dominet_domain::user::Role;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const CONTAINER: &str = "dominet-test-mediamtx-auth";
const RTSP: &str = "rtsp://127.0.0.1:28554";

struct Bench {
    dir: std::path::PathBuf,
    publisher: Option<std::process::Child>,
}

impl Drop for Bench {
    fn drop(&mut self) {
        if let Some(p) = &mut self.publisher {
            let _ = p.kill();
            let _ = p.wait();
        }
        let _ = Command::new("docker")
            .args(["rm", "-f", CONTAINER])
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn digest(s: &str) -> Vec<u8> {
    Sha256::digest(s.as_bytes()).to_vec()
}

/// `ffprobe` on a URL: did MediaMTX let the read through?
async fn probe(url: &str) -> bool {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || {
        Command::new("ffprobe")
            .args(["-v", "error", "-rtsp_transport", "tcp", "-i", &url])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
    .await
    .unwrap()
}

/// What MediaMTX logged (to explain a failure).
async fn logs() -> String {
    tokio::task::spawn_blocking(logs_now).await.unwrap()
}

fn logs_now() -> String {
    let out = Command::new("docker")
        .args(["logs", "--tail", "15", CONTAINER])
        .output()
        .unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Status of a WHEP offer, with an optional `Authorization` header.
async fn whep(authorization: Option<&str>) -> String {
    let authorization = authorization.map(str::to_string);
    tokio::task::spawn_blocking(move || {
    let offer = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=mid:0\r\na=recvonly\r\na=rtcp-mux\r\na=ice-ufrag:abcd\r\na=ice-pwd:abcdefghijklmnopqrstuv12\r\na=fingerprint:sha-256 AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB:AB\r\na=setup:actpass\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n";
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST"])
        .args([
            "-H",
            "Content-Type: application/sdp",
            "--data-binary",
            offer,
        ]);
    if let Some(a) = &authorization {
        cmd.args(["-H", &format!("Authorization: {a}")]);
    }
    cmd.arg("http://127.0.0.1:28889/cam1/whep");
    String::from_utf8_lossy(&cmd.output().unwrap().stdout).to_string()
    })
    .await
    .unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
#[ignore = "needs Docker, MediaMTX 1.21.3, ffmpeg and free ports 28554, 28889, 28189, 29997, 29998"]
async fn mediamtx_with_the_shipped_configuration_asks_dominet(pool: PgPool) {
    let state = state(
        pool,
        &[
            ("MEDIAMTX_READ_USER", "dominet-video"),
            ("MEDIAMTX_READ_PASSWORD", "service-pw"),
        ],
    );
    add_user(&state, "alice", Role::User).await;
    let alice = state
        .store
        .users()
        .find_credentials("alice")
        .await
        .unwrap()
        .unwrap()
        .user
        .id;
    let repo = state.store.stream_access();
    let new = NewStreamCredential::new("VLC", 30).unwrap();
    repo.create_credential(alice, &new, &digest("alice-secret"), "dns_x", Utc::now())
        .await
        .unwrap();
    repo.create_viewing(alice, &digest("view-token"), "dnv_x", Utc::now())
        .await
        .unwrap();

    // DomiNet's real router, on a real socket.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = dominet_server::web::router(state.clone())
        .into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The shipped configuration, with only the ports and DomiNet's address changed.
    let shipped = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/mediamtx/mediamtx.yml"
    ))
    .unwrap();
    assert!(shipped.contains("authHTTPAddress: http://dominet:8000/internal/mediamtx/auth"));
    let conf = shipped
        .replace("http://dominet:8000", &format!("http://127.0.0.1:{port}"))
        .replace("apiAddress: :9997", "apiAddress: 127.0.0.1:29997")
        .replace("metricsAddress: :9998", "metricsAddress: 127.0.0.1:29998")
        .replace("rtspAddress: :8554", "rtspAddress: :28554")
        .replace("webrtcAddress: :8889", "webrtcAddress: :28889")
        .replace(
            "webrtcLocalTCPAddress: :8189",
            "webrtcLocalTCPAddress: :28189",
        )
        .replace(
            "webrtcLocalUDPAddress: :8189",
            "webrtcLocalUDPAddress: :28189",
        );
    let dir = std::env::temp_dir().join(format!("dominet-mtx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mediamtx.yml"), conf).unwrap();
    let mut bench = Bench {
        dir: dir.clone(),
        publisher: None,
    };
    let _ = Command::new("docker")
        .args(["rm", "-f", CONTAINER])
        .output();
    let started = Command::new("docker")
        .args(["run", "-d", "--name", CONTAINER, "--network", "host", "-v"])
        .arg(format!(
            "{}:/mediamtx.yml:ro",
            dir.join("mediamtx.yml").display()
        ))
        .arg("bluenviron/mediamtx:1.21.3")
        .status()
        .unwrap();
    assert!(started.success());
    tokio::time::sleep(Duration::from_secs(2)).await;

    // A camera stream: a test pattern published from inside (publication is decided by DomiNet, from a private address).
    bench.publisher = Some(
        Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10",
            ])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "10",
            ])
            .args([
                "-f",
                "rtsp",
                "-rtsp_transport",
                "tcp",
                &format!("{RTSP}/cam1"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;

    // The API is reachable without any HTTP call (excluded): the stream exists.
    let paths = Command::new("curl")
        .args(["-s", "http://127.0.0.1:29997/v3/paths/list"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&paths.stdout).contains("\"cam1\""),
        "publication was refused"
    );

    // RTSP: nobody without credentials, an account with its personal credential, the video role with its service account.
    assert!(
        !probe(&format!("{RTSP}/cam1")).await,
        "anonymous read must be refused"
    );
    assert!(
        probe("rtsp://alice:alice-secret@127.0.0.1:28554/cam1").await,
        "MediaMTX said:\n{}",
        logs().await
    );
    assert!(!probe("rtsp://alice:wrong@127.0.0.1:28554/cam1").await);
    assert!(!probe("rtsp://bob:alice-secret@127.0.0.1:28554/cam1").await);
    assert!(probe("rtsp://dominet-video:service-pw@127.0.0.1:28554/cam1").await);
    // A viewing token is for the browser, not for RTSP.
    assert!(!probe("rtsp://alice:view-token@127.0.0.1:28554/cam1").await);

    // WebRTC (WHEP): the Bearer viewing token alone.
    assert_eq!(whep(None).await, "401");
    assert_eq!(whep(Some("Bearer view-token")).await, "201");
    assert_eq!(whep(Some("Bearer nope")).await, "401");
}
