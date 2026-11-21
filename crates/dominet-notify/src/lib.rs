// SPDX-License-Identifier: AGPL-3.0-or-later
//! Canaux d'envoi (R5, ADR 0038) : e-mail par SMTP (`lettre`) et SMS Free Mobile (HTTP).
//!
//! Aucun secret ne sort d'ici : ni `Debug`, ni les erreurs ne contiennent le mot de passe SMTP ni la cle Free Mobile (que
//! `reqwest` mettrait sinon dans l'adresse de ses erreurs). Les tests parlent a de **faux serveurs** (feature `testing`) :
//! aucun envoi reel (plan R5, D1).

mod email;
mod freemobile;
#[cfg(feature = "testing")]
pub mod testing;

pub use email::{Security, SmtpConfig, SmtpSender};
pub use freemobile::{DEFAULT_FREE_MOBILE_URL, FreeMobile, SMS_MAX_CHARS};

#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    #[error("{0} non configure")]
    NotConfigured(&'static str),
    #[error("adresse e-mail invalide : {0}")]
    Address(String),
    #[error("configuration SMTP invalide : {0}")]
    Config(String),
    #[error("envoi SMTP impossible : {0}")]
    Smtp(String),
    #[error("Free Mobile : {0}")]
    FreeMobile(String),
}
