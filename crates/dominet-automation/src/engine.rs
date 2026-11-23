// SPDX-License-Identifier: AGPL-3.0-or-later
//! Automation engine (R4 lot 3, ADR 0032): `mqtt_message` and `device_state` triggers, condition, actions
//! `mqtt_publish` / `scene` / `notification`.
//!
//! Ingestion pushes [`Event`]s into a queue; the engine processes them **one by one**, in order, without ever slowing
//! ingestion down (queue full: the event is dropped and counted). The active automations are kept in memory and reloaded
//! when the administration changes them. The effects (publish, activate a scene, notify) go through the [`Executor`] port:
//! the server implements it, the tests simulate it.
//!
//! **Anti-loop safeguard** (ADR 0031, D3): an automation that publishes on a topic it listens to would loop forever. At most
//! [`MAX_PER_MINUTE`] triggers per automation and per minute; the following ones are ignored and written to the log only
//! once per minute (status `limited`).

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dominet_domain::automation::{
    Action, Automation, AutomationId, AutomationRun, RunStatus, Trigger, device_state_holds,
    payload_filter_holds,
};
use dominet_domain::repo::{AutomationRepo, AutomationRunRepo};
use dominet_domain::scene::SceneId;
use dominet_mqtt::Payload;

use crate::script::{DeviceView, Effect, Limits, ScriptInput, payload_text, run_isolated};
use serde_json::Value;
use tokio::sync::mpsc;

/// Triggers admitted per automation and per minute.
pub const MAX_PER_MINUTE: usize = 30;
/// Size of the queue between ingestion and the engine.
pub const QUEUE: usize = 1024;
/// Mute duration of a failing source code, in seconds.
pub const HANDLER_MUTE_SECS: i64 = 10;
/// Retention of the execution log.
pub const KEEP_RUNS_DAYS: i64 = 30;

/// What can trigger an automation.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A received MQTT message (full topic, decoded payload).
    Message { topic: String, payload: Payload },
    /// A device state message: name (`friendly_name`), full topic and full state after the message.
    State {
        name: String,
        topic: String,
        state: Value,
    },
    /// A message for the processing code (`handler_code`) of an MQTT source.
    Handler {
        source: String,
        code: String,
        topic: String,
        payload: Payload,
    },
}

/// What a script sees: the message or the state that triggered it (empty for a scheduled trigger).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FireContext {
    pub topic: String,
    pub payload: Value,
    /// Name of the device concerned.
    pub device: Option<String>,
}

impl FireContext {
    fn of(event: &Event) -> Self {
        match event {
            Event::Message { topic, payload } | Event::Handler { topic, payload, .. } => Self {
                topic: topic.clone(),
                payload: payload_value(payload),
                device: None,
            },
            Event::State { name, topic, state } => Self {
                topic: topic.clone(),
                payload: state.clone(),
                device: Some(name.clone()),
            },
        }
    }
}

/// The effects of an automation. Each method says plainly what happened (`Ok`) or why it failed (`Err`).
pub trait Executor: Send + Sync + 'static {
    /// Publishes `payload` (text as is) on `topic`.
    fn publish(
        &self,
        topic: &str,
        payload: &str,
    ) -> impl Future<Output = Result<String, String>> + Send;
    /// Activates the scene.
    fn scene(&self, id: SceneId) -> impl Future<Output = Result<String, String>> + Send;
    /// Activates the scene of that name (scripts).
    fn scene_by_name(&self, name: &str) -> impl Future<Output = Result<String, String>> + Send;
    /// Records a value of a device in the history, whether it is tracked or not (scripts).
    fn log_value(
        &self,
        device: i64,
        key: &str,
        value: &Value,
    ) -> impl Future<Output = Result<(), String>> + Send;
    /// Snapshot of the active devices, for the scripts' `get_device` and `device`.
    fn devices(&self) -> impl Future<Output = Result<Vec<DeviceView>, String>> + Send;
    /// Notification (log and banner in the interface; sending by SMS / e-mail / Android waits for R5).
    fn notify(&self, message: &str, sms: bool);
}

/// Does the topic `filter` (with wildcards) match `topic`? **Like the reference** (checked on a grid of cases, see
/// `tests/matching.rs`): `#` accepts the end of any level, an empty filter matches nothing.
#[must_use]
pub fn topic_matches_py(filter: &str, topic: &str) -> bool {
    if filter.is_empty() {
        return false;
    }
    let levels: Vec<&str> = topic.split('/').collect();
    let parts: Vec<&str> = filter.split('/').collect();
    for (i, p) in parts.iter().enumerate() {
        if *p == "#" {
            return true;
        }
        let Some(level) = levels.get(i) else {
            return false;
        };
        if *p != "+" && p != level {
            return false;
        }
    }
    parts.len() == levels.len()
}

/// Why an automation triggers (log text), if it triggers for this event.
#[must_use]
pub fn fires_on(automation: &Automation, event: &Event) -> Option<String> {
    let draft = &automation.draft;
    let (label, payload) = match (&draft.trigger, event) {
        (
            Trigger::MqttMessage {
                topic: filter,
                payload_filter,
            },
            Event::Message { topic, payload },
        ) => {
            let value = payload_value(payload);
            if !topic_matches_py(filter, topic) || !payload_filter_holds(payload_filter, &value) {
                return None;
            }
            (format!("message {topic}"), value)
        }
        (
            Trigger::DeviceState {
                friendly_name,
                key,
                value,
            },
            Event::State { name, state, .. },
        ) => {
            if friendly_name != name || !device_state_holds(key, value, state) {
                return None;
            }
            (
                format!("state {name}: {key} = {}", shown(value)),
                state.clone(),
            )
        }
        _ => return None,
    };
    draft
        .condition
        .as_ref()
        .is_none_or(|c| c.holds(&payload))
        .then_some(label)
}

fn payload_value(payload: &Payload) -> Value {
    match payload {
        Payload::Json(v) => v.clone(),
        Payload::Text(t) => Value::String(t.clone()),
    }
}

fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// One-minute sliding window per automation.
#[derive(Debug, Default)]
struct Limiter {
    seen: HashMap<AutomationId, VecDeque<DateTime<Utc>>>,
    /// Automations whose overrun of the current minute is already in the log.
    reported: HashMap<AutomationId, DateTime<Utc>>,
}

/// Decision of the safeguard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Allow,
    /// Ignored; `true` if it is the first overrun of the minute (to be written to the log).
    Limited {
        first: bool,
    },
}

impl Limiter {
    fn check(&mut self, id: AutomationId, now: DateTime<Utc>) -> Verdict {
        let window = self.seen.entry(id).or_default();
        let from = now - chrono::Duration::seconds(60);
        while window.front().is_some_and(|t| *t <= from) {
            window.pop_front();
        }
        if window.len() < MAX_PER_MINUTE {
            window.push_back(now);
            self.reported.remove(&id);
            return Verdict::Allow;
        }
        let first = self.reported.get(&id).is_none_or(|t| *t <= from);
        if first {
            self.reported.insert(id, now);
        }
        Verdict::Limited { first }
    }

    fn forget(&mut self, keep: &[AutomationId]) {
        self.seen.retain(|id, _| keep.contains(id));
        self.reported.retain(|id, _| keep.contains(id));
    }
}

#[derive(Debug, Default)]
struct Shared {
    /// Active automations with an `mqtt_message` or `device_state` trigger.
    cache: RwLock<Arc<Vec<Automation>>>,
    dropped: AtomicU64,
}

impl Shared {
    fn snapshot(&self) -> Arc<Vec<Automation>> {
        self.cache
            .read()
            .map(|c| Arc::clone(&c))
            .unwrap_or_default()
    }
}

/// Ingestion handle: immediately discards what no automation expects, queues the rest.
#[derive(Debug, Clone)]
pub struct EngineHandle {
    shared: Arc<Shared>,
    tx: mpsc::Sender<Event>,
}

impl EngineHandle {
    fn push(&self, event: Event) {
        if self.tx.try_send(event).is_err() {
            let n = self.shared.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                tracing::warn!(ecartes = n, "automation engine queue full: event dropped");
            }
        }
    }

    /// An MQTT message (no effect if no `mqtt_message` automation targets this topic).
    pub fn message(&self, topic: &str, payload: &Payload) {
        let wanted = self.shared.snapshot().iter().any(|a| {
            matches!(&a.draft.trigger, Trigger::MqttMessage { topic: f, .. } if topic_matches_py(f, topic))
        });
        if wanted {
            self.push(Event::Message {
                topic: topic.to_string(),
                payload: payload.clone(),
            });
        }
    }

    /// A device state message (no effect if no `device_state` automation targets this device).
    pub fn state(&self, name: &str, topic: &str, state: &Value) {
        let wanted = self.shared.snapshot().iter().any(
            |a| matches!(&a.draft.trigger, Trigger::DeviceState { friendly_name, .. } if friendly_name == name),
        );
        if wanted {
            self.push(Event::State {
                name: name.to_string(),
                topic: topic.to_string(),
                state: state.clone(),
            });
        }
    }

    /// A message for a source's processing code (always queued: the source has code).
    pub fn handler(&self, source: &str, code: &str, topic: &str, payload: &Payload) {
        self.push(Event::Handler {
            source: source.to_string(),
            code: code.to_string(),
            topic: topic.to_string(),
            payload: payload.clone(),
        });
    }

    /// Topic filters of the active `mqtt_message` automations (to add to the client's subscriptions).
    #[must_use]
    pub fn message_filters(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .shared
            .snapshot()
            .iter()
            .filter_map(|a| match &a.draft.trigger {
                Trigger::MqttMessage { topic, .. } => Some(topic.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Events dropped because the queue was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

/// The engine.
pub struct Engine<A, R, X> {
    automations: A,
    runs: R,
    exec: X,
    shared: Arc<Shared>,
    limiter: Mutex<Limiter>,
    /// Sources whose code failed: muted until this instant.
    muted: Mutex<HashMap<String, DateTime<Utc>>>,
    clock: Arc<Clock>,
}

impl<A, R, X> std::fmt::Debug for Engine<A, R, X> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

impl<A, R, X> Engine<A, R, X>
where
    A: AutomationRepo + Send + Sync + 'static,
    R: AutomationRunRepo + Send + Sync + 'static,
    X: Executor,
{
    /// Creates the engine, its handle and the queue; `run` consumes the queue.
    #[must_use]
    pub fn new(
        automations: A,
        runs: R,
        exec: X,
    ) -> (Arc<Self>, EngineHandle, mpsc::Receiver<Event>) {
        Self::with_clock(automations, runs, exec, Arc::new(Utc::now))
    }

    /// With a supplied clock (safeguard tests).
    #[must_use]
    pub fn with_clock(
        automations: A,
        runs: R,
        exec: X,
        clock: Arc<Clock>,
    ) -> (Arc<Self>, EngineHandle, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel(QUEUE);
        let shared = Arc::new(Shared::default());
        let engine = Arc::new(Self {
            automations,
            runs,
            exec,
            shared: Arc::clone(&shared),
            limiter: Mutex::new(Limiter::default()),
            muted: Mutex::new(HashMap::new()),
            clock,
        });
        (engine, EngineHandle { shared, tx }, rx)
    }

    /// Re-reads the active automations. On a read error, the previous list is kept.
    pub async fn reload(&self) {
        match self.automations.list_enabled().await {
            Ok(all) => {
                let wanted: Vec<Automation> = all
                    .into_iter()
                    .filter(|a| {
                        matches!(
                            a.draft.trigger,
                            Trigger::MqttMessage { .. } | Trigger::DeviceState { .. }
                        )
                    })
                    .collect();
                let ids: Vec<AutomationId> = wanted.iter().map(|a| a.id).collect();
                if let Ok(mut l) = self.limiter.lock() {
                    l.forget(&ids);
                }
                if let Ok(mut c) = self.shared.cache.write() {
                    *c = Arc::new(wanted);
                }
            }
            Err(e) => tracing::warn!(error = %e, "unreadable automations: list unchanged"),
        }
    }

    /// Engine loop: one event after another until the queue is closed; log purge once a day.
    pub async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<Event>) {
        let mut purge = tokio::time::interval(Duration::from_hours(24));
        loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Some(event) => self.process(&event).await,
                    None => break,
                },
                _ = purge.tick() => {
                    let before = (self.clock)() - chrono::Duration::days(KEEP_RUNS_DAYS);
                    match self.runs.purge_before(before).await {
                        Ok(n) if n > 0 => tracing::info!(lignes = n, "journal des automations purge"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "cannot purge the automation log"),
                    }
                }
            }
        }
    }

    /// Processes an event: each automation concerned triggers, in the order of the list (by name).
    pub async fn process(&self, event: &Event) {
        if let Event::Handler {
            source,
            code,
            topic,
            payload,
        } = event
        {
            self.run_handler(source, code, topic, payload).await;
            return;
        }
        let context = FireContext::of(event);
        for automation in self.shared.snapshot().iter() {
            if let Some(trigger) = fires_on(automation, event) {
                let _ = self.fire(automation, &trigger, &context).await;
            }
        }
    }

    /// Triggers the automation now, without looking at its trigger (scheduler): same safeguard, same log, same
    /// counters as for a message. Returns the status and the detail written to the log.
    pub async fn fire(
        &self,
        automation: &Automation,
        trigger: &str,
        context: &FireContext,
    ) -> (RunStatus, String) {
        let now = (self.clock)();
        let verdict = self
            .limiter
            .lock()
            .map_or(Verdict::Allow, |mut l| l.check(automation.id, now));
        if let Verdict::Limited { first } = verdict {
            let message = format!(
                "more than {MAX_PER_MINUTE} firings per minute: the following ones are ignored"
            );
            if first {
                tracing::warn!(automation = %automation.draft.name, "more than {MAX_PER_MINUTE} firings per minute: ignored");
                self.journal(automation.id, now, trigger, RunStatus::Limited, 0, &message)
                    .await;
            }
            return (RunStatus::Limited, message);
        }
        let started = Instant::now();
        let (status, message) = self.execute(automation, context).await;
        let ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
        self.journal(automation.id, now, trigger, status, ms, &message)
            .await;
        if let Err(e) = self.automations.record_trigger(automation.id, now).await {
            tracing::warn!(automation = %automation.draft.name, error = %e, "firing counter not updated");
        }
        (status, message)
    }

    async fn execute(&self, automation: &Automation, context: &FireContext) -> (RunStatus, String) {
        let name = &automation.draft.name;
        let outcome = match &automation.draft.action {
            Action::MqttPublish { topic, payload } => {
                self.exec.publish(topic, &payload_text(payload)).await
            }
            Action::Scene { scene } => self.exec.scene(*scene).await,
            Action::Notification { message, sms } => {
                let text = if message.is_empty() {
                    format!("Automation {name} fired")
                } else {
                    message.clone()
                };
                self.exec.notify(&text, *sms);
                Ok(format!("notification : {text}"))
            }
            Action::Script => return self.execute_script(&automation.draft.script, context).await,
        };
        match outcome {
            Ok(message) => (RunStatus::Ok, message),
            Err(message) => (RunStatus::Error, message),
        }
    }

    /// Runs a script (automation or source) then applies its effects, in order. All or nothing on the script side: a script error
    /// applies nothing; a refused effect (broker down, unknown scene) does not stop the following ones and makes the whole
    /// fail.
    async fn execute_script(&self, code: &str, context: &FireContext) -> (RunStatus, String) {
        let devices = match self.exec.devices().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "unreadable devices for a script");
                vec![]
            }
        };
        let input = ScriptInput {
            topic: context.topic.clone(),
            payload: context.payload.clone(),
            device: context.device.clone(),
            devices,
        };
        let out = run_isolated(code.to_string(), input, Limits::default()).await;
        let logs = if out.logs.is_empty() {
            String::new()
        } else {
            format!(" | journal : {}", out.logs.join(" ; "))
        };
        if let Err(e) = out.result {
            return (RunStatus::Error, format!("script failed: {e}{logs}"));
        }
        let (mut done, mut failed) = (0usize, vec![]);
        for effect in &out.effects {
            let result = match effect {
                Effect::Publish { topic, payload } => self
                    .exec
                    .publish(topic, &payload_text(payload))
                    .await
                    .map(|_| ()),
                Effect::Scene(name) => self.exec.scene_by_name(name).await.map(|_| ()),
                Effect::LogValue { device, key, value } => {
                    self.exec.log_value(*device, key, value).await
                }
            };
            match result {
                Ok(()) => done += 1,
                Err(e) => failed.push(format!("{} : {e}", effect.describe())),
            }
        }
        let summary = format!(
            "script: {done} effect(s) applied in {} ms",
            out.elapsed.as_millis()
        );
        if failed.is_empty() {
            (RunStatus::Ok, format!("{summary}{logs}"))
        } else {
            (
                RunStatus::Error,
                format!("{summary}; failures: {}{logs}", failed.join(" ; ")),
            )
        }
    }

    /// Processing code of a source: same host as the automations, but without an execution log (no automation);
    /// the result goes to the server's log. A failing code is muted for [`HANDLER_MUTE_SECS`] seconds: a chatty source
    /// with a faulty code does not monopolise the engine.
    async fn run_handler(&self, source: &str, code: &str, topic: &str, payload: &Payload) {
        let now = (self.clock)();
        let muted = self
            .muted
            .lock()
            .is_ok_and(|m| m.get(source).is_some_and(|until| *until > now));
        if muted {
            return;
        }
        let context = FireContext {
            topic: topic.to_string(),
            payload: payload_value(payload),
            device: None,
        };
        let (status, message) = self.execute_script(code, &context).await;
        if status == RunStatus::Ok {
            tracing::debug!(%source, %topic, %message, "code de source execute");
        } else {
            tracing::warn!(%source, %topic, %message, "source handler failed: muted for {HANDLER_MUTE_SECS} s");
            if let Ok(mut m) = self.muted.lock() {
                m.insert(
                    source.to_string(),
                    now + chrono::Duration::seconds(HANDLER_MUTE_SECS),
                );
            }
        }
    }

    async fn journal(
        &self,
        automation: AutomationId,
        at: DateTime<Utc>,
        trigger: &str,
        status: RunStatus,
        duration_ms: i32,
        message: &str,
    ) {
        let run = AutomationRun {
            automation,
            at,
            trigger: trigger.to_string(),
            status,
            duration_ms,
            message: message.chars().take(500).collect(),
        };
        if let Err(e) = self.runs.record(&run).await {
            tracing::warn!(error = %e, "automation log not written");
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(s: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap() + chrono::Duration::seconds(s)
    }

    #[test]
    fn the_limiter_allows_thirty_per_minute_then_reports_once() {
        let mut l = Limiter::default();
        let id = AutomationId(1);
        for i in 0..MAX_PER_MINUTE {
            assert_eq!(
                l.check(id, at(i64::try_from(i).unwrap() / 10)),
                Verdict::Allow
            );
        }
        assert_eq!(l.check(id, at(5)), Verdict::Limited { first: true });
        assert_eq!(l.check(id, at(6)), Verdict::Limited { first: false });
        assert_eq!(l.check(id, at(59)), Verdict::Limited { first: false });
        // Another automation is not touched.
        assert_eq!(l.check(AutomationId(2), at(6)), Verdict::Allow);
        // A minute later the window is empty: we start over, and a new overrun will be reported again.
        assert_eq!(l.check(id, at(70)), Verdict::Allow);
        for _ in 1..MAX_PER_MINUTE {
            assert_eq!(l.check(id, at(71)), Verdict::Allow);
        }
        assert_eq!(l.check(id, at(72)), Verdict::Limited { first: true });
    }

    #[test]
    fn topic_matching_is_the_reference() {
        for (f, t, ok) in [
            ("a/#", "a", true),
            ("#", "$SYS/x", true),
            ("", "", false),
            ("a/+", "a/b/c", false),
            ("a/+/c", "a/b/c", true),
            ("a/b/#", "a/b", true),
        ] {
            assert_eq!(topic_matches_py(f, t), ok, "{f} {t}");
        }
    }
}
