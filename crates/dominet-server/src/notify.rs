// SPDX-License-Identifier: AGPL-3.0-or-later
//! Canaux d'envoi branches sur la configuration (R5, ADR 0038) : le serveur SMTP vient de l'environnement (`EMAIL_*`), l'expediteur,
//! le destinataire des alertes et les identifiants Free Mobile des reglages (modifiables sans redemarrage).

use dominet_notify::{FreeMobile, NotifyError, SmtpSender};

use crate::config::{Config, EmailConfig};
use crate::settings::SettingsService;

#[derive(Debug, Clone)]
pub struct Channels {
    email: Option<EmailConfig>,
    sms_free_url: String,
    settings: SettingsService,
}

impl Channels {
    #[must_use]
    pub fn new(config: &Config, settings: SettingsService) -> Self {
        Self {
            email: config.email.clone(),
            sms_free_url: config.sms_free_url.clone(),
            settings,
        }
    }

    /// Un serveur SMTP est-il configure (`EMAIL_HOST`) ?
    #[must_use]
    pub fn email_configured(&self) -> bool {
        self.email.is_some()
    }

    async fn setting(&self, key: &str) -> Result<String, NotifyError> {
        self.settings
            .get_string(key)
            .await
            .map(|v| v.trim().to_string())
            .map_err(|e| NotifyError::Config(format!("setting {key}: {e}")))
    }

    /// Un e-mail de `DEFAULT_FROM_EMAIL` vers `to`.
    ///
    /// # Errors
    /// `NotConfigured` sans `EMAIL_HOST`, sinon l'erreur du canal.
    pub async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<(), NotifyError> {
        let cfg = self
            .email
            .as_ref()
            .ok_or(NotifyError::NotConfigured("SMTP server (EMAIL_HOST)"))?;
        let from = self.setting("DEFAULT_FROM_EMAIL").await?;
        SmtpSender::new(&cfg.smtp())?
            .send(&from, to, subject, body)
            .await
    }

    /// Le destinataire des alertes : `EMAIL_RECIPIENT`, a defaut `DEFAULT_FROM_EMAIL` (comme la reference).
    ///
    /// # Errors
    /// Voir [`Self::setting`].
    pub async fn alert_recipient(&self) -> Result<String, NotifyError> {
        let to = self.setting("EMAIL_RECIPIENT").await?;
        if to.is_empty() {
            return self.setting("DEFAULT_FROM_EMAIL").await;
        }
        Ok(to)
    }

    /// Un e-mail d'alerte au destinataire des alertes.
    ///
    /// # Errors
    /// `NotConfigured` si aucun destinataire n'est defini.
    pub async fn send_alert_email(&self, subject: &str, body: &str) -> Result<(), NotifyError> {
        let to = self.alert_recipient().await?;
        if to.is_empty() {
            return Err(NotifyError::NotConfigured("recipient (EMAIL_RECIPIENT)"));
        }
        self.send_email(&to, subject, body).await
    }

    /// Notification libre (action d'automation `notification`, plan R5 D7) : un e-mail au destinataire des alertes et, si `sms`,
    /// un SMS Free Mobile. Chaque canal est tente meme si l'autre echoue ; renvoie les echecs (canal, raison). Un canal non
    /// configure n'est pas une panne : il est ignore sans bruit.
    pub async fn notify(&self, message: &str, sms: bool) -> Vec<(&'static str, String)> {
        self.notify_with_subject("[DomiNet] Notification", message, sms)
            .await
    }

    /// Comme [`Channels::notify`], avec l'objet de l'e-mail (mouvement, detection : `[DomiNet] Mouvement - <camera>`).
    pub async fn notify_with_subject(
        &self,
        subject: &str,
        message: &str,
        sms: bool,
    ) -> Vec<(&'static str, String)> {
        let mut failures = vec![];
        record(
            &mut failures,
            "email",
            self.send_alert_email(subject, message).await,
        );
        if sms {
            record(&mut failures, "sms", self.send_sms_free(message).await);
        }
        failures
    }

    /// Un SMS Free Mobile (au proprietaire du compte).
    ///
    /// # Errors
    /// `NotConfigured` sans identifiants (`SMS_FREE_USER` / `SMS_FREE_KEY`), sinon l'erreur du canal.
    pub async fn send_sms_free(&self, message: &str) -> Result<(), NotifyError> {
        let user = self.setting("SMS_FREE_USER").await?;
        let key = self.setting("SMS_FREE_KEY").await?;
        FreeMobile::new(&self.sms_free_url, &user, &key)?
            .send(message)
            .await
    }
}

/// Note le resultat d'un envoi : un canal non configure est ignore (information), toute autre erreur est un echec.
pub fn record(
    failures: &mut Vec<(&'static str, String)>,
    channel: &'static str,
    result: Result<(), NotifyError>,
) {
    match result {
        Ok(()) => tracing::info!(channel, "notification sent"),
        Err(NotifyError::NotConfigured(what)) => {
            tracing::info!(channel, %what, "channel not configured: notification not sent");
        }
        Err(e) => {
            tracing::warn!(channel, error = %e, "notification not sent");
            failures.push((channel, e.to_string()));
        }
    }
}
