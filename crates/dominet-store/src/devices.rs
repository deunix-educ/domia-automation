// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repositories of devices, groups, labels, state history and images (R3, ADR 0025).

use std::str::FromStr;

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::device::{
    Device, DeviceDraft, DeviceGroup, DeviceGroupDraft, DeviceGroupId, DeviceId, DeviceImage,
    DeviceLabel, DeviceLabelDraft, DeviceLabelId, DeviceType, Discovered, LogEntry, LogPoint,
};
use dominet_domain::location::LocationId;
use dominet_domain::repo::{
    DeviceFilter, DeviceGroupRepo, DeviceImageRepo, DeviceLabelRepo, DeviceRepo, RepoResult,
    StateLogRepo,
};
use serde_json::Value;

use crate::pg::{corrupt, repo_err};

// Mirror of a table row: the flags are independent.
#[allow(clippy::struct_excessive_bools)]
struct DeviceRow {
    id: i64,
    friendly_name: String,
    ieee: String,
    vendor: String,
    model_id: String,
    description: String,
    device_type: String,
    group_id: Option<i64>,
    location_id: Option<i64>,
    base_topic: String,
    state: Value,
    last_seen: Option<DateTime<Utc>>,
    available: bool,
    exposes: Value,
    active: bool,
    icon: String,
    image_url: String,
    tracked_keys: Vec<String>,
    hidden_keys: Vec<String>,
    grafana_dashboard: bool,
    /// True if the row was just inserted (`xmax = 0`); `false` for reads.
    created: bool,
}

impl DeviceRow {
    fn into_device(self) -> RepoResult<(Device, bool)> {
        let created = self.created;
        let device_type =
            DeviceType::from_str(&self.device_type).map_err(|_| corrupt("device type"))?;
        let device = Device {
            id: DeviceId(self.id),
            friendly_name: self.friendly_name,
            ieee: self.ieee,
            vendor: self.vendor,
            model_id: self.model_id,
            description: self.description,
            device_type,
            group: self.group_id.map(DeviceGroupId),
            location: self.location_id.map(LocationId),
            base_topic: self.base_topic,
            state: self.state,
            last_seen: self.last_seen,
            available: self.available,
            exposes: match self.exposes {
                Value::Array(a) => a,
                _ => vec![],
            },
            active: self.active,
            icon: self.icon,
            image_url: self.image_url,
            tracked_keys: self.tracked_keys,
            hidden_keys: self.hidden_keys,
            grafana_dashboard: self.grafana_dashboard,
        };
        Ok((device, created))
    }

    fn device(self) -> RepoResult<Device> {
        self.into_device().map(|(d, _)| d)
    }
}

fn devices(rows: Vec<DeviceRow>) -> RepoResult<Vec<Device>> {
    rows.into_iter().map(DeviceRow::device).collect()
}

/// Acces PostgreSQL aux appareils (implemente `DeviceRepo`).
#[derive(Debug, Clone)]
pub struct PgDevices {
    pool: sqlx::PgPool,
}

impl PgDevices {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl DeviceRepo for PgDevices {
    async fn list(&self, filter: &DeviceFilter) -> RepoResult<Vec<Device>> {
        let rows = sqlx::query_as!(
            DeviceRow,
            "SELECT id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\" FROM device
             WHERE ($1::bigint IS NULL OR group_id = $1) AND ($2::bigint IS NULL OR location_id = $2)
               AND ($3::text IS NULL OR device_type = $3) AND (NOT $4 OR active)
             ORDER BY friendly_name, id",
            filter.group.map(|g| g.0),
            filter.location.map(|l| l.0),
            filter.device_type.map(DeviceType::as_str),
            filter.active_only
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        devices(rows)
    }

    async fn get(&self, id: DeviceId) -> RepoResult<Option<Device>> {
        let row = sqlx::query_as!(DeviceRow, "SELECT id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\" FROM device WHERE id = $1", id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.map(DeviceRow::device).transpose()
    }

    async fn find_by_name(&self, friendly_name: &str) -> RepoResult<Option<Device>> {
        let row = sqlx::query_as!(
            DeviceRow,
            "SELECT id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\" FROM device WHERE friendly_name = $1",
            friendly_name
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(DeviceRow::device).transpose()
    }

    async fn create(&self, d: &DeviceDraft) -> RepoResult<Device> {
        let row = sqlx::query_as!(
            DeviceRow,
            "INSERT INTO device (friendly_name, description, device_type, group_id, location_id, base_topic, active, icon,
                                 image_url, tracked_keys, hidden_keys, grafana_dashboard)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             RETURNING id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\"",
            d.friendly_name,
            d.description,
            d.device_type.as_str(),
            d.group.map(|g| g.0),
            d.location.map(|l| l.0),
            d.base_topic,
            d.active,
            d.icon,
            d.image_url,
            &d.tracked_keys,
            &d.hidden_keys,
            d.grafana_dashboard
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.device()
    }

    async fn update(&self, id: DeviceId, d: &DeviceDraft) -> RepoResult<Device> {
        let row = sqlx::query_as!(
            DeviceRow,
            "UPDATE device SET friendly_name = $2, description = $3, device_type = $4, group_id = $5, location_id = $6,
                               base_topic = $7, active = $8, icon = $9, image_url = $10, tracked_keys = $11,
                               hidden_keys = $12, grafana_dashboard = $13
             WHERE id = $1
             RETURNING id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\"",
            id.0,
            d.friendly_name,
            d.description,
            d.device_type.as_str(),
            d.group.map(|g| g.0),
            d.location.map(|l| l.0),
            d.base_topic,
            d.active,
            d.icon,
            d.image_url,
            &d.tracked_keys,
            &d.hidden_keys,
            d.grafana_dashboard
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.ok_or(RepoError::NotFound)?.device()
    }

    async fn delete(&self, id: DeviceId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM device WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn upsert_discovered(
        &self,
        d: &Discovered,
        base_topic: &str,
    ) -> RepoResult<(Device, bool)> {
        let row = sqlx::query_as!(
            DeviceRow,
            "INSERT INTO device (friendly_name, ieee, vendor, model_id, description, exposes, device_type, base_topic, hidden_keys)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, ARRAY['update'])
             ON CONFLICT (friendly_name) DO UPDATE
                SET ieee = EXCLUDED.ieee, vendor = EXCLUDED.vendor, model_id = EXCLUDED.model_id,
                    description = EXCLUDED.description, exposes = EXCLUDED.exposes
             RETURNING id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, (xmax = 0) AS \"created!\"",
            d.friendly_name,
            d.ieee,
            d.vendor,
            d.model_id,
            d.description,
            Value::Array(d.exposes.clone()),
            d.device_type().as_str(),
            base_topic
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.into_device()
    }

    async fn apply_state(&self, friendly_name: &str, state: &Value) -> RepoResult<Option<Device>> {
        let row = sqlx::query_as!(
            DeviceRow,
            "UPDATE device SET state = $2, last_seen = now(), available = TRUE
             WHERE friendly_name = $1 AND active
             RETURNING id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\"",
            friendly_name,
            state
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(DeviceRow::device).transpose()
    }

    async fn set_available(
        &self,
        friendly_name: &str,
        available: bool,
    ) -> RepoResult<Option<Device>> {
        let row = sqlx::query_as!(
            DeviceRow,
            "UPDATE device SET available = $2 WHERE friendly_name = $1 RETURNING id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, false AS \"created!\"",
            friendly_name,
            available
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(DeviceRow::device).transpose()
    }

    async fn upsert_auto(
        &self,
        friendly_name: &str,
        base_topic: &str,
        device_type: DeviceType,
        state: &Value,
    ) -> RepoResult<(Device, bool)> {
        let row = sqlx::query_as!(
            DeviceRow,
            "INSERT INTO device (friendly_name, base_topic, device_type, state, available, last_seen, active)
             VALUES ($1, $2, $3, $4, TRUE, now(), TRUE)
             ON CONFLICT (friendly_name) DO UPDATE
                SET state = EXCLUDED.state, available = TRUE, last_seen = now(), active = TRUE
             RETURNING id, friendly_name, ieee, vendor, model_id, description, device_type, group_id, location_id, base_topic, state, last_seen, available, exposes, active, icon, image_url, tracked_keys, hidden_keys, grafana_dashboard, (xmax = 0) AS \"created!\"",
            friendly_name,
            base_topic,
            device_type.as_str(),
            state
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.into_device()
    }

    async fn touch_by_topic(&self, topic: &str) -> RepoResult<Vec<DeviceId>> {
        let ids = sqlx::query_scalar!(
            "UPDATE device SET available = TRUE, last_seen = now()
             WHERE active AND (base_topic || '/' || friendly_name = $1 OR base_topic = $1)
             RETURNING id",
            topic
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(ids.into_iter().map(DeviceId).collect())
    }
}

// ------------------------------------------------------------------ groupes

struct GroupRow {
    id: i64,
    name: String,
    location_id: Option<i64>,
    description: String,
    icon: String,
}

impl From<GroupRow> for DeviceGroup {
    fn from(r: GroupRow) -> Self {
        Self {
            id: DeviceGroupId(r.id),
            name: r.name,
            location: r.location_id.map(LocationId),
            description: r.description,
            icon: r.icon,
        }
    }
}

/// Acces PostgreSQL aux groupes d'appareils.
#[derive(Debug, Clone)]
pub struct PgDeviceGroups {
    pool: sqlx::PgPool,
}

impl PgDeviceGroups {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl DeviceGroupRepo for PgDeviceGroups {
    async fn list(&self) -> RepoResult<Vec<DeviceGroup>> {
        let rows = sqlx::query_as!(
            GroupRow,
            "SELECT id, name, location_id, description, icon FROM device_group ORDER BY name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get(&self, id: DeviceGroupId) -> RepoResult<Option<DeviceGroup>> {
        let row = sqlx::query_as!(
            GroupRow,
            "SELECT id, name, location_id, description, icon FROM device_group WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn create(&self, d: &DeviceGroupDraft) -> RepoResult<DeviceGroup> {
        let row = sqlx::query_as!(
            GroupRow,
            "INSERT INTO device_group (name, location_id, description, icon) VALUES ($1, $2, $3, $4)
             RETURNING id, name, location_id, description, icon",
            d.name,
            d.location.map(|l| l.0),
            d.description,
            d.icon
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn update(&self, id: DeviceGroupId, d: &DeviceGroupDraft) -> RepoResult<DeviceGroup> {
        let row = sqlx::query_as!(
            GroupRow,
            "UPDATE device_group SET name = $2, location_id = $3, description = $4, icon = $5 WHERE id = $1
             RETURNING id, name, location_id, description, icon",
            id.0,
            d.name,
            d.location.map(|l| l.0),
            d.description,
            d.icon
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(Into::into).ok_or(RepoError::NotFound)
    }

    async fn delete(&self, id: DeviceGroupId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM device_group WHERE id = $1", id.0)
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

// ------------------------------------------------------------------ libelles

struct LabelRow {
    id: i64,
    key: String,
    name: String,
    icon_name: String,
    unit: String,
}

impl From<LabelRow> for DeviceLabel {
    fn from(r: LabelRow) -> Self {
        Self {
            id: DeviceLabelId(r.id),
            key: r.key,
            name: r.name,
            icon_name: r.icon_name,
            unit: r.unit,
        }
    }
}

/// PostgreSQL access to the state-key labels.
#[derive(Debug, Clone)]
pub struct PgDeviceLabels {
    pool: sqlx::PgPool,
}

impl PgDeviceLabels {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl DeviceLabelRepo for PgDeviceLabels {
    async fn list(&self) -> RepoResult<Vec<DeviceLabel>> {
        let rows = sqlx::query_as!(
            LabelRow,
            "SELECT id, key, name, icon_name, unit FROM device_label ORDER BY key"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get(&self, id: DeviceLabelId) -> RepoResult<Option<DeviceLabel>> {
        let row = sqlx::query_as!(
            LabelRow,
            "SELECT id, key, name, icon_name, unit FROM device_label WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(Into::into))
    }

    async fn create(&self, d: &DeviceLabelDraft) -> RepoResult<DeviceLabel> {
        let row = sqlx::query_as!(
            LabelRow,
            "INSERT INTO device_label (key, name, icon_name, unit) VALUES ($1, $2, $3, $4)
             RETURNING id, key, name, icon_name, unit",
            d.key,
            d.name,
            d.icon_name,
            d.unit
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.into())
    }

    async fn update(&self, id: DeviceLabelId, d: &DeviceLabelDraft) -> RepoResult<DeviceLabel> {
        let row = sqlx::query_as!(
            LabelRow,
            "UPDATE device_label SET key = $2, name = $3, icon_name = $4, unit = $5 WHERE id = $1
             RETURNING id, key, name, icon_name, unit",
            id.0,
            d.key,
            d.name,
            d.icon_name,
            d.unit
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(Into::into).ok_or(RepoError::NotFound)
    }

    async fn delete(&self, id: DeviceLabelId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM device_label WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn create_missing(&self, drafts: &[DeviceLabelDraft]) -> RepoResult<u64> {
        let keys: Vec<String> = drafts.iter().map(|d| d.key.clone()).collect();
        let names: Vec<String> = drafts.iter().map(|d| d.name.clone()).collect();
        let icons: Vec<String> = drafts.iter().map(|d| d.icon_name.clone()).collect();
        let units: Vec<String> = drafts.iter().map(|d| d.unit.clone()).collect();
        let done = sqlx::query!(
            "INSERT INTO device_label (key, name, icon_name, unit)
             SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[])
             ON CONFLICT (key) DO NOTHING",
            &keys,
            &names,
            &icons,
            &units
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected())
    }

    async fn state_keys(&self) -> RepoResult<Vec<String>> {
        let keys = sqlx::query_scalar!(
            r#"SELECT DISTINCT jsonb_object_keys(state) AS "key!" FROM device WHERE state <> '{}'::jsonb ORDER BY 1"#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(keys)
    }
}

// ------------------------------------------------------------------ historique

/// PostgreSQL access to the history of the tracked keys.
#[derive(Debug, Clone)]
pub struct PgStateLog {
    pool: sqlx::PgPool,
}

impl PgStateLog {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl StateLogRepo for PgStateLog {
    async fn record(
        &self,
        device: DeviceId,
        ts: DateTime<Utc>,
        entries: &[LogEntry],
    ) -> RepoResult<u64> {
        if entries.is_empty() {
            return Ok(0);
        }
        let keys: Vec<String> = entries.iter().map(|e| e.key.clone()).collect();
        let nums: Vec<f64> = entries.iter().map(|e| e.value_num.unwrap_or(0.0)).collect();
        let has_num: Vec<bool> = entries.iter().map(|e| e.value_num.is_some()).collect();
        let strs: Vec<String> = entries.iter().map(|e| e.value_str.clone()).collect();
        let done = sqlx::query!(
            "INSERT INTO device_state_log (device_id, key, ts, value_num, value_str)
             SELECT $1, t.k, $2, CASE WHEN t.h THEN t.n END, t.s
             FROM UNNEST($3::text[], $4::float8[], $5::bool[], $6::text[]) AS t(k, n, h, s)
             ON CONFLICT (device_id, key, ts) DO UPDATE
                SET value_num = EXCLUDED.value_num, value_str = EXCLUDED.value_str",
            device.0,
            ts,
            &keys,
            &nums,
            &has_num,
            &strs
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected())
    }

    async fn min_max(
        &self,
        device: DeviceId,
        key: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> RepoResult<(Option<f64>, Option<f64>)> {
        let row = sqlx::query!(
            "SELECT MIN(value_num) AS lo, MAX(value_num) AS hi FROM device_state_log
             WHERE device_id = $1 AND key = $2 AND ts >= $3 AND ts < $4",
            device.0,
            key,
            from,
            to
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok((row.lo, row.hi))
    }

    async fn points(
        &self,
        device: DeviceId,
        key: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> RepoResult<Vec<LogPoint>> {
        let rows = sqlx::query!(
            "SELECT ts, value_num, value_str FROM device_state_log
             WHERE device_id = $1 AND key = $2 AND ts >= $3 ORDER BY ts LIMIT $4",
            device.0,
            key,
            since,
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| LogPoint {
                ts: r.ts,
                value_num: r.value_num,
                value_str: r.value_str,
            })
            .collect())
    }

    async fn purge_before(&self, before: DateTime<Utc>) -> RepoResult<u64> {
        let done = sqlx::query!("DELETE FROM device_state_log WHERE ts < $1", before)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(done.rows_affected())
    }
}

// ------------------------------------------------------------------ images

/// Acces PostgreSQL aux images d'appareils.
#[derive(Debug, Clone)]
pub struct PgDeviceImages {
    pool: sqlx::PgPool,
}

impl PgDeviceImages {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl DeviceImageRepo for PgDeviceImages {
    async fn put(&self, device: DeviceId, content_type: &str, data: &[u8]) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO device_image (device_id, content_type, data) VALUES ($1, $2, $3)
             ON CONFLICT (device_id) DO UPDATE SET content_type = EXCLUDED.content_type, data = EXCLUDED.data, updated_at = now()",
            device.0,
            content_type,
            data
        )
        .execute(&self.pool)
        .await
        // Foreign key: the device does not exist.
        .map_err(|e| match repo_err(e) {
            RepoError::Conflict { .. } => RepoError::NotFound,
            other => other,
        })?;
        Ok(())
    }

    async fn get(&self, device: DeviceId) -> RepoResult<Option<DeviceImage>> {
        let row = sqlx::query!(
            "SELECT content_type, data, updated_at FROM device_image WHERE device_id = $1",
            device.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(|r| DeviceImage {
            content_type: r.content_type,
            data: r.data,
            updated_at: r.updated_at,
        }))
    }

    async fn delete(&self, device: DeviceId) -> RepoResult<()> {
        sqlx::query!("DELETE FROM device_image WHERE device_id = $1", device.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn with_images(&self) -> RepoResult<Vec<(DeviceId, DateTime<Utc>)>> {
        let rows =
            sqlx::query!("SELECT device_id, updated_at FROM device_image ORDER BY device_id")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| (DeviceId(r.device_id), r.updated_at))
            .collect())
    }
}
