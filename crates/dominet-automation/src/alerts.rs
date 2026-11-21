// SPDX-License-Identifier: AGPL-3.0-or-later
//! Evaluation des regles d'alerte (R4 lot 6, ADR 0035) : le cycle des alertes, particularites comprises.
//!
//! Apres chaque message d'**etat** d'un appareil, chaque regle activee de cet appareil lit `etat[cle]` (absente ou `null` : on passe,
//! ni declenchement ni resolution). Condition vraie : une alerte est creee sauf si une alerte **en cours** (pas une acquittee !)
//! de la regle a ete declenchee depuis moins de `cooldown_minutes`. Condition fausse : les alertes en cours de la regle sont
//! resolues. Les envois (bandeau, e-mail, SMS, Android) passent par le port [`Notifier`].
//!
//! Comme pour les automations, une poignee ([`AlertHandle`]) ecarte a la source ce qu'aucune regle n'attend, une file bornee
//! alimente une seule boucle qui traite les etats dans l'ordre.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Duration, Utc};
use dominet_domain::alert::{Alert, AlertRule, AlertStatus, alert_messages, plain_str};
use dominet_domain::device::DeviceId;
use dominet_domain::repo::{AlertRepo, AlertRuleRepo, NewAlert};
use serde_json::Value;
use tokio::sync::mpsc;

/// Taille de la file entre l'ingestion et l'evaluation.
pub const QUEUE: usize = 1024;

/// Une alerte qui vient d'etre declenchee, avec ce qu'il faut pour la dire (le texte est celui de la reference).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertNotice {
    pub alert: Alert,
    pub rule: AlertRule,
    pub device_name: String,
    /// Texte long (e-mail).
    pub long: String,
    /// Texte court (SMS, Android, bandeau).
    pub short: String,
}

/// Port d'envoi des alertes. R4 : journal et bandeau dans l'interface ; R5 y a ajoute e-mail, SMS et Android selon les trois
/// canaux de la regle (`notify_email`, `notify_sms`, `notify_android`) et passe `notified` a vrai.
pub trait Notifier: Send + Sync + 'static {
    /// Une alerte vient d'etre declenchee.
    fn fired(&self, notice: &AlertNotice);
    /// Le nombre d'alertes en cours a pu changer (declenchement ou resolution) : a rediffuser aux interfaces.
    fn changed(&self);
}

/// Un etat d'appareil a evaluer.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertEvent {
    pub device: DeviceId,
    pub name: String,
    pub state: Value,
}

type RulesByDevice = HashMap<i64, Vec<AlertRule>>;

#[derive(Debug, Default)]
struct Shared {
    rules: RwLock<Arc<RulesByDevice>>,
    dropped: AtomicU64,
}

/// Poignee de l'ingestion.
#[derive(Debug, Clone)]
pub struct AlertHandle {
    shared: Arc<Shared>,
    tx: mpsc::Sender<AlertEvent>,
}

/// Cle d'etat lisible pour une regle : presente et non nulle.
fn watched(state: &Value, key: &str) -> bool {
    state.get(key).is_some_and(|v| !v.is_null())
}

impl AlertHandle {
    /// Un etat d'appareil (rien n'est empile si aucune regle de cet appareil ne regarde une cle presente).
    pub fn state(&self, device: DeviceId, name: &str, state: &Value) {
        let wanted = self.shared.rules.read().is_ok_and(|r| {
            r.get(&device.0)
                .is_some_and(|rules| rules.iter().any(|x| watched(state, &x.rule.key)))
        });
        if !wanted {
            return;
        }
        let event = AlertEvent {
            device,
            name: name.to_string(),
            state: state.clone(),
        };
        if self.tx.try_send(event).is_err() {
            let n = self.shared.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                tracing::warn!(ecartes = n, "alert queue full: state dropped");
            }
        }
    }

    /// Etats ecartes parce que la file etait pleine.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

/// Le moteur d'alertes.
pub struct AlertEngine<R, A, N> {
    rules: R,
    alerts: A,
    notifier: N,
    shared: Arc<Shared>,
    /// (regle, appareil) qui ont peut-etre une alerte en cours : evite une ecriture a chaque etat « faux ».
    maybe_firing: Mutex<HashSet<(i64, i64)>>,
    clock: Arc<Clock>,
}

impl<R, A, N> std::fmt::Debug for AlertEngine<R, A, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertEngine").finish_non_exhaustive()
    }
}

impl<R, A, N> AlertEngine<R, A, N>
where
    R: AlertRuleRepo + Send + Sync + 'static,
    A: AlertRepo + Send + Sync + 'static,
    N: Notifier,
{
    #[must_use]
    pub fn new(
        rules: R,
        alerts: A,
        notifier: N,
    ) -> (Arc<Self>, AlertHandle, mpsc::Receiver<AlertEvent>) {
        Self::with_clock(rules, alerts, notifier, Arc::new(Utc::now))
    }

    #[must_use]
    pub fn with_clock(
        rules: R,
        alerts: A,
        notifier: N,
        clock: Arc<Clock>,
    ) -> (Arc<Self>, AlertHandle, mpsc::Receiver<AlertEvent>) {
        let (tx, rx) = mpsc::channel(QUEUE);
        let shared = Arc::new(Shared::default());
        let engine = Arc::new(Self {
            rules,
            alerts,
            notifier,
            shared: Arc::clone(&shared),
            maybe_firing: Mutex::new(HashSet::new()),
            clock,
        });
        (engine, AlertHandle { shared, tx }, rx)
    }

    /// Relit les regles activees. Liste precedente gardee en cas d'erreur de lecture.
    pub async fn reload(&self) {
        match self.rules.list().await {
            Ok(all) => {
                let mut by_device: RulesByDevice = HashMap::new();
                for r in all.into_iter().filter(|r| r.rule.enabled) {
                    by_device.entry(r.rule.device.0).or_default().push(r);
                }
                if let Ok(mut slot) = self.shared.rules.write() {
                    *slot = Arc::new(by_device);
                }
            }
            Err(e) => tracing::warn!(error = %e, "unreadable alert rules: list unchanged"),
        }
    }

    /// Au demarrage : retient les couples (regle, appareil) qui ont une alerte en cours, pour que leur resolution soit vue.
    pub async fn load_firing(&self) {
        match self.alerts.list(Some(AlertStatus::Firing), 5000).await {
            Ok(list) => {
                if let Ok(mut set) = self.maybe_firing.lock() {
                    set.extend(
                        list.iter()
                            .filter_map(|a| a.rule.map(|r| (r.0, a.device.0))),
                    );
                }
            }
            Err(e) => tracing::warn!(error = %e, "unreadable active alerts at start-up"),
        }
    }

    /// Boucle : un etat apres l'autre jusqu'a la fermeture de la file.
    pub async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<AlertEvent>) {
        while let Some(event) = rx.recv().await {
            self.process(&event).await;
        }
    }

    /// Evalue les regles de l'appareil contre cet etat.
    pub async fn process(&self, event: &AlertEvent) {
        let rules: Vec<AlertRule> = self
            .shared
            .rules
            .read()
            .map(|r| r.get(&event.device.0).cloned().unwrap_or_default())
            .unwrap_or_default();
        let mut changed = false;
        for rule in rules {
            let Some(value) = event.state.get(&rule.rule.key).filter(|v| !v.is_null()) else {
                continue;
            };
            changed |= if rule.rule.evaluate(value) {
                self.maybe_fire(&rule, event, value).await
            } else {
                self.maybe_resolve(&rule, event.device).await
            };
        }
        if changed {
            self.notifier.changed();
        }
    }

    /// Cree l'alerte sauf si une alerte en cours est plus recente que le cooldown. Renvoie vrai si une alerte a ete creee.
    async fn maybe_fire(&self, rule: &AlertRule, event: &AlertEvent, value: &Value) -> bool {
        let now = (self.clock)();
        let cooldown = Duration::minutes(i64::from(rule.rule.cooldown_minutes));
        match self.alerts.latest_firing(rule.id, event.device).await {
            Ok(Some(recent)) if now - recent.fired_at < cooldown => return false,
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(rule = %rule.rule.name, error = %e, "unreadable active alert");
                return false;
            }
        }
        let shown = plain_str(value);
        let alert = match self
            .alerts
            .create(&NewAlert {
                rule: rule.id,
                device: event.device,
                severity: rule.rule.severity,
                value_triggered: shown.clone(),
                fired_at: now,
            })
            .await
        {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(rule = %rule.rule.name, error = %e, "alert not saved");
                return false;
            }
        };
        if let Ok(mut set) = self.maybe_firing.lock() {
            set.insert((rule.id.0, event.device.0));
        }
        tracing::warn!(rule = %rule.rule.name, value = %shown, "alert fired");
        let texts = alert_messages(
            &rule.rule.name,
            rule.rule.severity,
            &event.name,
            &shown,
            now,
        );
        self.notifier.fired(&AlertNotice {
            alert,
            rule: rule.clone(),
            device_name: event.name.clone(),
            long: texts.long,
            short: texts.short,
        });
        true
    }

    /// Resout les alertes en cours de la regle pour cet appareil. Renvoie vrai si au moins une l'a ete.
    async fn maybe_resolve(&self, rule: &AlertRule, device: DeviceId) -> bool {
        let key = (rule.id.0, device.0);
        let known = self.maybe_firing.lock().is_ok_and(|s| s.contains(&key));
        if !known {
            return false;
        }
        match self
            .alerts
            .resolve_firing(rule.id, device, (self.clock)())
            .await
        {
            Ok(n) => {
                if let Ok(mut set) = self.maybe_firing.lock() {
                    set.remove(&key);
                }
                n > 0
            }
            Err(e) => {
                tracing::warn!(rule = %rule.rule.name, error = %e, "resolution impossible");
                false
            }
        }
    }
}
