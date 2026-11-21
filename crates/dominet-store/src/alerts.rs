// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depots des regles d'alerte, des alertes et du bilan d'energie (R4, ADR 0031).

use chrono::{DateTime, NaiveDate, Utc};
use dominet_domain::RepoError;
use dominet_domain::alert::{
    Alert, AlertId, AlertRule, AlertRuleDraft, AlertRuleId, AlertStatus, Operator, Severity,
};
use dominet_domain::device::DeviceId;
use dominet_domain::energy::EnergyDay;
use dominet_domain::repo::{AlertRepo, AlertRuleRepo, EnergyRepo, NewAlert, RepoResult};
use dominet_domain::user::UserId;

use crate::pg::{corrupt, repo_err};

// Reflet d'une ligne de table : les indicateurs sont independants.
#[allow(clippy::struct_excessive_bools)]
struct RuleRow {
    id: i64,
    name: String,
    device_id: i64,
    key: String,
    operator: String,
    threshold_num: Option<f64>,
    threshold_str: String,
    severity: String,
    cooldown_minutes: i16,
    notify_email: bool,
    notify_sms: bool,
    notify_android: bool,
    enabled: bool,
    description: String,
}

impl RuleRow {
    fn rule(self) -> RepoResult<AlertRule> {
        Ok(AlertRule {
            id: AlertRuleId(self.id),
            rule: AlertRuleDraft {
                name: self.name,
                device: DeviceId(self.device_id),
                key: self.key,
                operator: Operator::parse(&self.operator)
                    .ok_or_else(|| corrupt("rule operator"))?,
                threshold_num: self.threshold_num,
                threshold_str: self.threshold_str,
                severity: Severity::parse(&self.severity)
                    .ok_or_else(|| corrupt("rule severity"))?,
                cooldown_minutes: self.cooldown_minutes,
                notify_email: self.notify_email,
                notify_sms: self.notify_sms,
                notify_android: self.notify_android,
                enabled: self.enabled,
                description: self.description,
            },
        })
    }
}

/// Acces PostgreSQL aux regles d'alerte.
#[derive(Debug, Clone)]
pub struct PgAlertRules {
    pool: sqlx::PgPool,
}

impl PgAlertRules {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl AlertRuleRepo for PgAlertRules {
    async fn list(&self) -> RepoResult<Vec<AlertRule>> {
        let rows = sqlx::query_as!(
            RuleRow,
            "SELECT id, name, device_id, key, operator, threshold_num, threshold_str, severity, cooldown_minutes, notify_email,
                    notify_sms, notify_android, enabled, description FROM alert_rule ORDER BY device_id, name, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(RuleRow::rule).collect()
    }

    async fn list_enabled_for_device(&self, device: DeviceId) -> RepoResult<Vec<AlertRule>> {
        let rows = sqlx::query_as!(
            RuleRow,
            "SELECT id, name, device_id, key, operator, threshold_num, threshold_str, severity, cooldown_minutes, notify_email,
                    notify_sms, notify_android, enabled, description FROM alert_rule WHERE device_id = $1 AND enabled ORDER BY name, id",
            device.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(RuleRow::rule).collect()
    }

    async fn get(&self, id: AlertRuleId) -> RepoResult<Option<AlertRule>> {
        sqlx::query_as!(
            RuleRow,
            "SELECT id, name, device_id, key, operator, threshold_num, threshold_str, severity, cooldown_minutes, notify_email,
                    notify_sms, notify_android, enabled, description FROM alert_rule WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(RuleRow::rule)
        .transpose()
    }

    async fn create(&self, d: &AlertRuleDraft) -> RepoResult<AlertRule> {
        let row = sqlx::query_as!(
            RuleRow,
            "INSERT INTO alert_rule (name, device_id, key, operator, threshold_num, threshold_str, severity, cooldown_minutes,
                                     notify_email, notify_sms, notify_android, enabled, description)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             RETURNING id, name, device_id, key, operator, threshold_num, threshold_str, severity, cooldown_minutes, notify_email,
                       notify_sms, notify_android, enabled, description",
            d.name,
            d.device.0,
            d.key,
            d.operator.as_str(),
            d.threshold_num,
            d.threshold_str,
            d.severity.as_str(),
            d.cooldown_minutes,
            d.notify_email,
            d.notify_sms,
            d.notify_android,
            d.enabled,
            d.description
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.rule()
    }

    async fn update(&self, id: AlertRuleId, d: &AlertRuleDraft) -> RepoResult<AlertRule> {
        let row = sqlx::query_as!(
            RuleRow,
            "UPDATE alert_rule SET name = $2, device_id = $3, key = $4, operator = $5, threshold_num = $6, threshold_str = $7,
                    severity = $8, cooldown_minutes = $9, notify_email = $10, notify_sms = $11, notify_android = $12, enabled = $13,
                    description = $14
             WHERE id = $1
             RETURNING id, name, device_id, key, operator, threshold_num, threshold_str, severity, cooldown_minutes, notify_email,
                       notify_sms, notify_android, enabled, description",
            id.0,
            d.name,
            d.device.0,
            d.key,
            d.operator.as_str(),
            d.threshold_num,
            d.threshold_str,
            d.severity.as_str(),
            d.cooldown_minutes,
            d.notify_email,
            d.notify_sms,
            d.notify_android,
            d.enabled,
            d.description
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.rule()
    }

    async fn delete(&self, id: AlertRuleId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM alert_rule WHERE id = $1", id.0)
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

struct AlertRow {
    id: i64,
    rule_id: Option<i64>,
    device_id: i64,
    status: String,
    severity: String,
    value_triggered: String,
    fired_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    acknowledged_at: Option<DateTime<Utc>>,
    acknowledged_by: Option<i64>,
    notified: bool,
}

impl AlertRow {
    fn alert(self) -> RepoResult<Alert> {
        Ok(Alert {
            id: AlertId(self.id),
            rule: self.rule_id.map(AlertRuleId),
            device: DeviceId(self.device_id),
            status: AlertStatus::parse(&self.status).ok_or_else(|| corrupt("alert status"))?,
            severity: Severity::parse(&self.severity).ok_or_else(|| corrupt("alert severity"))?,
            value_triggered: self.value_triggered,
            fired_at: self.fired_at,
            resolved_at: self.resolved_at,
            acknowledged_at: self.acknowledged_at,
            acknowledged_by: self.acknowledged_by.map(UserId),
            notified: self.notified,
        })
    }
}

/// Acces PostgreSQL aux alertes.
#[derive(Debug, Clone)]
pub struct PgAlerts {
    pool: sqlx::PgPool,
}

impl PgAlerts {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl AlertRepo for PgAlerts {
    async fn list(&self, status: Option<AlertStatus>, limit: i64) -> RepoResult<Vec<Alert>> {
        let rows = sqlx::query_as!(
            AlertRow,
            "SELECT id, rule_id, device_id, status, severity, value_triggered, fired_at, resolved_at, acknowledged_at, acknowledged_by,
                    notified FROM alert WHERE ($1::text IS NULL OR status = $1) ORDER BY fired_at DESC, id DESC LIMIT $2",
            status.map(AlertStatus::as_str),
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(AlertRow::alert).collect()
    }

    async fn latest_firing(
        &self,
        rule: AlertRuleId,
        device: DeviceId,
    ) -> RepoResult<Option<Alert>> {
        sqlx::query_as!(
            AlertRow,
            "SELECT id, rule_id, device_id, status, severity, value_triggered, fired_at, resolved_at, acknowledged_at, acknowledged_by,
                    notified FROM alert WHERE rule_id = $1 AND device_id = $2 AND status = 'firing' ORDER BY fired_at DESC, id DESC LIMIT 1",
            rule.0,
            device.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(AlertRow::alert)
        .transpose()
    }

    async fn create(&self, new: &NewAlert) -> RepoResult<Alert> {
        let row = sqlx::query_as!(
            AlertRow,
            "INSERT INTO alert (rule_id, device_id, status, severity, value_triggered, fired_at) VALUES ($1, $2, 'firing', $3, $4, $5)
             RETURNING id, rule_id, device_id, status, severity, value_triggered, fired_at, resolved_at, acknowledged_at, acknowledged_by,
                       notified",
            new.rule.0,
            new.device.0,
            new.severity.as_str(),
            new.value_triggered.chars().take(128).collect::<String>(),
            new.fired_at
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.alert()
    }

    async fn resolve_firing(
        &self,
        rule: AlertRuleId,
        device: DeviceId,
        at: DateTime<Utc>,
    ) -> RepoResult<u64> {
        let done = sqlx::query!(
            "UPDATE alert SET status = 'resolved', resolved_at = $3 WHERE rule_id = $1 AND device_id = $2 AND status = 'firing'",
            rule.0,
            device.0,
            at
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected())
    }

    async fn acknowledge(
        &self,
        id: AlertId,
        by: UserId,
        at: DateTime<Utc>,
    ) -> RepoResult<Option<Alert>> {
        sqlx::query_as!(
            AlertRow,
            "UPDATE alert SET status = 'acknowledged', acknowledged_at = $3, acknowledged_by = $2 WHERE id = $1 AND status = 'firing'
             RETURNING id, rule_id, device_id, status, severity, value_triggered, fired_at, resolved_at, acknowledged_at, acknowledged_by,
                       notified",
            id.0,
            by.0,
            at
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(AlertRow::alert)
        .transpose()
    }

    async fn acknowledge_all(&self, by: UserId, at: DateTime<Utc>) -> RepoResult<u64> {
        let done = sqlx::query!(
            "UPDATE alert SET status = 'acknowledged', acknowledged_at = $2, acknowledged_by = $1 WHERE status = 'firing'",
            by.0,
            at
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected())
    }

    async fn count_firing(&self) -> RepoResult<i64> {
        sqlx::query_scalar!("SELECT count(*) AS \"n!\" FROM alert WHERE status = 'firing'")
            .fetch_one(&self.pool)
            .await
            .map_err(repo_err)
    }

    async fn mark_notified(&self, id: AlertId) -> RepoResult<()> {
        sqlx::query!("UPDATE alert SET notified = TRUE WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

/// Acces PostgreSQL au bilan d'energie.
#[derive(Debug, Clone)]
pub struct PgEnergy {
    pool: sqlx::PgPool,
}

impl PgEnergy {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl EnergyRepo for PgEnergy {
    async fn upsert(&self, d: &EnergyDay) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO energy_daily_log (day, linky_hp_wh, linky_hc_wh, solar_wh, pac_wh) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (day) DO UPDATE SET linky_hp_wh = EXCLUDED.linky_hp_wh, linky_hc_wh = EXCLUDED.linky_hc_wh,
                 solar_wh = EXCLUDED.solar_wh, pac_wh = EXCLUDED.pac_wh",
            d.day,
            d.linky_hp_wh,
            d.linky_hc_wh,
            d.solar_wh,
            d.pac_wh
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn get(&self, day: NaiveDate) -> RepoResult<Option<EnergyDay>> {
        let row = sqlx::query!(
            "SELECT day, linky_hp_wh, linky_hc_wh, solar_wh, pac_wh FROM energy_daily_log WHERE day = $1",
            day
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(row.map(|r| EnergyDay {
            day: r.day,
            linky_hp_wh: r.linky_hp_wh,
            linky_hc_wh: r.linky_hc_wh,
            solar_wh: r.solar_wh,
            pac_wh: r.pac_wh,
        }))
    }

    async fn recent(&self, days: u32) -> RepoResult<Vec<EnergyDay>> {
        let rows = sqlx::query!(
            "SELECT day, linky_hp_wh, linky_hc_wh, solar_wh, pac_wh FROM (
                 SELECT * FROM energy_daily_log ORDER BY day DESC LIMIT $1
             ) recent ORDER BY day",
            i64::from(days)
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| EnergyDay {
                day: r.day,
                linky_hp_wh: r.linky_hp_wh,
                linky_hc_wh: r.linky_hc_wh,
                solar_wh: r.solar_wh,
                pac_wh: r.pac_wh,
            })
            .collect())
    }
}
