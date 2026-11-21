// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hub de diffusion temps reel : des *sujets* ([`Topic`]) a qui on publie des messages, et des abonnes qui les recoivent.
//!
//! Base de `/ws/notifications/` : notifications, etat des appareils (`device:<id>`), statistiques (`system`), puis les
//! cameras (R6). Un sujet est cree a la demande et disparait quand plus personne ne l'ecoute.
//!
//! **Dernier message retenu** : pour un sujet d'etat (appareil, statistiques), le dernier message publie avec
//! [`Hub::publish_retained`] est conserve ; un nouvel abonne le recoit tout de suite, et le rendu serveur des pages peut
//! afficher l'etat courant sans attendre le prochain message. Seul le code du serveur peut retenir un message (un
//! abonne ne le peut pas) : le cache est donc borne par le nombre d'appareils connus.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use dominet_proto::{Topic, WsServerMsg};
use tokio::sync::broadcast;

/// Messages en attente par abonne avant qu'un abonne trop lent n'en perde (il est alors prevenu, pas deconnecte).
const TOPIC_CAPACITY: usize = 64;

/// Connexions simultanees maximales par compte (onglets, appareils) : borne les ressources d'un compte.
pub const MAX_CONNECTIONS_PER_USER: usize = 8;

#[derive(Debug, Default)]
struct Inner {
    topics: Mutex<HashMap<Topic, broadcast::Sender<Arc<WsServerMsg>>>>,
    retained: Mutex<HashMap<Topic, Arc<WsServerMsg>>>,
    connections: Mutex<HashMap<i64, usize>>,
}

/// Hub partage (clonable a bas cout).
#[derive(Debug, Clone, Default)]
pub struct Hub {
    inner: Arc<Inner>,
}

/// Compte une connexion ouverte ; le compteur du compte redescend quand la garde est liberee.
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

// Sections critiques courtes, sans `await` : un verrou empoisonne ne contient que des compteurs, on le reprend.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Hub {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// S'abonne a un sujet (le cree au besoin).
    #[must_use]
    pub fn subscribe(&self, topic: &Topic) -> broadcast::Receiver<Arc<WsServerMsg>> {
        let mut topics = lock(&self.inner.topics);
        topics
            .entry(topic.clone())
            .or_insert_with(|| broadcast::channel(TOPIC_CAPACITY).0)
            .subscribe()
    }

    /// Publie un message ; renvoie le nombre d'abonnes qui le recevront. Un sujet sans abonne est supprime.
    #[allow(clippy::must_use_candidate)] // le decompte est informatif : l'ignorer est legitime
    pub fn publish(&self, topic: &Topic, msg: WsServerMsg) -> usize {
        self.send(topic, Arc::new(msg))
    }

    /// Comme [`Hub::publish`], et conserve le message comme **dernier etat** du sujet (voir [`Hub::retained`]).
    #[allow(clippy::must_use_candidate)]
    pub fn publish_retained(&self, topic: &Topic, msg: WsServerMsg) -> usize {
        let msg = Arc::new(msg);
        // Retenu AVANT de diffuser : un abonne qui arrive entre les deux lit le nouvel etat, jamais un etat plus ancien.
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

    /// Dernier message retenu pour ce sujet, s'il y en a un.
    #[must_use]
    pub fn retained(&self, topic: &Topic) -> Option<Arc<WsServerMsg>> {
        lock(&self.inner.retained).get(topic).cloned()
    }

    /// Reserve une connexion pour ce compte, ou `None` s'il en a deja [`MAX_CONNECTIONS_PER_USER`].
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

    /// Nombre total de connexions ouvertes.
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
        // Sans abonne, le message est quand meme retenu (c'est ce que lit le rendu serveur).
        assert_eq!(hub.publish_retained(&t, state(7, 1)), 0);
        assert_eq!(*hub.retained(&t).unwrap(), state(7, 1));
        assert_eq!(hub.publish_retained(&t, state(7, 2)), 0);
        assert_eq!(
            *hub.retained(&t).unwrap(),
            state(7, 2),
            "le dernier remplace le precedent"
        );
        // `publish` ne retient rien ; les autres sujets sont independants.
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
