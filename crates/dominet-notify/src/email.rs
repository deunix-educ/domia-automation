// SPDX-License-Identifier: AGPL-3.0-or-later
//! E-mail texte brut par SMTP .

use std::fmt;
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::NotifyError;

/// Protection de la connexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// `EMAIL_USE_TLS` (port 587) : connexion claire puis `STARTTLS` **obligatoire** (pas de repli en clair).
    StartTls,
    /// `EMAIL_USE_SSL` (port 465) : TLS des l'ouverture.
    Tls,
    /// Aucune : reserve a la boucle locale (faux serveur, relais local).
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
    /// Refuse ce qui enverrait un mot de passe en clair sur le reseau.
    ///
    /// # Errors
    /// `Config` si des identifiants accompagnent `Security::Plain` vers une machine qui n'est pas la boucle locale.
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

/// Envoyeur SMTP : une connexion par message (peu de messages, comme la reference).
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
    /// Configuration invalide (voir [`SmtpConfig::validate`]) ou parametres TLS inutilisables.
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

    /// Envoie un message texte brut (UTF-8) a un destinataire.
    ///
    /// # Errors
    /// `Address` si une adresse est invalide ; `Smtp` si le serveur refuse ou est injoignable.
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
        // Un saut de ligne dans le sujet ne doit pas pouvoir ouvrir un nouvel en-tete.
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
