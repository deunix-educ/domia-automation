// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ingestion of the devices' MQTT messages (R3); later automation engine, scheduler, Rhai host, alerts (R4).
//!
//! Ingestion is generic over the storage ports and over the [`ingest::Sink`]: it knows neither PostgreSQL nor the
//! WebSocket hub, which allows replaying it on message files (parity tests with the reference implementation).

pub mod alerts;
pub mod energy;
pub mod engine;
pub mod ingest;
pub mod next;
pub mod route;
pub mod scheduler;
pub mod script;

/// Is a publication topic allowed (no wildcard, non-empty, 256 characters at most)?
#[must_use]
pub(crate) fn topic_ok(topic: &str) -> bool {
    dominet_domain::device::is_valid_publish_topic(topic)
}
