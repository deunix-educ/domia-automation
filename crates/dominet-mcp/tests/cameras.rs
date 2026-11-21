// SPDX-License-Identifier: AGPL-3.0-or-later
//! The camera tools and the reads that came with them (R11 M3b): a real MCP client against simulated cameras (a fake ONVIF
//! device and a fake Thingino night API), a real snapshot file, and the energy balance.

#[macro_use]
mod common;

use chrono::{NaiveDate, Utc};
use common::{make_token, serve};
use dominet_domain::camera::{
    Camera, CameraDraft, NewRecording, NightApi, RecordingEvent, RecordingMode,
};
use dominet_domain::energy::EnergyDay;
use dominet_domain::mcp::AuditKind;
use dominet_domain::repo::{EnergyRepo, EventRepo, McpAuditRepo, RecordingRepo, SnapshotRepo};
use dominet_server::AppState;
use dominet_server::cameras::{CameraInput, SecretInput};
use dominet_server::secrets::MasterKey;
use dominet_surveillance::testing::night::FakeThingino;
use dominet_surveillance::testing::onvif::{FakeOnvif, Family};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use sqlx::PgPool;

macro_rules! call {
    ($client:expr, $tool:expr, $args:expr) => {{
        let args = $args.as_object().cloned().unwrap_or_default();
        $client
            .call_tool(CallToolRequestParams::new($tool).with_arguments(args))
            .await
    }};
}

macro_rules! ok {
    ($client:expr, $tool:expr, $args:expr) => {{
        let result = call!($client, $tool, $args).unwrap();
        assert_ne!(
            result.is_error,
            Some(true),
            "{}: {:?}",
            $tool,
            result.content
        );
        result.structured_content.expect("structured output")
    }};
}

/// A state with a master key (camera secrets are sealed) and a private snapshots folder.
fn camera_state(pool: PgPool, dir: &std::path::Path) -> AppState {
    let key = MasterKey::generate();
    common::state(
        pool,
        &[
            ("MCP_ENABLED", "true"),
            ("DOMINET_MASTER_KEY", &key),
            ("SNAPSHOTS_DIR", dir.to_str().unwrap()),
        ],
    )
}

fn input(name: &str) -> CameraInput {
    let mut draft = CameraDraft::new(name);
    draft.ip = "127.0.0.1".into();
    draft.rtsp_user = "admin".into();
    CameraInput {
        draft,
        rtsp_pass: SecretInput::Set("flux-secret".into()),
        onvif_pass: SecretInput::Keep,
        web_pass: SecretInput::Keep,
        web_api_key: SecretInput::Keep,
    }
}

async fn save(st: &AppState, input: CameraInput) -> Camera {
    st.cameras.save(None, input).await.unwrap().0
}

/// A PTZ camera backed by a fake ONVIF device.
async fn ptz_camera(st: &AppState, name: &str) -> (Camera, FakeOnvif) {
    let onvif = FakeOnvif::start(Family::Thingino, "admin", "mot-de-passe").await;
    let mut i = input(name);
    i.draft.onvif = true;
    i.draft.onvif_port = i32::from(onvif.port());
    i.draft.ptz = true;
    i.rtsp_pass = SecretInput::Set("mot-de-passe".into());
    let camera = save(st, i).await;
    // `ptz` is something the ONVIF synchronisation reads: set it as it would.
    st.cameras.onvif_sync(camera.id).await.unwrap();
    (camera, onvif)
}

async fn control_lines(st: &AppState) -> Vec<dominet_domain::mcp::McpAuditEntry> {
    st.store
        .mcp_audit()
        .recent(Some(AuditKind::ToolCall), 20)
        .await
        .unwrap()
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dominet-mcp-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_ptz_move_is_short_always_stopped_audited_and_a_dry_run_moves_nothing(pool: PgPool) {
    let dir = temp_dir("ptz");
    let st = camera_state(pool, &dir);
    let (camera, onvif) = ptz_camera(&st, "Salon").await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let moves = |onvif: &FakeOnvif| {
        onvif
            .ptz_state()
            .log
            .iter()
            .filter(|l| l.starts_with("ContinuousMove"))
            .count()
    };

    let dry = ok!(
        client,
        "camera_ptz",
        json!({"camera": "salon", "direction": "left", "dry_run": true})
    );
    assert_eq!(
        (dry["dry_run"].as_bool(), dry["done"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(moves(&onvif), 0, "a dry run moves nothing");
    assert!(control_lines(&st).await.is_empty());

    let done = ok!(
        client,
        "camera_ptz",
        json!({"camera": camera.uuid.to_string(), "direction": "left", "duration_ms": 200, "speed": 0.5})
    );
    assert_eq!(done["done"], true, "{done}");
    let state = onvif.ptz_state();
    assert_eq!(moves(&onvif), 1);
    assert_eq!(
        state.log.last().map(String::as_str),
        Some("Stop"),
        "the move ends with a stop: {:?}",
        state.log
    );
    assert!(state.velocity.is_none(), "the camera is at rest");
    let lines = control_lines(&st).await;
    assert_eq!(
        (
            lines.len(),
            lines[0].tool.as_str(),
            lines[0].outcome.as_str()
        ),
        (1, "camera_ptz", "ok")
    );

    // Mistakes are refused before anything moves or is written.
    for (what, args) in [
        (
            "direction",
            json!({"camera": "Salon", "direction": "sideways"}),
        ),
        ("camera", json!({"camera": "Inconnue", "direction": "up"})),
    ] {
        assert!(call!(client, "camera_ptz", args).is_err(), "{what}");
    }
    let plain = save(&st, input("Sans-PTZ")).await;
    assert!(
        call!(
            client,
            "camera_ptz",
            json!({"camera": plain.id.0.to_string(), "direction": "up"})
        )
        .is_err()
    );
    assert_eq!(moves(&onvif), 1);
    assert_eq!(control_lines(&st).await.len(), 1);
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_night_mode_goes_through_the_configured_method_and_says_when_it_is_not_set(
    pool: PgPool,
) {
    let dir = temp_dir("night");
    let st = camera_state(pool, &dir);
    let fake = FakeThingino::start("cle-api").await;
    let mut i = input("Nuit");
    i.draft.night_mode_api = NightApi::Thingino;
    i.draft.web_port = i32::from(fake.port());
    i.web_api_key = SecretInput::Set("cle-api".into());
    save(&st, i).await;
    save(&st, input("Rien")).await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let dry = ok!(
        client,
        "camera_night",
        json!({"camera": "Nuit", "action": "daynight", "dry_run": true})
    );
    assert_eq!(dry["dry_run"], true);
    assert!(fake.bodies().is_empty());
    let done = ok!(
        client,
        "camera_night",
        json!({"camera": "Nuit", "action": "daynight"})
    );
    assert_eq!(done["done"], true, "{done}");
    assert_eq!(done["details"]["value"], "night");
    assert_eq!(fake.bodies().len(), 1);
    // A camera with no night method: the tool answers (not done) and the line says error.
    let none = ok!(
        client,
        "camera_night",
        json!({"camera": "Rien", "action": "ircut"})
    );
    assert_eq!(none["done"], false);
    let lines = control_lines(&st).await;
    let outcomes: Vec<_> = lines.iter().map(|l| l.outcome.as_str()).collect();
    assert_eq!(outcomes, ["error", "ok"]);
    assert!(
        call!(
            client,
            "camera_night",
            json!({"camera": "Nuit", "action": "volume"})
        )
        .is_err()
    );
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn manual_recording_is_refused_with_a_reason_when_the_camera_is_not_in_manual_mode(
    pool: PgPool,
) {
    let dir = temp_dir("rec");
    let st = camera_state(pool, &dir);
    let mut i = input("Garage");
    i.draft.recording_mode = RecordingMode::Continuous;
    save(&st, i).await;
    let token = make_token(&st, "assistant", &["pilotage"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let dry = ok!(
        client,
        "camera_record",
        json!({"camera": "Garage", "on": true, "dry_run": true})
    );
    assert!(dry["summary"].as_str().unwrap().starts_with("Demarrerait"));
    let refused = ok!(
        client,
        "camera_record",
        json!({"camera": "Garage", "on": true})
    );
    assert_eq!(refused["done"], false);
    assert!(
        refused["summary"]
            .as_str()
            .unwrap()
            .contains("mode d'enregistrement manuel"),
        "{refused}"
    );
    assert_eq!(control_lines(&st).await[0].outcome, "error");

    // A picture cannot be taken without a recent image (the video role is not running here): said, not hidden.
    let shot = ok!(client, "camera_snapshot", json!({"camera": "Garage"}));
    assert_eq!(shot["done"], false);
    assert!(
        shot["summary"].as_str().unwrap().contains("no picture"),
        "{shot}"
    );
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_picture_is_listed_and_returned_as_an_image_within_a_size_limit(pool: PgPool) {
    let dir = temp_dir("snap");
    let st = camera_state(pool, &dir);
    let camera = save(&st, input("Entree")).await;
    let jpeg = b"\xFF\xD8\xFF\xE0 une image de test \xFF\xD9".to_vec();
    std::fs::write(dir.join("a.jpg"), &jpeg).unwrap();
    std::fs::write(dir.join("big.jpg"), vec![0u8; 2_100_000]).unwrap();
    let repo = st.store.snapshots();
    let small = repo
        .create(
            camera.id,
            Utc::now(),
            "a.jpg",
            i32::try_from(jpeg.len()).unwrap(),
        )
        .await
        .unwrap();
    let big = repo
        .create(camera.id, Utc::now(), "big.jpg", 2_100_000)
        .await
        .unwrap();
    let token = make_token(&st, "lecteur", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let listed = ok!(client, "snapshots_list", json!({"camera": "entree"}));
    assert_eq!(listed["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(listed["snapshots"][0]["camera"], "Entree");
    let today = dominet_server::snapshots::local_today(&st.config.timezone);
    assert_eq!(
        ok!(client, "snapshots_list", json!({"day": today}))["snapshots"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        ok!(client, "snapshots_list", json!({"day": "2001-01-01"}))["snapshots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(call!(client, "snapshots_list", json!({"day": "hier"})).is_err());

    let image = call!(client, "snapshot_get", json!({"snapshot": small.id.0})).unwrap();
    assert_ne!(image.is_error, Some(true));
    let text = serde_json::to_string(&image.content).unwrap();
    assert!(
        text.contains("image/jpeg") && text.contains("Entree"),
        "{text}"
    );
    use base64::Engine;
    assert!(text.contains(&base64::engine::general_purpose::STANDARD.encode(&jpeg)));
    // Too big, unknown: refused with a reason.
    assert!(call!(client, "snapshot_get", json!({"snapshot": big.id.0})).is_err());
    assert!(call!(client, "snapshot_get", json!({"snapshot": 999_999})).is_err());
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn recordings_events_and_the_energy_balance_are_read(pool: PgPool) {
    let dir = temp_dir("lists");
    let st = camera_state(pool, &dir);
    let camera = save(&st, input("Jardin")).await;
    st.store
        .recordings()
        .create(
            camera.id,
            &NewRecording {
                started_at: Utc::now(),
                segment_path: "jardin/seg1.mp4".into(),
                event_type: RecordingEvent::Motion,
                size_bytes: Some(1_048_576),
                duration_ms: Some(60_000),
            },
        )
        .await
        .unwrap();
    st.store
        .camera_events()
        .record_motion(camera.id, Utc::now(), 0.234)
        .await
        .unwrap();
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
    for (d, hp, hc, solar, pac) in [(14, 5000, 3000, 1200, 800), (15, 4000, 2000, 900, 700)] {
        st.store
            .energy()
            .upsert(&EnergyDay {
                day: day(d),
                linky_hp_wh: hp,
                linky_hc_wh: hc,
                solar_wh: solar,
                pac_wh: pac,
            })
            .await
            .unwrap();
    }
    let token = make_token(&st, "lecteur", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);

    let rec = ok!(client, "recordings_list", json!({}));
    assert_eq!(rec["recordings"][0]["camera"], "Jardin");
    assert_eq!(
        (
            rec["recordings"][0]["event"].as_str(),
            rec["recordings"][0]["duration_seconds"].as_f64()
        ),
        (Some("motion"), Some(60.0))
    );
    let ev = ok!(client, "detections_list", json!({"camera": "Jardin"}));
    assert_eq!(ev["events"][0]["kind"], "motion");
    assert_eq!(ev["events"][0]["motion_percent"], 23.4);
    assert!(call!(client, "detections_list", json!({"camera": "Inconnue"})).is_err());
    let e = ok!(client, "energy_summary", json!({"days": 30}));
    assert_eq!(e["days"].as_array().unwrap().len(), 2);
    assert_eq!(e["days"][0]["day"], "2026-11-15");
    assert_eq!(
        (
            e["total_linky_wh"].as_i64(),
            e["total_solar_wh"].as_i64(),
            e["total_heat_pump_wh"].as_i64()
        ),
        (Some(14_000), Some(2_100), Some(1_500))
    );
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
    let _: Value = json!(null);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_read_token_sees_the_picture_reads_but_none_of_the_camera_actions(pool: PgPool) {
    let dir = temp_dir("scopes");
    let st = camera_state(pool, &dir);
    let token = make_token(&st, "lecteur", &["lecture"]).await;
    let addr = serve(&st).await;
    let client = connect!(addr, token);
    let names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    for read in [
        "snapshots_list",
        "snapshot_get",
        "recordings_list",
        "detections_list",
        "energy_summary",
    ] {
        assert!(names.contains(&read.to_string()), "{read}");
    }
    for action in [
        "camera_ptz",
        "camera_night",
        "camera_record",
        "camera_snapshot",
    ] {
        assert!(!names.contains(&action.to_string()), "{action}");
    }
    let refused = call!(
        client,
        "camera_ptz",
        json!({"camera": "x", "direction": "up"})
    )
    .unwrap();
    assert_eq!(refused.is_error, Some(true));
    client.cancel().await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}
