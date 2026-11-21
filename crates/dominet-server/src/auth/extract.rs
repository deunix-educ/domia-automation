// SPDX-License-Identifier: AGPL-3.0-or-later
//! Extracteurs Axum : compte connecte et controle de role.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use dominet_domain::repo::UserRepo;
use dominet_domain::user::{User, UserId};
use dominet_store::USER_ID_KEY;
use tower_sessions::Session;

use super::session::{AUTH_AT_KEY, close};
use crate::AppState;
use crate::error::AppError;

/// Compte connecte. Refuse (401) si la session est absente, trop ancienne ou si le compte n'existe plus
/// ou est desactive.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let session = Session::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::Unauthorized)?;
        let user_id: Option<i64> = session.get(USER_ID_KEY).await.map_err(AppError::internal)?;
        let auth_at: Option<i64> = session.get(AUTH_AT_KEY).await.map_err(AppError::internal)?;
        let (Some(user_id), Some(auth_at)) = (user_id, auth_at) else {
            return Err(AppError::Unauthorized);
        };

        // Duree de vie absolue : meme une session active expire apres SESSION_MAX_DAYS.
        let max_age = i64::from(state.config.session_max_days) * 86_400;
        if chrono::Utc::now().timestamp() - auth_at > max_age {
            close(&session).await?;
            return Err(AppError::Unauthorized);
        }

        match state.store.users().find_by_id(UserId(user_id)).await? {
            Some(user) if user.is_active => Ok(Self(user)),
            _ => {
                close(&session).await?;
                Err(AppError::Unauthorized)
            }
        }
    }
}

/// Compte connecte **et administrateur** (403 sinon).
#[derive(Debug, Clone)]
pub struct RequireAdmin(pub User);

impl FromRequestParts<AppState> for RequireAdmin {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        if user.role.is_admin() {
            Ok(Self(user))
        } else {
            Err(AppError::Forbidden)
        }
    }
}
