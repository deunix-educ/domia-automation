// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera viewing access (ADR 0064): short-lived viewing tokens for the browser and personal stream credentials for external
//! players (VLC, phone applications).
//!
//! Pure rules only. A secret is never kept here: the domain knows the public part (name, prefix, dates). How a secret is
//! generated and hashed lives in `dominet-server`, where it is stored hashed by `dominet-store`.

use chrono::{DateTime, Duration, Utc};

use crate::automation::Problems;
use crate::error::Problem;
use crate::user::UserId;

/// Lifetime of a viewing token, in minutes: long enough to open the live page and renew in the background, short enough that a
/// leaked one is of little use.
pub const VIEWING_TTL_MINUTES: i64 = 5;
/// Longest lifetime of a personal stream credential, in days.
pub const MAX_LIFETIME_DAYS: u32 = 365;
/// Lifetime proposed when creating a credential.
pub const DEFAULT_LIFETIME_DAYS: u32 = 90;
/// Longest credential name (the device it is for: "Phone of Anna").
pub const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamAccessId(pub i64);

/// What a secret is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamAccessKind {
    /// Sent by the browser as `Authorization: Bearer` (arrives in MediaMTX's `token`).
    Viewing,
    /// User name and password typed in an external player (arrives in `user` and `password`).
    Credential,
}

impl StreamAccessKind {
    /// Name stored in the database.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            StreamAccessKind::Viewing => "viewing",
            StreamAccessKind::Credential => "credential",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<StreamAccessKind> {
        [StreamAccessKind::Viewing, StreamAccessKind::Credential]
            .into_iter()
            .find(|k| k.as_str() == name)
    }
}

/// A personal credential to create, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStreamCredential {
    pub name: String,
    pub lifetime_days: u32,
}

impl NewStreamCredential {
    /// # Errors
    /// Field by field: `name` (required, 64 characters at most), `lifetime_days` (1 to 365: a credential without an end date
    /// does not exist).
    pub fn new(name: &str, lifetime_days: u32) -> Result<Self, Problems> {
        let mut problems: Problems = vec![];
        let name = name.trim();
        if name.is_empty() {
            problems.push(("name", Problem::Required));
        } else if name.chars().count() > MAX_NAME_CHARS {
            problems.push((
                "name",
                Problem::TooLong {
                    max: MAX_NAME_CHARS,
                },
            ));
        }
        if !(1..=MAX_LIFETIME_DAYS).contains(&lifetime_days) {
            problems.push((
                "lifetime_days",
                Problem::OutOfRange {
                    min: 1,
                    max: i64::from(MAX_LIFETIME_DAYS),
                },
            ));
        }
        if problems.is_empty() {
            Ok(Self {
                name: name.to_string(),
                lifetime_days,
            })
        } else {
            Err(problems)
        }
    }

    /// End of validity for a credential created at `now`.
    #[must_use]
    pub fn expires_at(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now + Duration::days(i64::from(self.lifetime_days))
    }
}

/// End of validity of a viewing token created at `now`.
#[must_use]
pub fn viewing_expires_at(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::minutes(VIEWING_TTL_MINUTES)
}

/// A stored secret, as the account page shows it. The secret itself is not recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAccess {
    pub id: StreamAccessId,
    pub user: UserId,
    pub kind: StreamAccessKind,
    /// The device a credential is for; empty for a viewing token.
    pub name: String,
    /// First characters of the secret, to recognise it without being able to use it.
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessState {
    Active,
    Expired,
    Revoked,
}

impl StreamAccess {
    /// A revoked secret stays revoked even once its date has passed.
    #[must_use]
    pub fn state(&self, now: DateTime<Utc>) -> AccessState {
        if self.revoked_at.is_some() {
            AccessState::Revoked
        } else if self.expires_at <= now {
            AccessState::Expired
        } else {
            AccessState::Active
        }
    }
}

/// An active secret together with the account it belongs to (an active account: a disabled one never matches).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveStreamAccess {
    pub access: StreamAccess,
    pub username: String,
}

impl ActiveStreamAccess {
    /// Whether the user name presented with a credential is the account's (case-insensitive, like the login). A viewing token
    /// carries no user name: the caller does not check one.
    #[must_use]
    pub fn is_user(&self, presented: &str) -> bool {
        self.username.to_lowercase() == presented.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, min: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-11-24T{hour:02}:{min:02}:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    fn access(kind: StreamAccessKind) -> StreamAccess {
        StreamAccess {
            id: StreamAccessId(1),
            user: UserId(7),
            kind,
            name: String::new(),
            prefix: "dnv_ab12cd34".into(),
            created_at: at(10, 0),
            expires_at: at(10, 5),
            last_used_at: None,
            revoked_at: None,
        }
    }

    #[test]
    fn kinds_round_trip() {
        for k in [StreamAccessKind::Viewing, StreamAccessKind::Credential] {
            assert_eq!(StreamAccessKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(StreamAccessKind::parse("autre"), None);
    }

    #[test]
    fn a_viewing_token_lives_a_few_minutes() {
        assert_eq!(viewing_expires_at(at(10, 0)), at(10, 5));
    }

    #[test]
    fn a_new_credential_is_validated_field_by_field() {
        let ok = NewStreamCredential::new("  Phone of Anna  ", 90).unwrap();
        assert_eq!(ok.name, "Phone of Anna");
        assert_eq!(ok.expires_at(at(10, 0)), at(10, 0) + Duration::days(90));
        assert_eq!(
            NewStreamCredential::new("", 0).unwrap_err(),
            [
                ("name", Problem::Required),
                ("lifetime_days", Problem::OutOfRange { min: 1, max: 365 }),
            ]
        );
        assert_eq!(
            NewStreamCredential::new("x", 366).unwrap_err(),
            [("lifetime_days", Problem::OutOfRange { min: 1, max: 365 })]
        );
        assert_eq!(
            NewStreamCredential::new(&"n".repeat(65), 30).unwrap_err(),
            [("name", Problem::TooLong { max: 64 })]
        );
        assert!(NewStreamCredential::new(&"n".repeat(64), 365).is_ok());
    }

    #[test]
    fn the_state_depends_on_revocation_first_then_the_date() {
        let mut a = access(StreamAccessKind::Viewing);
        assert_eq!(a.state(at(10, 4)), AccessState::Active);
        // The end date itself is already expired.
        assert_eq!(a.state(at(10, 5)), AccessState::Expired);
        a.revoked_at = Some(at(10, 1));
        assert_eq!(a.state(at(10, 2)), AccessState::Revoked);
        assert_eq!(a.state(at(11, 0)), AccessState::Revoked);
    }

    #[test]
    fn the_user_name_of_a_credential_is_compared_without_case() {
        let a = ActiveStreamAccess {
            access: access(StreamAccessKind::Credential),
            username: "Alice".into(),
        };
        assert!(a.is_user("alice") && a.is_user("ALICE"));
        assert!(!a.is_user("bob") && !a.is_user(""));
    }
}
