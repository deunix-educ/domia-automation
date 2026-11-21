// SPDX-License-Identifier: AGPL-3.0-or-later
//! Publisher meteo contre une API Open-Meteo **simulee** (serveur local) et la base jetable.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use dominet_domain::device::WeatherStation;
use dominet_domain::repo::WeatherRepo;
use dominet_integrations::run::{Publisher, supervise, weather_cycle, weather_pause};
use dominet_integrations::weather::OpenMeteo;
use dominet_store::PgStore;
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(Default)]
struct Api {
    requests: Mutex<Vec<BTreeMap<String, String>>>,
}

/// API simulee : latitude 0 -> erreur 500 ; sinon un releve dont la temperature vaut la latitude.
async fn forecast(
    State(api): State<Arc<Api>>,
    Query(q): Query<BTreeMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    api.requests.lock().unwrap().push(q.clone());
    let lat: f64 = q["latitude"].parse().unwrap();
    if lat == 0.0 {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": true})),
        );
    }
    (
        StatusCode::OK,
        Json(
            json!({"current": {"temperature_2m": lat, "relative_humidity_2m": 60, "wind_speed_10m": 11.5, "weather_code": 2}}),
        ),
    )
}

async fn start_api() -> (String, Arc<Api>) {
    let api = Arc::new(Api::default());
    let app = Router::new()
        .route("/v1/forecast", get(forecast))
        .with_state(Arc::clone(&api));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/forecast", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, api)
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

struct Shared(Arc<Recorder>);

impl Publisher for Shared {
    fn publish(&self, topic: &str, payload: &str) -> Result<(), String> {
        self.0
            .0
            .lock()
            .unwrap()
            .push((topic.into(), serde_json::from_str(payload).unwrap()));
        Ok(())
    }
}

fn station(name: &str, lat: f64, suffix: &str, refresh: i16, enabled: bool) -> WeatherStation {
    let mut s = WeatherStation::draft(name, lat, 5.4);
    s.mqtt_topic_suffix = suffix.into();
    s.refresh_minutes = refresh;
    s.enabled = enabled;
    s
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn each_enabled_station_is_published_on_its_topic_and_a_failing_one_does_not_stop_the_others(
    pool: PgPool,
) {
    let repo = PgStore::new(pool).weather();
    repo.create(&station("Maison", 43.3, "meteo_locale", 10, true))
        .await
        .unwrap();
    repo.create(&station("Cave", 0.0, "cave", 5, true))
        .await
        .unwrap();
    repo.create(&station("Eteinte", 44.0, "eteinte", 1, false))
        .await
        .unwrap();
    let (url, api) = start_api().await;
    let recorder = Arc::new(Recorder::default());
    let out = Shared(Arc::clone(&recorder));

    let report = weather_cycle(&repo, &OpenMeteo::new(&url).unwrap(), &out)
        .await
        .unwrap();
    assert_eq!(report.published, ["Maison"]);
    assert_eq!(report.failed, ["Cave"]);
    let sent = recorder.0.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "meteo/meteo_locale");
    assert_eq!(sent[0].1["temperature"], 43.3);
    assert_eq!(sent[0].1["wind_speed"], 11.5);
    assert_eq!(
        sent[0].1["uv_index"],
        Value::Null,
        "valeur absente de l'API : null"
    );
    // L'API recoit les parametres de la reference, et la station eteinte n'est pas interrogee.
    let requests = api.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["wind_speed_unit"], "kmh");
    assert_eq!(requests[0]["timezone"], "Europe/Paris");
    assert!(requests.iter().all(|r| r["latitude"] != "44"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_pause_is_the_shortest_refresh_of_enabled_stations_or_a_short_idle_poll(pool: PgPool) {
    let repo = PgStore::new(pool).weather();
    assert_eq!(
        weather_pause(&repo).await,
        Duration::from_secs(30),
        "aucune station : simple relecture de la base"
    );
    repo.create(&station("A", 1.0, "a", 10, true))
        .await
        .unwrap();
    repo.create(&station("B", 2.0, "b", 3, true)).await.unwrap();
    repo.create(&station("C", 3.0, "c", 1, false))
        .await
        .unwrap();
    assert_eq!(weather_pause(&repo).await, Duration::from_secs(180));
}

#[tokio::test]
async fn an_unreachable_api_is_an_error_not_a_publication() {
    let api = OpenMeteo::new("http://127.0.0.1:1/v1/forecast").unwrap();
    assert!(
        api.fetch(&WeatherStation::draft("x", 1.0, 1.0))
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn a_task_that_panics_is_restarted_and_one_that_ends_is_not() {
    let runs = Arc::new(Mutex::new(0u32));
    let counter = Arc::clone(&runs);
    supervise("essai", move || {
        let counter = Arc::clone(&counter);
        async move {
            let n = {
                let mut r = counter.lock().unwrap();
                *r += 1;
                *r
            };
            assert!(n >= 3, "panique volontaire {n}");
        }
    })
    .await;
    assert_eq!(*runs.lock().unwrap(), 3);
}
