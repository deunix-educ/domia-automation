// SPDX-License-Identifier: AGPL-3.0-or-later
//! Free Mobile SMS: `GET <url>?user=&pass=&msg=` (`msg` cut to 160 characters like the reference, 5 s timeout).

use std::fmt;
use std::time::Duration;

use crate::NotifyError;

pub const DEFAULT_FREE_MOBILE_URL: &str = "https://smsapi.free-mobile.fr/sendmsg";
/// Maximum message length (`msg[:160]` of the reference, in characters).
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
    /// `NotConfigured` if the identifier or the key is empty; `FreeMobile` if the HTTP client cannot be built.
    pub fn new(url: &str, user: &str, key: &str) -> Result<Self, NotifyError> {
        if user.is_empty() || key.is_empty() {
            return Err(NotifyError::NotConfigured("SMS Free Mobile"));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            // The key travels in the address: never redirect to another machine.
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

    /// Sends an SMS to **its owner** (the API can only write to the account's number). The HTTP status is checked (the reference ignored it).
    ///
    /// # Errors
    /// `FreeMobile` with the cause in clear, never the key.
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
