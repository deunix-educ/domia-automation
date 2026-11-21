// SPDX-License-Identifier: AGPL-3.0-or-later
//! Role `scheduler` (R4 lot 4, ADR 0033) : le planificateur et ses taches cote serveur.

use std::sync::Arc;

use dominet_automation::engine::{Engine, FireContext};
use dominet_automation::scheduler::{
    Scheduler, TASK_DAILY_ENERGY, TASK_RUN_AUTOMATION, TaskRunner,
};
use dominet_domain::automation::{AutomationId, RunStatus, Trigger};
use dominet_domain::repo::AutomationRepo;
use dominet_domain::schedule::Job;
use dominet_store::{PgAutomationRuns, PgAutomations};

use crate::AppState;
use crate::automation::ServerExecutor;

type ServerEngine = Engine<PgAutomations, PgAutomationRuns, ServerExecutor>;

/// Taches des travaux : `run_automation` (declenche une automation planifiee) et `compute_daily_energy`.
#[derive(Debug)]
pub struct ServerRunner {
    engine: Arc<ServerEngine>,
    automations: PgAutomations,
    store: dominet_store::PgStore,
    energy: dominet_automation::energy::EnergyDevices,
    tz: chrono_tz::Tz,
}

impl ServerRunner {
    #[must_use]
    pub fn new(state: &AppState) -> Self {
        // Le planificateur publie et active des scenes par le client MQTT de ce processus, s'il y en a un.
        let (engine, _handle, _queue) = Engine::new(
            state.store.automations(),
            state.store.automation_runs(),
            ServerExecutor::new(state.clone()),
        );
        Self {
            engine,
            automations: state.store.automations(),
            store: state.store.clone(),
            energy: state.config.energy.clone(),
            tz: state
                .config
                .timezone
                .parse()
                .unwrap_or(chrono_tz::Europe::Paris),
        }
    }
}

/// Texte du journal pour un declenchement horaire ou solaire.
fn trigger_label(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Schedule { cron } => format!("schedule {cron}"),
        Trigger::Solar { event, .. } => format!("sun: {}", event.as_str()),
        other => other.kind().to_string(),
    }
}

impl ServerRunner {
    /// Bilan d'energie d'un jour (`args.date`, `AAAA-MM-JJ`) ou d'hier (jour local) par defaut.
    async fn daily_energy(&self, job: &Job) -> Result<String, String> {
        use dominet_automation::energy::{compute_and_store, yesterday};

        let day = match job
            .draft
            .args
            .get("date")
            .and_then(serde_json::Value::as_str)
        {
            Some(text) => text
                .parse::<chrono::NaiveDate>()
                .map_err(|_| format!("invalid date: {text}"))?,
            None => yesterday(self.tz, chrono::Utc::now()),
        };
        let d = compute_and_store(
            &self.store.devices(),
            &self.store.state_log(),
            &self.store.energy(),
            &self.energy,
            self.tz,
            day,
        )
        .await
        .map_err(|e| format!("balance failed: {e}"))?;
        Ok(format!(
            "balance of {day}: solar {} Wh, Linky {} Wh, heat pump {} Wh, net {:+} Wh",
            d.solar_wh,
            d.linky_total_wh(),
            d.pac_wh,
            d.net_wh()
        ))
    }
}

impl TaskRunner for ServerRunner {
    async fn run(&self, job: &Job) -> Result<String, String> {
        let started = std::time::Instant::now();
        let result = self.dispatch(job).await;
        crate::metrics::scheduler_task(&job.draft.task, result.is_ok(), started.elapsed());
        result
    }
}

impl ServerRunner {
    async fn dispatch(&self, job: &Job) -> Result<String, String> {
        match job.draft.task.as_str() {
            TASK_RUN_AUTOMATION => {
                let id = job
                    .draft
                    .args
                    .get("automation_id")
                    .and_then(serde_json::Value::as_i64)
                    .ok_or("automation_id absent des arguments")?;
                let automation = self
                    .automations
                    .get(AutomationId(id))
                    .await
                    .map_err(|e| format!("unreadable automation: {e}"))?
                    .filter(|a| a.draft.enabled)
                    .ok_or("automation deleted or disabled")?;
                let (status, detail) = self
                    .engine
                    .fire(
                        &automation,
                        &trigger_label(&automation.draft.trigger),
                        &FireContext::default(),
                    )
                    .await;
                match status {
                    RunStatus::Ok | RunStatus::Skipped => Ok(detail),
                    RunStatus::Error | RunStatus::Limited => Err(detail),
                }
            }
            TASK_DAILY_ENERGY => self.daily_energy(job).await,
            other => Err(format!("unknown task: {other}")),
        }
    }
}

/// Client MQTT propre au planificateur quand le role `mqtt` n'est pas lance dans ce processus.
///
/// # Errors
/// Erreur si `MQTT_HOST` n'est pas renseigne.
pub fn own_client(state: &AppState) -> anyhow::Result<dominet_mqtt::MqttClient> {
    use anyhow::Context;
    let mut cfg = state
        .config
        .mqtt
        .clone()
        .context("the scheduler needs MQTT_HOST to publish")?;
    cfg.client_id = format!("{}-scheduler", cfg.client_id);
    Ok(dominet_mqtt::start(&crate::mqtt::settings(&cfg)).client)
}

/// Demarre le planificateur (supervise par tokio : une panique ne touche pas les autres roles).
pub fn spawn(state: &AppState) {
    let tz = state.config.timezone.parse().unwrap_or_else(|_| {
        tracing::warn!(tz = %state.config.timezone, "unknown time zone: Europe/Paris");
        chrono_tz::Europe::Paris
    });
    let scheduler = Arc::new(Scheduler::new(
        state.store.schedule(),
        state.store.automations(),
        ServerRunner::new(state),
        tz,
    ));
    tokio::spawn(Arc::clone(&scheduler).run(state.schedule_changed.clone()));
    tracing::info!(tz = %tz.name(), "scheduler started");
}
