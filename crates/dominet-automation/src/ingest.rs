// SPDX-License-Identifier: AGPL-3.0-or-later
//! Zigbee2MQTT ingestion: `bridge/devices`, `bridge/info`, states and availabilities (ADR 0027).
//!
//! Each processing follows the semantics described in `dominet_domain::device`. QoS 1 duplicates are
//! harmless: the state is replaced, the history is per second. In case of a burst, the state messages of the same device
//! received in the same second and already queued are only processed in their last version ([`coalesce`]).

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

/// Largest packet processed at once (beyond it, the messages stay in the client's queue).
const MAX_BATCH: usize = 256;

/// What ingestion feeds outside the database: real-time broadcast (server), and later automations and alerts (R4).
pub trait Sink: Send + Sync + 'static {
    /// The state or availability of a device has just changed (the device is as in the database after the change).
    fn device_changed(&self, device: &Device);
    /// `permit_join` state read from `bridge/info`.
    fn permit_join(&self, enabled: bool);
    /// Zigbee2MQTT reported an **error** on `bridge/logging` (a command that did not reach its device, for instance): the
    /// only trace of a delivery failure, since a published command always succeeds from the broker's point of view.
    /// No effect by default.
    fn bridge_error(&self, _message: &str) {}
    /// A message has just been processed (all received messages, including non-Zigbee2MQTT ones, except `bridge/…`, commands
    /// `…/set` / `…/get`): `mqtt_message` trigger of the automations. No effect by default.
    fn message(&self, _topic: &str, _payload: &Payload) {}
    /// A **state** message has just been saved (Zigbee2MQTT or automatic device): `device_state` trigger.
    fn device_state(&self, _device: &Device, _state: &Value) {}
    /// A state message was **coalesced** (another state of the same device follows in the same second, see [`coalesce`]): it
    /// is neither saved nor broadcast, but a `device_state` automation must still see it (a remote control sends
    /// `{"action":"1_single"}` then `{"action":""}` a few milliseconds apart). No effect by default.
    fn state_skipped(&self, _name: &str, _topic: &str, _state: &Value) {}
    /// A message targets an active MQTT source that has a processing code (`handler_code`, Rhai).
    fn source_handler(&self, _source: &MqttSource, _topic: &str, _payload: &Payload) {}
    /// Tracked keys have just been recorded in the database history: `state` is the full state after the message. Raw archive
    /// (ReductStore); no effect by default.
    fn archive(&self, _device: &Device, _at: DateTime<Utc>, _entries: &[LogEntry]) {}
}

type Clock = dyn Fn() -> DateTime<Utc> + Send + Sync;

/// Processing of device messages.
pub struct Ingestor<D, L, S> {
    devices: D,
    log: L,
    sink: S,
    base_topic: RwLock<String>,
    /// Active MQTT sources (outside Zigbee2MQTT), replaced as a block when they change.
    sources: RwLock<Arc<Vec<MqttSource>>>,
    clock: Arc<Clock>,
}

impl<D, L, S> std::fmt::Debug for Ingestor<D, L, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ingestor").finish_non_exhaustive()
    }
}

/// A message with the second at which it was received (for coalescing).
struct Stamped {
    msg: Incoming,
    route: Route,
    second: i64,
}

/// Indices of the messages to process after coalescing. A state message is dropped if the **next message about the same
/// device** is also a state received in the same second; `bridge/devices` cuts any coalescing.
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

    /// With a supplied clock (test replays: the time comes from the message file).
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

    /// Current Zigbee2MQTT prefix.
    #[must_use]
    pub fn base_topic(&self) -> String {
        self.base_topic
            .read()
            .map(|b| b.clone())
            .unwrap_or_default()
    }

    /// Changes the prefix (`Z2M_BASE_TOPIC` setting modified).
    pub fn set_base_topic(&self, base: &str) {
        if let Ok(mut b) = self.base_topic.write() {
            *b = normalize(base);
        }
    }

    /// Replaces the generic MQTT sources (only the active ones are kept).
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

    /// Republishes to the sink the known state of each active device (start-up: the widgets show the last known state).
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

    /// Ingestion loop: takes the messages in packets (coalesced in case of delay) until the queue is closed.
    pub async fn run(self: Arc<Self>, mut rx: mpsc::Receiver<Incoming>) {
        let mut batch = Vec::with_capacity(MAX_BATCH);
        while rx.recv_many(&mut batch, MAX_BATCH).await > 0 {
            self.handle_batch(std::mem::take(&mut batch)).await;
        }
    }

    /// Processes a single message (tests).
    pub async fn handle(&self, msg: Incoming) {
        self.handle_batch(vec![msg]).await;
    }

    /// Processes a packet of messages, in order, after coalescing.
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
                // Coalesced: not saved, but the automations see it (in its place in the message order).
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
        // Bridge messages and commands do not trigger automations (an automation that commands a device
        // would trigger itself on seeing its own command go by).
        if matches!(
            route,
            Route::State(_) | Route::Availability(_) | Route::Other
        ) {
            self.sink.message(&msg.topic, payload);
        }
    }

    /// `bridge/devices`: creates or refreshes each device (the coordinator and invalid names are ignored).
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

    /// `bridge/logging`: only errors count (`{"level":"error","message":"z2m: Publish 'set' 'state' to 'x' failed: …"}`).
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
        // Reference: `permit_join` absent or `null` -> nothing; otherwise `bool(value)`.
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

    /// `{name}/availability`: `{"state":"online"}` or the text `online`.
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

    /// `{name}`: the state (object) replaces that of the active device; history of the tracked keys.
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

    /// History of the tracked keys (per second) then raw archive.
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

    /// Message outside the Zigbee2MQTT prefix: MQTT sources whose filter matches (log, automatic device), then
    /// the devices whose topic is the message's are marked seen. The handling code (`handler_code`, Rhai) is entrusted to the
    /// automation engine by the `Sink`.
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

    /// Creates or updates the device of a source in `auto_device` (name = topic without the source's prefix).
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

/// Beginning of a payload for the log (sources may send voluminous messages).
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
        // An availability message between two states: both states count (the following state is no longer "the next one").
        assert_eq!(coalesce(&[st("a", 1), av("a"), st("a", 1)]), [0, 1, 2]);
        // One from another device separates nothing.
        assert_eq!(coalesce(&[st("a", 1), av("b"), st("a", 1)]), [1, 2]);
        // `bridge/devices` cuts the coalescing.
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
