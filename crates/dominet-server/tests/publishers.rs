// SPDX-License-Identifier: AGPL-3.0-or-later
//! Publishers end to end: **simulated** weather API and inverter -> publisher -> dev Mosquitto -> MQTT source -> device in the database.

mod common;

use std::time::Duration;

use axum::routing::get;
use axum::{Json, Router};
use common::state;
use dominet_domain::device::{DeviceType, MqttSource, WeatherStation};
use dominet_domain::repo::{DeviceFilter, DeviceRepo, MqttSourceRepo, WeatherRepo};
use dominet_integrations::solarmax::sim::{SimInverter, SimState};
use dominet_server::{mqtt, publishers};
use serde_json::json;
use sqlx::PgPool;

fn port() -> String {
    std::env::var("MQTT_TEST_PORT").unwrap_or_else(|_| "11883".into())
}

async fn until<F: std::future::Future<Output = bool>>(what: &str, mut check: impl FnMut() -> F) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !check().await {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("delai depasse : {what}"));
}

async fn start_weather_api() -> String {
    let app = Router::new().route(
        "/v1/forecast",
        get(|| async { Json(json!({"current": {"temperature_2m": 17.5, "relative_humidity_2m": 61, "wind_speed_10m": 9.0}})) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/forecast", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn simulated_weather_and_inverter_reach_devices_through_the_broker_and_the_sources(
    pool: PgPool,
) {
    let pid = std::process::id();
    let api = start_weather_api().await;
    let inverter = SimInverter::start(SimState::default()).await.unwrap();
    let (meteo_suffix, sun_prefix) = (format!("essai{pid}"), format!("sun{pid}"));
    let port = port();
    let inverter_port = inverter.addr.port().to_string();
    let sun_topic = format!("{sun_prefix}/inv/prod");
    let mut st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &format!("dominet-test-pub-{pid}")),
            ("OPEN_METEO_URL", &api),
            ("SOLARMAX_HOST", "127.0.0.1"),
            ("SOLARMAX_PORT", &inverter_port),
            ("SOLARMAX_TOPIC", &sun_topic),
            ("SOLARMAX_INTERVAL_SECS", "1"),
        ],
    );
    // The sources that turn the messages into devices (like `dominet seed-weather` and the production's SolarMax source).
    let mut meteo = MqttSource::draft("Météo", "meteo/#");
    meteo.base_topic = "meteo".into();
    meteo.auto_device = true;
    let mut sun = MqttSource::draft("Soleil", &format!("{sun_prefix}/#"));
    sun.base_topic = sun_prefix.clone();
    sun.auto_device = true;
    st.store.mqtt_sources().create(&meteo).await.unwrap();
    st.store.mqtt_sources().create(&sun).await.unwrap();
    let mut station = WeatherStation::draft("Maison", 43.3, 5.4);
    station.mqtt_topic_suffix = meteo_suffix.clone();
    station.refresh_minutes = 1;
    st.store.weather().create(&station).await.unwrap();

    let client = mqtt::spawn(&st).await.unwrap();
    st.mqtt = Some(client.clone());
    publishers::spawn(&st, &client);

    let devices = st.store.devices();
    let weather_name = meteo_suffix.clone();
    until("appareil meteo", || async {
        devices.find_by_name(&weather_name).await.unwrap().is_some()
    })
    .await;
    let weather = devices.find_by_name(&meteo_suffix).await.unwrap().unwrap();
    assert_eq!(
        (weather.base_topic.as_str(), weather.device_type),
        ("meteo", DeviceType::Sensor)
    );
    assert_eq!(weather.state["temperature"], 17.5);
    assert_eq!(weather.state["uv_index"], serde_json::Value::Null);

    until("appareil onduleur", || async {
        devices.find_by_name("inv/prod").await.unwrap().is_some()
    })
    .await;
    let sun_device = devices.find_by_name("inv/prod").await.unwrap().unwrap();
    assert_eq!(sun_device.base_topic, sun_prefix);
    assert_eq!(sun_device.state["pac"], 632.5);
    assert_eq!(sun_device.state["status"], "Operating at MPP");
    assert!(inverter.connections() >= 1);
    client.shutdown();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_a_station_or_an_inverter_host_nothing_is_requested(pool: PgPool) {
    // Unreachable API (closed port) and no station: the weather publisher tries nothing; no SOLARMAX_HOST: no inverter.
    let pid = std::process::id();
    let port = port();
    let mut st = state(
        pool,
        &[
            ("MQTT_HOST", "127.0.0.1"),
            ("MQTT_PORT", &port),
            ("MQTT_CLIENT_ID", &format!("dominet-test-idle-{pid}")),
            ("OPEN_METEO_URL", "http://127.0.0.1:1/v1/forecast"),
        ],
    );
    assert!(st.config.solarmax.is_none());
    let client = mqtt::spawn(&st).await.unwrap();
    st.mqtt = Some(client.clone());
    publishers::spawn(&st, &client);
    tokio::time::sleep(Duration::from_millis(800)).await;
    // The dev broker may keep retained Zigbee2MQTT messages: we only look at what the publishers would have created.
    let all = st
        .store
        .devices()
        .list(&DeviceFilter::default())
        .await
        .unwrap();
    assert!(
        all.iter()
            .all(|d| d.base_topic != "meteo" && !d.friendly_name.contains("prod")),
        "{all:?}"
    );
    client.shutdown();
}
