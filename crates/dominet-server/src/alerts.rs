// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side alerts (R4 lot 6, ADR 0035; real channels R5 lot 6, ADR 0042): sending (banner in the interface, e-mail, SMS,
//! Android) and broadcast of the number of ongoing alerts.

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

/// Reads the number of ongoing alerts (devices, plus Prometheus and Grafana) and broadcasts it (last state kept: the badge
/// shows as soon as the page arrives).
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

/// The `Notifier` port: banner in the interface of all the logged-in accounts, then the rule's channels (plan R5, D8):
/// e-mail to the alerts recipient, Free Mobile SMS, `alert_notification` push to all the online Android devices. Each
/// channel is attempted even if another fails (the failures are logged); `notified` becomes true after the attempt, like the reference.
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

    /// Serves the rule's channels then marks the alert `notified`. Returns the failures `(channel, reason)`.
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
        // Sending can take a while (SMTP, Free Mobile): the alert evaluation does not wait for it.
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
