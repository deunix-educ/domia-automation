// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scheduler (R4 lot 4, ADR 0033): replaces an external scheduler. The jobs are in `schedule_job`; the next run
//! is persisted (`next_run_at`) so that it can be seen in the administration, but it is **recomputed** at start-up and at each
//! definition change.
//!
//! - **No catch-up**: a run missed during a stop is not replayed (like a classic cron), unless it is discovered within
//!   the [`TOLERANCE_SECS`] seconds that follow its time (late tick).
//! - **Synchronisation**: each active automation with a `schedule` or `solar` trigger has an `automation-<id>` job that
//!   follows its definition; a job whose automation is deactivated, deleted or became `mqtt_message` disappears. The
//!   `compute_daily_energy` system job (00:05 local) is created if missing.
//! - The effects go through the [`TaskRunner`] port (the server implements it), the time through an injected clock: the tests
//!   advance the time by hand with [`Scheduler::tick`].

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use dominet_domain::automation::{Automation, Trigger};
use dominet_domain::repo::{AutomationRepo, ScheduleRepo};
use dominet_domain::schedule::{Job, JobDraft, JobSchedule};
use serde_json::json;
use tokio::sync::Notify;

use crate::next::next_run;

/// Delay allowed to still run a run discovered after its time.
pub const TOLERANCE_SECS: i64 = 60;
/// Maximum sleep between two rounds (another process may have modified the database).
pub const MAX_SLEEP_SECS: u64 = 30;
/// Retry when the schedule has no near run (polar sun): no tight loop.
const RETRY_HOURS: i64 = 6;

/// Name of the task of a scheduled automation.
pub const TASK_RUN_AUTOMATION: &str = "run_automation";
/// Name of the daily energy summary task.
pub const TASK_DAILY_ENERGY: &str = "compute_daily_energy";
/// Name of the energy summary job.
pub const JOB_DAILY_ENERGY: &str = "compute_daily_energy";

/// What the jobs can do. `Ok(detail)` or `Err(reason)`.
pub trait TaskRunner: Send + Sync + 'static {
    fn run(&self, job: &Job) -> impl Future<Output = Result<String, String>> + Send;
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

pub struct Scheduler<S, A, T> {
    jobs: S,
    automations: A,
    runner: T,
    /// Time zone of the new jobs (`TZ` setting).
    timezone: Tz,
    clock: Arc<Clock>,
}

impl<S, A, T> std::fmt::Debug for Scheduler<S, A, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

/// Job expected for a scheduled automation.
fn job_of(a: &Automation, tz: Tz) -> Option<JobDraft> {
    if !a.draft.enabled {
        return None;
    }
    let schedule = match &a.draft.trigger {
        Trigger::Schedule { cron } => JobSchedule::Cron(cron.clone()),
        Trigger::Solar {
            event,
            latitude,
            longitude,
        } => JobSchedule::Solar {
            event: *event,
            latitude: *latitude,
            longitude: *longitude,
        },
        _ => return None,
    };
    Some(JobDraft {
        name: format!("automation-{}", a.id.0),
        task: TASK_RUN_AUTOMATION.to_string(),
        args: json!({"automation_id": a.id.0}),
        schedule,
        timezone: tz.name().to_string(),
        enabled: true,
        automation: Some(a.id),
    })
}

fn zone(name: &str, fallback: Tz) -> Tz {
    name.parse().unwrap_or(fallback)
}

impl<S, A, T> Scheduler<S, A, T>
where
    S: ScheduleRepo + Send + Sync + 'static,
    A: AutomationRepo + Send + Sync + 'static,
    T: TaskRunner,
{
    #[must_use]
    pub fn new(jobs: S, automations: A, runner: T, timezone: Tz) -> Self {
        Self::with_clock(jobs, automations, runner, timezone, Arc::new(Utc::now))
    }

    #[must_use]
    pub fn with_clock(jobs: S, automations: A, runner: T, timezone: Tz, clock: Arc<Clock>) -> Self {
        Self {
            jobs,
            automations,
            runner,
            timezone,
            clock,
        }
    }

    /// Aligns the jobs with the automations and creates the missing system job.
    pub async fn sync(&self) {
        let automations = match self.automations.list().await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(error = %e, "unreadable automations: jobs unchanged");
                return;
            }
        };
        let existing = match self.jobs.list().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "unreadable jobs");
                return;
            }
        };
        let mut wanted: Vec<JobDraft> = automations
            .iter()
            .filter_map(|a| job_of(a, self.timezone))
            .collect();
        wanted.push(JobDraft {
            name: JOB_DAILY_ENERGY.to_string(),
            task: TASK_DAILY_ENERGY.to_string(),
            args: json!({}),
            schedule: JobSchedule::Cron("5 0 * * *".to_string()),
            timezone: self.timezone.name().to_string(),
            enabled: true,
            automation: None,
        });
        for draft in &wanted {
            // The system job keeps the time zone it was given, the schedule that was edited is not overwritten.
            let current = existing.iter().find(|j| j.draft.name == draft.name);
            let unchanged = current.is_some_and(|j| &j.draft == draft);
            let system_kept = draft.automation.is_none() && current.is_some();
            if unchanged || system_kept {
                continue;
            }
            if let Err(e) = self.jobs.upsert(draft).await {
                tracing::warn!(job = %draft.name, error = %e, "job not saved");
            }
        }
        let keep: Vec<_> = automations
            .iter()
            .filter(|a| job_of(a, self.timezone).is_some())
            .map(|a| a.id)
            .collect();
        match self.jobs.prune_automation_jobs(&keep).await {
            Ok(n) if n > 0 => tracing::info!(travaux = n, "automation jobs removed"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "cannot clean up the jobs"),
        }
    }

    /// One round: computes the missing runs, executes those that are due, and returns the instant of the next known run.
    pub async fn tick(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let jobs = match self.jobs.list().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "unreadable jobs");
                return None;
            }
        };
        let mut soonest: Option<DateTime<Utc>> = None;
        for job in jobs.iter().filter(|j| j.draft.enabled) {
            let tz = zone(&job.draft.timezone, self.timezone);
            let after_now = || {
                next_run(&job.draft.schedule, tz, now)
                    .or_else(|| Some(now + chrono::Duration::hours(RETRY_HOURS)))
            };
            let next = match job.next_run_at {
                None => {
                    let n = after_now();
                    if let Err(e) = self.jobs.set_next(job.id, n).await {
                        tracing::warn!(job = %job.draft.name, error = %e, "next run not saved");
                    }
                    n
                }
                Some(due) if due > now => Some(due),
                Some(due) => {
                    let late = (now - due).num_seconds();
                    let (ran, status) = if late <= TOLERANCE_SECS {
                        let status = match self.runner.run(job).await {
                            Ok(detail) => format!("ok : {detail}"),
                            Err(reason) => format!("failure: {reason}"),
                        };
                        (Some(now), status)
                    } else {
                        (None, format!("missed ({late} s late, no catch-up)"))
                    };
                    let n = after_now();
                    if let Err(e) = self.jobs.record(job.id, ran, &status, n).await {
                        tracing::warn!(job = %job.draft.name, error = %e, "run not saved");
                    }
                    tracing::info!(job = %job.draft.name, %status, "scheduled job");
                    n
                }
            };
            soonest = match (soonest, next) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        soonest
    }

    /// Loop of the `scheduler` role: synchronises, executes what is due, sleeps until the next run (at most
    /// [`MAX_SLEEP_SECS`] s) or until a wake-up from the administration.
    pub async fn run(self: Arc<Self>, wake: Arc<Notify>) {
        loop {
            self.sync().await;
            let now = (self.clock)();
            let sleep = match self.tick(now).await {
                Some(next) => (next - now)
                    .to_std()
                    .unwrap_or(Duration::ZERO)
                    .min(Duration::from_secs(MAX_SLEEP_SECS)),
                None => Duration::from_secs(MAX_SLEEP_SECS),
            };
            tokio::select! {
                () = tokio::time::sleep(sleep.max(Duration::from_millis(200))) => {}
                () = wake.notified() => {}
            }
        }
    }
}
