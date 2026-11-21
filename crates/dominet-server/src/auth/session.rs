// SPDX-License-Identifier: AGPL-3.0-or-later
//! Configuration des cookies de session et ouverture/fermeture de session.

use dominet_domain::user::User;
use dominet_store::{PgSessionStore, USER_ID_KEY};
use tower_sessions::cookie::SameSite;
use tower_sessions::{Expiry, Session, SessionManagerLayer};

use crate::config::Config;
use crate::error::{AppError, AppResult};

pub const COOKIE_NAME: &str = "dominet_session";
/// Instant de la connexion (secondes Unix) : sert a faire respecter la duree de vie maximale.
pub const AUTH_AT_KEY: &str = "auth_at";

/// Couche de session : cookie `HttpOnly`, `SameSite=Lax`, `Secure` selon `COOKIE_SECURE`, expiration par inactivite.
#[must_use]
pub fn layer(store: PgSessionStore, config: &Config) -> SessionManagerLayer<PgSessionStore> {
    SessionManagerLayer::new(store)
        .with_name(COOKIE_NAME)
        .with_http_only(true)
        .with_secure(config.cookie_secure)
        .with_same_site(SameSite::Lax)
        .with_path("/")
        .with_expiry(Expiry::OnInactivity(time::Duration::minutes(i64::from(
            config.session_idle_minutes,
        ))))
}

/// Ouvre une session pour `user` : l'ancienne session (anonyme ou autre compte) est detruite et un nouvel
/// identifiant est genere (anti fixation de session).
///
/// # Errors
/// Erreur interne si le store de sessions echoue.
pub async fn open(session: &Session, user: &User) -> AppResult<()> {
    // `flush` detruit l'ancienne session mais garde son identifiant en memoire : sans `cycle_id`, il serait
    // reutilise a l'enregistrement et la fixation de session resterait possible.
    session.flush().await.map_err(AppError::internal)?;
    session.cycle_id().await.map_err(AppError::internal)?;
    session
        .insert(USER_ID_KEY, user.id.0)
        .await
        .map_err(AppError::internal)?;
    session
        .insert(AUTH_AT_KEY, chrono::Utc::now().timestamp())
        .await
        .map_err(AppError::internal)?;
    Ok(())
}

/// Ferme la session courante (donnees et enregistrement supprimes).
///
/// # Errors
/// Erreur interne si le store de sessions echoue.
pub async fn close(session: &Session) -> AppResult<()> {
    session.flush().await.map_err(AppError::internal)
}
