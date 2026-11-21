// SPDX-License-Identifier: AGPL-3.0-or-later
//! SMS Free Mobile : `GET <url>?user=&pass=&msg=` (`msg` coupe a 160 caracteres comme la reference, delai de 5 s).

use std::fmt;
use std::time::Duration;

use crate::NotifyError;

pub const DEFAULT_FREE_MOBILE_URL: &str = "https://smsapi.free-mobile.fr/sendmsg";
/// Longueur maximale du message (`msg[:160]` de la reference, en caracteres).
pub const SMS_MAX_CHARS: usize = 160;

#[derive(Clone)]
pub struct FreeMobile {
    client: reqwest::Client,
    url: String,
    user: String,
    key: String,
}

impl fmt::Debug for FreeMobile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FreeMobile")
            .field("url", &self.url)
            .field("user", &self.user)
            .field("key", &"***")
            .finish_non_exhaustive()
    }
}

impl FreeMobile {
    /// # Errors
    /// `NotConfigured` si l'identifiant ou la cle est vide ; `FreeMobile` si le client HTTP ne se construit pas.
    pub fn new(url: &str, user: &str, key: &str) -> Result<Self, NotifyError> {
        if user.is_empty() || key.is_empty() {
            return Err(NotifyError::NotConfigured("SMS Free Mobile"));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            // La cle voyage dans l'adresse : jamais de redirection vers une autre machine.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| NotifyError::FreeMobile(e.without_url().to_string()))?;
        Ok(Self {
            client,
            url: url.to_string(),
            user: user.to_string(),
            key: key.to_string(),
        })
    }

    /// Envoie un SMS a **son proprietaire** (l'API ne sait ecrire qu au numero du compte). Le code HTTP est verifie (reference l ignorait).
    ///
    /// # Errors
    /// `FreeMobile` avec la cause en clair, sans jamais la cle.
    pub async fn send(&self, message: &str) -> Result<(), NotifyError> {
        let msg: String = message.chars().take(SMS_MAX_CHARS).collect();
        let response = self
            .client
            .get(&self.url)
            .query(&[
                ("user", self.user.as_str()),
                ("pass", self.key.as_str()),
                ("msg", msg.as_str()),
            ])
            .send()
            .await
            .map_err(|e| NotifyError::FreeMobile(e.without_url().to_string()))?;
        let status = response.status().as_u16();
        match status {
            200 => Ok(()),
            400 => Err(NotifyError::FreeMobile("missing parameter (400)".into())),
            402 => Err(NotifyError::FreeMobile(
                "too many SMS sent in a short time (402)".into(),
            )),
            403 => Err(NotifyError::FreeMobile(
                "service not activated or credentials refused (403)".into(),
            )),
            500 => Err(NotifyError::FreeMobile(
                "Free Mobile server error (500)".into(),
            )),
            other => Err(NotifyError::FreeMobile(format!(
                "unexpected response ({other})"
            ))),
        }
    }
}
