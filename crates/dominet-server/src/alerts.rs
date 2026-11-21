// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alertes cote serveur (R4 lot 6, ADR 0035 ; canaux reels R5 lot 6, ADR 0042) : envoi (bandeau dans l'interface, e-mail, SMS,
//! Android) et diffusion du nombre d'alertes en cours.

use dominet_automation::alerts::{AlertNotice, Notifier};
use dominet_domain::alert::Severity;
use dominet_domain::repo::AlertRepo;
use dominet_proto::{NotificationLevel, Topic, WsServerMsg};
use dominet_store::PgStore;

use crate::AppState;
use crate::android::AndroidRelay;
use crate::external_alerts::ExternalAlerts;
use crate::hub::Hub;
use crate::notify::{Channels, record};

/// Lit le nombre d'alertes en cours (appareils, plus Prometheus et Grafana) et le diffuse (dernier etat conserve : la pastille
/// s'affiche des l'arrivee).
pub async fn publish_count(hub: &Hub, store: &PgStore, external: &ExternalAlerts) {
    match store.alerts().count_firing().await {
        Ok(n) => {
            hub.publish_retained(
                &Topic::Alerts,
                WsServerMsg::AlertCount {
                    firing: u32::try_from(n)
                        .unwrap_or(u32::MAX)
                        .saturating_add(external.firing()),
                },
            );
        }
        Err(e) => tracing::warn!(error = %e, "unreadable current alert count"),
    }
}

/// Le port `Notifier` : bandeau dans l'interface de tous les comptes connectes, puis les canaux de la regle (plan R5, D8) :
/// e-mail au destinataire des alertes, SMS Free Mobile, push `alert_notification` a tous les appareils Android en ligne. Chaque
/// canal est tente meme si un autre echoue (les echecs sont journalises) ; `notified` passe a vrai apres l'essai, comme la reference.
#[derive(Debug, Clone)]
pub struct AlertNotifier {
    hub: Hub,
    store: PgStore,
    channels: Channels,
    android: AndroidRelay,
    external: ExternalAlerts,
}

impl AlertNotifier {
    #[must_use]
    pub fn new(state: &AppState) -> Self {
        Self {
            hub: state.hub.clone(),
            store: state.store.clone(),
            channels: state.channels.clone(),
            android: state.android.clone(),
            external: state.external_alerts.clone(),
        }
    }

    /// Sert les canaux de la regle puis marque l'alerte `notified`. Renvoie les echecs `(canal, raison)`.
    pub async fn deliver(&self, notice: &AlertNotice) -> Vec<(&'static str, String)> {
        let rule = &notice.rule.rule;
        let mut failures = vec![];
        if rule.notify_email {
            let subject = format!("[DomiNet Alert {}] {}", rule.severity.label(), rule.name);
            record(
                &mut failures,
                "email",
                self.channels.send_alert_email(&subject, &notice.long).await,
            );
        }
        if rule.notify_sms {
            record(
                &mut failures,
                "sms",
                self.channels.send_sms_free(&notice.short).await,
            );
        }
        if rule.notify_android {
            let reached = self.android.push_alert(
                notice.alert.id.0,
                &rule.name,
                rule.severity.as_str(),
                &notice.short,
            );
            tracing::info!(appareils = reached, "alert pushed to the Android devices");
        }
        if let Err(e) = self.store.alerts().mark_notified(notice.alert.id).await {
            tracing::warn!(alert = notice.alert.id.0, error = %e, "alert not marked as notified");
        }
        failures
    }
}

fn level(severity: Severity) -> NotificationLevel {
    match severity {
        Severity::Info => NotificationLevel::Info,
        Severity::Warning => NotificationLevel::Warning,
        Severity::Critical => NotificationLevel::Error,
    }
}

impl Notifier for AlertNotifier {
    fn fired(&self, notice: &AlertNotice) {
        tracing::warn!(
            rule = %notice.rule.rule.name,
            device = %notice.device_name,
            email = notice.rule.rule.notify_email,
            sms = notice.rule.rule.notify_sms,
            android = notice.rule.rule.notify_android,
            "alert: {}",
            notice.short
        );
        self.hub.publish(
            &Topic::Notifications,
            WsServerMsg::Notification {
                level: level(notice.rule.rule.severity),
                text: notice.short.chars().take(200).collect(),
            },
        );
        // Les envois peuvent durer (SMTP, Free Mobile) : l'evaluation des alertes ne les attend pas.
        let (this, notice) = (self.clone(), notice.clone());
        tokio::spawn(async move {
            this.deliver(&notice).await;
        });
    }

    fn changed(&self) {
        let (hub, store, external) = (self.hub.clone(), self.store.clone(), self.external.clone());
        tokio::spawn(async move { publish_count(&hub, &store, &external).await });
    }
}
