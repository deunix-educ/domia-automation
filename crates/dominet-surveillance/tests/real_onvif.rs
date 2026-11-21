// SPDX-License-Identifier: AGPL-3.0-or-later
//! Le client contre les **vraies cameras de test** du reseau local (lecture seule : identite, capacites, profils, URL de flux masquee).
//! Ignore par defaut : demande `deploy/.env.cameras` (git-ignore, jamais affiche). Les mots de passe ne sont ni imprimes ni journalises.
//! A lancer a la main : `cargo test -p dominet-surveillance --test real_onvif -- --ignored --nocapture`.

use dominet_surveillance::onvif::{Credentials, ONVIF_PORTS, OnvifClient, probe_port};

/// `CAM_<NOM>_HOST` + identifiants lus dans `CAM_<NOM>_HTTP` / `_RTSP` (`schema://utilisateur:motdepasse@hote`).
/// (nom, hote, couples identifiant / mot de passe)
type Camera = (String, String, Vec<(String, String)>);

fn cameras() -> Vec<Camera> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../deploy/.env.cameras");
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let kv: Vec<(&str, &str)> = content
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .collect();
    let get = |k: &str| kv.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
    kv.iter()
        .filter_map(|(k, host)| {
            let name = k.strip_prefix("CAM_")?.strip_suffix("_HOST")?;
            let mut creds = vec![];
            for key in [format!("CAM_{name}_HTTP"), format!("CAM_{name}_RTSP")] {
                let Some(rest) = get(&key).and_then(|u| u.split_once("://").map(|(_, r)| r)) else {
                    continue;
                };
                let Some((userinfo, _)) = rest.rsplit_once('@') else {
                    continue;
                };
                if let Some((user, pass)) = userinfo.split_once(':') {
                    let c = (user.to_string(), pass.to_string());
                    if !creds.contains(&c) {
                        creds.push(c);
                    }
                }
            }
            Some((name.to_string(), (*host).to_string(), creds))
        })
        .collect()
}

#[tokio::test]
#[ignore = "demande deploy/.env.cameras et les cameras de test du reseau local"]
async fn the_client_understands_the_real_test_cameras() {
    let cams = cameras();
    assert!(
        !cams.is_empty(),
        "deploy/.env.cameras absent ou vide (modele : deploy/.env.example.cameras)"
    );
    let mut ok = 0;
    for (name, host, creds) in cams {
        let Some(port) = probe_port(&host, &ONVIF_PORTS).await else {
            println!("{name} : pas de service ONVIF (normal pour une OpenIPC)");
            continue;
        };
        let mut done = false;
        for (user, pass) in &creds {
            let client = OnvifClient::new(
                &host,
                port,
                Some(Credentials {
                    user: user.clone(),
                    pass: pass.clone(),
                }),
            )
            .unwrap();
            if let Ok(r) = client.inspect().await {
                println!(
                    "{name} : {} {} fw {} · profil {} · {} profil(s) · ptz={} · flux {}",
                    r.info.manufacturer,
                    r.info.model,
                    r.info.firmware,
                    r.main_token,
                    r.profiles.len(),
                    r.ptz,
                    r.stream_uri
                );
                assert!(
                    !r.stream_uri.contains(pass.as_str()),
                    "le flux affiche ne doit pas contenir le mot de passe"
                );
                done = true;
                ok += 1;
                break;
            }
        }
        if !done {
            println!("{name} : service ONVIF sur le port {port} mais aucun identifiant accepte");
        }
    }
    assert!(ok > 0, "aucune camera n'a repondu");
}
