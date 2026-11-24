// SPDX-License-Identifier: AGPL-3.0-or-later
//! MediaMTX client against the fake MediaMTX (R6 lot 2): no contact with a real server.

use std::collections::BTreeMap;

use dominet_domain::camera::CameraDraft;
use dominet_surveillance::testing::FakeMediaMtx;
use dominet_surveillance::{MediaMtx, MediaMtxError, PathConf};
use serde_json::json;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn conf(ip: &str, tcp: bool) -> PathConf {
    let mut d = CameraDraft::new("c");
    d.ip = ip.into();
    d.rtsp_user = "admin".into();
    d.rtsp_transport_tcp = tcp;
    PathConf::for_camera(&d, "mot-de-passe-secret")
}

#[tokio::test]
async fn replace_creates_then_replaces_and_the_replacement_is_total() {
    let fake = FakeMediaMtx::start().await;
    let api = MediaMtx::new(&fake.url()).unwrap();
    api.replace(A, &conf("10.0.0.1", true)).await.unwrap();
    assert_eq!(fake.path(A).unwrap()["sourceProtocol"], "tcp");
    api.replace(A, &conf("10.0.0.2", false)).await.unwrap();
    let now = fake.path(A).unwrap();
    assert!(now["source"].as_str().unwrap().contains("10.0.0.2"));
    assert!(
        now.get("sourceProtocol").is_none(),
        "l'option retiree disparait"
    );
    assert!(api.get(A).await.unwrap().is_some());
    assert!(api.get(B).await.unwrap().is_none());
}

#[tokio::test]
async fn sync_adds_replaces_removes_ours_and_leaves_the_others_alone() {
    let fake = FakeMediaMtx::start().await;
    fake.insert("autre-usage", json!({"source": "publisher"}));
    fake.insert(B, json!({"source": "rtsp://vieille"}));
    let api = MediaMtx::new(&fake.url()).unwrap();
    api.add(A, &conf("10.0.0.1", false)).await.unwrap();

    let wanted = BTreeMap::from([(A.to_string(), conf("10.0.0.9", false))]);
    let report = api.sync(&wanted).await.unwrap();
    assert_eq!(report.replaced, [A]);
    assert_eq!(
        report.removed,
        [B],
        "une camera qui n'est plus voulue disparait"
    );
    assert!(report.added.is_empty() && report.is_clean());
    let names = fake.names();
    assert!(names.contains(&"~.*".to_string()) && names.contains(&"autre-usage".to_string()));
    assert!(!names.contains(&B.to_string()));

    let more = BTreeMap::from([
        (A.to_string(), conf("10.0.0.9", false)),
        (B.to_string(), conf("10.0.0.3", false)),
    ]);
    let report = api.sync(&more).await.unwrap();
    assert_eq!(report.added, [B]);
}

#[tokio::test]
async fn removing_an_absent_path_is_fine_and_an_outage_is_reported() {
    let fake = FakeMediaMtx::start().await;
    let api = MediaMtx::new(&fake.url()).unwrap();
    api.remove(A).await.unwrap();
    fake.fail_with(Some(500));
    let err = api.replace(A, &conf("10.0.0.1", false)).await.unwrap_err();
    assert!(matches!(err, MediaMtxError::Status { status: 500, .. }));
    assert!(api.sync(&BTreeMap::new()).await.is_err());
    let down = MediaMtx::new("http://127.0.0.1:1").unwrap();
    assert!(matches!(
        down.list_names().await,
        Err(MediaMtxError::Unreachable(_))
    ));
}

#[tokio::test]
async fn errors_and_debug_never_carry_the_camera_password() {
    let fake = FakeMediaMtx::start().await;
    let api = MediaMtx::new(&fake.url()).unwrap();
    api.add(A, &conf("10.0.0.1", false)).await.unwrap();
    // A second `add` fails; the message must not contain the password, even if the server had repeated it.
    let err = api.add(A, &conf("10.0.0.1", false)).await.unwrap_err();
    assert!(!err.to_string().contains("mot-de-passe-secret"), "{err}");
    assert!(!format!("{:?}", conf("10.0.0.1", false)).contains("mot-de-passe-secret"));
}

#[tokio::test]
async fn the_list_is_read_page_by_page() {
    let fake = FakeMediaMtx::start().await;
    for i in 0..7 {
        fake.insert(&format!("{i:032x}"), json!({"source": "x"}));
    }
    let api = MediaMtx::new(&fake.url()).unwrap().with_page_size(3);
    assert_eq!(api.list_names().await.unwrap().len(), 8);
}

#[tokio::test]
async fn set_record_changes_only_that_field_and_refuses_an_unknown_path() {
    let fake = FakeMediaMtx::start().await;
    let api = MediaMtx::new(&fake.url()).unwrap();
    api.replace(A, &conf("10.0.0.1", true)).await.unwrap();
    api.set_record(A, true).await.unwrap();
    let now = fake.path(A).unwrap();
    assert_eq!(now["record"], true);
    assert!(
        now["source"].as_str().unwrap().contains("10.0.0.1"),
        "le reste de la configuration est intact"
    );
    api.set_record(A, false).await.unwrap();
    assert_eq!(fake.path(A).unwrap()["record"], false);
    assert!(api.set_record(B, true).await.unwrap_err().is_not_found());
}

#[tokio::test]
async fn kicking_a_user_closes_their_sessions_only() {
    let fake = FakeMediaMtx::start().await;
    fake.add_session("rtspsessions", "r1", "alice");
    fake.add_session("rtspsessions", "r2", "bob");
    fake.add_session("rtspsessions", "r3", "");
    fake.add_session("webrtcsessions", "w1", "Alice");
    fake.add_session("webrtcsessions", "w2", "");
    let api = MediaMtx::new(&fake.url()).unwrap();
    // The user name is compared without case, like the accounts; sessions without a user name (a viewing token) are never matched.
    assert_eq!(api.kick_user("ALICE").await.unwrap(), 2);
    assert_eq!(fake.kicked(), ["r1", "w1"]);
    assert_eq!(api.kick_user("alice").await.unwrap(), 0);
    assert_eq!(api.kick_user("").await.unwrap(), 0);
    fake.fail_with(Some(500));
    assert!(api.kick_user("bob").await.is_err());
}
