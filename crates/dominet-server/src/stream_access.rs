// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera viewing secrets (ADR 0064): creation of the viewing token that the browser sends to MediaMTX.
//!
//! A viewing token is 256 random bits, written `dnv_` + URL-safe base64, stored as its SHA-256 only and bound to the account that
//! asked for it. It lives [`dominet_domain::stream_access::VIEWING_TTL_MINUTES`] minutes: long enough to post the WHEP offer, which
//! is the only moment MediaMTX checks it (the session it opens is not cut when the token expires).

use chrono::{DateTime, Duration, Utc};
use dominet_domain::repo::{RepoResult, StreamAccessRepo};
use dominet_domain::stream_access::{NewStreamCredential, StreamAccess, StreamAccessId};
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

/// A personal stream credential just created. `password` is the only copy of the secret: show it once, then drop it.
pub struct IssuedCredential {
    pub access: StreamAccess,
    pub password: String,
}

impl std::fmt::Debug for IssuedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedCredential")
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

/// Creates a personal credential for an account: the user name is the account's, the password is generated.
///
/// # Errors
/// Storage failure.
pub async fn issue_credential(
    state: &AppState,
    user: UserId,
    new: &NewStreamCredential,
) -> RepoResult<IssuedCredential> {
    let secret = generate_with(CREDENTIAL_PREFIX);
    let access = state
        .store
        .stream_access()
        .create_credential(user, new, &secret.hash, &secret.prefix, Utc::now())
        .await?;
    Ok(IssuedCredential {
        access,
        password: secret.value,
    })
}

/// What revoking a credential did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revocation {
    /// The credential was active and is now revoked.
    pub revoked: bool,
    /// Open sessions closed at MediaMTX; `None` if they could not be (MediaMTX unreachable): the credential is refused from the next
    /// connection anyway, and its end date bounds a session that stays open.
    pub closed: Option<u32>,
}

/// Revokes a credential of an account and closes the sessions that account has open with a user name: MediaMTX records the account's
/// user name, not which credential was used, so a second device of the same account reconnects with its own credential.
///
/// # Errors
/// Storage failure.
pub async fn revoke_credential(
    state: &AppState,
    user: UserId,
    username: &str,
    id: StreamAccessId,
) -> RepoResult<Revocation> {
    let revoked = state
        .store
        .stream_access()
        .revoke(id, user, Utc::now())
        .await?;
    if !revoked {
        return Ok(Revocation {
            revoked: false,
            closed: Some(0),
        });
    }
    let closed = match state.cameras.kick_viewer(username).await {
        Ok(n) => Some(n),
        Err(e) => {
            tracing::warn!(error = %e, "revoked credential: open sessions could not be closed");
            None
        }
    };
    Ok(Revocation { revoked, closed })
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
