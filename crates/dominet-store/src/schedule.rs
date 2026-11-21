// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depot des travaux du planificateur (R4 lot 4, ADR 0033) : table `schedule_job`.

use chrono::{DateTime, Utc};
use dominet_domain::automation::{AutomationId, SolarEvent};
use dominet_domain::repo::{RepoResult, ScheduleRepo};
use dominet_domain::schedule::{IntervalPeriod, Job, JobDraft, JobId, JobSchedule};

use crate::pg::{corrupt, repo_err};

/// Acces PostgreSQL aux travaux du planificateur.
#[derive(Debug, Clone)]
pub struct PgSchedule {
    pool: sqlx::PgPool,
}

impl PgSchedule {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

/// Colonnes d'un horaire, dans l'ordre : type, repetition, periode, expression cron, evenement solaire, latitude, longitude.
type Columns = (
    &'static str,
    Option<i32>,
    Option<&'static str>,
    Option<String>,
    Option<&'static str>,
    Option<f64>,
    Option<f64>,
);

fn columns(s: &JobSchedule) -> Columns {
    match s {
        JobSchedule::Interval { every, period } => (
            "interval",
            Some(i32::try_from(*every).unwrap_or(i32::MAX)),
            Some(period.as_str()),
            None,
            None,
            None,
            None,
        ),
        JobSchedule::Cron(expr) => ("cron", None, None, Some(expr.clone()), None, None, None),
        JobSchedule::Solar {
            event,
            latitude,
            longitude,
        } => (
            "solar",
            None,
            None,
            None,
            Some(event.as_str()),
            Some(*latitude),
            Some(*longitude),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn schedule_from(
    kind: &str,
    every: Option<i32>,
    period: Option<&str>,
    cron: Option<String>,
    event: Option<&str>,
    lat: Option<f64>,
    lon: Option<f64>,
) -> RepoResult<JobSchedule> {
    Ok(match kind {
        "interval" => JobSchedule::Interval {
            every: u32::try_from(every.ok_or_else(|| corrupt("interval"))?)
                .map_err(|_| corrupt("interval"))?,
            period: period
                .and_then(IntervalPeriod::parse)
                .ok_or_else(|| corrupt("period"))?,
        },
        "cron" => JobSchedule::Cron(cron.ok_or_else(|| corrupt("cron"))?),
        "solar" => JobSchedule::Solar {
            event: event
                .and_then(SolarEvent::parse)
                .ok_or_else(|| corrupt("solar event"))?,
            latitude: lat.ok_or_else(|| corrupt("latitude"))?,
            longitude: lon.ok_or_else(|| corrupt("longitude"))?,
        },
        other => return Err(corrupt(format!("schedule type {other}"))),
    })
}

macro_rules! job_from {
    ($r:expr) => {{
        let r = $r;
        Ok::<Job, dominet_domain::RepoError>(Job {
            id: JobId(r.id),
            draft: JobDraft {
                name: r.name,
                task: r.task,
                args: r.args,
                schedule: schedule_from(
                    &r.kind,
                    r.interval_every,
                    r.interval_period.as_deref(),
                    r.cron_expr,
                    r.solar_event.as_deref(),
                    r.latitude,
                    r.longitude,
                )?,
                timezone: r.timezone,
                enabled: r.enabled,
                automation: r.automation_id.map(AutomationId),
            },
            last_run_at: r.last_run_at,
            next_run_at: r.next_run_at,
            last_status: r.last_status,
        })
    }};
}

impl ScheduleRepo for PgSchedule {
    async fn list(&self) -> RepoResult<Vec<Job>> {
        sqlx::query!(
            "SELECT id, name, task, args, kind, interval_every, interval_period, cron_expr, timezone, solar_event,
                    latitude, longitude, enabled, last_run_at, next_run_at, last_status, automation_id
             FROM schedule_job ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(|r| job_from!(r))
        .collect()
    }

    async fn upsert(&self, d: &JobDraft) -> RepoResult<Job> {
        let (kind, every, period, cron, event, lat, lon) = columns(&d.schedule);
        let r = sqlx::query!(
            "INSERT INTO schedule_job (name, task, args, kind, interval_every, interval_period, cron_expr, timezone, solar_event,
                                       latitude, longitude, enabled, automation_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (name) DO UPDATE SET
                 next_run_at = CASE WHEN (schedule_job.task, schedule_job.args, schedule_job.kind, schedule_job.interval_every,
                                          schedule_job.interval_period, schedule_job.cron_expr, schedule_job.timezone,
                                          schedule_job.solar_event, schedule_job.latitude, schedule_job.longitude,
                                          schedule_job.enabled)
                                IS NOT DISTINCT FROM (EXCLUDED.task, EXCLUDED.args, EXCLUDED.kind, EXCLUDED.interval_every,
                                          EXCLUDED.interval_period, EXCLUDED.cron_expr, EXCLUDED.timezone,
                                          EXCLUDED.solar_event, EXCLUDED.latitude, EXCLUDED.longitude, EXCLUDED.enabled)
                               THEN schedule_job.next_run_at ELSE NULL END,
                 task = EXCLUDED.task, args = EXCLUDED.args, kind = EXCLUDED.kind, interval_every = EXCLUDED.interval_every,
                 interval_period = EXCLUDED.interval_period, cron_expr = EXCLUDED.cron_expr, timezone = EXCLUDED.timezone,
                 solar_event = EXCLUDED.solar_event, latitude = EXCLUDED.latitude, longitude = EXCLUDED.longitude,
                 enabled = EXCLUDED.enabled, automation_id = EXCLUDED.automation_id
             RETURNING id, name, task, args, kind, interval_every, interval_period, cron_expr, timezone, solar_event,
                       latitude, longitude, enabled, last_run_at, next_run_at, last_status, automation_id",
            d.name,
            d.task,
            d.args,
            kind,
            every,
            period,
            cron,
            d.timezone,
            event,
            lat,
            lon,
            d.enabled,
            d.automation.map(|a| a.0)
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        job_from!(r)
    }

    async fn prune_automation_jobs(&self, keep: &[AutomationId]) -> RepoResult<u64> {
        let ids: Vec<i64> = keep.iter().map(|a| a.0).collect();
        let done = sqlx::query!(
            "DELETE FROM schedule_job WHERE automation_id IS NOT NULL AND NOT (automation_id = ANY($1))",
            &ids
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected())
    }

    async fn set_next(&self, id: JobId, next: Option<DateTime<Utc>>) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE schedule_job SET next_run_at = $2 WHERE id = $1",
            id.0,
            next
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn record(
        &self,
        id: JobId,
        ran_at: Option<DateTime<Utc>>,
        status: &str,
        next: Option<DateTime<Utc>>,
    ) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE schedule_job SET last_run_at = COALESCE($2, last_run_at), last_status = $3, next_run_at = $4 WHERE id = $1",
            id.0,
            ran_at,
            status.chars().take(500).collect::<String>(),
            next
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn automation_next_runs(&self) -> RepoResult<Vec<(AutomationId, Option<DateTime<Utc>>)>> {
        let rows = sqlx::query!(
            "SELECT automation_id AS \"automation_id!\", next_run_at FROM schedule_job
             WHERE automation_id IS NOT NULL AND enabled"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| (AutomationId(r.automation_id), r.next_run_at))
            .collect())
    }
}
