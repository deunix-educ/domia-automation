// SPDX-License-Identifier: AGPL-3.0-or-later
//! Erreurs du domaine.

/// Une valeur refusee par une regle metier ; le message est destine a l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub String);

impl ValidationError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Motif de refus d'un champ de formulaire (permet a l'interface de traduire le message et de le placer sous le
/// bon champ). Le texte francais de [`ValidationError`] reste pour l'API et la CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Required,
    TooLong {
        max: usize,
    },
    /// Format ou valeur non admis (le champ porte une aide sur la forme attendue).
    Invalid,
    OutOfRange {
        min: i64,
        max: i64,
    },
}

/// Erreur d'un depot (port de stockage), independante de la technologie sous-jacente.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("introuvable")]
    NotFound,
    /// Unicite ou reference violee ; `constraint` nomme la contrainte pour un message precis cote UI.
    #[error("conflit ({constraint})")]
    Conflict { constraint: String },
    /// L'operation laisserait la base sans aucun administrateur actif.
    #[error("dernier administrateur actif")]
    LastAdmin,
    #[error("stockage indisponible")]
    Unavailable,
    #[error("erreur de stockage")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}
