// SPDX-License-Identifier: AGPL-3.0-or-later
//! Metriques Prometheus (ADR 0051) : un enregistreur global, un ecouteur `/metrics` **a part** (jamais sur le port public), des
//! compteurs poses aux endroits utiles (HTTP, WebSocket, MQTT, planificateur, video) et un collecteur periodique pour les jauges.
//!
//! Les noms suivent la convention Prometheus (`dominet_*`, unite en suffixe, `_total` pour les compteurs). Les etiquettes sont
//! **bornees** : route normalisee (jamais l'URL brute), statut, nom de tache, identifiant de camera. Aucun secret ni identifiant
//! de compte n'y figure.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

use crate::AppState;

/// Intervalle de rafraichissement des jauges (alignee sur le `scrape_interval` de Prometheus).
const COLLECT_EVERY: Duration = Duration::from_secs(15);
/// Seaux de la latence HTTP (secondes) : de la page legere au rendu lent.
const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Installe l'enregistreur global au premier appel (les suivants rendent la meme poignee). Les tests y passent aussi.
pub fn handle() -> &'static PrometheusHandle {
    HANDLE.get_or_init(|| {
        let recorder = PrometheusBuilder::new()
            .set_buckets_for_metric(
                Matcher::Full("dominet_http_request_duration_seconds".into()),
                LATENCY_BUCKETS,
            )
            .expect("valid latency buckets")
            .build_recorder();
        let handle = recorder.handle();
        // Un enregistreur deja installe (autre test, autre bibliotheque) ne doit pas faire echouer le serveur.
        if metrics::set_global_recorder(recorder).is_err() {
            tracing::warn!("metrics recorder already installed");
        }
        describe();
        handle
    })
}

/// Nature d'une metrique, pour sa description.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Counter,
    Gauge,
    Seconds,
}

/// Toutes les metriques de DomiNet : nom, nature, aide. Les tableaux Grafana et les regles Prometheus ne peuvent citer que ces noms
/// (test `grafana_assets`).
const METRICS: &[(&str, Kind, &str)] = &[
    (
        "dominet_http_requests_total",
        Kind::Counter,
        "HTTP requests handled, by method, route and status.",
    ),
    (
        "dominet_http_request_duration_seconds",
        Kind::Seconds,
        "HTTP request handling time.",
    ),
    (
        "dominet_http_requests_in_flight",
        Kind::Gauge,
        "HTTP requests being handled.",
    ),
    (
        "dominet_ws_connections",
        Kind::Gauge,
        "Open WebSocket connections.",
    ),
    (
        "dominet_mqtt_messages_received_total",
        Kind::Counter,
        "MQTT messages received from the broker.",
    ),
    (
        "dominet_mqtt_disconnections_total",
        Kind::Counter,
        "MQTT connection drops.",
    ),
    (
        "dominet_mqtt_connected",
        Kind::Gauge,
        "1 if the MQTT client is connected, 0 otherwise.",
    ),
    (
        "dominet_scheduler_tasks_total",
        Kind::Counter,
        "Scheduler tasks run, by task and result.",
    ),
    (
        "dominet_scheduler_task_duration_seconds",
        Kind::Seconds,
        "Scheduler task run time.",
    ),
    (
        "dominet_recordings_total",
        Kind::Counter,
        "Recording segments processed (indexed, archived, failed).",
    ),
    (
        "dominet_motion_events_total",
        Kind::Counter,
        "Motions detected, by camera.",
    ),
    (
        "dominet_detection_events_total",
        Kind::Counter,
        "Object detections recorded, by camera.",
    ),
    (
        "dominet_db_pool_connections",
        Kind::Gauge,
        "PostgreSQL pool connections (idle, in_use).",
    ),
    (
        "dominet_prometheus_alerts_firing",
        Kind::Gauge,
        "Firing alerts read from Prometheus.",
    ),
    (
        "dominet_backup_runs_total",
        Kind::Counter,
        "Backups run, by result (ok, error).",
    ),
    (
        "dominet_backup_last_success_timestamp_seconds",
        Kind::Gauge,
        "Time (Unix seconds) of the last successful backup.",
    ),
    (
        "dominet_backup_last_duration_seconds",
        Kind::Gauge,
        "Duration of the last backup.",
    ),
    (
        "dominet_backup_last_size_bytes",
        Kind::Gauge,
        "Size of the last backup, by kind (postgres, reductstore).",
    ),
    (
        "dominet_build_info",
        Kind::Gauge,
        "DomiNet version (value 1, version as a label).",
    ),
    (
        "dominet_process_start_time_seconds",
        Kind::Seconds,
        "Process start time (Unix seconds).",
    ),
];

/// Noms des metriques de DomiNet.
#[must_use]
pub fn names() -> Vec<&'static str> {
    METRICS.iter().map(|(n, _, _)| *n).collect()
}

fn describe() {
    use metrics::{Unit, describe_counter, describe_gauge, describe_histogram};
    for &(name, kind, help) in METRICS {
        match kind {
            Kind::Counter => describe_counter!(name, help),
            Kind::Gauge => describe_gauge!(name, help),
            Kind::Seconds => describe_histogram!(name, Unit::Seconds, help),
        }
    }
}

/// Route normalisee d'une requete : le modele de route quand il existe, sinon une famille fixe. Jamais l'URL brute (identifiants,
/// jetons) : cardinalite bornee et rien de sensible.
fn route_label(req: &Request) -> String {
    if let Some(m) = req.extensions().get::<MatchedPath>() {
        return m.as_str().to_string();
    }
    let path = req.uri().path();
    for (prefix, label) in [("/sf/", "/sf/*"), ("/pkg/", "/pkg/*"), ("/api/", "/api/*")] {
        if path.starts_with(prefix) {
            return label.into();
        }
    }
    "ui".into()
}

/// Fonctions serveur (`/sf/<point>`) : leurs adresses sont fixes (un point par fonction), donc sans danger pour la cardinalite
/// **quand le serveur les connait** (statut autre que 404) ; une adresse inventee reste dans la famille `/sf/*`.
fn refine_route(route: String, path: &str, status: u16) -> String {
    if route == "/sf/*" && status != 404 && path.len() <= 64 {
        return path.to_string();
    }
    route
}

/// Intergiciel : compte les requetes et mesure leur duree.
pub async fn track(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let route = route_label(&req);
    let path = req.uri().path().to_string();
    let gauge = metrics::gauge!("dominet_http_requests_in_flight");
    gauge.increment(1.0);
    let started = Instant::now();
    let response = next.run(req).await;
    gauge.decrement(1.0);
    let route = refine_route(route, &path, response.status().as_u16());
    let status = response.status().as_u16().to_string();
    metrics::histogram!("dominet_http_request_duration_seconds", "method" => method.clone(), "route" => route.clone())
        .record(started.elapsed().as_secs_f64());
    metrics::counter!("dominet_http_requests_total", "method" => method, "route" => route, "status" => status).increment(1);
    response
}

pub(crate) fn scheduler_task(task: &str, ok: bool, took: Duration) {
    metrics::counter!("dominet_scheduler_tasks_total", "task" => task.to_string(), "result" => if ok { "ok" } else { "error" }).increment(1);
    metrics::histogram!("dominet_scheduler_task_duration_seconds", "task" => task.to_string())
        .record(took.as_secs_f64());
}

pub(crate) fn backup_success_at(timestamp: f64) {
    metrics::gauge!("dominet_backup_last_success_timestamp_seconds").set(timestamp);
}

#[allow(
    clippy::cast_precision_loss,
    reason = "file sizes: rounding beyond 2^52 bytes does not matter"
)]
pub(crate) fn backup_finished(
    ok: bool,
    took: Duration,
    postgres_bytes: u64,
    reductstore_bytes: Option<u64>,
) {
    metrics::counter!("dominet_backup_runs_total", "result" => if ok { "ok" } else { "error" })
        .increment(1);
    metrics::gauge!("dominet_backup_last_duration_seconds").set(took.as_secs_f64());
    if ok {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        backup_success_at(now);
        metrics::gauge!("dominet_backup_last_size_bytes", "kind" => "postgres")
            .set(postgres_bytes as f64);
        if let Some(b) = reductstore_bytes {
            metrics::gauge!("dominet_backup_last_size_bytes", "kind" => "reductstore")
                .set(b as f64);
        }
    }
}

pub(crate) fn prometheus_alerts_firing(n: usize) {
    metrics::gauge!("dominet_prometheus_alerts_firing")
        .set(f64::from(u32::try_from(n).unwrap_or(u32::MAX)));
}

pub(crate) fn recordings(indexed: usize, archived: usize, failed: usize) {
    for (outcome, n) in [
        ("indexed", indexed),
        ("archived", archived),
        ("failed", failed),
    ] {
        if n > 0 {
            metrics::counter!("dominet_recordings_total", "outcome" => outcome).increment(n as u64);
        }
    }
}

pub(crate) fn motion(camera: &str) {
    metrics::counter!("dominet_motion_events_total", "camera" => camera.to_string()).increment(1);
}

pub(crate) fn detection(camera: &str) {
    metrics::counter!("dominet_detection_events_total", "camera" => camera.to_string())
        .increment(1);
}

/// Jauges lues a la demande (pool, MQTT) et version : mises a jour par le collecteur et juste avant chaque lecture.
fn refresh_gauges(state: &AppState) {
    let pool = &state.pool;
    let size = f64::from(pool.size());
    let idle = f64::from(u32::try_from(pool.num_idle()).unwrap_or(u32::MAX));
    metrics::gauge!("dominet_db_pool_connections", "state" => "idle").set(idle);
    metrics::gauge!("dominet_db_pool_connections", "state" => "in_use").set((size - idle).max(0.0));
    if let Some(mqtt) = &state.mqtt {
        let up = mqtt.status() == dominet_mqtt::Status::Connected;
        metrics::gauge!("dominet_mqtt_connected").set(if up { 1.0 } else { 0.0 });
    }
}

/// Routeur de l'ecouteur : `GET /metrics` (format texte Prometheus).
pub fn router(state: AppState) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let state = state.clone();
            async move {
                refresh_gauges(&state);
                (
                    StatusCode::OK,
                    [(
                        header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    handle().render(),
                )
                    .into_response()
            }
        }),
    )
}

/// Demarre l'ecouteur `/metrics` si `METRICS_ADDR` n'est pas `off`. Un port deja pris n'arrete pas le serveur : on le dit et on continue.
pub async fn spawn_listener(state: &AppState) {
    let _ = handle();
    metrics::gauge!("dominet_build_info", "version" => env!("CARGO_PKG_VERSION")).set(1.0);
    metrics::gauge!("dominet_process_start_time_seconds").set(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64()),
    );
    let Some(addr) = state.config.metrics_addr else {
        tracing::info!("metrics disabled (METRICS_ADDR=off)");
        return;
    };
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            tracing::info!(%addr, "Prometheus metrics on /metrics");
            serve(listener, state.clone());
        }
        Err(e) => {
            tracing::warn!(%addr, error = %e, "metrics: port unavailable, /metrics is not exposed");
        }
    }
}

/// Sert `/metrics` sur un ecouteur deja lie (aussi utilise par les tests).
pub fn serve(listener: tokio::net::TcpListener, state: AppState) {
    let app = router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(COLLECT_EVERY);
        loop {
            tick.tick().await;
            refresh_gauges(&state);
        }
    });
}
