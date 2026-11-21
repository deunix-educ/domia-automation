// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ingestion des messages MQTT des appareils (R3) ; plus tard moteur d'automations, planificateur, hote Rhai, alertes (R4).
//!
//! L'ingestion est generique sur les ports de stockage et sur le [`ingest::Sink`] : elle ne connait ni PostgreSQL ni le hub
//! WebSocket, ce qui permet de la rejouer sur des fichiers de messages (tests de parite avec l'implementation de reference).

pub mod alerts;
pub mod energy;
pub mod engine;
pub mod ingest;
pub mod next;
pub mod route;
pub mod scheduler;
pub mod script;

/// Un topic de publication est-il admis (pas de joker, non vide, 256 caracteres au plus) ?
#[must_use]
pub(crate) fn topic_ok(topic: &str) -> bool {
    dominet_domain::device::is_valid_publish_topic(topic)
}
