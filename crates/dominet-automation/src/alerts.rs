// SPDX-License-Identifier: AGPL-3.0-or-later
//! Evaluation of the alert rules (R4 lot 6, ADR 0035): the alert cycle, quirks included.
//!
//! After each device **state** message, each enabled rule of that device reads `state[key]` (absent or `null`: skipped,
//! neither trigger nor resolution). Condition true: an alert is created unless an **ongoing** alert (not an acknowledged one!)
//! of the rule was raised less than `cooldown_minutes` ago. Condition false: the ongoing alerts of the rule are
//! resolved. Sends (banner, e-mail, SMS, Android) go through the [`Notifier`] port.
//!
//! As for the automations, a handle ([`AlertHandle`]) discards at the source what no rule expects, a bounded queue
//! feeds a single loop that processes the states in order.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Duration, Utc};
use dominet_domain::alert::{Alert, AlertRule, AlertStatus, alert_messages, plain_str};
use dominet_domain::device::DeviceId;
use dominet_domain::repo::{AlertRepo, AlertRuleRepo, NewAlert};
use serde_json::Value;
use tokio::sync::mpsc;

/// Size of the queue between ingestion and evaluation.
pub const QUEUE: usize = 1024;

/// An alert that has just been raised, with what is needed to state it (the text is the reference's).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertNotice {
    pub alert: Alert,
    pub rule: AlertRule,
    pub device_name: String,
    /// Long text (e-mail).
    pub long: String,
    /// Short text (SMS, Android, banner).
    pub short: String,
}

/// Alert sending port. R4: log and banner in the interface; R5 added e-mail, SMS and Android according to the three
/// channels of the rule (`notify_email`, `notify_sms`, `notify_android`) and sets `notified` to true.
pub trait Notifier: Send + Sync + 'static {
    /// An alert has just been raised.
    fn fired(&self, notice: &AlertNotice);
    /// The number of ongoing alerts may have changed (raised or resolved): to rebroadcast to the interfaces.
    fn changed(&self);
}

/// A device state to evaluate.
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

/// Ingestion handle.
#[derive(Debug, Clone)]
pub struct AlertHandle {
    shared: Arc<Shared>,
    tx: mpsc::Sender<AlertEvent>,
}

/// State key readable for a rule: present and non-null.
fn watched(state: &Value, key: &str) -> bool {
    state.get(key).is_some_and(|v| !v.is_null())
}

impl AlertHandle {
    /// A device state (nothing is queued if no rule of that device looks at a present key).
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

    /// States dropped because the queue was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

/// The alert engine.
pub struct AlertEngine<R, A, N> {
    rules: R,
    alerts: A,
    notifier: N,
    shared: Arc<Shared>,
    /// (rule, device) pairs that may have an ongoing alert: avoids a write on every "false" state.
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

    /// Re-reads the enabled rules. The previous list is kept on a read error.
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

    /// At start-up: remembers the (rule, device) pairs that have an ongoing alert, so that their resolution is seen.
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

    /// Loop: one state after another until the queue is closed.
    pub async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<AlertEvent>) {
        while let Some(event) = rx.recv().await {
            self.process(&event).await;
        }
    }

    /// Evaluates the device's rules against this state.
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

    /// Creates the alert unless an ongoing alert is more recent than the cooldown. Returns true if an alert was created.
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

    /// Resolves the ongoing alerts of the rule for this device. Returns true if at least one was.
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
