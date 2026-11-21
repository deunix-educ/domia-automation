// SPDX-License-Identifier: AGPL-3.0-or-later
//! Travaux du planificateur (R4 lot 4, ADR 0033) : table `schedule_job` (migration 0006, remplace un planificateur externe).
//!
//! Un travail a un horaire (intervalle, cron a cinq champs dans un fuseau, ou evenement solaire), une tache a executer et ses
//! arguments. Ceux des automations `schedule` / `solar` portent leur `automation` (supprimes avec elle) ; les travaux du
//! systeme (`compute_daily_energy`) n'en ont pas.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::automation::{AutomationId, SolarEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalPeriod {
    Minutes,
    Hours,
    Days,
}

impl IntervalPeriod {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Minutes, Self::Hours, Self::Days]
            .into_iter()
            .find(|p| p.as_str() == s)
    }

    /// Duree d'une unite, en secondes.
    #[must_use]
    pub fn seconds(self) -> i64 {
        match self {
            Self::Minutes => 60,
            Self::Hours => 3600,
            Self::Days => 86_400,
        }
    }
}

/// Quand un travail s'execute.
#[derive(Debug, Clone, PartialEq)]
pub enum JobSchedule {
    /// Toutes les `every` minutes / heures / jours.
    Interval { every: u32, period: IntervalPeriod },
    /// Expression cron a cinq champs, a l'heure du fuseau du travail.
    Cron(String),
    Solar {
        event: SolarEvent,
        latitude: f64,
        longitude: f64,
    },
}

/// Un travail a enregistrer.
#[derive(Debug, Clone, PartialEq)]
pub struct JobDraft {
    pub name: String,
    pub task: String,
    pub args: Value,
    pub schedule: JobSchedule,
    /// Nom IANA (`Europe/Paris`).
    pub timezone: String,
    pub enabled: bool,
    pub automation: Option<AutomationId>,
}

/// Un travail enregistre.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: JobId,
    pub draft: JobDraft,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_status: String,
}
