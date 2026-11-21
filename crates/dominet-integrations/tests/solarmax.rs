// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client SolarMax contre un onduleur **simule** local (jamais le vrai) : cycle complet, alarmes, silence, bruit, reprise.

use std::time::Duration;

use dominet_integrations::run::{Publisher, SolarMaxSettings, solarmax_cycle};
use dominet_integrations::solarmax::sim::{Behaviour, SimInverter, SimState};
use dominet_integrations::solarmax::{SolarMaxClient, SolarMaxError};
use std::sync::Mutex;

const LIMIT: Duration = Duration::from_millis(400);

fn client(sim: &SimInverter) -> SolarMaxClient {
    SolarMaxClient::new("127.0.0.1", sim.addr.port(), LIMIT)
}

#[tokio::test]
async fn a_cycle_reads_detects_and_derives_like_the_reference_publisher() {
    let sim = SimInverter::start(SimState::default()).await.unwrap();
    let mut c = client(&sim);
    let p = c.read_production(1).await.unwrap();
    // Valeurs de reference calculees avec l'arithmetique de la reference ().
    assert_eq!(
        (p.tmpr, p.pac, p.eac, p.pdc, p.edc, p.qdy, p.qt0),
        (45, 632.5, 31, 936.9, 67, 20.0, 8000)
    );
    assert_eq!(p.status, "Operating at MPP");
    // Detection une seule fois par connexion : une connexion, trois requetes par cycle apres la premiere.
    c.read_production(1).await.unwrap();
    assert_eq!(sim.connections(), 1);
}

#[tokio::test]
async fn an_alarm_is_an_error_and_names_the_causes() {
    let sim = SimInverter::start(SimState::default()).await.unwrap();
    sim.set("SAL", "A"); // bits 2 et 8
    let err = client(&sim).read_production(1).await.unwrap_err();
    let SolarMaxError::Alarm { errors, .. } = &err else {
        panic!("{err:?}")
    };
    assert!(
        errors.contains("insulation") && errors.contains("fuse"),
        "{errors}"
    );
}

#[tokio::test]
async fn an_unknown_model_or_inverter_is_not_detected() {
    let sim = SimInverter::start(SimState::default()).await.unwrap();
    sim.set("TYP", "1");
    assert!(matches!(
        client(&sim).read_production(1).await,
        Err(SolarMaxError::NotDetected(1))
    ));
    sim.set("TYP", "4E3E");
    // Un autre numero d'onduleur : le simulateur ne repond pas.
    assert!(matches!(
        client(&sim).read_production(2).await,
        Err(SolarMaxError::Timeout | SolarMaxError::NoAnswer(2))
    ));
}

#[tokio::test]
async fn silence_and_noise_are_errors_and_the_next_cycle_reconnects() {
    let sim = SimInverter::start(SimState::default()).await.unwrap();
    let mut c = client(&sim);
    sim.set_behaviour(Behaviour::Silent);
    assert!(matches!(
        c.read_production(1).await,
        Err(SolarMaxError::Timeout)
    ));
    sim.set_behaviour(Behaviour::Garbage);
    assert!(matches!(
        c.read_production(1).await,
        Err(SolarMaxError::Protocol(_))
    ));
    sim.set_behaviour(Behaviour::Normal);
    assert!(
        c.read_production(1).await.is_ok(),
        "reprise sans intervention"
    );
    assert!(
        sim.connections() >= 2,
        "la connexion a ete reprise : {}",
        sim.connections()
    );
}

#[tokio::test]
async fn an_unreachable_inverter_is_a_connection_error() {
    // Port ferme de la boucle locale : jamais une adresse du reseau.
    let mut c = SolarMaxClient::new("127.0.0.1", 1, LIMIT);
    assert!(matches!(
        c.read_production(1).await,
        Err(SolarMaxError::Connect(_) | SolarMaxError::Timeout)
    ));
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String)>>);

impl Publisher for &Recorder {
    fn publish(&self, topic: &str, payload: &str) -> Result<(), String> {
        self.0.lock().unwrap().push((topic.into(), payload.into()));
        Ok(())
    }
}

#[tokio::test]
async fn the_cycle_publishes_the_production_on_the_configured_topic() {
    let sim = SimInverter::start(SimState::default()).await.unwrap();
    let settings = SolarMaxSettings {
        host: "127.0.0.1".into(),
        port: sim.addr.port(),
        inverter: 1,
        topic: "solarmax/essai/prod".into(),
        interval: Duration::from_secs(10),
        timeout: LIMIT,
    };
    let out = Recorder::default();
    let mut c = SolarMaxClient::new(&settings.host, settings.port, LIMIT);
    solarmax_cycle(&mut c, &settings, &&out).await.unwrap();
    let sent = out.0.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "solarmax/essai/prod");
    let json: serde_json::Value = serde_json::from_str(&sent[0].1).unwrap();
    assert_eq!(
        (
            json["pac"].as_f64(),
            json["eac"].as_i64(),
            json["status"].as_str()
        ),
        (Some(632.5), Some(31), Some("Operating at MPP"))
    );
    assert_eq!(
        json.as_object().unwrap().len(),
        8,
        "memes cles que reference"
    );
}
