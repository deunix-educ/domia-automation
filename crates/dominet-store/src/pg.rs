// SPDX-License-Identifier: AGPL-3.0-or-later
//! PostgreSQL entry point: one access per domain (`users()`, `settings()`, `navbar()`, `locations()`).
//!
//! One object per port avoids ambiguous calls (`create`, `list`, `get`... exist in several ports).

use dominet_domain::RepoError;
use sqlx::PgPool;

use crate::alerts::{PgAlertRules, PgAlerts, PgEnergy};
use crate::android::{PgAndroidDevices, PgCalls, PgMessages, PgSms, PgSmsOutbox};
use crate::attempts::PgAttempts;
use crate::automation::{PgAutomationRuns, PgAutomations, PgScenes};
use crate::cameras::{
    PgCameraEvents, PgCameras, PgDetectorConfig, PgPtzPresets, PgRecordings, PgSnapshots,
};
use crate::dashboards::{PgDashboards, PgWidgets};
use crate::devices::{PgDeviceGroups, PgDeviceImages, PgDeviceLabels, PgDevices, PgStateLog};
use crate::locations::PgLocations;
use crate::navbar::PgNavbar;
use crate::sessions::PgSessionStore;
use crate::settings::PgSettings;
use crate::sources::{PgMqttSources, PgWeather};
use crate::users::PgUsers;

#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    #[must_use]
    pub fn android_devices(&self) -> PgAndroidDevices {
        PgAndroidDevices::new(self.pool.clone())
    }

    #[must_use]
    pub fn sms(&self) -> PgSms {
        PgSms::new(self.pool.clone())
    }

    #[must_use]
    pub fn calls(&self) -> PgCalls {
        PgCalls::new(self.pool.clone())
    }

    #[must_use]
    pub fn sms_outbox(&self) -> PgSmsOutbox {
        PgSmsOutbox::new(self.pool.clone())
    }

    #[must_use]
    pub fn messages(&self) -> PgMessages {
        PgMessages::new(self.pool.clone())
    }

    #[must_use]
    pub fn cameras(&self) -> PgCameras {
        PgCameras::new(self.pool.clone())
    }

    #[must_use]
    pub fn ptz_presets(&self) -> PgPtzPresets {
        PgPtzPresets::new(self.pool.clone())
    }

    #[must_use]
    pub fn recordings(&self) -> PgRecordings {
        PgRecordings::new(self.pool.clone())
    }

    #[must_use]
    pub fn snapshots(&self) -> PgSnapshots {
        PgSnapshots::new(self.pool.clone())
    }

    #[must_use]
    pub fn camera_events(&self) -> PgCameraEvents {
        PgCameraEvents::new(self.pool.clone())
    }

    #[must_use]
    pub fn detector_config(&self) -> PgDetectorConfig {
        PgDetectorConfig::new(self.pool.clone())
    }

    #[must_use]
    pub fn users(&self) -> PgUsers {
        PgUsers::new(self.pool.clone())
    }

    #[must_use]
    pub fn settings(&self) -> PgSettings {
        PgSettings::new(self.pool.clone())
    }

    #[must_use]
    pub fn navbar(&self) -> PgNavbar {
        PgNavbar::new(self.pool.clone())
    }

    #[must_use]
    pub fn prometheus_rules(&self) -> crate::PgPrometheusRules {
        crate::PgPrometheusRules::new(self.pool.clone())
    }

    #[must_use]
    pub fn grafana_rules(&self) -> crate::PgGrafanaRules {
        crate::PgGrafanaRules::new(self.pool.clone())
    }

    #[must_use]
    pub fn locations(&self) -> PgLocations {
        PgLocations::new(self.pool.clone())
    }

    #[must_use]
    pub fn dashboards(&self) -> PgDashboards {
        PgDashboards::new(self.pool.clone())
    }

    #[must_use]
    pub fn widgets(&self) -> PgWidgets {
        PgWidgets::new(self.pool.clone())
    }

    #[must_use]
    pub fn devices(&self) -> PgDevices {
        PgDevices::new(self.pool.clone())
    }

    #[must_use]
    pub fn device_groups(&self) -> PgDeviceGroups {
        PgDeviceGroups::new(self.pool.clone())
    }

    #[must_use]
    pub fn device_labels(&self) -> PgDeviceLabels {
        PgDeviceLabels::new(self.pool.clone())
    }

    #[must_use]
    pub fn state_log(&self) -> PgStateLog {
        PgStateLog::new(self.pool.clone())
    }

    #[must_use]
    pub fn device_images(&self) -> PgDeviceImages {
        PgDeviceImages::new(self.pool.clone())
    }

    #[must_use]
    pub fn mqtt_sources(&self) -> PgMqttSources {
        PgMqttSources::new(self.pool.clone())
    }

    #[must_use]
    pub fn weather(&self) -> PgWeather {
        PgWeather::new(self.pool.clone())
    }

    #[must_use]
    pub fn scenes(&self) -> PgScenes {
        PgScenes::new(self.pool.clone())
    }

    #[must_use]
    pub fn automations(&self) -> PgAutomations {
        PgAutomations::new(self.pool.clone())
    }

    #[must_use]
    pub fn schedule(&self) -> crate::schedule::PgSchedule {
        crate::schedule::PgSchedule::new(self.pool.clone())
    }

    #[must_use]
    pub fn automation_runs(&self) -> PgAutomationRuns {
        PgAutomationRuns::new(self.pool.clone())
    }

    #[must_use]
    pub fn alert_rules(&self) -> PgAlertRules {
        PgAlertRules::new(self.pool.clone())
    }

    #[must_use]
    pub fn mcp_tokens(&self) -> crate::mcp::PgMcpTokens {
        crate::mcp::PgMcpTokens::new(self.pool.clone())
    }

    #[must_use]
    pub fn mcp_audit(&self) -> crate::mcp::PgMcpAudit {
        crate::mcp::PgMcpAudit::new(self.pool.clone())
    }

    #[must_use]
    pub fn alerts(&self) -> PgAlerts {
        PgAlerts::new(self.pool.clone())
    }

    #[must_use]
    pub fn energy(&self) -> PgEnergy {
        PgEnergy::new(self.pool.clone())
    }

    #[must_use]
    pub fn attempts(&self) -> PgAttempts {
        PgAttempts::new(self.pool.clone())
    }

    #[must_use]
    pub fn sessions(&self) -> PgSessionStore {
        PgSessionStore::new(self.pool.clone())
    }
}

/// Translates a `sqlx` error into a repository error (without leaking SQL to the caller).
pub(crate) fn repo_err(err: sqlx::Error) -> RepoError {
    match &err {
        sqlx::Error::RowNotFound => RepoError::NotFound,
        sqlx::Error::Database(db)
            if db.is_unique_violation()
                || db.is_foreign_key_violation()
                || matches!(db.kind(), sqlx::error::ErrorKind::ExclusionViolation) =>
        {
            RepoError::Conflict {
                constraint: db.constraint().unwrap_or("unknown").to_string(),
            }
        }
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
            RepoError::Unavailable
        }
        _ => RepoError::Backend(Box::new(err)),
    }
}

/// Error for invalid stored data (e.g. unknown role): a bug or a database edited by hand.
pub(crate) fn corrupt(what: impl std::fmt::Display) -> RepoError {
    RepoError::Backend(format!("invalid stored data: {what}").into())
}
