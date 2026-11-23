// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repositories of the Prometheus and Grafana alert rules (`prometheus_rule`, `grafana_alert_rule`).

use dominet_domain::RepoError;
use dominet_domain::alert::Severity;
use dominet_domain::observability::{
    GrafanaRule, GrafanaRuleDraft, GrafanaRuleId, PrometheusRule, PrometheusRuleDraft,
    PrometheusRuleId, slugify,
};
use dominet_domain::repo::{GrafanaRuleRepo, PrometheusRuleRepo, RepoResult};

use crate::pg::{corrupt, repo_err};

fn severity(s: &str) -> RepoResult<Severity> {
    Severity::parse(s).ok_or_else(|| corrupt(format!("severity {s}")))
}

struct PromRow {
    id: i64,
    name: String,
    rule_group: String,
    severity: String,
    expr: String,
    for_duration: String,
    summary: String,
    description: String,
    eval_interval: String,
    active: bool,
}

impl PromRow {
    fn into_rule(self) -> RepoResult<PrometheusRule> {
        Ok(PrometheusRule {
            id: PrometheusRuleId(self.id),
            draft: PrometheusRuleDraft {
                severity: severity(&self.severity)?,
                name: self.name,
                group: self.rule_group,
                expr: self.expr,
                for_duration: self.for_duration,
                summary: self.summary,
                description: self.description,
                eval_interval: self.eval_interval,
                active: self.active,
            },
        })
    }
}

/// Acces PostgreSQL aux regles Prometheus.
#[derive(Debug, Clone)]
pub struct PgPrometheusRules {
    pool: sqlx::PgPool,
}

impl PgPrometheusRules {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl PrometheusRuleRepo for PgPrometheusRules {
    async fn list(&self) -> RepoResult<Vec<PrometheusRule>> {
        let rows = sqlx::query_as!(
            PromRow,
            "SELECT id, name, rule_group, severity, expr, for_duration, summary, description, eval_interval, active
             FROM prometheus_rule ORDER BY rule_group, name"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(PromRow::into_rule).collect()
    }

    async fn get(&self, id: PrometheusRuleId) -> RepoResult<Option<PrometheusRule>> {
        let row = sqlx::query_as!(
            PromRow,
            "SELECT id, name, rule_group, severity, expr, for_duration, summary, description, eval_interval, active
             FROM prometheus_rule WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(PromRow::into_rule).transpose()
    }

    async fn create(&self, d: &PrometheusRuleDraft) -> RepoResult<PrometheusRule> {
        let row = sqlx::query_as!(
            PromRow,
            "INSERT INTO prometheus_rule (name, rule_group, severity, expr, for_duration, summary, description, eval_interval, active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id, name, rule_group, severity, expr, for_duration, summary, description, eval_interval, active",
            d.name,
            d.group,
            d.severity.as_str(),
            d.expr,
            d.for_duration,
            d.summary,
            d.description,
            d.eval_interval,
            d.active
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.into_rule()
    }

    async fn update(
        &self,
        id: PrometheusRuleId,
        d: &PrometheusRuleDraft,
    ) -> RepoResult<PrometheusRule> {
        let row = sqlx::query_as!(
            PromRow,
            "UPDATE prometheus_rule SET name = $2, rule_group = $3, severity = $4, expr = $5, for_duration = $6, summary = $7,
                    description = $8, eval_interval = $9, active = $10, updated_at = now()
             WHERE id = $1
             RETURNING id, name, rule_group, severity, expr, for_duration, summary, description, eval_interval, active",
            id.0,
            d.name,
            d.group,
            d.severity.as_str(),
            d.expr,
            d.for_duration,
            d.summary,
            d.description,
            d.eval_interval,
            d.active
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.ok_or(RepoError::NotFound)?.into_rule()
    }

    async fn delete(&self, id: PrometheusRuleId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM prometheus_rule WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn insert_if_absent(&self, d: &PrometheusRuleDraft) -> RepoResult<bool> {
        let done = sqlx::query!(
            "INSERT INTO prometheus_rule (name, rule_group, severity, expr, for_duration, summary, description, eval_interval, active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (name) DO NOTHING",
            d.name,
            d.group,
            d.severity.as_str(),
            d.expr,
            d.for_duration,
            d.summary,
            d.description,
            d.eval_interval,
            d.active
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected() > 0)
    }
}

struct GrafanaRow {
    id: i64,
    uid: String,
    title: String,
    folder: String,
    rule_group: String,
    group_interval: String,
    datasource_uid: String,
    expr: String,
    query_window: i32,
    for_duration: String,
    severity: String,
    summary: String,
    description: String,
    no_data_state: String,
    exec_err_state: String,
    active: bool,
}

impl GrafanaRow {
    fn into_rule(self) -> RepoResult<GrafanaRule> {
        Ok(GrafanaRule {
            id: GrafanaRuleId(self.id),
            draft: GrafanaRuleDraft {
                severity: severity(&self.severity)?,
                uid: self.uid,
                title: self.title,
                folder: self.folder,
                group: self.rule_group,
                group_interval: self.group_interval,
                datasource_uid: self.datasource_uid,
                expr: self.expr,
                query_window: self.query_window,
                for_duration: self.for_duration,
                summary: self.summary,
                description: self.description,
                no_data_state: self.no_data_state,
                exec_err_state: self.exec_err_state,
                active: self.active,
            },
        })
    }
}

/// Acces PostgreSQL aux regles Grafana.
#[derive(Debug, Clone)]
pub struct PgGrafanaRules {
    pool: sqlx::PgPool,
}

impl PgGrafanaRules {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    /// `uid` of the rule: the draft's, or derived from the title and made unique like the reference (`title`, `title-1`...).
    async fn uid_for(&self, d: &GrafanaRuleDraft, except: Option<i64>) -> RepoResult<String> {
        if !d.uid.is_empty() {
            return Ok(d.uid.clone());
        }
        let base = match slugify(&d.title) {
            s if s.is_empty() => "rule".to_string(),
            s => s,
        };
        let (mut uid, mut n) = (base.clone(), 1);
        loop {
            let taken: bool = sqlx::query_scalar!(
                r#"SELECT EXISTS(SELECT 1 FROM grafana_alert_rule WHERE uid = $1 AND id IS DISTINCT FROM $2) AS "taken!""#,
                uid,
                except
            )
            .fetch_one(&self.pool)
            .await
            .map_err(repo_err)?;
            if !taken {
                return Ok(uid);
            }
            uid = format!("{base}-{n}");
            n += 1;
        }
    }
}

impl GrafanaRuleRepo for PgGrafanaRules {
    async fn list(&self) -> RepoResult<Vec<GrafanaRule>> {
        let rows = sqlx::query_as!(
            GrafanaRow,
            "SELECT id, uid, title, folder, rule_group, group_interval, datasource_uid, expr, query_window, for_duration,
                    severity, summary, description, no_data_state, exec_err_state, active
             FROM grafana_alert_rule ORDER BY folder, rule_group, title, id"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(GrafanaRow::into_rule).collect()
    }

    async fn get(&self, id: GrafanaRuleId) -> RepoResult<Option<GrafanaRule>> {
        let row = sqlx::query_as!(
            GrafanaRow,
            "SELECT id, uid, title, folder, rule_group, group_interval, datasource_uid, expr, query_window, for_duration,
                    severity, summary, description, no_data_state, exec_err_state, active
             FROM grafana_alert_rule WHERE id = $1",
            id.0
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(GrafanaRow::into_rule).transpose()
    }

    async fn create(&self, d: &GrafanaRuleDraft) -> RepoResult<GrafanaRule> {
        let uid = self.uid_for(d, None).await?;
        let row = sqlx::query_as!(
            GrafanaRow,
            "INSERT INTO grafana_alert_rule (uid, title, folder, rule_group, group_interval, datasource_uid, expr, query_window,
                    for_duration, severity, summary, description, no_data_state, exec_err_state, active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             RETURNING id, uid, title, folder, rule_group, group_interval, datasource_uid, expr, query_window, for_duration,
                       severity, summary, description, no_data_state, exec_err_state, active",
            uid,
            d.title,
            d.folder,
            d.group,
            d.group_interval,
            d.datasource_uid,
            d.expr,
            d.query_window,
            d.for_duration,
            d.severity.as_str(),
            d.summary,
            d.description,
            d.no_data_state,
            d.exec_err_state,
            d.active
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        row.into_rule()
    }

    async fn update(&self, id: GrafanaRuleId, d: &GrafanaRuleDraft) -> RepoResult<GrafanaRule> {
        let uid = self.uid_for(d, Some(id.0)).await?;
        let row = sqlx::query_as!(
            GrafanaRow,
            "UPDATE grafana_alert_rule SET uid = $2, title = $3, folder = $4, rule_group = $5, group_interval = $6,
                    datasource_uid = $7, expr = $8, query_window = $9, for_duration = $10, severity = $11, summary = $12,
                    description = $13, no_data_state = $14, exec_err_state = $15, active = $16, updated_at = now()
             WHERE id = $1
             RETURNING id, uid, title, folder, rule_group, group_interval, datasource_uid, expr, query_window, for_duration,
                       severity, summary, description, no_data_state, exec_err_state, active",
            id.0,
            uid,
            d.title,
            d.folder,
            d.group,
            d.group_interval,
            d.datasource_uid,
            d.expr,
            d.query_window,
            d.for_duration,
            d.severity.as_str(),
            d.summary,
            d.description,
            d.no_data_state,
            d.exec_err_state,
            d.active
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.ok_or(RepoError::NotFound)?.into_rule()
    }

    async fn delete(&self, id: GrafanaRuleId) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM grafana_alert_rule WHERE id = $1", id.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn insert_if_absent(&self, d: &GrafanaRuleDraft) -> RepoResult<bool> {
        let uid = self.uid_for(d, None).await?;
        let done = sqlx::query!(
            "INSERT INTO grafana_alert_rule (uid, title, folder, rule_group, group_interval, datasource_uid, expr, query_window,
                    for_duration, severity, summary, description, no_data_state, exec_err_state, active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) ON CONFLICT (uid) DO NOTHING",
            uid,
            d.title,
            d.folder,
            d.group,
            d.group_interval,
            d.datasource_uid,
            d.expr,
            d.query_window,
            d.for_duration,
            d.severity.as_str(),
            d.summary,
            d.description,
            d.no_data_state,
            d.exec_err_state,
            d.active
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected() > 0)
    }
}
