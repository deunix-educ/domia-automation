// SPDX-License-Identifier: AGPL-3.0-or-later
//! Supervised MQTT client (`rumqttc`, ADR 0005 and 0026), payload decoding and topic matching.
//!
//! The client runs in a task: it (re)connects by itself, **resubscribes on every `ConnAck`** (the broker's session may
//! be lost), applies the subscription changes requested at run time and hands the received messages, already decoded, to the
//! consumer. A publication fails **immediately** if the link is down: a pending command must not run ten minutes later.

mod client;
mod matching;
mod payload;

pub use client::{MqttClient, MqttSettings, PublishError, Started, Status, start};
pub use matching::{matches_any, topic_matches};
pub use payload::{Incoming, Payload, decode_payload};
