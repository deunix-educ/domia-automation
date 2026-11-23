// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repositories of the MQTT sources and weather stations (R3, ADR 0025).

use std::str::FromStr;

use dominet_domain::RepoError;
use dominet_domain::device::{
    DeviceType, MqttSource, MqttSourceDraft, MqttSourceId, WeatherStation, WeatherStationDraft,
    WeatherStationId,
};
use dominet_domain::repo::{MqttSourceRepo, RepoResult, WeatherRepo};

use crate::pg::{corrupt, repo_err};

struct SourceRow {
    id: i64,
    name: String,
    topic_pattern: String,
    qos: i16,
    active: bool,
    log_messages: bool,
    auto_device: bool,
    auto_device_type: String,
    base_topic: String,
    handler_code: String,
    description: String,
}

impl SourceRow {
    fn source(self) -> RepoResult<MqttSource> {
        let auto_device_type = if self.auto_device_type.is_empty() {
            None
        } else {
            Some(
                DeviceType::from_str(&self.auto_device_type)
                    .map_err(|_| corrupt("device type of a source"))?,
            )
        };
        Ok(MqttSource {
            id: MqttSourceId(self.id),
            name: self.name,
            topic_pattern: self.topic_pattern,
            qos: u8::try_from(self.qos).map_err(|_| corrupt("QoS d'une source"))?,
            active: self.active,
            log_messages: self.log_messages,
            auto_device: self.auto_device,
            auto_device_type,
            base_topic: self.base_topic,
            handler_code: self.handler_code,
            description: self.description,
        })
    }
}

/// Acces PostgreSQL aux sources MQTT.
#[derive(Debug, Clone)]
pub struct PgMqttSources {
    pool: sqlx::PgPool,
}

impl PgMqttSources {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl MqttSourceRepo for PgMqttSources {
    async fn list(&self) -> RepoResult<Vec<MqttSource>> {
        let rows = sqlx::query_as!(
            SourceRow,
            "SELECT id, name, topic_pattern, qos, active, log_messages, auto_device, auto_device_type, base_topic,
                    handler_code, description FROM mqtt_source ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(SourceRow::source).collect()
    }

    async fn list_active(&self) -> RepoResult<Vec<MqttSource>> {
        let rows = sqlx::query_as!(
            SourceRow,
            "SELECT id, name, topic_pattern, qos, active, log_messages, auto_device, auto_device_type, base_topic,
                    handler_code, description FROM mqtt_source WHERE active ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(SourceRow::source).collect()
    }

    async fn get(&self, id: MqttSourceId) -> RepoResult<Option<MqttSource>> {
        let row = sqlx::query_as!(
            SourceRow,
            "SELECT id, name, topic_pattern, qos, active, log_messages, auto_device, auto_device_type, base_topic,
                    handler_code, description FROM mqtt_source WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(SourceRow::source).transpose()
    }

    async fn create(&self, d: &MqttSourceDraft) -> RepoResult<MqttSource> {
        let row = sqlx::query_as!(
            SourceRow,
            "INSERT INTO mqtt_source (name, topic_pattern, qos, active, log_messages, auto_device, auto_device_type,
                                      base_topic, handler_code, description)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             RETURNING id, name, topic_pattern, qos, active, log_messages, auto_device, auto_device_type, base_topic,
                       handler_code, description",
            d.name,
            d.topic_pattern,
            i16::from(d.qos),
            d.active,
            d.log_messages,
            d.auto_device,
            d.auto_device_type.map_or("", DeviceType::as_str),
            d.base_topic,
            d.handler_code,
            d.description
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.source()
    }

    async fn update(&self, id: MqttSourceId, d: &MqttSourceDraft) -> RepoResult<MqttSource> {
        let row = sqlx::query_as!(
            SourceRow,
            "UPDATE mqtt_source SET name = $2, topic_pattern = $3, qos = $4, active = $5, log_messages = $6,
                    auto_device = $7, auto_device_type = $8, base_topic = $9, handler_code = $10, description = $11
             WHERE id = $1
             RETURNING id, name, topic_pattern, qos, active, log_messages, auto_device, auto_device_type, base_topic,
                       handler_code, description",
            id.0,
            d.name,
            d.topic_pattern,
            i16::from(d.qos),
            d.active,
            d.log_messages,
            d.auto_device,
            d.auto_device_type.map_or("", DeviceType::as_str),
            d.base_topic,
            d.handler_code,
            d.description
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.ok_or(RepoError::NotFound)?.source()
    }

    async fn delete(&self, id: MqttSourceId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM mqtt_source WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }
}

struct StationRow {
    id: i64,
    name: String,
    latitude: f64,
    longitude: f64,
    altitude: i32,
    timezone: String,
    mqtt_topic_suffix: String,
    refresh_minutes: i16,
    enabled: bool,
    description: String,
}

impl From<StationRow> for WeatherStation {
    fn from(r: StationRow) -> Self {
        Self {
            id: WeatherStationId(r.id),
            name: r.name,
            latitude: r.latitude,
            longitude: r.longitude,
            altitude: r.altitude,
            timezone: r.timezone,
            mqtt_topic_suffix: r.mqtt_topic_suffix,
            refresh_minutes: r.refresh_minutes,
            enabled: r.enabled,
            description: r.description,
        }
    }
}

/// Acces PostgreSQL aux stations meteo.
#[derive(Debug, Clone)]
pub struct PgWeather {
    pool: sqlx::PgPool,
}

impl PgWeather {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl WeatherRepo for PgWeather {
    async fn list(&self) -> RepoResult<Vec<WeatherStation>> {
        let rows = sqlx::query_as!(
            StationRow,
            "SELECT id, name, latitude, longitude, altitude, timezone, mqtt_topic_suffix, refresh_minutes, enabled,
                    description FROM weather_station ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_enabled(&self) -> RepoResult<Vec<WeatherStation>> {
        let rows = sqlx::query_as!(
            StationRow,
            "SELECT id, name, latitude, longitude, altitude, timezone, mqtt_topic_suffix, refresh_minutes, enabled,
                    description FROM weather_station WHERE enabled ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get(&self, id: WeatherStationId) -> RepoResult<Option<WeatherStation>> {
        let row = sqlx::query_as!(
            StationRow,
            "SELECT id, name, latitude, longitude, altitude, timezone, mqtt_topic_suffix, refresh_minutes, enabled,
                    description FROM weather_station WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn create(&self, d: &WeatherStationDraft) -> RepoResult<WeatherStation> {
        let row = sqlx::query_as!(
            StationRow,
            "INSERT INTO weather_station (name, latitude, longitude, altitude, timezone, mqtt_topic_suffix, refresh_minutes,
                                          enabled, description)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id, name, latitude, longitude, altitude, timezone, mqtt_topic_suffix, refresh_minutes, enabled,
                       description",
            d.name,
            d.latitude,
            d.longitude,
            d.altitude,
            d.timezone,
            d.mqtt_topic_suffix,
            d.refresh_minutes,
            d.enabled,
            d.description
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn update(
        &self,
        id: WeatherStationId,
        d: &WeatherStationDraft,
    ) -> RepoResult<WeatherStation> {
        let row = sqlx::query_as!(
            StationRow,
            "UPDATE weather_station SET name = $2, latitude = $3, longitude = $4, altitude = $5, timezone = $6,
                    mqtt_topic_suffix = $7, refresh_minutes = $8, enabled = $9, description = $10
             WHERE id = $1
             RETURNING id, name, latitude, longitude, altitude, timezone, mqtt_topic_suffix, refresh_minutes, enabled,
                       description",
            id.0,
            d.name,
            d.latitude,
            d.longitude,
            d.altitude,
            d.timezone,
            d.mqtt_topic_suffix,
            d.refresh_minutes,
            d.enabled,
            d.description
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(Into::into).ok_or(RepoError::NotFound)
    }

    async fn delete(&self, id: WeatherStationId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM weather_station WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }
}
