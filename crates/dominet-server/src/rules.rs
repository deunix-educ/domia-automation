// SPDX-License-Identifier: AGPL-3.0-or-later
//! Prometheus and Grafana alert rules (R7 lot 2, ADR 0052): the rules edited in the administration are written as
//! provisioning files (`rules.yml`, `rules.yaml`) then the service concerned is invited to re-read them. As in the reference
//! , except that the writing is atomic (temporary file then rename).

use std::path::{Path, PathBuf};
use std::time::Duration;

use dominet_domain::observability::{
    GrafanaRule, PrometheusRule, grafana_rules_document, prometheus_rules_document, to_yaml,
};
use dominet_domain::repo::{GrafanaRuleRepo, PrometheusRuleRepo};
use dominet_store::PgStore;

use crate::config::Config;

const RELOAD_TIMEOUT: Duration = Duration::from_secs(5);

/// What happened for a service (Prometheus or Grafana).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    /// Neither a files folder (`*_RULES_PATH`) nor an address is configured: nothing to do.
    Disabled,
    /// File written; `reloaded` says whether the service took it into account (`None`: no address, reload to be done by hand).
    Done {
        groups: usize,
        reloaded: Option<bool>,
    },
    Failed(String),
}

impl SyncStatus {
    /// Everything went well (or nothing was requested).
    #[must_use]
    pub fn is_ok(&self) -> bool {
        match self {
            Self::Disabled => true,
            Self::Done { reloaded, .. } => *reloaded != Some(false),
            Self::Failed(_) => false,
        }
    }
}

/// Rules writing and reloading service.
#[derive(Debug, Clone)]
pub struct RulesService {
    store: PgStore,
    http: reqwest::Client,
    prometheus_path: Option<PathBuf>,
    prometheus_url: Option<String>,
    grafana_path: Option<PathBuf>,
    grafana_url: Option<String>,
    grafana_user: String,
    grafana_password: Option<String>,
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .map_or_else(|| "rules".into(), |n| n.to_string_lossy().into_owned());
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl RulesService {
    #[must_use]
    pub fn new(store: PgStore, config: &Config) -> Self {
        Self {
            store,
            http: reqwest::Client::builder()
                .timeout(RELOAD_TIMEOUT)
                .build()
                .unwrap_or_default(),
            prometheus_path: config.prometheus_rules_path.as_ref().map(PathBuf::from),
            prometheus_url: config.prometheus_url.clone(),
            grafana_path: config.grafana_alerting_path.as_ref().map(PathBuf::from),
            grafana_url: config.grafana_url.clone(),
            grafana_user: config.grafana_user.clone(),
            grafana_password: config
                .grafana_password
                .as_ref()
                .map(|p| p.expose().to_string()),
        }
    }

    /// Do the two services have what they need to be synchronised?
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.prometheus_path.is_some() || self.grafana_path.is_some()
    }

    /// Writes `rules.yml` from the database then reloads Prometheus (`POST /-/reload`).
    pub async fn sync_prometheus(&self) -> SyncStatus {
        let Some(path) = &self.prometheus_path else {
            return SyncStatus::Disabled;
        };
        let rules: Vec<PrometheusRule> = match self.store.prometheus_rules().list().await {
            Ok(r) => r,
            Err(e) => return SyncStatus::Failed(format!("unreadable rules: {e}")),
        };
        let doc = prometheus_rules_document(&rules);
        let groups = doc["groups"].as_array().map_or(0, Vec::len);
        if let Err(e) = write_atomically(path, &to_yaml(&doc)) {
            tracing::warn!(error = %e, path = %path.display(), "rules.yml not written");
            return SyncStatus::Failed(format!("cannot write {}: {e}", path.display()));
        }
        tracing::info!(groups, path = %path.display(), "Prometheus rules.yml written");
        let reloaded = match &self.prometheus_url {
            None => None,
            Some(url) => Some(
                self.post(&format!("{}/-/reload", url.trim_end_matches('/')), None)
                    .await,
            ),
        };
        SyncStatus::Done { groups, reloaded }
    }

    /// Writes `rules.yaml` from the database then reloads Grafana's alerting provisioning.
    pub async fn sync_grafana(&self) -> SyncStatus {
        let Some(path) = &self.grafana_path else {
            return SyncStatus::Disabled;
        };
        let rules: Vec<GrafanaRule> = match self.store.grafana_rules().list().await {
            Ok(r) => r,
            Err(e) => return SyncStatus::Failed(format!("unreadable rules: {e}")),
        };
        let doc = grafana_rules_document(&rules);
        let groups = doc["groups"].as_array().map_or(0, Vec::len);
        if let Err(e) = write_atomically(path, &to_yaml(&doc)) {
            tracing::warn!(error = %e, path = %path.display(), "rules.yaml not written");
            return SyncStatus::Failed(format!("cannot write {}: {e}", path.display()));
        }
        tracing::info!(groups, path = %path.display(), "Grafana rules.yaml written");
        let reloaded = match &self.grafana_url {
            None => None,
            Some(url) => {
                let endpoint = format!(
                    "{}/api/admin/provisioning/alerting/reload",
                    url.trim_end_matches('/')
                );
                Some(
                    self.post(
                        &endpoint,
                        Some((&self.grafana_user, self.grafana_password.as_deref())),
                    )
                    .await,
                )
            }
        };
        SyncStatus::Done { groups, reloaded }
    }

    /// Reload `POST`; never discloses the identifier or the password in the logs.
    async fn post(&self, url: &str, auth: Option<(&String, Option<&str>)>) -> bool {
        let mut req = self.http.post(url);
        if let Some((user, password)) = auth {
            req = req.basic_auth(user, password);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(url, "rule reload requested");
                true
            }
            Ok(resp) => {
                tracing::warn!(url, status = resp.status().as_u16(), "rule reload refused");
                false
            }
            Err(e) => {
                tracing::warn!(url, error = %e.without_url(), "rule reload failed");
                false
            }
        }
    }

    /// The two services, one after the other (after a modification, at start-up, `dominet sync-rules`).
    pub async fn sync_all(&self) -> (SyncStatus, SyncStatus) {
        (self.sync_prometheus().await, self.sync_grafana().await)
    }
}
