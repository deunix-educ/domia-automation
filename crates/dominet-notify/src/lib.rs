// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sending channels (R5, ADR 0038): e-mail over SMTP (`lettre`) and Free Mobile SMS (HTTP).
//!
//! No secret leaves this crate: neither `Debug` nor the errors contain the SMTP password or the Free Mobile key (which
//! `reqwest` would otherwise put in the address of its errors). The tests talk to **fake servers** (`testing` feature):
//! nothing is ever really sent (plan R5, D1).

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
