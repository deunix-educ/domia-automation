// SPDX-License-Identifier: AGPL-3.0-or-later
//! Supervision task of the MQTT client (see the crate documentation).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use rumqttc::{
    AsyncClient, ConnAck, ConnectReturnCode, Event, EventLoop, MqttOptions, Packet, QoS,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::payload::{Incoming, decode_payload};

/// Maximum size of a received message (`bridge/devices` of Zigbee2MQTT can exceed 100 KiB) and of a sent one.
const MAX_INCOMING: usize = 1024 * 1024;
const MAX_OUTGOING: usize = 256 * 1024;
/// Queue of received messages: when it is full, reading from the network waits and the broker queues. Nothing is lost as long
/// as the broker's queue is enough: `max_queued_messages` (1000 by default in Mosquitto, 100000 in our configurations).
const INBOX: usize = 8192;
/// Queue of requests to the broker.
const OUTBOX: usize = 4096;
const BACKOFF_MIN: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Connection settings. The password never appears in `Debug`.
#[derive(Clone)]
pub struct MqttSettings {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    /// **Stable** client identifier (persistent session), distinct from that of any other service.
    pub client_id: String,
    pub keep_alive: Duration,
}

impl MqttSettings {
    #[must_use]
    pub fn new(host: &str, port: u16, client_id: &str) -> Self {
        Self {
            host: host.to_string(),
            port,
            user: None,
            password: None,
            client_id: client_id.to_string(),
            keep_alive: Duration::from_secs(30),
        }
    }
}

impl fmt::Debug for MqttSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("client_id", &self.client_id)
            .field("keep_alive", &self.keep_alive)
            .finish()
    }
}

/// State of the link with the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// First connection in progress.
    Connecting,
    Connected,
    /// Link down; reconnections continue.
    Disconnected,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PublishError {
    /// No link with the broker: the publication is refused rather than queued.
    #[error("MQTT broker unreachable")]
    NotConnected,
    #[error("topic de publication invalide")]
    InvalidTopic,
    #[error("message trop gros")]
    TooLarge,
    #[error("QoS invalide")]
    InvalidQos,
    /// The queue to the broker is full.
    #[error("file d'envoi pleine")]
    Busy,
}

/// Shareable handle of the client.
#[derive(Debug, Clone)]
pub struct MqttClient {
    client: AsyncClient,
    desired: Arc<watch::Sender<BTreeMap<String, u8>>>,
    status: watch::Receiver<Status>,
    shutdown: Arc<watch::Sender<bool>>,
}

/// What [`start`] returns.
#[derive(Debug)]
pub struct Started {
    pub client: MqttClient,
    /// Received messages, decoded, in order of arrival.
    pub messages: mpsc::Receiver<Incoming>,
    /// Supervision task; ends after [`MqttClient::shutdown`].
    pub task: JoinHandle<()>,
}

fn qos_of(level: u8) -> Option<QoS> {
    match level {
        0 => Some(QoS::AtMostOnce),
        1 => Some(QoS::AtLeastOnce),
        2 => Some(QoS::ExactlyOnce),
        _ => None,
    }
}

fn level_of(qos: QoS) -> u8 {
    match qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 1,
        QoS::ExactlyOnce => 2,
    }
}

/// A publication topic has no wildcard and no null character and is not empty.
fn valid_publish_topic(topic: &str) -> bool {
    !topic.is_empty() && topic.len() <= 65_535 && !topic.contains(['+', '#', '\0'])
}

impl MqttClient {
    /// Current state of the link.
    #[must_use]
    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    /// Follow the state changes.
    #[must_use]
    pub fn watch_status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// Replaces the whole set of wanted subscriptions (filter -> QoS). The differences are applied immediately if the link is
    /// up, otherwise at the next connection; after a reconnection, the whole set is subscribed again.
    pub fn set_subscriptions(&self, filters: BTreeMap<String, u8>) {
        self.desired.send_replace(filters);
    }

    /// Publishes a message.
    ///
    /// # Errors
    /// See [`PublishError`]: link down, invalid topic or QoS, message too big, queue full.
    pub fn publish(
        &self,
        topic: &str,
        payload: impl Into<Vec<u8>>,
        qos: u8,
        retain: bool,
    ) -> Result<(), PublishError> {
        if !valid_publish_topic(topic) {
            return Err(PublishError::InvalidTopic);
        }
        let qos = qos_of(qos).ok_or(PublishError::InvalidQos)?;
        let payload = payload.into();
        if payload.len() > MAX_OUTGOING {
            return Err(PublishError::TooLarge);
        }
        if self.status() != Status::Connected {
            return Err(PublishError::NotConnected);
        }
        self.client
            .try_publish(topic, qos, retain, payload)
            .map_err(|_| PublishError::Busy)
    }

    /// Requests the stop: clean disconnection, then the end of the task.
    pub fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }
}

/// Removes duplicates and computes what to subscribe / unsubscribe to go from `current` to `desired`. A QoS change
/// counts as a new subscription.
#[must_use]
pub fn diff_subscriptions(
    desired: &BTreeMap<String, u8>,
    current: &BTreeMap<String, u8>,
) -> (Vec<(String, u8)>, Vec<String>) {
    let add = desired
        .iter()
        .filter(|(f, q)| current.get(*f) != Some(q))
        .map(|(f, q)| (f.clone(), *q))
        .collect();
    let remove = current
        .keys()
        .filter(|f| !desired.contains_key(*f))
        .cloned()
        .collect();
    (add, remove)
}

/// Starts the client. Nothing blocks: the connection and its retries happen in the task.
#[must_use]
pub fn start(settings: &MqttSettings) -> Started {
    let mut opts = MqttOptions::new(&settings.client_id, &settings.host, settings.port);
    opts.set_keep_alive(settings.keep_alive);
    // Persistent session: the broker keeps the subscriptions and the QoS 1 messages during a short outage (and we
    // resubscribe on every connection anyway).
    opts.set_clean_session(false);
    opts.set_max_packet_size(MAX_INCOMING, MAX_OUTGOING);
    if let (Some(user), password) = (&settings.user, &settings.password) {
        opts.set_credentials(user, password.as_deref().unwrap_or(""));
    }
    let (client, eventloop) = AsyncClient::new(opts, OUTBOX);
    let (status_tx, status_rx) = watch::channel(Status::Connecting);
    let (desired_tx, desired_rx) = watch::channel(BTreeMap::new());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (inbox_tx, inbox_rx) = mpsc::channel(INBOX);
    let handle = MqttClient {
        client: client.clone(),
        desired: Arc::new(desired_tx),
        status: status_rx,
        shutdown: Arc::new(shutdown_tx),
    };
    let task = tokio::spawn(supervise(
        settings.host.clone(),
        settings.port,
        client,
        eventloop,
        desired_rx,
        status_tx,
        shutdown_rx,
        inbox_tx,
    ));
    Started {
        client: handle,
        messages: inbox_rx,
        task,
    }
}

fn sync_subscriptions(
    client: &AsyncClient,
    desired: &BTreeMap<String, u8>,
    current: &mut BTreeMap<String, u8>,
) {
    let (add, remove) = diff_subscriptions(desired, current);
    for filter in remove {
        if client.try_unsubscribe(&filter).is_ok() {
            current.remove(&filter);
        }
    }
    for (filter, level) in add {
        let Some(qos) = qos_of(level) else { continue };
        match client.try_subscribe(&filter, qos) {
            Ok(()) => {
                current.insert(filter, level);
            }
            Err(e) => tracing::warn!(%filter, error = %e, "MQTT subscription failed"),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    host: String,
    port: u16,
    client: AsyncClient,
    mut eventloop: EventLoop,
    mut desired: watch::Receiver<BTreeMap<String, u8>>,
    status: watch::Sender<Status>,
    mut shutdown: watch::Receiver<bool>,
    inbox: mpsc::Sender<Incoming>,
) {
    let mut subscribed: BTreeMap<String, u8> = BTreeMap::new();
    let mut backoff = BACKOFF_MIN;
    // Second handle on the stop, for the pauses (the first one is borrowed by the `select!`).
    let mut pause_rx = shutdown.clone();
    loop {
        tokio::select! {
            biased;
            () = stopped(&mut shutdown) => break,
            changed = desired.changed() => {
                if changed.is_err() {
                    break; // no handle left: nobody is waiting for this client
                }
                if *status.borrow() == Status::Connected {
                    let wanted = desired.borrow_and_update().clone();
                    sync_subscriptions(&client, &wanted, &mut subscribed);
                }
            }
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::ConnAck(ConnAck { code: ConnectReturnCode::Success, .. }))) => {
                    backoff = BACKOFF_MIN;
                    // Fresh connection: we assume nothing about the broker's session.
                    subscribed.clear();
                    let wanted = desired.borrow_and_update().clone();
                    sync_subscriptions(&client, &wanted, &mut subscribed);
                    if status.send_replace(Status::Connected) != Status::Connected {
                        tracing::info!(%host, port, "MQTT connected");
                    }
                }
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    tracing::error!(%host, port, code = ?ack.code, "MQTT connection refused by the broker");
                    status.send_replace(Status::Disconnected);
                    pause(&mut pause_rx, &mut backoff).await;
                }
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    let message = Incoming {
                        payload: decode_payload(&p.payload),
                        topic: p.topic,
                        retain: p.retain,
                        qos: level_of(p.qos),
                    };
                    metrics::counter!("dominet_mqtt_messages_received_total").increment(1);
                    if inbox.send(message).await.is_err() {
                        tracing::warn!("no more MQTT consumer: stopping the client");
                        break;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    if status.send_replace(Status::Disconnected) != Status::Disconnected {
                        metrics::counter!("dominet_mqtt_disconnections_total").increment(1);
                        tracing::warn!(%host, port, error = %e, "MQTT disconnected, reconnecting");
                    }
                    pause(&mut pause_rx, &mut backoff).await;
                }
            },
        }
    }
    // Stop: clean disconnection (short wait for the packet to be sent), then nothing more.
    let _ = client.try_disconnect();
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while eventloop.poll().await.is_ok() {}
    })
    .await;
    status.send_replace(Status::Disconnected);
}

/// Waits until the stop is requested (returns immediately if it already was). The channel's read guard stays inside this
/// function: it must not cross an `await` of a task.
async fn stopped(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|stop| *stop).await;
}

/// Waits before retrying a connection (growing delay), unless the stop is requested.
async fn pause(shutdown: &mut watch::Receiver<bool>, backoff: &mut Duration) {
    tokio::select! {
        () = tokio::time::sleep(*backoff) => {}
        () = stopped(shutdown) => {}
    }
    *backoff = (*backoff * 2).min(BACKOFF_MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(&str, u8)]) -> BTreeMap<String, u8> {
        items.iter().map(|(f, q)| ((*f).to_string(), *q)).collect()
    }

    #[test]
    fn subscription_changes_are_the_difference_between_wanted_and_current() {
        let (add, remove) = diff_subscriptions(
            &map(&[("a/#", 1), ("b/+", 0)]),
            &map(&[("a/#", 1), ("old/#", 1)]),
        );
        assert_eq!(add, [("b/+".to_string(), 0)]);
        assert_eq!(remove, ["old/#".to_string()]);
        // Nothing to do when everything is already subscribed; a QoS change is resubscribed.
        assert_eq!(
            diff_subscriptions(&map(&[("a", 1)]), &map(&[("a", 1)])),
            (vec![], vec![])
        );
        assert_eq!(
            diff_subscriptions(&map(&[("a", 1)]), &map(&[("a", 0)])).0,
            [("a".to_string(), 1)]
        );
        // After a reconnection, `current` is empty: everything is to be subscribed.
        assert_eq!(
            diff_subscriptions(&map(&[("a", 1), ("b", 0)]), &map(&[]))
                .0
                .len(),
            2
        );
    }

    #[test]
    fn publish_topics_cannot_contain_wildcards() {
        for ok in ["a/b", "zigbee2mqtt/salon/lampe/set", "meteo/paris"] {
            assert!(valid_publish_topic(ok), "{ok}");
        }
        for bad in ["", "a/#", "a/+/b", "a\0b", &"x".repeat(70_000)] {
            assert!(!valid_publish_topic(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_password_never_shows_in_debug() {
        let mut s = MqttSettings::new("broker", 1883, "dominet");
        s.user = Some("dominet".into());
        s.password = Some("tres-secret".into());
        let shown = format!("{s:?}");
        assert!(
            shown.contains("dominet") && !shown.contains("tres-secret"),
            "{shown}"
        );
    }

    #[test]
    fn qos_levels_convert_both_ways() {
        for level in 0..=2 {
            assert_eq!(level_of(qos_of(level).unwrap()), level);
        }
        assert!(qos_of(3).is_none());
    }

    #[tokio::test]
    async fn without_a_broker_publishing_fails_fast_and_status_stays_down() {
        // Closed port: the connection fails, no publication is queued.
        let started = start(&MqttSettings::new("127.0.0.1", 1, "test-no-broker"));
        let c = started.client.clone();
        assert_eq!(
            c.publish("a/b", "x", 1, false),
            Err(PublishError::NotConnected)
        );
        assert_eq!(
            c.publish("a/#", "x", 1, false),
            Err(PublishError::InvalidTopic)
        );
        assert_eq!(
            c.publish("a/b", "x", 3, false),
            Err(PublishError::InvalidQos)
        );
        assert_eq!(
            c.publish("a/b", vec![0u8; MAX_OUTGOING + 1], 1, false),
            Err(PublishError::TooLarge)
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(c.status(), Status::Disconnected);
        c.shutdown();
        tokio::time::timeout(Duration::from_secs(5), started.task)
            .await
            .expect("arret")
            .unwrap();
    }
}
