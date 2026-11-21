// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alertes de Prometheus relayees par e-mail et SMS (R7 lot 3, ADR 0053) : port de `check_prometheus_alerts` de la reference
//!. Toutes les `PROMETHEUS_ALERTS_POLL_SECS` secondes, on lit `/api/v1/alerts` ; une alerte `firing`
//! **nouvelle** est notifiee (e-mail, et SMS si elle est `critical`) puis tue pendant `PROMETHEUS_ALERT_COOLDOWN` (1 h).
//!
//! Deux ecarts voulus avec reference, qui venaient de defauts de la version d'origine (ADR 0053) :
//! - l'identite d'une alerte est tiree de **ses etiquettes** : l'API d'alertes de Prometheus ne fournit pas de `fingerprint`, si
//!   bien que reference donnait la meme cle a toutes les alertes et que la deuxieme alerte passait sous silence pendant une heure ;
//! - une alerte notifiee qui **disparait** de la liste (Prometheus ne renvoie jamais l'etat `resolved`) envoie le message « RESOLVED »
//!   que reference prevoyait mais ne pouvait jamais envoyer.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::AppState;

/// Severites qui declenchent aussi un SMS (`_SMS_SEVERITIES` de la reference).
const SMS_SEVERITIES: [&str; 1] = ["critical"];

/// Une alerte lue sur `/api/v1/alerts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromAlert {
    pub state: String,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
}

impl PromAlert {
    fn label(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }

    /// Identite stable : toutes les etiquettes, triees.
    fn key(&self) -> String {
        self.labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Lit la reponse de `/api/v1/alerts` (`{"data": {"alerts": [...]}}`). Une alerte mal formee est ignoree.
#[must_use]
pub fn parse_alerts(body: &Value) -> Vec<PromAlert> {
    let strings = |v: Option<&Value>| -> BTreeMap<String, String> {
        v.and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    body["data"]["alerts"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|a| PromAlert {
                    state: a["state"].as_str().unwrap_or_default().to_string(),
                    labels: strings(a.get("labels")),
                    annotations: strings(a.get("annotations")),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Message envoye (la reference) : titre, resume, description eventuelle.
#[must_use]
pub fn build_message(
    name: &str,
    severity: &str,
    summary: &str,
    description: &str,
    status: &str,
) -> String {
    let mut lines = vec![
        format!("[DomiNet — {status}] {name} ({})", severity.to_uppercase()),
        summary.to_string(),
    ];
    if !description.is_empty() {
        lines.push(description.to_string());
    }
    lines.join("\n")
}

/// Ce qu'il faut envoyer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub message: String,
    pub sms: bool,
}

/// Memoire des alertes deja notifiees (equivalent du cache de la reference) et decision d'envoi, sans entree-sortie : l'horloge est
/// fournie, ce qui la rend testable.
#[derive(Debug)]
pub struct AlertMonitor {
    cooldown: Duration,
    /// Cle d'alerte -> (expiration du silence, message de resolution a envoyer si elle disparait).
    notified: BTreeMap<String, (Instant, Notification)>,
}

impl AlertMonitor {
    #[must_use]
    pub fn new(cooldown: Duration) -> Self {
        Self {
            cooldown,
            notified: BTreeMap::new(),
        }
    }

    /// Passage sur la liste courante : les notifications a envoyer, dans l'ordre.
    pub fn check(&mut self, alerts: &[PromAlert], now: Instant) -> Vec<Notification> {
        let mut out = vec![];
        // Le silence d'une alerte expire apres le delai (Reference : `timeout` du cache) : elle sera renotifiee si elle dure.
        self.notified.retain(|_, (until, _)| *until > now);
        let mut present = vec![];
        for alert in alerts {
            let name = alert.label("alertname").unwrap_or("alert");
            let severity = alert.label("severity").unwrap_or("warning");
            let summary = alert
                .annotations
                .get("summary")
                .map_or(name, String::as_str);
            let description = alert
                .annotations
                .get("description")
                .map_or("", String::as_str);
            let key = alert.key();
            match alert.state.as_str() {
                "firing" => {
                    present.push(key.clone());
                    if !self.notified.contains_key(&key) {
                        let notification = Notification {
                            message: build_message(name, severity, summary, description, "FIRING"),
                            sms: SMS_SEVERITIES.contains(&severity),
                        };
                        let resolved = Notification {
                            message: build_message(
                                name,
                                severity,
                                summary,
                                description,
                                "RESOLVED",
                            ),
                            sms: false,
                        };
                        self.notified.insert(key, (now + self.cooldown, resolved));
                        out.push(notification);
                    }
                }
                "resolved" => {
                    if let Some((_, resolved)) = self.notified.remove(&key) {
                        out.push(resolved);
                    }
                }
                _ => present.push(key),
            }
        }
        // Une alerte notifiee qui n'est plus dans la liste est terminee.
        let gone: Vec<String> = self
            .notified
            .keys()
            .filter(|k| !present.contains(k))
            .cloned()
            .collect();
        for key in gone {
            if let Some((_, resolved)) = self.notified.remove(&key) {
                out.push(resolved);
            }
        }
        out
    }
}

/// Lit `/api/v1/alerts` puis notifie ; `None` si Prometheus ne repond pas (journalise, on reessaie au prochain passage).
async fn fetch(client: &reqwest::Client, url: &str) -> Option<Vec<PromAlert>> {
    let resp = client
        .get(format!("{}/api/v1/alerts", url.trim_end_matches('/')))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status);
    match resp {
        Ok(r) => match r.json::<Value>().await {
            Ok(body) => Some(parse_alerts(&body)),
            Err(e) => {
                tracing::warn!(error = %e.without_url(), "unreadable Prometheus alerts");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e.without_url(), "unreachable Prometheus alerts");
            None
        }
    }
}

/// Demarre le sondage (rôle `scheduler`) ; sans `PROMETHEUS_URL`, rien ne tourne.
pub fn spawn(state: &AppState) {
    let Some(url) = state.config.prometheus_url.clone() else {
        return;
    };
    let channels = state.channels.clone();
    let every = Duration::from_secs(u64::from(state.config.prometheus_alerts_poll_secs));
    let mut monitor = AlertMonitor::new(Duration::from_secs(u64::from(
        state.config.prometheus_alert_cooldown_secs,
    )));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    tokio::spawn(async move {
        tracing::info!(
            poll_secs = every.as_secs(),
            "Prometheus alert polling started"
        );
        loop {
            if let Some(alerts) = fetch(&client, &url).await {
                crate::metrics::prometheus_alerts_firing(
                    alerts.iter().filter(|a| a.state == "firing").count(),
                );
                for n in monitor.check(&alerts, Instant::now()) {
                    let failures = channels
                        .notify_with_subject("[DomiNet] Alert", &n.message, n.sms)
                        .await;
                    for (channel, why) in failures {
                        tracing::warn!(channel, error = %why, "Prometheus alert: send failed");
                    }
                    tracing::info!(sms = n.sms, "Prometheus alert notified");
                }
            }
            tokio::time::sleep(every).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(
        state: &str,
        name: &str,
        severity: &str,
        summary: &str,
        description: &str,
        job: &str,
    ) -> PromAlert {
        let mut labels = BTreeMap::from([
            ("alertname".to_string(), name.to_string()),
            ("job".to_string(), job.to_string()),
        ]);
        if !severity.is_empty() {
            labels.insert("severity".into(), severity.into());
        }
        let mut annotations = BTreeMap::new();
        if !summary.is_empty() {
            annotations.insert("summary".into(), summary.into());
        }
        if !description.is_empty() {
            annotations.insert("description".into(), description.into());
        }
        PromAlert {
            state: state.into(),
            labels,
            annotations,
        }
    }

    #[test]
    fn two_different_alerts_are_both_notified_and_pending_ones_are_not() {
        let mut m = AlertMonitor::new(Duration::from_secs(3600));
        let t0 = Instant::now();
        let got = m.check(
            &[
                alert("firing", "A", "critical", "A en panne", "", "x"),
                alert("firing", "B", "warning", "B lent", "p95 = 3 s", "y"),
                alert("pending", "C", "warning", "C bientot", "", "z"),
            ],
            t0,
        );
        assert_eq!(
            got.len(),
            2,
            "l'alerte en attente (`pending`) n'est pas notifiee"
        );
        assert!(got[0].sms && !got[1].sms);
        assert_eq!(
            got[1].message,
            "[DomiNet — FIRING] B (WARNING)\nB lent\np95 = 3 s"
        );
    }

    #[test]
    fn an_alert_is_silent_for_the_cooldown_then_notified_again_if_it_lasts() {
        let mut m = AlertMonitor::new(Duration::from_secs(3600));
        let t0 = Instant::now();
        let a = [alert("firing", "A", "warning", "s", "", "x")];
        assert_eq!(m.check(&a, t0).len(), 1);
        assert_eq!(m.check(&a, t0 + Duration::from_secs(60)).len(), 0);
        assert_eq!(m.check(&a, t0 + Duration::from_secs(3599)).len(), 0);
        assert_eq!(
            m.check(&a, t0 + Duration::from_secs(3601)).len(),
            1,
            "le silence a expire"
        );
    }

    #[test]
    fn a_notified_alert_that_disappears_is_announced_resolved_once_without_sms() {
        let mut m = AlertMonitor::new(Duration::from_secs(3600));
        let t0 = Instant::now();
        let a = alert(
            "firing",
            "Down",
            "critical",
            "Hors ligne",
            "depuis 1 min",
            "x",
        );
        m.check(std::slice::from_ref(&a), t0);
        let gone = m.check(&[], t0 + Duration::from_secs(120));
        assert_eq!(gone.len(), 1);
        assert_eq!(
            gone[0].message,
            "[DomiNet — RESOLVED] Down (CRITICAL)\nHors ligne\ndepuis 1 min"
        );
        assert!(!gone[0].sms, "pas de SMS pour une resolution");
        assert!(
            m.check(&[], t0 + Duration::from_secs(180)).is_empty(),
            "une seule fois"
        );
        // Une alerte jamais notifiee qui disparait ne dit rien.
        assert_eq!(
            m.check(&[alert("pending", "P", "warning", "s", "", "x")], t0)
                .len(),
            0
        );
        assert_eq!(m.check(&[], t0).len(), 0);
    }

    #[test]
    fn a_resolved_state_in_the_list_is_announced_like_reference_does() {
        let mut m = AlertMonitor::new(Duration::from_secs(3600));
        let t0 = Instant::now();
        m.check(&[alert("firing", "A", "warning", "s", "", "x")], t0);
        let out = m.check(&[alert("resolved", "A", "warning", "s", "", "x")], t0);
        assert_eq!(out.len(), 1);
        assert!(
            out[0]
                .message
                .starts_with("[DomiNet — RESOLVED] A (WARNING)")
        );
    }

    #[test]
    fn missing_labels_fall_back_like_reference() {
        let mut m = AlertMonitor::new(Duration::from_secs(60));
        let bare = PromAlert {
            state: "firing".into(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        };
        let out = m.check(&[bare], Instant::now());
        assert_eq!(out[0].message, "[DomiNet — FIRING] alert (WARNING)\nalert");
        assert!(!out[0].sms);
    }

    #[test]
    fn the_prometheus_answer_is_read_and_malformed_alerts_are_skipped() {
        let body = serde_json::json!({"status": "success", "data": {"alerts": [
            {"labels": {"alertname": "A", "severity": "info"}, "annotations": {"summary": "s"}, "state": "firing", "value": "1e+00"},
            {"labels": "pas un objet", "state": "firing"},
        ]}});
        let alerts = parse_alerts(&body);
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].label("alertname"), Some("A"));
        assert!(alerts[1].labels.is_empty());
        assert_eq!(parse_alerts(&serde_json::json!({"nope": 1})).len(), 0);
    }
}
