// SPDX-License-Identifier: AGPL-3.0-or-later
//! ONVIF client against the fake server that replays the responses of the four camera families (R6 lot 3) and checks the
//! WS-Security digest like a real camera. No network contact outside the loopback.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use dominet_surveillance::onvif::discovery::{Discovered, discover_at};
use dominet_surveillance::onvif::{Credentials, OnvifClient, OnvifError, probe_port};
use dominet_surveillance::testing::discovery::FakeResponders;
use dominet_surveillance::testing::onvif::{FakeOnvif, Family};

const USER: &str = "admin";
const PASS: &str = "mot-de-passe-secret";

fn creds(pass: &str) -> Credentials {
    Credentials {
        user: USER.into(),
        pass: pass.into(),
    }
}

async fn client(family: Family) -> (FakeOnvif, OnvifClient) {
    let fake = FakeOnvif::start(family, USER, PASS).await;
    let c = OnvifClient::new("127.0.0.1", fake.port(), Some(creds(PASS))).unwrap();
    (fake, c)
}

#[tokio::test]
async fn each_camera_family_is_understood() {
    for (family, model, main, profiles, ptz, services) in [
        (
            Family::Thingino,
            "Q3R",
            "profile_1",
            2,
            true,
            "/onvif/media_service",
        ),
        (
            Family::Vatilon,
            "PB1",
            "IPCProfilesToken0",
            2,
            true,
            "/onvif/Media",
        ),
        (
            Family::Xiaomi,
            "Xiaomi",
            "profile_1",
            1,
            false,
            "/onvif/media_service",
        ),
        (
            Family::Nvt,
            "IPC_NT98566",
            "Profile_1",
            2,
            true,
            "/onvif/Media",
        ),
    ] {
        let (fake, c) = client(family).await;
        let r = c
            .inspect()
            .await
            .unwrap_or_else(|e| panic!("{family:?} : {e}"));
        assert_eq!(r.info.model, model, "{family:?}");
        assert_eq!(r.main_token, main, "{family:?}");
        assert_eq!(r.profiles.len(), profiles, "{family:?}");
        assert_eq!(r.ptz, ptz, "{family:?}");
        let s = c.services().await.unwrap();
        assert!(s.media.unwrap().ends_with(services), "{family:?}");
        assert_eq!(s.ptz.is_some(), ptz);
        assert_eq!(s.imaging.is_some(), ptz);
        assert!(
            fake.calls().iter().all(|c| !c.ends_with('!')),
            "aucun refus : {:?}",
            fake.calls()
        );
    }
}

#[tokio::test]
async fn profiles_carry_resolution_codec_and_ptz() {
    let (_f, c) = client(Family::Nvt).await;
    let media = c.services().await.unwrap().media.unwrap();
    let profiles = c.profiles(&media).await.unwrap();
    assert_eq!(
        (
            profiles[0].width,
            profiles[0].height,
            profiles[0].encoding.as_str(),
            profiles[0].has_ptz
        ),
        (4096, 2160, "H264", true)
    );
    assert_eq!((profiles[1].width, profiles[1].height), (704, 576));
    assert_eq!(profiles[0].source_token.as_deref(), Some("VideoSource_1"));
}

#[tokio::test]
async fn the_clock_of_the_camera_is_corrected_for_the_digest() {
    // Camera clock one hour ahead: a `Created` at our time would be refused.
    let (fake, c) = client(Family::Thingino).await;
    fake.set_clock_offset(3600);
    c.inspect().await.expect("la correction d'horloge suffit");
    // Camera behind.
    let (fake, c) = client(Family::Vatilon).await;
    fake.set_clock_offset(-7200);
    c.inspect().await.unwrap();
    // The digest is only checked after reading the time: the first request is without authentication.
    assert_eq!(fake.calls()[0], "GetSystemDateAndTime");
}

#[tokio::test]
async fn a_wrong_password_is_reported_as_not_authorized_in_both_refusal_styles() {
    let fake = FakeOnvif::start(Family::Thingino, USER, PASS).await;
    let bad = OnvifClient::new("127.0.0.1", fake.port(), Some(creds("autre"))).unwrap();
    assert!(matches!(
        bad.device_information().await,
        Err(OnvifError::NotAuthorized)
    ));
    fake.refuse_with_http_401();
    assert!(matches!(
        bad.device_information().await,
        Err(OnvifError::NotAuthorized)
    ));
    let anonymous = OnvifClient::new("127.0.0.1", fake.port(), None).unwrap();
    assert!(matches!(
        anonymous.device_information().await,
        Err(OnvifError::NotAuthorized)
    ));
    // The right response goes through.
    let good = OnvifClient::new("127.0.0.1", fake.port(), Some(creds(PASS))).unwrap();
    assert!(good.device_information().await.is_ok());
}

#[tokio::test]
async fn the_nvt_stream_url_never_shows_its_clear_text_password() {
    let (_f, c) = client(Family::Nvt).await;
    let media = c.services().await.unwrap().media.unwrap();
    let uri = c.stream_uri(&media, "Profile_1").await.unwrap();
    assert!(
        uri.expose().contains(PASS),
        "la camera le renvoie bien en clair"
    );
    for shown in [uri.masked(), format!("{uri:?}")] {
        assert!(!shown.contains(PASS), "{shown}");
        assert!(shown.contains("password=***"), "{shown}");
    }
    let r = c.inspect().await.unwrap();
    assert!(!r.stream_uri.contains(PASS) && !format!("{r:?}").contains(PASS));
    assert!(!format!("{:?}", creds(PASS)).contains(PASS));
}

#[tokio::test]
async fn services_always_use_the_registered_host_even_if_the_camera_announces_another() {
    let (fake, c) = client(Family::Thingino).await;
    fake.announce_host("10.99.99.99");
    let s = c.services().await.unwrap();
    let media = s.media.unwrap();
    assert!(
        media.starts_with(&format!("http://127.0.0.1:{}/", fake.port())),
        "{media}"
    );
    // And the next call succeeds: we were not sent to 10.99.99.99.
    assert!(c.profiles(&media).await.is_ok());
}

#[tokio::test]
async fn an_unreachable_camera_is_an_error_not_a_hang() {
    let c = OnvifClient::new("127.0.0.1", 1, Some(creds(PASS))).unwrap();
    assert!(matches!(
        c.sync_clock().await,
        Err(OnvifError::Unreachable(_))
    ));
}

#[tokio::test]
async fn the_onvif_port_is_found_among_the_candidates() {
    let fake = FakeOnvif::start(Family::Nvt, USER, PASS).await;
    // Closed ports are ignored; only the one that speaks ONVIF is kept.
    assert_eq!(
        probe_port("127.0.0.1", &[1, 2, fake.port(), 3]).await,
        Some(fake.port())
    );
    assert_eq!(probe_port("127.0.0.1", &[1, 2]).await, None);
    // An open port that does not speak ONVIF is not enough.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = silent.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((s, _)) = silent.accept().await {
            drop(s);
        }
    });
    assert_eq!(probe_port("127.0.0.1", &[port]).await, None);
}

#[tokio::test]
async fn discovery_finds_each_responder_once_even_if_it_answers_every_probe() {
    let responders = FakeResponders::start(vec![(
        "IPC-model".into(),
        "NVT%20IPC".into(),
        "http://192.168.1.29:8899/onvif/device_service".into(),
    )])
    .await;
    let mut found: Vec<Discovered> = vec![];
    discover_at(
        responders.addr(),
        None,
        3,
        Duration::from_millis(2500),
        |d| found.push(d),
    )
    .await
    .unwrap();
    assert_eq!(found.len(), 1, "3 sondes, 1 seule annonce : {found:?}");
    assert_eq!(found[0].ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(
        (
            found[0].port,
            found[0].name.as_str(),
            found[0].hardware.as_str()
        ),
        (8899, "IPC-model", "NVT IPC")
    );
}

#[tokio::test]
async fn discovery_without_any_answer_ends_empty_after_the_listening_time() {
    let silent = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let start = std::time::Instant::now();
    let mut n = 0;
    discover_at(
        silent.local_addr().unwrap(),
        None,
        2,
        Duration::from_millis(1200),
        |_| n += 1,
    )
    .await
    .unwrap();
    assert_eq!(n, 0);
    assert!(
        start.elapsed() >= Duration::from_millis(1200) && start.elapsed() < Duration::from_secs(3)
    );
}

// ------------------------------------------------------------------ PTZ and imaging (R6 lot 5)

use dominet_surveillance::onvif::imaging::IrCut;
use dominet_surveillance::onvif::ptz::PtzPosition;

#[tokio::test]
async fn continuous_moves_and_stop_reach_the_camera_with_only_the_axes_in_motion() {
    for family in [Family::Thingino, Family::Vatilon, Family::Nvt] {
        let (fake, c) = client(family).await;
        let ptz = c.ptz().await.unwrap();
        ptz.continuous_move(0.5, -0.25, 0.0).await.unwrap();
        assert_eq!(
            fake.ptz_state().velocity,
            Some((0.5, -0.25, 0.0)),
            "{family:?}"
        );
        ptz.stop().await.unwrap();
        assert_eq!(fake.ptz_state().velocity, None);
        ptz.continuous_move(0.0, 0.0, 1.0).await.unwrap();
        assert_eq!(fake.ptz_state().velocity, Some((0.0, 0.0, 1.0)));
        // Out-of-range values are brought back into -1..1; nothing ordered = a stop.
        ptz.continuous_move(9.0, -9.0, 0.0).await.unwrap();
        assert_eq!(fake.ptz_state().velocity, Some((1.0, -1.0, 0.0)));
        ptz.continuous_move(0.0, 0.0, 0.0).await.unwrap();
        assert_eq!(fake.ptz_state().velocity, None, "aucun axe : c'est un Stop");
    }
}

#[tokio::test]
async fn a_camera_without_ptz_says_so() {
    let (_f, c) = client(Family::Xiaomi).await;
    assert!(matches!(c.ptz().await, Err(OnvifError::NoService("PTZ"))));
    assert!(matches!(
        c.toggle_ir_cut().await,
        Err(OnvifError::NoService("Imaging"))
    ));
}

#[tokio::test]
async fn presets_are_set_listed_gone_to_and_removed() {
    let (fake, c) = client(Family::Thingino).await;
    let ptz = c.ptz().await.unwrap();
    assert_eq!(ptz.presets().await.unwrap().len(), 0);
    let token = ptz.set_preset("Porte").await.unwrap();
    assert_eq!(token, "preset_1");
    ptz.set_preset("Jardin <1>").await.unwrap();
    let list = ptz.presets().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0].token.as_str(), list[0].name.as_str()),
        ("preset_1", "Porte")
    );
    assert_eq!(
        list[1].name, "Jardin <1>",
        "le nom est echappe puis relu tel quel"
    );

    // GotoPreset waits for the movement to end (the camera answers MOVING twice).
    let before = fake.calls().iter().filter(|c| *c == "GetStatus").count();
    ptz.goto_preset("preset_1").await.unwrap();
    let polls = fake.calls().iter().filter(|c| *c == "GetStatus").count() - before;
    assert!(
        polls >= 3,
        "attente de la fin du mouvement : {polls} lectures"
    );
    assert!(
        fake.ptz_state()
            .log
            .contains(&"GotoPreset preset_1".to_string())
    );

    ptz.remove_preset("preset_1").await.unwrap();
    assert_eq!(ptz.presets().await.unwrap().len(), 1);
}

#[tokio::test]
async fn status_gives_the_position_and_the_space_except_on_the_nvt_which_reports_zero() {
    let (fake, c) = client(Family::Thingino).await;
    let ptz = c.ptz().await.unwrap();
    ptz.absolute_move(&PtzPosition {
        pan: 46.4,
        tilt: -3.0,
        zoom: 0.0,
        space: Some("http://www.onvif.org/ver10/tptz/PanTiltSpaces/SphericalPositionSpace".into()),
    })
    .await
    .unwrap();
    let st = ptz.status().await.unwrap();
    let pos = st.position.unwrap();
    assert!((pos.pan - 46.4).abs() < 1e-9 && (pos.tilt + 3.0).abs() < 1e-9);
    assert!(pos.space.unwrap().ends_with("SphericalPositionSpace"));
    assert!(st.idle);
    assert!(
        fake.ptz_state()
            .log
            .last()
            .unwrap()
            .ends_with("SphericalPositionSpace"),
        "l'espace est repris dans l'ordre"
    );

    let (_nvt, nvt) = client(Family::Nvt).await;
    let nvt_ptz = nvt.ptz().await.unwrap();
    nvt_ptz.continuous_move(1.0, 0.0, 0.0).await.unwrap();
    let pos = nvt_ptz.status().await.unwrap().position.unwrap();
    assert_eq!(
        (pos.pan, pos.tilt),
        (0.0, 0.0),
        "la NVT ne donne jamais sa position (ADR 0008)"
    );
}

#[tokio::test]
async fn the_ir_cut_toggles_like_reference_and_keeps_every_other_imaging_setting() {
    let (fake, c) = client(Family::Thingino).await;
    let others = |f: &FakeOnvif| {
        f.imaging()
            .into_iter()
            .filter(|(k, _)| k != "IrCutFilter")
            .collect::<Vec<_>>()
    };
    let before_others = others(&fake);
    assert_eq!(fake.imaging_value("IrCutFilter").as_deref(), Some("AUTO"));
    assert_eq!(
        c.toggle_ir_cut().await.unwrap(),
        (IrCut::Auto, IrCut::Night)
    );
    assert_eq!(fake.imaging_value("IrCutFilter").as_deref(), Some("NIGHT"));
    assert_eq!(c.toggle_ir_cut().await.unwrap(), (IrCut::Night, IrCut::Day));
    assert_eq!(c.toggle_ir_cut().await.unwrap(), (IrCut::Day, IrCut::Night));
    assert_eq!(
        others(&fake),
        before_others,
        "luminosite, nettete, contre-jour : intacts"
    );
    assert!(before_others.len() >= 3);
}

#[tokio::test]
async fn the_light_toggles_the_brightness_between_zero_and_eighty_like_reference() {
    let (fake, c) = client(Family::Vatilon).await;
    // 35 (< 40) -> 80; then 80 -> 0; then 0 (read as 50 by the reference) -> 0.
    assert!((c.toggle_light_brightness().await.unwrap() - 80.0).abs() < f64::EPSILON);
    assert_eq!(fake.imaging_value("Brightness").as_deref(), Some("80"));
    assert!(c.toggle_light_brightness().await.unwrap().abs() < f64::EPSILON);
    assert_eq!(
        fake.imaging_value("IrCutFilter").as_deref(),
        Some("AUTO"),
        "le filtre IR n'a pas bouge"
    );
}
