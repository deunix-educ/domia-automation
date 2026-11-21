// SPDX-License-Identifier: AGPL-3.0-or-later
//! Le client contre le **vrai MediaMTX de la stack de dev** (`make dev-up`, API sur 127.0.0.1:19997). Ignore par defaut : la CI
//! reste autonome. A lancer a la main : `cargo test -p dominet-surveillance --test real_mediamtx -- --ignored`.
//! Un path de test (nom hexadecimal fixe) est cree puis supprime ; la source pointe vers un port ferme, aucun flux n'est lu.

use std::collections::BTreeMap;

use dominet_domain::camera::CameraDraft;
use dominet_surveillance::{MediaMtx, PathConf};

const NAME: &str = "deadbeefdeadbeefdeadbeefdeadbeef";

fn conf(path: &str, tcp: bool) -> PathConf {
    let mut d = CameraDraft::new("essai");
    d.ip = "127.0.0.1".into();
    d.port = 1;
    d.path = path.into();
    d.rtsp_user = "u".into();
    d.rtsp_transport_tcp = tcp;
    PathConf::for_camera(&d, "mot-de-passe-secret")
}

#[tokio::test]
#[ignore = "demande le MediaMTX de la stack de dev"]
async fn the_client_speaks_the_real_mediamtx_api() {
    let url = std::env::var("MEDIAMTX_DEV_API").unwrap_or_else(|_| "http://127.0.0.1:19997".into());
    let api = MediaMtx::new(&url).unwrap();
    api.remove(NAME).await.unwrap();

    let report = api
        .sync(&BTreeMap::from([(NAME.to_string(), conf("a", true))]))
        .await
        .unwrap();
    assert_eq!(report.added, [NAME]);
    let got = api.get(NAME).await.unwrap().expect("path cree");
    assert_eq!(got["sourceProtocol"], "tcp");
    assert_eq!(got["source"], "rtsp://u:mot-de-passe-secret@127.0.0.1:1/a");

    api.replace(NAME, &conf("b", false)).await.unwrap();
    let got = api.get(NAME).await.unwrap().unwrap();
    assert!(
        got["sourceProtocol"].is_null() || got["sourceProtocol"] == "automatic",
        "l'option retiree revient a sa valeur par defaut"
    );
    assert!(got["source"].as_str().unwrap().ends_with("/b"));

    let report = api.sync(&BTreeMap::new()).await.unwrap();
    assert_eq!(report.removed, [NAME]);
    assert!(
        api.list_names().await.unwrap().contains(&"~.*".to_string()),
        "les autres paths restent"
    );
    assert!(api.get(NAME).await.unwrap().is_none());
}
