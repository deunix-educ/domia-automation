// SPDX-License-Identifier: AGPL-3.0-or-later
//! Session cookie configuration and session opening/closing.

use dominet_domain::user::User;
use dominet_store::{PgSessionStore, USER_ID_KEY};
use tower_sessions::cookie::SameSite;
use tower_sessions::{Expiry, Session, SessionManagerLayer};

use crate::config::Config;
use crate::error::{AppError, AppResult};

pub const COOKIE_NAME: &str = "dominet_session";
/// Login instant (Unix seconds): used to enforce the maximum lifetime.
pub const AUTH_AT_KEY: &str = "auth_at";

/// Session layer: `HttpOnly` cookie, `SameSite=Lax`, `Secure` according to `COOKIE_SECURE`, expiry by inactivity.
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

/// Opens a session for `user`: the old session (anonymous or another account) is destroyed and a new
/// identifier is generated (anti session fixation).
///
/// # Errors
/// Internal error if the session store fails.
pub async fn open(session: &Session, user: &User) -> AppResult<()> {
    // `flush` destroys the old session but keeps its identifier in memory: without `cycle_id`, it would be
    // reused when saving and session fixation would remain possible.
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

/// Closes the current session (data and record deleted).
///
/// # Errors
/// Internal error if the session store fails.
pub async fn close(session: &Session) -> AppResult<()> {
    session.flush().await.map_err(AppError::internal)
}
