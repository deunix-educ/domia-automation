// SPDX-License-Identifier: AGPL-3.0-or-later
//! Open-Meteo: querying a station and normalising the response (like the reference).
//!
//! The API address is a parameter (`OPEN_METEO_URL`): the tests and the CI talk to a **simulated** API; only the demonstration
//! (with consent) makes a real call.

use std::time::Duration;

use dominet_domain::device::WeatherStation;
use serde_json::{Value, json};

/// Public API, no identifier.
pub const DEFAULT_URL: &str = "https://api.open-meteo.com/v1/forecast";
/// Requested variables (`current`).
const CURRENT_VARS: &str = "temperature_2m,relative_humidity_2m,apparent_temperature,precipitation,wind_speed_10m,wind_direction_10m,weather_code,uv_index,surface_pressure,cloud_cover";
/// Maximum duration of a request (like the reference).
const TIMEOUT: Duration = Duration::from_secs(15);
/// A normal response is under 2 KB.
const MAX_BODY: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WeatherError {
    #[error("client HTTP impossible : {0}")]
    Client(String),
    #[error("requete echouee : {0}")]
    Http(String),
    #[error("reponse HTTP {0}")]
    Status(u16),
    #[error("reponse trop grande")]
    TooLarge,
    #[error("reponse illisible : {0}")]
    Json(String),
}

/// Request parameters of a station.
#[must_use]
pub fn request_params(station: &WeatherStation) -> Vec<(&'static str, String)> {
    vec![
        ("latitude", station.latitude.to_string()),
        ("longitude", station.longitude.to_string()),
        ("elevation", station.altitude.to_string()),
        ("current", CURRENT_VARS.to_string()),
        ("timezone", station.timezone.clone()),
        ("wind_speed_unit", "kmh".to_string()),
    ]
}

/// The `current` block of the response -> normalised object (missing keys = `null`, like the reference).
#[must_use]
pub fn normalize_current(response: &Value) -> Value {
    let raw = response.get("current").unwrap_or(&Value::Null);
    let pick = |key: &str| raw.get(key).cloned().unwrap_or(Value::Null);
    json!({
        "temperature": pick("temperature_2m"),
        "humidity": pick("relative_humidity_2m"),
        "feels_like": pick("apparent_temperature"),
        "precipitation": pick("precipitation"),
        "wind_speed": pick("wind_speed_10m"),
        "wind_direction": pick("wind_direction_10m"),
        "weather_code": pick("weather_code"),
        "uv_index": pick("uv_index"),
        "pressure": pick("surface_pressure"),
        "cloud_cover": pick("cloud_cover"),
    })
}

/// Open-Meteo client.
#[derive(Debug, Clone)]
pub struct OpenMeteo {
    client: reqwest::Client,
    url: String,
}

impl OpenMeteo {
    /// # Errors
    /// The HTTP client cannot be built (for example without root certificates).
    pub fn new(url: &str) -> Result<Self, WeatherError> {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("dominet/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| WeatherError::Client(e.to_string()))?;
        Ok(Self {
            client,
            url: url.to_string(),
        })
    }

    /// Current reading of the station, normalised.
    ///
    /// # Errors
    /// Network, HTTP status, response too large or unreadable.
    pub async fn fetch(&self, station: &WeatherStation) -> Result<Value, WeatherError> {
        let response = self
            .client
            .get(&self.url)
            .query(&request_params(station))
            .send()
            .await
            .map_err(|e| WeatherError::Http(e.to_string()))?;
        if !response.status().is_success() {
            return Err(WeatherError::Status(response.status().as_u16()));
        }
        let body = response
            .bytes()
            .await
            .map_err(|e| WeatherError::Http(e.to_string()))?;
        if body.len() > MAX_BODY {
            return Err(WeatherError::TooLarge);
        }
        let parsed: Value =
            serde_json::from_slice(&body).map_err(|e| WeatherError::Json(e.to_string()))?;
        Ok(normalize_current(&parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_carries_the_station_and_the_reference_parameters() {
        let mut s = WeatherStation::draft("Maison", 43.3, 5.4);
        s.altitude = 120;
        s.timezone = "Europe/Paris".into();
        let params = request_params(&s);
        let get = |k: &str| {
            params
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("latitude"), Some("43.3"));
        assert_eq!(get("longitude"), Some("5.4"));
        assert_eq!(get("elevation"), Some("120"));
        assert_eq!(get("timezone"), Some("Europe/Paris"));
        assert_eq!(get("wind_speed_unit"), Some("kmh"));
        assert!(get("current").unwrap().contains("surface_pressure"));
    }

    #[test]
    fn the_current_block_is_renamed_and_missing_values_become_null() {
        let out = normalize_current(&json!({"current": {
            "temperature_2m": 13.4, "relative_humidity_2m": 68, "apparent_temperature": 11.0,
            "wind_speed_10m": 12.5, "weather_code": 3, "surface_pressure": 1015.2
        }}));
        assert_eq!(out["temperature"], 13.4);
        assert_eq!(out["humidity"], 68);
        assert_eq!(out["feels_like"], 11.0);
        assert_eq!(out["pressure"], 1015.2);
        assert_eq!(out["precipitation"], Value::Null);
        assert_eq!(out["uv_index"], Value::Null);
        assert_eq!(out.as_object().unwrap().len(), 10);
        // Response without a `current` block: ten null values, like the reference.
        let empty = normalize_current(&json!({"error": true}));
        assert!(empty.as_object().unwrap().values().all(Value::is_null));
    }
}
