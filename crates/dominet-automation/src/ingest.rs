// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ingestion Zigbee2MQTT : `bridge/devices`, `bridge/info`, etats et disponibilites (ADR 0027).
//!
//! Chaque traitement suit la semantique decrite dans `dominet_domain::device`. Les doublons QoS 1 sont
//! sans consequence : l'etat est remplace, l'historique est a la seconde. En cas de rafale, les messages d'etat d'un meme appareil
//! recus dans la meme seconde et deja en file ne sont traites que dans leur derniere version ([`coalesce`]).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Timelike, Utc};
use dominet_domain::device::{
    Device, DeviceType, Discovered, LogEntry, MqttSource, is_valid_friendly_name, log_entries,
};
use dominet_domain::repo::{DeviceFilter, DeviceRepo, StateLogRepo};
use dominet_mqtt::{Incoming, Payload, topic_matches};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::route::{Route, route};

/// Plus gros paquet traite d'un coup (au-dela, les messages restent dans la file du client).
const MAX_BATCH: usize = 256;

/// Ce que l'ingestion alimente en dehors de la base : diffusion temps reel (serveur), et plus tard automations et alertes (R4).
pub trait Sink: Send + Sync + 'static {
    /// L'etat ou la disponibilite d'un appareil vient de changer (l'appareil est tel qu'en base apres le changement).
    fn device_changed(&self, device: &Device);
    /// Etat `permit_join` lu dans `bridge/info`.
    fn permit_join(&self, enabled: bool);
    /// Zigbee2MQTT reported an **error** on `bridge/logging` (a command that did not reach its device, for instance): the
    /// only trace of a delivery failure, since a published command always succeeds from the broker's point of view.
    /// No effect by default.
    fn bridge_error(&self, _message: &str) {}
    /// Un message vient d'etre traite (tous les messages recus, y compris hors Zigbee2MQTT, sauf `bridge/…`, commandes
    /// `…/set` / `…/get`) : declencheur `mqtt_message` des automations. Sans effet par defaut.
    fn message(&self, _topic: &str, _payload: &Payload) {}
    /// Un message d'**etat** vient d'etre enregistre (Zigbee2MQTT ou appareil automatique) : declencheur `device_state`.
    fn device_state(&self, _device: &Device, _state: &Value) {}
    /// Un message d'etat a ete **regroupe** (un autre etat du meme appareil suit dans la meme seconde, voir [`coalesce`]) : il
    /// n'est ni enregistre ni diffuse, mais une automation `device_state` doit le voir quand meme (une telecommande envoie
    /// `{"action":"1_single"}` puis `{"action":""}` a quelques millisecondes d'ecart). Sans effet par defaut.
    fn state_skipped(&self, _name: &str, _topic: &str, _state: &Value) {}
    /// Un message vise une source MQTT active qui a un code de traitement (`handler_code`, Rhai).
    fn source_handler(&self, _source: &MqttSource, _topic: &str, _payload: &Payload) {}
    /// Des cles suivies viennent d'etre historisees en base : `state` est l'etat complet apres le message. Archive brute
    /// (ReductStore) ; sans effet par defaut.
    fn archive(&self, _device: &Device, _at: DateTime<Utc>, _entries: &[LogEntry]) {}
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

/// Traitement des messages d'appareils.
pub struct Ingestor<D, L, S> {
    devices: D,
    log: L,
    sink: S,
    base_topic: RwLock<String>,
    /// Sources MQTT actives (hors Zigbee2MQTT), remplacees d'un bloc quand elles changent.
    sources: RwLock<Arc<Vec<MqttSource>>>,
    clock: Arc<Clock>,
}

impl<D, L, S> std::fmt::Debug for Ingestor<D, L, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ingestor").finish_non_exhaustive()
    }
}

/// Un message avec la seconde a laquelle il a ete recu (pour le regroupement).
struct Stamped {
    msg: Incoming,
    route: Route,
    second: i64,
}

/// Indices des messages a traiter apres regroupement. Un message d'etat est ecarte si le **message suivant concernant le meme
/// appareil** est aussi un etat recu dans la meme seconde ; `bridge/devices` coupe tout regroupement.
#[must_use]
fn coalesce(batch: &[(Route, i64)]) -> Vec<usize> {
    #[derive(Clone, Copy, PartialEq)]
    enum Next {
        State(i64),
        Other,
    }
    let mut next: HashMap<&str, Next> = HashMap::new();
    let mut keep = vec![true; batch.len()];
    for (i, (route, second)) in batch.iter().enumerate().rev() {
        match route {
            Route::State(name) => {
                if next.get(name.as_str()) == Some(&Next::State(*second)) {
                    keep[i] = false;
                }
                next.insert(name, Next::State(*second));
            }
            Route::Availability(name) => {
                next.insert(name, Next::Other);
            }
            Route::BridgeDevices => next.clear(),
            _ => {}
        }
    }
    keep.iter()
        .enumerate()
        .filter(|(_, k)| **k)
        .map(|(i, _)| i)
        .collect()
}

impl<D, L, S> Ingestor<D, L, S>
where
    D: DeviceRepo + Send + Sync + 'static,
    L: StateLogRepo + Send + Sync + 'static,
    S: Sink,
{
    #[must_use]
    pub fn new(devices: D, log: L, sink: S, base_topic: &str) -> Self {
        Self::with_clock(devices, log, sink, base_topic, Arc::new(Utc::now))
    }

    /// Avec une horloge fournie (rejeux de tests : le temps vient du fichier de messages).
    #[must_use]
    pub fn with_clock(devices: D, log: L, sink: S, base_topic: &str, clock: Arc<Clock>) -> Self {
        Self {
            devices,
            log,
            sink,
            base_topic: RwLock::new(normalize(base_topic)),
            sources: RwLock::new(Arc::new(Vec::new())),
            clock,
        }
    }

    /// Prefixe Zigbee2MQTT courant.
    #[must_use]
    pub fn base_topic(&self) -> String {
        self.base_topic
            .read()
            .map(|b| b.clone())
            .unwrap_or_default()
    }

    /// Change le prefixe (reglage `Z2M_BASE_TOPIC` modifie).
    pub fn set_base_topic(&self, base: &str) {
        if let Ok(mut b) = self.base_topic.write() {
            *b = normalize(base);
        }
    }

    /// Remplace les sources MQTT generiques (seules les actives sont gardees).
    pub fn set_sources(&self, sources: Vec<MqttSource>) {
        let active: Vec<MqttSource> = sources.into_iter().filter(|s| s.active).collect();
        if let Ok(mut slot) = self.sources.write() {
            *slot = Arc::new(active);
        }
    }

    fn current_sources(&self) -> Arc<Vec<MqttSource>> {
        self.sources
            .read()
            .map(|s| Arc::clone(&s))
            .unwrap_or_default()
    }

    /// Republie vers le sink l'etat connu de chaque appareil actif (demarrage : les widgets montrent le dernier etat connu).
    pub async fn announce_known_states(&self) {
        match self
            .devices
            .list(&DeviceFilter {
                active_only: true,
                ..DeviceFilter::default()
            })
            .await
        {
            Ok(all) => all
                .iter()
                .filter(|d| d.state.as_object().is_some_and(|o| !o.is_empty()))
                .for_each(|d| self.sink.device_changed(d)),
            Err(e) => tracing::warn!(error = %e, "unreadable known states at start-up"),
        }
    }

    /// Boucle d'ingestion : prend les messages par paquets (regroupes en cas de retard) jusqu'a la fermeture de la file.
    pub async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<Incoming>) {
        let mut batch = Vec::with_capacity(MAX_BATCH);
        while rx.recv_many(&mut batch, MAX_BATCH).await > 0 {
            self.handle_batch(std::mem::take(&mut batch)).await;
        }
    }

    /// Traite un seul message (tests).
    pub async fn handle(&self, msg: Incoming) {
        self.handle_batch(vec![msg]).await;
    }

    /// Traite un paquet de messages, dans l'ordre, apres regroupement.
    pub async fn handle_batch(&self, messages: Vec<Incoming>) {
        let base = self.base_topic();
        let now = (self.clock)();
        let stamped: Vec<Stamped> = messages
            .into_iter()
            .map(|msg| {
                let route = route(&base, &msg.topic, &msg.payload);
                Stamped {
                    msg,
                    route,
                    second: now.timestamp(),
                }
            })
            .collect();
        let plan: Vec<(Route, i64)> = stamped
            .iter()
            .map(|s| (s.route.clone(), s.second))
            .collect();
        let keep = coalesce(&plan);
        if keep.len() < stamped.len() {
            tracing::debug!(
                recus = stamped.len(),
                traites = keep.len(),
                "state burst grouped"
            );
        }
        let kept: std::collections::HashSet<usize> = keep.into_iter().collect();
        for (i, s) in stamped.into_iter().enumerate() {
            if kept.contains(&i) {
                self.dispatch(&base, &s.route, &s.msg).await;
            } else if let (Route::State(name), Payload::Json(state @ Value::Object(_))) =
                (&s.route, &s.msg.payload)
            {
                // Regroupe : pas enregistre, mais les automations le voient (a sa place dans l'ordre des messages).
                self.sink.state_skipped(name, &s.msg.topic, state);
            }
        }
    }

    async fn dispatch(&self, base: &str, route: &Route, msg: &Incoming) {
        let payload = &msg.payload;
        match route {
            Route::BridgeDevices => self.bridge_devices(base, payload).await,
            Route::BridgeInfo => self.bridge_info(payload),
            Route::BridgeLog => self.bridge_log(payload),
            Route::Availability(name) => self.availability(name, payload).await,
            Route::State(name) => self.state(name, payload).await,
            Route::Other => self.source_message(&msg.topic, payload).await,
            Route::Ignored => {}
        }
        // Les messages de bridge et les commandes ne declenchent pas d'automation (une automation qui commande un appareil
        // se declencherait elle-meme en voyant passer sa propre commande).
        if matches!(
            route,
            Route::State(_) | Route::Availability(_) | Route::Other
        ) {
            self.sink.message(&msg.topic, payload);
        }
    }

    /// `bridge/devices` : cree ou rafraichit chaque appareil (le coordinateur et les noms invalides sont ignores).
    async fn bridge_devices(&self, base: &str, payload: &Payload) {
        let Payload::Json(Value::Array(entries)) = payload else {
            return;
        };
        for entry in entries {
            let Some(discovered) = Discovered::from_bridge_entry(entry) else {
                continue;
            };
            match self.devices.upsert_discovered(&discovered, base).await {
                Ok((device, true)) => {
                    tracing::info!(name = %device.friendly_name, kind = device.device_type.as_str(), "device discovered");
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(name = %discovered.friendly_name, error = %e, "device discovery failed");
                }
            }
        }
    }

    /// `bridge/logging` : seules les erreurs comptent (`{"level":"error","message":"z2m: Publish 'set' 'state' to 'x' failed: …"}`).
    fn bridge_log(&self, payload: &Payload) {
        let Some(o) = payload.as_object() else {
            return;
        };
        if o.get("level").and_then(Value::as_str) != Some("error") {
            return;
        }
        let message = o.get("message").and_then(Value::as_str).unwrap_or("");
        if message.is_empty() {
            return;
        }
        let message: String = message.chars().take(500).collect();
        tracing::warn!("error reported by Zigbee2MQTT: {message}");
        self.sink.bridge_error(&message);
    }

    fn bridge_info(&self, payload: &Payload) {
        // Reference : `permit_join` absent ou `null` -> rien ; sinon `bool(valeur)`.
        let Some(value) = payload.as_object().and_then(|o| o.get("permit_join")) else {
            return;
        };
        let enabled = match value {
            Value::Null => return,
            Value::Bool(b) => *b,
            Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        };
        self.sink.permit_join(enabled);
    }

    /// `{nom}/availability` : `{"state":"online"}` ou le texte `online`.
    async fn availability(&self, name: &str, payload: &Payload) {
        let available = match payload {
            Payload::Json(Value::Object(o)) => {
                o.get("state").and_then(Value::as_str) == Some("online")
            }
            other => other.as_text().is_some_and(|t| t.trim() == "online"),
        };
        match self.devices.set_available(name, available).await {
            Ok(Some(device)) => self.sink.device_changed(&device),
            Ok(None) => {}
            Err(e) => tracing::warn!(%name, error = %e, "availability not saved"),
        }
    }

    /// `{nom}` : l'etat (objet) remplace celui de l'appareil actif ; historique des cles suivies.
    async fn state(&self, name: &str, payload: &Payload) {
        let Payload::Json(state @ Value::Object(_)) = payload else {
            return;
        };
        let device = match self.devices.apply_state(name, state).await {
            Ok(Some(d)) => d,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(%name, error = %e, "state not saved");
                return;
            }
        };
        self.sink.device_changed(&device);
        self.log_state(&device, state).await;
        self.sink.device_state(&device, state);
    }

    /// Historique des cles suivies (a la seconde) puis archive brute.
    async fn log_state(&self, device: &Device, state: &Value) {
        let entries = log_entries(&device.tracked_keys, state);
        if entries.is_empty() {
            return;
        }
        let now = (self.clock)();
        let ts = now.with_nanosecond(0).unwrap_or(now);
        match self.log.record(device.id, ts, &entries).await {
            Ok(_) => self.sink.archive(device, now, &entries),
            Err(e) => {
                tracing::warn!(name = %device.friendly_name, error = %e, "history not saved");
            }
        }
    }

    /// Message hors du prefixe Zigbee2MQTT : sources MQTT dont le filtre correspond (journal, appareil automatique), puis
    /// les appareils dont le topic est celui du message sont marques vus. Le code de gestion (`handler_code`, Rhai) est confie au
    /// moteur d'automations par le `Sink`.
    async fn source_message(&self, topic: &str, payload: &Payload) {
        let sources = self.current_sources();
        let mut matched = false;
        for source in sources
            .iter()
            .filter(|s| topic_matches(&s.topic_pattern, topic))
        {
            matched = true;
            if source.log_messages {
                tracing::info!(source = %source.name, %topic, payload = %preview(payload), "MQTT source message");
            }
            if source.auto_device
                && let Payload::Json(state @ Value::Object(_)) = payload
            {
                self.auto_device(source, topic, state).await;
            }
            if !source.handler_code.trim().is_empty() {
                self.sink.source_handler(source, topic, payload);
            }
        }
        if matched && let Err(e) = self.devices.touch_by_topic(topic).await {
            tracing::warn!(%topic, error = %e, "topic devices not marked seen");
        }
    }

    /// Cree ou met a jour l'appareil d'une source en `auto_device` (nom = topic sans le prefixe de la source).
    async fn auto_device(&self, source: &MqttSource, topic: &str, state: &Value) {
        let prefix = source.base_topic.trim_end_matches('/');
        let (name, base) = match topic
            .strip_prefix(prefix)
            .and_then(|r| r.strip_prefix('/'))
            .filter(|_| !prefix.is_empty())
        {
            Some(rest) => (rest, prefix),
            None => (topic, ""),
        };
        if !is_valid_friendly_name(name) {
            tracing::debug!(source = %source.name, %topic, "invalid automatic device name, message ignored");
            return;
        }
        let kind = source.auto_device_type.unwrap_or(DeviceType::Sensor);
        match self.devices.upsert_auto(name, base, kind, state).await {
            Ok((device, created)) => {
                if created {
                    tracing::info!(name = %device.friendly_name, source = %source.name, "device created by a source");
                }
                self.sink.device_changed(&device);
                self.log_state(&device, state).await;
                self.sink.device_state(&device, state);
            }
            Err(e) => {
                tracing::warn!(%name, source = %source.name, error = %e, "source device not saved");
            }
        }
    }
}

/// Debut d'une charge utile pour le journal (les sources peuvent envoyer des messages volumineux).
fn preview(payload: &Payload) -> String {
    let text = match payload {
        Payload::Json(v) => v.to_string(),
        Payload::Text(t) => t.clone(),
    };
    if text.chars().count() > 512 {
        let cut: String = text.chars().take(512).collect();
        format!("{cut}...")
    } else {
        text
    }
}

fn normalize(base: &str) -> String {
    base.trim().trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(name: &str, second: i64) -> (Route, i64) {
        (Route::State(name.into()), second)
    }

    fn av(name: &str) -> (Route, i64) {
        (Route::Availability(name.into()), 0)
    }

    #[test]
    fn only_the_last_state_of_a_device_in_the_same_second_is_kept() {
        let batch = [st("a", 1), st("a", 1), st("b", 1), st("a", 1)];
        assert_eq!(coalesce(&batch), [2, 3]);
    }

    #[test]
    fn states_in_different_seconds_are_all_kept() {
        assert_eq!(coalesce(&[st("a", 1), st("a", 2), st("a", 2)]), [0, 2]);
    }

    #[test]
    fn availability_and_bridge_devices_separate_states() {
        // Un message de disponibilite entre deux etats : les deux etats comptent (l'etat suivant n'est plus « le suivant »).
        assert_eq!(coalesce(&[st("a", 1), av("a"), st("a", 1)]), [0, 1, 2]);
        // Celui d'un autre appareil ne separe rien.
        assert_eq!(coalesce(&[st("a", 1), av("b"), st("a", 1)]), [1, 2]);
        // `bridge/devices` coupe le regroupement.
        assert_eq!(
            coalesce(&[st("a", 1), (Route::BridgeDevices, 1), st("a", 1)]),
            [0, 1, 2]
        );
    }

    #[test]
    fn a_single_message_and_an_empty_batch_are_unchanged() {
        assert_eq!(coalesce(&[st("a", 1)]), [0]);
        assert_eq!(coalesce(&[]), Vec::<usize>::new());
    }
}
