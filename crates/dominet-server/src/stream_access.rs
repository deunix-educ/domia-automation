// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera viewing secrets (ADR 0064): creation of the viewing token that the browser sends to MediaMTX.
//!
//! A viewing token is 256 random bits, written `dnv_` + URL-safe base64, stored as its SHA-256 only and bound to the account that
//! asked for it. It lives [`dominet_domain::stream_access::VIEWING_TTL_MINUTES`] minutes: long enough to post the WHEP offer, which
//! is the only moment MediaMTX checks it (the session it opens is not cut when the token expires).

use chrono::{DateTime, Duration, Utc};
use dominet_domain::repo::{RepoResult, StreamAccessRepo};
use dominet_domain::user::UserId;

use crate::AppState;
use crate::mcp_tokens::generate_with;

/// Start of every viewing token: lets a secret scanner (and a human) recognise one.
pub const VIEWING_PREFIX: &str = "dnv_";
/// Start of every personal stream credential (the password part).
pub const CREDENTIAL_PREFIX: &str = "dns_";
/// Expired viewing tokens are deleted once they are this old (no use keeping them).
const KEEP_EXPIRED: Duration = Duration::hours(1);

/// A viewing token to hand to the browser. `value` is the only copy of the secret.
pub struct IssuedViewing {
    pub value: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for IssuedViewing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedViewing")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Creates a viewing token for an account and tidies up the expired ones.
///
/// # Errors
/// Storage failure.
pub async fn issue_viewing(state: &AppState, user: UserId) -> RepoResult<IssuedViewing> {
    let now = Utc::now();
    let repo = state.store.stream_access();
    repo.purge_expired_viewing(now - KEEP_EXPIRED).await?;
    let secret = generate_with(VIEWING_PREFIX);
    let stored = repo
        .create_viewing(user, &secret.hash, &secret.prefix, now)
        .await?;
    Ok(IssuedViewing {
        value: secret.value,
        expires_at: stored.expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewing_token_is_prefixed_random_and_hashed_like_the_others() {
        let (a, b) = (generate_with(VIEWING_PREFIX), generate_with(VIEWING_PREFIX));
        assert!(a.value.starts_with(VIEWING_PREFIX) && a.value.len() == 47);
        assert_ne!(a.value, b.value);
        assert!(a.value.starts_with(&a.prefix) && a.prefix.len() == 12);
        assert_eq!(a.hash, crate::mcp_tokens::hash(&a.value));
    }

    #[test]
    fn credentials_and_viewing_tokens_are_told_apart_by_their_start() {
        assert_ne!(VIEWING_PREFIX, CREDENTIAL_PREFIX);
        assert_ne!(VIEWING_PREFIX, crate::mcp_tokens::PREFIX);
    }
}
