// SPDX-License-Identifier: AGPL-3.0-or-later
//! Moteur d'automations (R4 lot 3, ADR 0032) : declencheurs `mqtt_message` et `device_state`, condition, actions
//! `mqtt_publish` / `scene` / `notification`.
//!
//! L'ingestion pousse des [`Event`] dans une file ; le moteur les traite **un par un**, dans l'ordre, sans jamais ralentir
//! l'ingestion (file pleine : l'evenement est ecarte et compte). Les automations actives sont gardees en memoire et rechargees
//! quand l'administration les change. Les effets (publier, activer une scene, notifier) passent par le port [`Executor`] :
//! le serveur l'implemente, les tests le simulent.
//!
//! **Garde-fou anti-boucle** (ADR 0031, D3) : une automation qui publie sur un topic qu'elle ecoute bouclerait a l'infini. Au plus
//! [`MAX_PER_MINUTE`] declenchements par automation et par minute ; les suivants sont ignores et inscrits au journal une seule
//! fois par minute (statut `limited`).

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

/// Declenchements admis par automation et par minute.
pub const MAX_PER_MINUTE: usize = 30;
/// Taille de la file entre l'ingestion et le moteur.
pub const QUEUE: usize = 1024;
/// Sourdine d'un code de source en echec, en secondes.
pub const HANDLER_MUTE_SECS: i64 = 10;
/// Duree de conservation du journal d'execution.
pub const KEEP_RUNS_DAYS: i64 = 30;

/// Ce qui peut declencher une automation.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Un message MQTT recu (topic complet, charge decodee).
    Message { topic: String, payload: Payload },
    /// Un message d'etat d'appareil : nom (`friendly_name`), topic complet et etat complet apres le message.
    State {
        name: String,
        topic: String,
        state: Value,
    },
    /// Un message pour le code de traitement (`handler_code`) d'une source MQTT.
    Handler {
        source: String,
        code: String,
        topic: String,
        payload: Payload,
    },
}

/// Ce que voit un script : le message ou l'etat qui l'a declenche (vide pour un declenchement horaire).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FireContext {
    pub topic: String,
    pub payload: Value,
    /// Nom de l'appareil concerne.
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

/// Les effets d'une automation. Chaque methode dit en clair ce qui s'est passe (`Ok`) ou pourquoi ca a echoue (`Err`).
pub trait Executor: Send + Sync + 'static {
    /// Publie `payload` (texte tel quel) sur `topic`.
    fn publish(
        &self,
        topic: &str,
        payload: &str,
    ) -> impl Future<Output = Result<String, String>> + Send;
    /// Active la scene.
    fn scene(&self, id: SceneId) -> impl Future<Output = Result<String, String>> + Send;
    /// Active la scene de ce nom (scripts).
    fn scene_by_name(&self, name: &str) -> impl Future<Output = Result<String, String>> + Send;
    /// Historise une valeur d'un appareil, qu'elle soit suivie ou non (scripts).
    fn log_value(
        &self,
        device: i64,
        key: &str,
        value: &Value,
    ) -> impl Future<Output = Result<(), String>> + Send;
    /// Instantane des appareils actifs, pour `get_device` et `device` des scripts.
    fn devices(&self) -> impl Future<Output = Result<Vec<DeviceView>, String>> + Send;
    /// Notification (journal et bandeau dans l'interface ; l'envoi par SMS / e-mail / Android attend R5).
    fn notify(&self, message: &str, sms: bool);
}

/// Le topic `filter` (avec jokers) vise-t-il `topic` ? **Comme la reference** (verifie sur une grille de cas, voir
/// `tests/matching.rs`) : `#` accepte la fin de n'importe quel niveau, un filtre vide ne correspond a rien.
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

/// Pourquoi une automation se declenche (texte du journal), si elle se declenche pour cet evenement.
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

/// Fenetre glissante d'une minute par automation.
#[derive(Debug, Default)]
struct Limiter {
    seen: HashMap<AutomationId, VecDeque<DateTime<Utc>>>,
    /// Automations dont le depassement de la minute en cours est deja au journal.
    reported: HashMap<AutomationId, DateTime<Utc>>,
}

/// Decision du garde-fou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Allow,
    /// Ignore ; `true` si c'est le premier depassement de la minute (a inscrire au journal).
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
    /// Automations actives a declencheur `mqtt_message` ou `device_state`.
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

/// Poignee de l'ingestion : ecarte tout de suite ce qu'aucune automation n'attend, empile le reste.
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

    /// Un message MQTT (aucun coup si aucune automation `mqtt_message` ne vise ce topic).
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

    /// Un message d'etat d'appareil (aucun coup si aucune automation `device_state` ne vise cet appareil).
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

    /// Un message pour le code de traitement d'une source (toujours empile : la source a du code).
    pub fn handler(&self, source: &str, code: &str, topic: &str, payload: &Payload) {
        self.push(Event::Handler {
            source: source.to_string(),
            code: code.to_string(),
            topic: topic.to_string(),
            payload: payload.clone(),
        });
    }

    /// Filtres de topics des automations `mqtt_message` actives (a ajouter aux abonnements du client).
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

    /// Evenements ecartes parce que la file etait pleine.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

/// Le moteur.
pub struct Engine<A, R, X> {
    automations: A,
    runs: R,
    exec: X,
    shared: Arc<Shared>,
    limiter: Mutex<Limiter>,
    /// Sources dont le code a echoue : muettes jusqu'a cet instant.
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
    /// Cree le moteur, sa poignee et la file ; `run` consomme la file.
    #[must_use]
    pub fn new(
        automations: A,
        runs: R,
        exec: X,
    ) -> (Arc<Self>, EngineHandle, mpsc::Receiver<Event>) {
        Self::with_clock(automations, runs, exec, Arc::new(Utc::now))
    }

    /// Avec une horloge fournie (tests du garde-fou).
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

    /// Relit les automations actives. En cas d'erreur de lecture, la liste precedente est gardee.
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

    /// Boucle du moteur : un evenement apres l'autre jusqu'a la fermeture de la file ; purge du journal une fois par jour.
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

    /// Traite un evenement : chaque automation concernee se declenche, dans l'ordre de la liste (par nom).
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

    /// Declenche l'automation maintenant, sans regarder son declencheur (planificateur) : meme garde-fou, meme journal, memes
    /// compteurs que pour un message. Renvoie le statut et le detail ecrits au journal.
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

    /// Execute un script (automation ou source) puis applique ses effets, dans l'ordre. Tout ou rien cote script : une erreur
    /// du script n'applique rien ; un effet refuse (broker coupe, scene inconnue) n'arrete pas les suivants et rend l'ensemble
    /// en echec.
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

    /// Code de traitement d'une source : meme hote que les automations, mais sans journal d'execution (pas d'automation) ;
    /// le resultat va au journal du serveur. Un code en echec est mis en sourdine [`HANDLER_MUTE_SECS`] secondes : une source
    /// bavarde avec un code defectueux ne monopolise pas le moteur.
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
        // Une autre automation n'est pas touchee.
        assert_eq!(l.check(AutomationId(2), at(6)), Verdict::Allow);
        // Une minute plus tard la fenetre est vide : on repart, et un nouveau depassement sera de nouveau rapporte.
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
