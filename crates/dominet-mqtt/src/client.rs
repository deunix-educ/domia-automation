// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tache de supervision du client MQTT (voir la documentation du crate).

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

/// Taille maximale d'un message recu (`bridge/devices` de Zigbee2MQTT peut depasser 100 Kio) et emis.
const MAX_INCOMING: usize = 1024 * 1024;
const MAX_OUTGOING: usize = 256 * 1024;
/// File de messages recus : quand elle est pleine, la lecture du reseau attend et le broker met en attente. Rien n'est perdu tant
/// que la file du broker suffit : `max_queued_messages` (1 000 par defaut dans Mosquitto, 100 000 dans nos configurations).
const INBOX: usize = 8192;
/// File des requetes vers le broker.
const OUTBOX: usize = 4096;
const BACKOFF_MIN: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Reglages de connexion. Le mot de passe n'apparait jamais dans `Debug`.
#[derive(Clone)]
pub struct MqttSettings {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    /// Identifiant de client **stable** (session persistante) et distinct de celui d'un autre service.
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

/// Etat du lien avec le broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Premiere connexion en cours.
    Connecting,
    Connected,
    /// Lien coupe ; les reconnexions se poursuivent.
    Disconnected,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PublishError {
    /// Pas de lien avec le broker : la publication est refusee plutot que mise en attente.
    #[error("MQTT broker unreachable")]
    NotConnected,
    #[error("topic de publication invalide")]
    InvalidTopic,
    #[error("message trop gros")]
    TooLarge,
    #[error("QoS invalide")]
    InvalidQos,
    /// La file vers le broker est pleine.
    #[error("file d'envoi pleine")]
    Busy,
}

/// Poignee partageable du client.
#[derive(Debug, Clone)]
pub struct MqttClient {
    client: AsyncClient,
    desired: Arc<watch::Sender<BTreeMap<String, u8>>>,
    status: watch::Receiver<Status>,
    shutdown: Arc<watch::Sender<bool>>,
}

/// Ce que renvoie [`start`].
#[derive(Debug)]
pub struct Started {
    pub client: MqttClient,
    /// Messages recus, decodes, dans l'ordre d'arrivee.
    pub messages: mpsc::Receiver<Incoming>,
    /// Tache de supervision ; se termine apres [`MqttClient::shutdown`].
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

/// Un topic de publication n'a ni joker ni caractere nul et n'est pas vide.
fn valid_publish_topic(topic: &str) -> bool {
    !topic.is_empty() && topic.len() <= 65_535 && !topic.contains(['+', '#', '\0'])
}

impl MqttClient {
    /// Etat courant du lien.
    #[must_use]
    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    /// Suivre les changements d'etat.
    #[must_use]
    pub fn watch_status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// Remplace l'ensemble des abonnements voulus (filtre -> QoS). Les differences sont appliquees tout de suite si le lien est
    /// etabli, sinon a la prochaine connexion ; apres une reconnexion, tout l'ensemble est souscrit de nouveau.
    pub fn set_subscriptions(&self, filters: BTreeMap<String, u8>) {
        self.desired.send_replace(filters);
    }

    /// Publie un message.
    ///
    /// # Errors
    /// Voir [`PublishError`] : lien coupe, topic ou QoS invalide, message trop gros, file pleine.
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

    /// Demande l'arret : deconnexion propre puis fin de la tache.
    pub fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }
}

/// Retire les doublons et calcule ce qu'il faut souscrire / resilier pour passer de `current` a `desired`. Un changement de QoS
/// compte comme une nouvelle souscription.
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

/// Demarre le client. Rien ne bloque : la connexion et ses reprises se font dans la tache.
#[must_use]
pub fn start(settings: &MqttSettings) -> Started {
    let mut opts = MqttOptions::new(&settings.client_id, &settings.host, settings.port);
    opts.set_keep_alive(settings.keep_alive);
    // Session persistante : le broker garde les abonnements et les messages QoS 1 pendant une coupure courte (et on
    // resouscrit de toute facon a chaque connexion).
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
    // Deuxieme poignee sur l'arret, pour les pauses (la premiere est empruntee par le `select!`).
    let mut pause_rx = shutdown.clone();
    loop {
        tokio::select! {
            biased;
            () = stopped(&mut shutdown) => break,
            changed = desired.changed() => {
                if changed.is_err() {
                    break; // plus aucune poignee : personne n'attend ce client
                }
                if *status.borrow() == Status::Connected {
                    let wanted = desired.borrow_and_update().clone();
                    sync_subscriptions(&client, &wanted, &mut subscribed);
                }
            }
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::ConnAck(ConnAck { code: ConnectReturnCode::Success, .. }))) => {
                    backoff = BACKOFF_MIN;
                    // Connexion neuve : on ne suppose rien de la session du broker.
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
    // Arret : deconnexion propre (courte attente de l'envoi du paquet), puis plus rien.
    let _ = client.try_disconnect();
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while eventloop.poll().await.is_ok() {}
    })
    .await;
    status.send_replace(Status::Disconnected);
}

/// Attend que l'arret soit demande (rend la main tout de suite s'il l'a deja ete). Le garde de lecture du canal reste dans cette
/// fonction : il ne doit pas traverser un `await` d'une tache.
async fn stopped(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|stop| *stop).await;
}

/// Attend avant de retenter une connexion (delai croissant), sauf si l'arret est demande.
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
        // Rien a faire quand tout est deja souscrit ; un changement de QoS se resouscrit.
        assert_eq!(
            diff_subscriptions(&map(&[("a", 1)]), &map(&[("a", 1)])),
            (vec![], vec![])
        );
        assert_eq!(
            diff_subscriptions(&map(&[("a", 1)]), &map(&[("a", 0)])).0,
            [("a".to_string(), 1)]
        );
        // Apres une reconnexion, `current` est vide : tout est a souscrire.
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
        // Port ferme : la connexion echoue, aucune publication n'est mise en attente.
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
