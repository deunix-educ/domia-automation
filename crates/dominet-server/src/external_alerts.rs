// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alertes de la plateforme vues par l'interface (R13 lot E1, ADR 0061) : celles de Prometheus et de Grafana, lues en lecture seule
//! pour que la page des alertes et la pastille de la barre laterale montrent **tout ce qui ne va pas**, pas seulement les alertes
//! d'appareil.
//!
//! Rien n'est ecrit en base : l'etat vient des services, relu toutes les `PROMETHEUS_ALERTS_POLL_SECS` secondes (60 par defaut).
//! Une alerte disparait de la liste quand sa cause disparait. Ce module ne notifie rien : l'e-mail et le SMS restent le travail du
//! relais de `prometheus_alerts` (Prometheus) et du point de contact de Grafana.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::Value;

use crate::AppState;
use crate::config::Config;

/// Qui a leve l'alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Prometheus,
    Grafana,
}

impl Source {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Source::Prometheus => "prometheus",
            Source::Grafana => "grafana",
        }
    }
}

/// Une alerte en cours chez Prometheus ou Grafana.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAlert {
    pub source: Source,
    pub name: String,
    /// `info`, `warning` ou `critical` (`warning` si l'etiquette manque ou est inconnue).
    pub severity: String,
    pub summary: String,
    pub description: String,
    /// Debut de l'alerte (RFC 3339), si la source le donne.
    pub since: Option<String>,
}

/// Ce que l'on sait d'une source a la derniere lecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    /// Pas d'adresse (ou pas d'identifiants pour Grafana) : la source n'est pas surveillee.
    NotConfigured,
    /// La derniere lecture a echoue : on ne sait rien, et la page doit le dire.
    Unreachable,
    Ok(Vec<ExternalAlert>),
}

/// Instantane des deux sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub prometheus: SourceState,
    pub grafana: SourceState,
}

impl Snapshot {
    /// Les alertes en cours, Prometheus d'abord, du plus grave au moins grave.
    #[must_use]
    pub fn alerts(&self) -> Vec<ExternalAlert> {
        let mut all: Vec<ExternalAlert> = [&self.prometheus, &self.grafana]
            .into_iter()
            .filter_map(|s| match s {
                SourceState::Ok(list) => Some(list.iter().cloned()),
                _ => None,
            })
            .flatten()
            .collect();
        all.sort_by_key(|a| severity_rank(&a.severity));
        all
    }

    #[must_use]
    pub fn firing(&self) -> u32 {
        [&self.prometheus, &self.grafana]
            .into_iter()
            .map(|s| match s {
                SourceState::Ok(list) => u32::try_from(list.len()).unwrap_or(u32::MAX),
                _ => 0,
            })
            .fold(0, u32::saturating_add)
    }
}

fn severity_rank(severity: &str) -> u8 {
    match severity {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn severity_of(labels: Option<&Value>) -> String {
    let raw = labels
        .and_then(|l| l.get("severity"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    match raw.as_str() {
        "info" | "warning" | "critical" => raw,
        _ => "warning".to_string(),
    }
}

fn text(v: Option<&Value>, key: &str) -> String {
    v.and_then(|m| m.get(key))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn build(
    source: Source,
    labels: Option<&Value>,
    annotations: Option<&Value>,
    since: Option<&str>,
) -> ExternalAlert {
    let name = text(labels, "alertname");
    let summary = text(annotations, "summary");
    ExternalAlert {
        source,
        name: if name.is_empty() {
            "alerte".into()
        } else {
            name
        },
        severity: severity_of(labels),
        summary,
        description: text(annotations, "description"),
        since: since.filter(|s| !s.is_empty()).map(str::to_string),
    }
}

/// Lit `/api/v1/alerts` de Prometheus (`{"data": {"alerts": [...]}}`) : seules les alertes `firing` comptent (`pending` : le delai
/// de la regle n'est pas ecoule). Une entree mal formee est ignoree.
#[must_use]
pub fn parse_prometheus(body: &Value) -> Vec<ExternalAlert> {
    body["data"]["alerts"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|a| a["state"].as_str() == Some("firing"))
                .map(|a| {
                    build(
                        Source::Prometheus,
                        a.get("labels"),
                        a.get("annotations"),
                        a["activeAt"].as_str(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lit `/api/alertmanager/grafana/api/v2/alerts` de Grafana (une liste d'alertes) : seules les alertes `active` comptent (ni
/// `suppressed`, ni `unprocessed`).
#[must_use]
pub fn parse_grafana(body: &Value) -> Vec<ExternalAlert> {
    body.as_array()
        .map(|list| {
            list.iter()
                .filter(|a| a["status"]["state"].as_str() == Some("active"))
                .map(|a| {
                    build(
                        Source::Grafana,
                        a.get("labels"),
                        a.get("annotations"),
                        a["startsAt"].as_str(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Compte de Grafana pour la lecture des alertes.
#[derive(Clone)]
struct GrafanaAccess {
    url: String,
    user: String,
    password: String,
}

/// Les alertes de la plateforme, lues en fond et partagees (clonable : un seul instantane derriere).
#[derive(Clone)]
pub struct ExternalAlerts {
    http: reqwest::Client,
    prometheus_url: Option<String>,
    grafana: Option<GrafanaAccess>,
    last: Arc<RwLock<Snapshot>>,
}

impl std::fmt::Debug for ExternalAlerts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalAlerts")
            .field("prometheus", &self.prometheus_url.is_some())
            .field("grafana", &self.grafana.is_some())
            .finish_non_exhaustive()
    }
}

impl ExternalAlerts {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let grafana = match (&config.grafana_url, &config.grafana_password) {
            (Some(url), Some(password)) => Some((
                url.clone(),
                config.grafana_user.clone(),
                password.expose().to_string(),
            )),
            _ => None,
        };
        Self::with_endpoints(config.prometheus_url.clone(), grafana)
    }

    /// `grafana` : adresse, compte, mot de passe.
    #[must_use]
    pub fn with_endpoints(
        prometheus_url: Option<String>,
        grafana: Option<(String, String, String)>,
    ) -> Self {
        let state = |on: bool| {
            if on {
                SourceState::Unreachable
            } else {
                SourceState::NotConfigured
            }
        };
        let snapshot = Snapshot {
            prometheus: state(prometheus_url.is_some()),
            grafana: state(grafana.is_some()),
        };
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            prometheus_url,
            grafana: grafana.map(|(url, user, password)| GrafanaAccess {
                url,
                user,
                password,
            }),
            last: Arc::new(RwLock::new(snapshot)),
        }
    }

    /// Au moins une source est surveillee.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.prometheus_url.is_some() || self.grafana.is_some()
    }

    /// Le dernier instantane lu.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.last
            .read()
            .map_or_else(|e| e.into_inner().clone(), |s| s.clone())
    }

    /// Nombre d'alertes en cours chez Prometheus et Grafana.
    #[must_use]
    pub fn firing(&self) -> u32 {
        self.snapshot().firing()
    }

    async fn get_json(&self, request: reqwest::RequestBuilder, what: &str) -> Option<Value> {
        match request
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(r) => match r.json::<Value>().await {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(error = %e.without_url(), "unreadable {what} alerts");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e.without_url(), "unreachable {what} alerts");
                None
            }
        }
    }

    /// Relit les deux sources ; renvoie vrai si quelque chose a change depuis la lecture precedente.
    pub async fn refresh(&self) -> bool {
        let prometheus = match &self.prometheus_url {
            None => SourceState::NotConfigured,
            Some(url) => {
                let req = self
                    .http
                    .get(format!("{}/api/v1/alerts", url.trim_end_matches('/')));
                match self.get_json(req, "Prometheus").await {
                    Some(body) => SourceState::Ok(parse_prometheus(&body)),
                    None => SourceState::Unreachable,
                }
            }
        };
        let grafana = match &self.grafana {
            None => SourceState::NotConfigured,
            Some(g) => {
                let req = self
                    .http
                    .get(format!(
                        "{}/api/alertmanager/grafana/api/v2/alerts",
                        g.url.trim_end_matches('/')
                    ))
                    .query(&[
                        ("active", "true"),
                        ("silenced", "false"),
                        ("inhibited", "false"),
                    ])
                    .basic_auth(&g.user, Some(&g.password));
                match self.get_json(req, "Grafana").await {
                    Some(body) => SourceState::Ok(parse_grafana(&body)),
                    None => SourceState::Unreachable,
                }
            }
        };
        let next = Snapshot {
            prometheus,
            grafana,
        };
        let mut guard = self
            .last
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = *guard != next;
        *guard = next;
        changed
    }
}

/// Demarre la lecture en fond (role `web` : c'est lui qui sert la page et la pastille). Sans Prometheus ni Grafana configures, rien
/// ne tourne. La pastille est rediffusee a chaque changement.
pub fn spawn(state: &AppState) {
    if !state.external_alerts.is_configured() {
        return;
    }
    let state = state.clone();
    let every = Duration::from_secs(u64::from(state.config.prometheus_alerts_poll_secs));
    tokio::spawn(async move {
        tracing::info!(
            poll_secs = every.as_secs(),
            "reading of Prometheus and Grafana alerts started"
        );
        loop {
            if state.external_alerts.refresh().await {
                crate::alerts::publish_count(&state.hub, &state.store, &state.external_alerts)
                    .await;
            }
            tokio::time::sleep(every).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_firing_prometheus_alerts_are_read_and_a_missing_severity_is_a_warning() {
        let body = json!({"data": {"alerts": [
            {"state": "firing", "activeAt": "2026-11-18T08:00:00Z",
             "labels": {"alertname": "MqttDown", "severity": "Critical"},
             "annotations": {"summary": "MQTT hors ligne", "description": "depuis 5 min"}},
            {"state": "pending", "labels": {"alertname": "Lent"}, "annotations": {}},
            {"state": "firing", "labels": {}, "annotations": {}},
            "pas une alerte"
        ]}});
        let alerts = parse_prometheus(&body);
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].name, "MqttDown");
        assert_eq!(alerts[0].severity, "critical");
        assert_eq!(alerts[0].since.as_deref(), Some("2026-11-18T08:00:00Z"));
        assert_eq!(alerts[1].name, "alerte");
        assert_eq!(alerts[1].severity, "warning");
    }

    #[test]
    fn only_active_grafana_alerts_are_read() {
        let body = json!([
            {"status": {"state": "active"}, "startsAt": "2026-11-18T09:00:00Z",
             "labels": {"alertname": "Canicule", "severity": "warning"},
             "annotations": {"summary": "36 degres"}},
            {"status": {"state": "suppressed"}, "labels": {"alertname": "Muette"}, "annotations": {}}
        ]);
        let alerts = parse_grafana(&body);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].source, Source::Grafana);
        assert_eq!(alerts[0].summary, "36 degres");
        assert_eq!(
            parse_grafana(&json!({"message": "erreur"})),
            Vec::<ExternalAlert>::new()
        );
    }

    #[test]
    fn the_worst_alerts_come_first_and_an_unreachable_source_counts_for_nothing() {
        let mk = |source, severity: &str| ExternalAlert {
            source,
            name: "x".into(),
            severity: severity.into(),
            summary: String::new(),
            description: String::new(),
            since: None,
        };
        let snap = Snapshot {
            prometheus: SourceState::Ok(vec![mk(Source::Prometheus, "info")]),
            grafana: SourceState::Ok(vec![
                mk(Source::Grafana, "critical"),
                mk(Source::Grafana, "warning"),
            ]),
        };
        let severities: Vec<String> = snap.alerts().into_iter().map(|a| a.severity).collect();
        assert_eq!(severities, ["critical", "warning", "info"]);
        assert_eq!(snap.firing(), 3);
        let down = Snapshot {
            prometheus: SourceState::Unreachable,
            grafana: SourceState::NotConfigured,
        };
        assert_eq!(down.firing(), 0);
        assert_eq!(down.alerts(), Vec::<ExternalAlert>::new());
    }
}
