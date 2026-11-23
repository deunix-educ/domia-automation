// SPDX-License-Identifier: AGPL-3.0-or-later
//! Real-time broadcast hub: *subjects* ([`Topic`]) to which messages are published, and subscribers that receive them.
//!
//! Basis of `/ws/notifications/`: notifications, device state (`device:<id>`), statistics (`system`), then the
//! cameras (R6). A subject is created on demand and disappears when nobody listens to it any more.
//!
//! **Last message kept**: for a state subject (device, statistics), the last message published with
//! [`Hub::publish_retained`] is kept; a new subscriber receives it right away, and the server render of the pages can
//! display the current state without waiting for the next message. Only the server's code can retain a message (a
//! subscriber cannot): the cache is therefore bounded by the number of known devices.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use dominet_proto::{Topic, WsServerMsg};
use tokio::sync::broadcast;

/// Pending messages per subscriber before a too-slow subscriber loses some (it is then warned, not disconnected).
const TOPIC_CAPACITY: usize = 64;

/// Maximum simultaneous connections per account (tabs, devices): bounds an account's resources.
pub const MAX_CONNECTIONS_PER_USER: usize = 8;

#[derive(Debug, Default)]
struct Inner {
    topics: Mutex<HashMap<Topic, broadcast::Sender<Arc<WsServerMsg>>>>,
    retained: Mutex<HashMap<Topic, Arc<WsServerMsg>>>,
    connections: Mutex<HashMap<i64, usize>>,
}

/// Shared hub (cheap to clone).
#[derive(Debug, Clone, Default)]
pub struct Hub {
    inner: Arc<Inner>,
}

/// Counts an open connection; the account's counter goes back down when the guard is released.
#[derive(Debug)]
pub struct ConnectionGuard {
    inner: Arc<Inner>,
    user_id: i64,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        metrics::gauge!("dominet_ws_connections").decrement(1.0);
        let mut conns = lock(&self.inner.connections);
        if let Some(n) = conns.get_mut(&self.user_id) {
            *n -= 1;
            if *n == 0 {
                conns.remove(&self.user_id);
            }
        }
    }
}

// Short critical sections, without `await`: a poisoned lock only holds counters, we take it back.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Hub {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribes to a subject (creating it if needed).
    #[must_use]
    pub fn subscribe(&self, topic: &Topic) -> broadcast::Receiver<Arc<WsServerMsg>> {
        let mut topics = lock(&self.inner.topics);
        topics
            .entry(topic.clone())
            .or_insert_with(|| broadcast::channel(TOPIC_CAPACITY).0)
            .subscribe()
    }

    /// Publishes a message; returns the number of subscribers that will receive it. A subject without subscribers is removed.
    #[allow(clippy::must_use_candidate)] // the count is informative: ignoring it is legitimate
    pub fn publish(&self, topic: &Topic, msg: WsServerMsg) -> usize {
        self.send(topic, Arc::new(msg))
    }

    /// Like [`Hub::publish`], and keeps the message as the subject's **last state** (see [`Hub::retained`]).
    #[allow(clippy::must_use_candidate)]
    pub fn publish_retained(&self, topic: &Topic, msg: WsServerMsg) -> usize {
        let msg = Arc::new(msg);
        // Retained BEFORE broadcasting: a subscriber arriving between the two reads the new state, never an older one.
        lock(&self.inner.retained).insert(topic.clone(), Arc::clone(&msg));
        self.send(topic, msg)
    }

    fn send(&self, topic: &Topic, msg: Arc<WsServerMsg>) -> usize {
        let mut topics = lock(&self.inner.topics);
        let Some(tx) = topics.get(topic) else {
            return 0;
        };
        let receivers = tx.send(msg).unwrap_or(0);
        if receivers == 0 {
            topics.remove(topic);
        }
        receivers
    }

    /// Last message kept for this subject, if there is one.
    #[must_use]
    pub fn retained(&self, topic: &Topic) -> Option<Arc<WsServerMsg>> {
        lock(&self.inner.retained).get(topic).cloned()
    }

    /// Reserves a connection for this account, or `None` if it already has [`MAX_CONNECTIONS_PER_USER`].
    #[must_use]
    pub fn try_connect(&self, user_id: i64) -> Option<ConnectionGuard> {
        let mut conns = lock(&self.inner.connections);
        let n = conns.entry(user_id).or_insert(0);
        if *n >= MAX_CONNECTIONS_PER_USER {
            return None;
        }
        *n += 1;
        metrics::gauge!("dominet_ws_connections").increment(1.0);
        Some(ConnectionGuard {
            inner: Arc::clone(&self.inner),
            user_id,
        })
    }

    /// Total number of open connections.
    #[must_use]
    pub fn connection_count(&self) -> usize {
        lock(&self.inner.connections).values().sum()
    }
}

#[cfg(test)]
mod tests {
    use dominet_proto::DeviceStateDto;

    use super::*;

    fn state(id: i64, v: i64) -> WsServerMsg {
        WsServerMsg::DeviceState(DeviceStateDto {
            device_id: id,
            friendly_name: format!("d{id}"),
            state: serde_json::json!({ "v": v }),
            available: true,
        })
    }

    #[tokio::test]
    async fn messages_reach_every_subscriber_of_a_topic_only() {
        let hub = Hub::new();
        let (t, other_topic) = (Topic::Device(1), Topic::Device(2));
        let (mut a, mut b, mut other) = (
            hub.subscribe(&t),
            hub.subscribe(&t),
            hub.subscribe(&other_topic),
        );
        assert_eq!(hub.publish(&t, WsServerMsg::Ping), 2);
        assert_eq!(*a.recv().await.unwrap(), WsServerMsg::Ping);
        assert_eq!(*b.recv().await.unwrap(), WsServerMsg::Ping);
        assert!(other.try_recv().is_err(), "un autre sujet ne recoit rien");
    }

    #[test]
    fn a_topic_without_listeners_disappears_and_publishing_to_nobody_is_harmless() {
        let hub = Hub::new();
        let t = Topic::Device(1);
        assert_eq!(hub.publish(&t, WsServerMsg::Ping), 0);
        drop(hub.subscribe(&t));
        assert_eq!(hub.publish(&t, WsServerMsg::Ping), 0);
        assert!(lock(&hub.inner.topics).is_empty(), "sujet supprime");
    }

    #[test]
    fn the_last_retained_message_survives_the_topic_and_is_replaced_by_the_next() {
        let hub = Hub::new();
        let t = Topic::Device(7);
        assert!(hub.retained(&t).is_none());
        // Without a subscriber, the message is still retained (this is what the server render reads).
        assert_eq!(hub.publish_retained(&t, state(7, 1)), 0);
        assert_eq!(*hub.retained(&t).unwrap(), state(7, 1));
        assert_eq!(hub.publish_retained(&t, state(7, 2)), 0);
        assert_eq!(
            *hub.retained(&t).unwrap(),
            state(7, 2),
            "le dernier remplace le precedent"
        );
        // `publish` retains nothing; the other subjects are independent.
        hub.publish(&t, state(7, 3));
        assert_eq!(*hub.retained(&t).unwrap(), state(7, 2));
        assert!(hub.retained(&Topic::Device(8)).is_none());
        assert!(hub.retained(&Topic::System).is_none());
    }

    #[tokio::test]
    async fn a_retained_publication_is_also_delivered_live() {
        let hub = Hub::new();
        let t = Topic::Device(7);
        let mut rx = hub.subscribe(&t);
        assert_eq!(hub.publish_retained(&t, state(7, 1)), 1);
        assert_eq!(*rx.recv().await.unwrap(), state(7, 1));
    }

    #[test]
    fn connections_are_limited_per_user_and_released_on_drop() {
        let hub = Hub::new();
        let guards: Vec<_> = (0..MAX_CONNECTIONS_PER_USER)
            .map(|_| hub.try_connect(1).unwrap())
            .collect();
        assert!(hub.try_connect(1).is_none(), "limite atteinte");
        assert!(
            hub.try_connect(2).is_some(),
            "les autres comptes ne sont pas touches"
        );
        assert_eq!(hub.connection_count(), MAX_CONNECTIONS_PER_USER);
        drop(guards);
        assert_eq!(hub.connection_count(), 0);
        assert!(hub.try_connect(1).is_some());
    }

    #[tokio::test]
    async fn a_slow_subscriber_is_told_it_lagged_instead_of_blocking_publishers() {
        let hub = Hub::new();
        let t = Topic::Notifications;
        let mut slow = hub.subscribe(&t);
        for _ in 0..(TOPIC_CAPACITY + 5) {
            hub.publish(&t, WsServerMsg::Ping);
        }
        assert!(matches!(
            slow.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
    }
}
