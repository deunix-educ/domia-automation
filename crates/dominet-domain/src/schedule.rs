// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scheduler jobs (R4 lot 4, ADR 0033): `schedule_job` table (migration 0006, replaces an external scheduler).
//!
//! A job has a schedule (interval, five-field cron in a time zone, or solar event), a task to run and its
//! arguments. Those of the `schedule` / `solar` automations carry their `automation` (deleted with it); the system
//! jobs (`compute_daily_energy`) have none.

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

    /// Duration of a unit, in seconds.
    #[must_use]
    pub fn seconds(self) -> i64 {
        match self {
            Self::Minutes => 60,
            Self::Hours => 3600,
            Self::Days => 86_400,
        }
    }
}

/// When a job runs.
#[derive(Debug, Clone, PartialEq)]
pub enum JobSchedule {
    /// Every `every` minutes / hours / days.
    Interval { every: u32, period: IntervalPeriod },
    /// Five-field cron expression, in the job's time zone.
    Cron(String),
    Solar {
        event: SolarEvent,
        latitude: f64,
        longitude: f64,
    },
}

/// A job to save.
#[derive(Debug, Clone, PartialEq)]
pub struct JobDraft {
    pub name: String,
    pub task: String,
    pub args: Value,
    pub schedule: JobSchedule,
    /// IANA name (`Europe/Paris`).
    pub timezone: String,
    pub enabled: bool,
    pub automation: Option<AutomationId>,
}

/// A saved job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: JobId,
    pub draft: JobDraft,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_status: String,
}
