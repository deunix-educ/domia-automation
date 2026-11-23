// SPDX-License-Identifier: AGPL-3.0-or-later
//! The four day / night methods against fake servers (R6 lot 5): no network contact outside the loopback.

use dominet_surveillance::night::{
    NightAction, NightBackend, NightControl, NightError, NightState, NightTarget,
};
use dominet_surveillance::testing::night::{FakeGoform, FakeOpenIpc, FakeThingino};
use dominet_surveillance::testing::onvif::{FakeOnvif, Family};
use serde_json::json;

fn target(port: u16) -> NightTarget {
    NightTarget {
        id: "0123456789abcdef0123456789abcdef".into(),
        host: "127.0.0.1".into(),
        web_port: port,
        web_user: String::new(),
        web_pass: String::new(),
        web_api_key: String::new(),
        onvif_port: port,
        onvif_user: "admin".into(),
        onvif_pass: "mot-de-passe".into(),
    }
}

#[tokio::test]
async fn openipc_maps_daynight_and_ircut_to_toggle_and_authenticates_when_it_has_a_user() {
    let fake = FakeOpenIpc::start(Some(("web", "secret"))).await;
    let mut t = target(fake.port());
    // Without an identifier: refused.
    let backend = NightBackend::OpenIpc(t.clone());
    assert!(matches!(
        backend.run(NightAction::Light).await,
        Err(NightError::Denied)
    ));
    t.web_user = "web".into();
    t.web_pass = "secret".into();
    let backend = NightBackend::OpenIpc(t);
    for action in [
        NightAction::DayNight,
        NightAction::IrCut,
        NightAction::Light,
    ] {
        let out = backend.run(action).await.unwrap();
        assert_eq!(out.to_json()["ok"], true);
        assert_eq!(out.to_json()["action"], action.as_str());
        assert_eq!(out.to_json()["body"], "ok");
    }
    // /night/ircut does not move the motor on these boards: everything that is not the light goes through `toggle`.
    assert_eq!(fake.paths(), ["toggle", "toggle", "light"]);
}

#[tokio::test]
async fn thingino_toggles_from_the_remembered_state_with_the_api_key() {
    let fake = FakeThingino::start("cle-api").await;
    let mut t = target(fake.port());
    t.web_api_key = "cle-api".into();
    let backend = NightBackend::Thingino(t.clone(), NightState::default());
    // The reference's default states: daynight "day", ircut 1, light 0; each toggle sends the opposite.
    assert_eq!(
        backend.run(NightAction::DayNight).await.unwrap().to_json()["value"],
        "night"
    );
    assert_eq!(
        backend.run(NightAction::DayNight).await.unwrap().to_json()["value"],
        "day"
    );
    assert_eq!(
        backend.run(NightAction::IrCut).await.unwrap().to_json()["value"],
        0
    );
    assert_eq!(
        backend.run(NightAction::Light).await.unwrap().to_json()["value"],
        1
    );
    assert_eq!(
        backend.run(NightAction::Light).await.unwrap().to_json()["value"],
        0
    );
    assert_eq!(
        fake.bodies(),
        [
            json!({"cmd": "daynight", "val": "night"}),
            json!({"cmd": "daynight", "val": "day"}),
            json!({"cmd": "ircut", "val": 0}),
            json!({"cmd": "white", "val": 1}),
            json!({"cmd": "white", "val": 0}),
        ]
    );
    // A wrong key is refused, and the remembered state does not advance.
    let state = NightState::default();
    let mut bad = t;
    bad.web_api_key = "autre".into();
    let backend = NightBackend::Thingino(bad.clone(), state.clone());
    assert!(matches!(
        backend.run(NightAction::DayNight).await,
        Err(NightError::Denied)
    ));
    bad.web_api_key = "cle-api".into();
    assert_eq!(
        NightBackend::Thingino(bad, state)
            .run(NightAction::DayNight)
            .await
            .unwrap()
            .to_json()["value"],
        "night",
        "l'echec n'a pas fait avancer la bascule"
    );
}

#[tokio::test]
async fn onvif_toggles_the_ir_cut_and_the_brightness_through_the_imaging_service() {
    let fake = FakeOnvif::start(Family::Thingino, "admin", "mot-de-passe").await;
    let backend = NightBackend::Onvif(target(fake.port()));
    let out = backend.run(NightAction::IrCut).await.unwrap().to_json();
    assert_eq!(
        (out["mode"].as_str(), out["was"].as_str()),
        (Some("NIGHT"), Some("AUTO"))
    );
    let out = backend.run(NightAction::DayNight).await.unwrap().to_json();
    assert_eq!(
        (out["mode"].as_str(), out["was"].as_str()),
        (Some("DAY"), Some("NIGHT"))
    );
    let out = backend.run(NightAction::Light).await.unwrap().to_json();
    assert_eq!(out["brightness"], 80.0);
    // Wrong credentials: refused, nothing changes.
    let mut bad = target(fake.port());
    bad.onvif_pass = "faux".into();
    assert!(matches!(
        NightBackend::Onvif(bad).run(NightAction::IrCut).await,
        Err(NightError::Onvif(_))
    ));
    assert_eq!(fake.imaging_value("IrCutFilter").as_deref(), Some("DAY"));
}

#[tokio::test]
async fn goform_answers_the_digest_challenge_and_changes_only_the_field_it_means_to() {
    let fake = FakeGoform::start("admin", "mot-de-passe").await;
    let mut t = target(fake.port());
    t.web_user = "admin".into();
    t.web_pass = "mot-de-passe".into();
    let backend = NightBackend::Goform(t.clone());

    // Day / night: 1 (auto) -> 2 (night) -> 1 (auto).
    let out = backend.run(NightAction::DayNight).await.unwrap().to_json();
    assert_eq!(
        (out["mode"].as_i64(), out["was"].as_i64()),
        (Some(2), Some(1))
    );
    assert_eq!(fake.day_night(), 2);
    let out = backend.run(NightAction::IrCut).await.unwrap().to_json();
    assert_eq!(
        (out["mode"].as_i64(), out["was"].as_i64()),
        (Some(1), Some(2))
    );
    // The rest of the object is sent back as is.
    let (path, body) = &fake.posts()[0];
    assert_eq!(path, "/goform/frmVideoParaEx");
    assert_eq!(body["Type"], 1);
    assert_eq!(body["Data"]["DayNightMode"]["Sensitivity"], 3);
    assert_eq!(body["Data"]["Mirror"], 0);

    // Light: 2 (auto) -> 1 (on) -> 2; `ModeSupportList` is not sent back.
    assert_eq!(
        backend.run(NightAction::Light).await.unwrap().to_json()["control"],
        1
    );
    assert_eq!(fake.light(), 1);
    assert_eq!(
        backend.run(NightAction::Light).await.unwrap().to_json()["control"],
        2
    );
    let light_body = fake
        .posts()
        .into_iter()
        .rev()
        .find(|(p, _)| p.ends_with("frmIotLightCfg"))
        .unwrap()
        .1;
    assert!(light_body["Data"].get("ModeSupportList").is_none());
    assert_eq!(light_body["Data"]["Brightness"], 60);
    // Every useful request was preceded by a challenge.
    assert!(fake.refused() >= 8, "{}", fake.refused());

    // A wrong password: refused (the challenge is replayed, the response stays wrong), nothing changes.
    t.web_pass = "faux".into();
    let before = fake.day_night();
    assert!(matches!(
        NightBackend::Goform(t).run(NightAction::DayNight).await,
        Err(NightError::Denied)
    ));
    assert_eq!(fake.day_night(), before);
}

#[tokio::test]
async fn goform_uses_the_stream_credentials_when_there_are_no_web_ones() {
    let fake = FakeGoform::start("admin", "flux").await;
    let mut t = target(fake.port());
    t.onvif_user = "admin".into();
    t.onvif_pass = "flux".into();
    assert!(
        NightBackend::Goform(t)
            .run(NightAction::DayNight)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn an_unreachable_camera_is_an_error_not_a_hang_and_never_shows_a_secret() {
    let mut t = target(1);
    t.web_user = "u".into();
    t.web_pass = "motdepasse-secret".into();
    t.web_api_key = "cle-secrete".into();
    for backend in [
        NightBackend::OpenIpc(t.clone()),
        NightBackend::Thingino(t.clone(), NightState::default()),
        NightBackend::Goform(t.clone()),
    ] {
        let err = backend.run(NightAction::DayNight).await.unwrap_err();
        let shown = format!("{err} {err:?}");
        assert!(matches!(err, NightError::Http(_)), "{shown}");
        assert!(!shown.contains("secret"), "{shown}");
    }
}
