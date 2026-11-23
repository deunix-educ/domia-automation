// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sending channels wired to the configuration (R5, ADR 0038): the SMTP server comes from the environment (`EMAIL_*`), the sender,
//! the alerts recipient and the Free Mobile credentials from the settings (modifiable without a restart).

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

    /// Is an SMTP server configured (`EMAIL_HOST`)?
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

    /// An e-mail from `DEFAULT_FROM_EMAIL` to `to`.
    ///
    /// # Errors
    /// `NotConfigured` without `EMAIL_HOST`, otherwise the channel's error.
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

    /// The alerts recipient: `EMAIL_RECIPIENT`, failing that `DEFAULT_FROM_EMAIL` (like the reference).
    ///
    /// # Errors
    /// See [`Self::setting`].
    pub async fn alert_recipient(&self) -> Result<String, NotifyError> {
        let to = self.setting("EMAIL_RECIPIENT").await?;
        if to.is_empty() {
            return self.setting("DEFAULT_FROM_EMAIL").await;
        }
        Ok(to)
    }

    /// An alert e-mail to the alerts recipient.
    ///
    /// # Errors
    /// `NotConfigured` if no recipient is defined.
    pub async fn send_alert_email(&self, subject: &str, body: &str) -> Result<(), NotifyError> {
        let to = self.alert_recipient().await?;
        if to.is_empty() {
            return Err(NotifyError::NotConfigured("recipient (EMAIL_RECIPIENT)"));
        }
        self.send_email(&to, subject, body).await
    }

    /// Free-form notification (`notification` automation action, plan R5 D7): an e-mail to the alerts recipient and, if `sms`,
    /// a Free Mobile SMS. Each channel is attempted even if the other fails; returns the failures (channel, reason). An unconfigured
    /// channel is not an outage: it is ignored silently.
    pub async fn notify(&self, message: &str, sms: bool) -> Vec<(&'static str, String)> {
        self.notify_with_subject("[DomiNet] Notification", message, sms)
            .await
    }

    /// Like [`Channels::notify`], with the e-mail's subject (motion, detection: `[DomiNet] Mouvement - <camera>`).
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

    /// A Free Mobile SMS (to the account's owner).
    ///
    /// # Errors
    /// `NotConfigured` without credentials (`SMS_FREE_USER` / `SMS_FREE_KEY`), otherwise the channel's error.
    pub async fn send_sms_free(&self, message: &str) -> Result<(), NotifyError> {
        let user = self.setting("SMS_FREE_USER").await?;
        let key = self.setting("SMS_FREE_KEY").await?;
        FreeMobile::new(&self.sms_free_url, &user, &key)?
            .send(message)
            .await
    }
}

/// Notes the result of a sending: an unconfigured channel is ignored (information), any other error is a failure.
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
