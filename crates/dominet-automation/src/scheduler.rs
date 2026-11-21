// SPDX-License-Identifier: AGPL-3.0-or-later
//! Planificateur (R4 lot 4, ADR 0033) : remplace un planificateur externe. Les travaux sont dans `schedule_job` ; le prochain passage
//! est persiste (`next_run_at`) pour qu'on le voie dans l'administration, mais il est **recalcule** au demarrage et a chaque
//! changement de definition.
//!
//! - **Pas de rattrapage** : un passage manque pendant un arret n'est pas rejoue (comme un cron classique), sauf si on le decouvre dans
//!   les [`TOLERANCE_SECS`] secondes qui suivent son heure (tick tardif).
//! - **Synchronisation** : chaque automation active a declencheur `schedule` ou `solar` a un travail `automation-<id>` qui
//!   suit sa definition ; un travail dont l'automation est desactivee, supprimee ou devenue `mqtt_message` disparait. Le
//!   travail systeme `compute_daily_energy` (00:05 locale) est cree s'il manque.
//! - Les effets passent par le port [`TaskRunner`] (le serveur l'implemente), le temps par une horloge injectee : les tests
//!   avancent l'heure a la main avec [`Scheduler::tick`].

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

/// Retard admis pour executer encore un passage decouvert apres son heure.
pub const TOLERANCE_SECS: i64 = 60;
/// Veille maximale entre deux tours (un autre processus peut avoir modifie la base).
pub const MAX_SLEEP_SECS: u64 = 30;
/// Reessai quand l'horaire n'a pas de passage proche (soleil polaire) : pas de boucle serree.
const RETRY_HOURS: i64 = 6;

/// Nom de la tache d'une automation planifiee.
pub const TASK_RUN_AUTOMATION: &str = "run_automation";
/// Nom de la tache du bilan d'energie quotidien.
pub const TASK_DAILY_ENERGY: &str = "compute_daily_energy";
/// Nom du travail du bilan d'energie.
pub const JOB_DAILY_ENERGY: &str = "compute_daily_energy";

/// Ce que savent faire les travaux. `Ok(detail)` ou `Err(raison)`.
pub trait TaskRunner: Send + Sync + 'static {
    fn run(&self, job: &Job) -> impl Future<Output = Result<String, String>> + Send;
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

pub struct Scheduler<S, A, T> {
    jobs: S,
    automations: A,
    runner: T,
    /// Fuseau des nouveaux travaux (reglage `TZ`).
    timezone: Tz,
    clock: Arc<Clock>,
}

impl<S, A, T> std::fmt::Debug for Scheduler<S, A, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

/// Travail attendu pour une automation planifiee.
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

    /// Aligne les travaux sur les automations et cree le travail systeme manquant.
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
            // Le travail systeme garde le fuseau qu'on lui a donne, l'horaire qu'on lui a edite n'est pas ecrase.
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

    /// Un tour : calcule les passages qui manquent, execute ceux qui sont dus, et renvoie l'instant du prochain passage connu.
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

    /// Boucle du role `scheduler` : synchronise, execute ce qui est du, dort jusqu'au prochain passage (au plus
    /// [`MAX_SLEEP_SECS`] s) ou jusqu'a un reveil de l'administration.
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
