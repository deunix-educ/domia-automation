// SPDX-License-Identifier: AGPL-3.0-or-later
//! Domain errors.

/// A value refused by a business rule; the message is meant for the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub String);

impl ValidationError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Reason a form field is refused (lets the interface translate the message and put it under the
/// right field). The French text of [`ValidationError`] stays for the API and the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Required,
    TooLong {
        max: usize,
    },
    /// Format or value not allowed (the field carries a hint about the expected form).
    Invalid,
    OutOfRange {
        min: i64,
        max: i64,
    },
}

/// Error of a repository (storage port), independent of the underlying technology.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("introuvable")]
    NotFound,
    /// Uniqueness or reference violated; `constraint` names the constraint for a precise message on the UI side.
    #[error("conflit ({constraint})")]
    Conflict { constraint: String },
    /// The operation would leave the database without any active administrator.
    #[error("dernier administrateur actif")]
    LastAdmin,
    #[error("stockage indisponible")]
    Unavailable,
    #[error("erreur de stockage")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}
