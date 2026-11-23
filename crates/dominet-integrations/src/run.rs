// SPDX-License-Identifier: AGPL-3.0-or-later
//! Publisher loops (weather, SolarMax) and supervision (R3 lot 6, ADR 0030).
//!
//! Each cycle reads an external source and **publishes on the broker**; the ingestion (MQTT sources of lot 4) does the rest. The loops don't
//! know the server or the MQTT client: they publish through the [`Publisher`] trait.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dominet_domain::repo::WeatherRepo;
use tokio::sync::Notify;

use crate::solarmax::SolarMaxClient;
use crate::weather::OpenMeteo;

/// Publication of a message (QoS 1, not retained); refused if the link with the broker is down.
pub trait Publisher: Send + Sync {
    /// # Errors
    /// Error text (link down, invalid topic...).
    fn publish(&self, topic: &str, payload: &str) -> Result<(), String>;
}

/// Pause when no station is enabled: a simple re-read of the database (no request to the API), so that a station created by
/// another process (`dominet seed-weather`) is picked up quickly. The reference waited 10 minutes.
const IDLE_PAUSE: Duration = Duration::from_secs(30);

/// Result of a weather cycle.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WeatherReport {
    /// Stations whose reading was published.
    pub published: Vec<String>,
    /// Stations that failed (API unreachable, publication refused).
    pub failed: Vec<String>,
}

/// One cycle: all the enabled stations, each on its `meteo/<suffix>` topic. A failing station does not stop the others.
///
/// # Errors
/// The stations cannot be read.
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

/// Time before the next cycle: the shortest `refresh_minutes` of the enabled stations, otherwise [`IDLE_PAUSE`] (30 s).
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

/// Weather loop: one cycle, then wait until the next cycle or until a wake-up (`wake`: station created or modified).
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

/// Settings of the SolarMax publisher.
#[derive(Debug, Clone)]
pub struct SolarMaxSettings {
    pub host: String,
    pub port: u16,
    pub inverter: u8,
    pub topic: String,
    pub interval: Duration,
    /// Timeout of a network operation.
    pub timeout: Duration,
}

/// One SolarMax cycle: reads, publishes. Returns the published summary.
///
/// # Errors
/// Error text (inverter, publication).
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

/// SolarMax loop (every `interval`). Failures are logged once, and so is the recovery (not one line per cycle).
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

/// Restarts a task that stops on a panic (never when it ends normally), with a pause growing from 1 to 60 s.
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
        // A task that stayed alive for a long time starts again with a short pause.
        if started.elapsed() > Duration::from_secs(300) {
            pause = Duration::from_secs(1);
        }
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(Duration::from_secs(60));
    }
}
