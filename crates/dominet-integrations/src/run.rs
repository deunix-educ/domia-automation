// SPDX-License-Identifier: AGPL-3.0-or-later
//! Boucles des publishers (meteo, SolarMax) et supervision (R3 lot 6, ADR 0030).
//!
//! Chaque cycle lit une source externe et **publie sur le broker** ; l'ingestion (sources MQTT du lot 4) fait le reste. Les boucles ne
//! connaissent ni le serveur ni le client MQTT : elles publient par le trait [`Publisher`].

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dominet_domain::repo::WeatherRepo;
use tokio::sync::Notify;

use crate::solarmax::SolarMaxClient;
use crate::weather::OpenMeteo;

/// Publication d'un message (QoS 1, sans retenue) ; refusee si le lien avec le broker est coupe.
pub trait Publisher: Send + Sync {
    /// # Errors
    /// Texte de l'erreur (lien coupe, topic invalide...).
    fn publish(&self, topic: &str, payload: &str) -> Result<(), String>;
}

/// Pause quand aucune station n'est activee : une simple relecture de la base (aucune requete a l'API), pour qu'une station creee par
/// un autre processus (`dominet seed-weather`) soit prise en compte rapidement. reference attendait 10 minutes.
const IDLE_PAUSE: Duration = Duration::from_secs(30);

/// Resultat d'un cycle meteo.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WeatherReport {
    /// Stations dont le releve a ete publie.
    pub published: Vec<String>,
    /// Stations en echec (API injoignable, publication refusee).
    pub failed: Vec<String>,
}

/// Un cycle : toutes les stations activees, chacune sur son topic `meteo/<suffixe>`. Une station en echec n'arrete pas les autres.
///
/// # Errors
/// Lecture des stations impossible.
pub async fn weather_cycle<R: WeatherRepo + Sync, P: Publisher>(
    repo: &R,
    api: &OpenMeteo,
    out: &P,
) -> Result<WeatherReport, dominet_domain::RepoError> {
    let mut report = WeatherReport::default();
    for station in repo.list_enabled().await? {
        let outcome = match api.fetch(&station).await {
            Ok(data) => out.publish(&station.mqtt_topic(), &data.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match outcome {
            Ok(()) => report.published.push(station.name),
            Err(e) => {
                tracing::warn!(station = %station.name, error = %e, "weather not published");
                report.failed.push(station.name);
            }
        }
    }
    Ok(report)
}

/// Duree avant le prochain cycle : le plus court `refresh_minutes` des stations activees, sinon [`IDLE_PAUSE`] (30 s).
pub async fn weather_pause<R: WeatherRepo + Sync>(repo: &R) -> Duration {
    repo.list_enabled()
        .await
        .ok()
        .and_then(|s| {
            s.iter()
                .map(|st| st.refresh_minutes)
                .filter(|m| *m > 0)
                .min()
        })
        .map_or(IDLE_PAUSE, |m| {
            Duration::from_secs(u64::from(m.unsigned_abs()) * 60)
        })
}

/// Boucle meteo : un cycle, puis attente jusqu'au prochain cycle ou jusqu'a un reveil (`wake` : station creee ou modifiee).
pub async fn run_weather<R: WeatherRepo + Sync, P: Publisher>(
    repo: R,
    api: OpenMeteo,
    out: P,
    wake: Arc<Notify>,
) -> ! {
    loop {
        match weather_cycle(&repo, &api, &out).await {
            Ok(report) if report.published.is_empty() && report.failed.is_empty() => {
                tracing::debug!("no active weather station");
            }
            Ok(report) => tracing::info!(
                publiees = report.published.len(),
                en_echec = report.failed.len(),
                "cycle meteo"
            ),
            Err(e) => tracing::warn!(error = %e, "unreadable weather stations"),
        }
        let pause = weather_pause(&repo).await;
        tokio::select! {
            () = tokio::time::sleep(pause) => {}
            () = wake.notified() => {}
        }
    }
}

/// Reglages du publisher SolarMax.
#[derive(Debug, Clone)]
pub struct SolarMaxSettings {
    pub host: String,
    pub port: u16,
    pub inverter: u8,
    pub topic: String,
    pub interval: Duration,
    /// Delai d'une operation reseau.
    pub timeout: Duration,
}

/// Un cycle SolarMax : lit, publie. Renvoie le resume publie.
///
/// # Errors
/// Texte de l'erreur (onduleur, publication).
pub async fn solarmax_cycle<P: Publisher>(
    client: &mut SolarMaxClient,
    settings: &SolarMaxSettings,
    out: &P,
) -> Result<serde_json::Value, String> {
    let production = client
        .read_production(settings.inverter)
        .await
        .map_err(|e| e.to_string())?;
    let json = production.to_json();
    out.publish(&settings.topic, &json.to_string())?;
    Ok(json)
}

/// Boucle SolarMax (toutes les `interval`). Les echecs sont journalises une fois, la reprise aussi (pas une ligne par cycle).
pub async fn run_solarmax<P: Publisher>(settings: SolarMaxSettings, out: P) -> ! {
    let mut client = SolarMaxClient::new(&settings.host, settings.port, settings.timeout);
    let mut failing = false;
    loop {
        match solarmax_cycle(&mut client, &settings, &out).await {
            Ok(json) => {
                if failing {
                    tracing::info!("inverter reachable again");
                    failing = false;
                }
                tracing::debug!(pac = %json["pac"], status = %json["status"], "production published");
            }
            Err(e) => {
                if !failing {
                    tracing::warn!(error = %e, "inverter unreadable or unreachable, retrying at the next cycle");
                    failing = true;
                }
            }
        }
        tokio::time::sleep(settings.interval).await;
    }
}

/// Relance une tache qui s'arrete sur une panique (jamais quand elle se termine normalement), avec une pause croissante de 1 a 60 s.
pub async fn supervise<F, Fut>(name: &'static str, make: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut pause = Duration::from_secs(1);
    loop {
        let started = tokio::time::Instant::now();
        match tokio::spawn(make()).await {
            Ok(()) => return,
            Err(e) => {
                tracing::error!(task = name, error = %e, "task stopped, restarting in {} s", pause.as_secs());
            }
        }
        // Une tache restee en vie longtemps repart d'une pause courte.
        if started.elapsed() > Duration::from_secs(300) {
            pause = Duration::from_secs(1);
        }
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(Duration::from_secs(60));
    }
}
