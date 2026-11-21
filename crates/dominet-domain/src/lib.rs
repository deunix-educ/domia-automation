// SPDX-License-Identifier: AGPL-3.0-or-later
//! Types metier purs et ports (traits) : aucune I/O, aucun framework.
//!
//! Les regles de validation vivent ici (noms, mobiles, URL de navigation, valeurs de reglages) pour
//! etre appliquees de la meme facon par le serveur, la CLI et les tests.

pub mod alert;
pub mod android;
pub mod auth;
pub mod automation;
pub mod camera;
pub mod command;
pub mod context;
pub mod cron;
pub mod dashboard;
pub mod device;
pub mod energy;
pub mod error;
pub mod grafana;
pub mod location;
pub mod mcp;
pub mod messaging;
pub mod navbar;
pub mod observability;
pub mod public_url;
pub mod registry;
pub mod repo;
pub mod scenarios;
pub mod scene;
pub mod schedule;
pub mod setting;
pub mod solar;
pub mod user;
pub mod widget_view;

pub use error::{Problem, RepoError, ValidationError};
