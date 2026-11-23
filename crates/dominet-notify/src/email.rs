// SPDX-License-Identifier: AGPL-3.0-or-later
//! Plain-text e-mail over SMTP.

use std::fmt;
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::NotifyError;

/// Connection protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// `EMAIL_USE_TLS` (port 587): clear connection, then **mandatory** `STARTTLS` (no fallback to clear text).
    StartTls,
    /// `EMAIL_USE_SSL` (port 465): TLS from the start.
    Tls,
    /// None: reserved for the loopback interface (fake server, local relay).
    Plain,
}

#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub security: Security,
    pub timeout: Duration,
}

impl fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("security", &self.security)
            .field("timeout", &self.timeout)
            .finish()
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl SmtpConfig {
    /// Refuses anything that would send a password in clear text over the network.
    ///
    /// # Errors
    /// `Config` if credentials come with `Security::Plain` towards a machine that is not the loopback interface.
    pub fn validate(&self) -> Result<(), NotifyError> {
        if self.host.trim().is_empty() {
            return Err(NotifyError::NotConfigured("SMTP server"));
        }
        if self.security == Security::Plain
            && (self.user.is_some() || self.password.is_some())
            && !is_loopback(&self.host)
        {
            return Err(NotifyError::Config(
                "credentials without TLS are only accepted towards the loopback (EMAIL_USE_TLS or EMAIL_USE_SSL)".into(),
            ));
        }
        Ok(())
    }
}

/// SMTP sender: one connection per message (few messages, like the reference).
#[derive(Clone)]
pub struct SmtpSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl fmt::Debug for SmtpSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SmtpSender")
    }
}

impl SmtpSender {
    /// # Errors
    /// Invalid configuration (see [`SmtpConfig::validate`]) or unusable TLS parameters.
    pub fn new(cfg: &SmtpConfig) -> Result<Self, NotifyError> {
        cfg.validate()?;
        let tls = |e: lettre::transport::smtp::Error| NotifyError::Config(e.to_string());
        let params = || TlsParameters::new(cfg.host.clone()).map_err(tls);
        let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            .port(cfg.port)
            .timeout(Some(cfg.timeout))
            .tls(match cfg.security {
                Security::StartTls => Tls::Required(params()?),
                Security::Tls => Tls::Wrapper(params()?),
                Security::Plain => Tls::None,
            });
        if let (Some(user), Some(password)) = (&cfg.user, &cfg.password) {
            b = b.credentials(Credentials::new(user.clone(), password.clone()));
        }
        Ok(Self {
            transport: b.build(),
        })
    }

    /// Sends a plain-text (UTF-8) message to one recipient.
    ///
    /// # Errors
    /// `Address` if an address is invalid; `Smtp` if the server refuses or cannot be reached.
    pub async fn send(
        &self,
        from: &str,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), NotifyError> {
        let mailbox = |s: &str| {
            s.trim()
                .parse::<Mailbox>()
                .map_err(|e| NotifyError::Address(format!("{s} ({e})")))
        };
        // A line break in the subject must not be able to open a new header.
        let subject: String = subject
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let message = Message::builder()
            .from(mailbox(from)?)
            .to(mailbox(to)?)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| NotifyError::Smtp(e.to_string()))?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|e| NotifyError::Smtp(e.to_string()))
    }
}
