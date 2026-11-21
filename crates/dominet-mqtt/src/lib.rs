// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client MQTT supervise (`rumqttc`, ADR 0005 et 0026), decodage des charges utiles et correspondance de topics.
//!
//! Le client tourne dans une tache : il se (re)connecte tout seul, **resouscrit a chaque `ConnAck`** (la session du broker peut
//! etre perdue), applique les changements d'abonnements demandes a chaud et remet les messages recus, deja decodes, au
//! consommateur. Une publication echoue **tout de suite** si le lien est coupe : une commande en attente ne doit pas s'executer
//! dix minutes plus tard.

mod client;
mod matching;
mod payload;

pub use client::{MqttClient, MqttSettings, PublishError, Started, Status, start};
pub use matching::{matches_any, topic_matches};
pub use payload::{Incoming, Payload, decode_payload};
