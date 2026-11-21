// SPDX-License-Identifier: AGPL-3.0-or-later
//! Les quatre methodes jour / nuit contre des faux serveurs (R6 lot 5) : aucun contact reseau hors boucle locale.

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
    // Sans identifiant : refuse.
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
    // /night/ircut ne bouge pas le moteur sur ces cartes : tout ce qui n'est pas la lumiere passe par `toggle`.
    assert_eq!(fake.paths(), ["toggle", "toggle", "light"]);
}

#[tokio::test]
async fn thingino_toggles_from_the_remembered_state_with_the_api_key() {
    let fake = FakeThingino::start("cle-api").await;
    let mut t = target(fake.port());
    t.web_api_key = "cle-api".into();
    let backend = NightBackend::Thingino(t.clone(), NightState::default());
    // Etats par defaut de la reference : daynight « day », ircut 1, lumiere 0 ; chaque bascule envoie l'inverse.
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
    // Une mauvaise cle est refusee, et l'etat memorise n'avance pas.
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
    // Mauvais identifiants : refuses, rien ne change.
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

    // Jour / nuit : 1 (auto) -> 2 (nuit) -> 1 (auto).
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
    // Le reste de l'objet est renvoye tel quel.
    let (path, body) = &fake.posts()[0];
    assert_eq!(path, "/goform/frmVideoParaEx");
    assert_eq!(body["Type"], 1);
    assert_eq!(body["Data"]["DayNightMode"]["Sensitivity"], 3);
    assert_eq!(body["Data"]["Mirror"], 0);

    // Lumiere : 2 (auto) -> 1 (allumee) -> 2 ; `ModeSupportList` n'est pas renvoye.
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
    // Chaque requete utile a ete precedee d'un defi.
    assert!(fake.refused() >= 8, "{}", fake.refused());

    // Un mauvais mot de passe : refus (le defi se rejoue, la reponse reste fausse), rien ne change.
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
