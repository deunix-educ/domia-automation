// SPDX-License-Identifier: AGPL-3.0-or-later
//! Access tokens of the MCP server (R11, ADR 0059): generation, hashing, lookup and audit helpers, shared by the administration
//! page (`dominet-web`) and the `/mcp` routes (`dominet-mcp`).
//!
//! A token is 256 random bits, written `dnm_` + URL-safe base64. Only its SHA-256 is stored: the value is shown once, when it is
//! created, and cannot be read back. The value is high-entropy, so a fast hash is enough (a slow one would only cost time on
//! every call).

use aes_gcm::aead::Generate;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use dominet_domain::mcp::{McpToken, NewAuditEntry};
use dominet_domain::repo::{McpAuditRepo, McpTokenRepo, RepoResult};
use sha2::{Digest, Sha256};

use crate::AppState;

/// Start of every token: lets a secret scanner (and a human) recognise one.
pub const PREFIX: &str = "dnm_";
/// Characters of the random part kept visible in the list (48 bits out of 256).
const SHOWN_CHARS: usize = 8;

/// A freshly generated token. `value` is the only copy of the secret: show it once, then drop it.
pub struct Generated {
    pub value: String,
    pub hash: [u8; 32],
    /// Start of the value, safe to keep (`dnm_ab12cd34`).
    pub prefix: String,
}

impl std::fmt::Debug for Generated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Generated")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

/// SHA-256 of a presented token.
#[must_use]
pub fn hash(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

/// A new random token.
#[must_use]
pub fn generate() -> Generated {
    generate_with(PREFIX)
}

/// A new random secret of 256 bits that starts with `prefix` (also used by the camera viewing secrets, ADR 0064).
#[must_use]
pub fn generate_with(prefix: &str) -> Generated {
    let bytes = aes_gcm::Key::<aes_gcm::Aes256Gcm>::generate();
    let random = URL_SAFE_NO_PAD.encode(bytes.as_slice());
    let value = format!("{prefix}{random}");
    Generated {
        hash: hash(&value),
        prefix: format!("{prefix}{}", &random[..SHOWN_CHARS]),
        value,
    }
}

/// The token behind a presented value, if it is active (neither revoked nor expired); its use is noted.
///
/// # Errors
/// Storage failure.
pub async fn authenticate(state: &AppState, presented: &str) -> RepoResult<Option<McpToken>> {
    if !presented.starts_with(PREFIX) {
        return Ok(None);
    }
    let now = Utc::now();
    let tokens = state.store.mcp_tokens();
    let found = tokens.find_active(&hash(presented), now).await?;
    if let Some(token) = &found {
        tokens.touch(token.id, now).await?;
    }
    Ok(found)
}

/// Writes an audit line.
///
/// # Errors
/// Storage failure: the caller decides whether the operation may go on without a trace.
pub async fn record_audit(state: &AppState, entry: &NewAuditEntry) -> RepoResult<()> {
    state
        .store
        .mcp_audit()
        .record(entry, Utc::now())
        .await
        .map(|_| ())
}

/// Writes the audit line of an action **before** it runs (outcome `pending`) and returns its number: the caller must refuse the
/// action if this fails, so that nothing changes without a trace.
///
/// # Errors
/// Storage failure.
pub async fn begin_audit(state: &AppState, entry: &NewAuditEntry) -> RepoResult<i64> {
    state.store.mcp_audit().record(entry, Utc::now()).await
}

/// Closes a line written by [`begin_audit`] with what happened.
///
/// # Errors
/// Storage failure (the action has already run: the line stays `pending`).
pub async fn finish_audit(
    state: &AppState,
    id: i64,
    outcome: &str,
    detail: &str,
) -> RepoResult<()> {
    state.store.mcp_audit().complete(id, outcome, detail).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_random_prefixed_and_hashed_consistently() {
        let (a, b) = (generate(), generate());
        assert_ne!(a.value, b.value);
        assert!(a.value.starts_with(PREFIX));
        // 4 + 43 characters: 32 bytes in URL-safe base64 without padding.
        assert_eq!(a.value.len(), 47);
        assert!(a.value.starts_with(&a.prefix) && a.prefix.len() == 12);
        assert_eq!(a.hash, hash(&a.value));
        assert_ne!(a.hash, b.hash);
        assert!(
            a.value[PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn debug_never_shows_the_value() {
        let t = generate();
        let shown = format!("{t:?}");
        assert!(!shown.contains(&t.value));
        assert!(shown.contains(&t.prefix));
    }
}
