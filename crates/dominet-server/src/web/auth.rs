// SPDX-License-Identifier: AGPL-3.0-or-later
//! Routes d'authentification et de gestion des comptes (`/api/auth/*`, `/api/users`).

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use dominet_domain::repo::UserRepo;
use dominet_domain::user::{Role, User};
use dominet_proto::{ChangePasswordRequest, LoginRequest, RoleDto, UserDto};
use tower_sessions::Session;

use crate::AppState;
use crate::auth::client_ip::ClientIp;
use crate::auth::extract::{CurrentUser, RequireAdmin};
use crate::auth::session;
use crate::error::AppResult;

#[must_use]
pub fn user_dto(user: &User) -> UserDto {
    UserDto {
        id: user.id.0,
        username: user.username.clone(),
        display_name: user.display_name(),
        email: user.email.clone(),
        mobile: user.mobile.clone(),
        role: match user.role {
            Role::Admin => RoleDto::Admin,
            Role::User => RoleDto::User,
        },
        is_active: user.is_active,
        last_login_at: user.last_login_at.map(|t| t.to_rfc3339()),
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/auth/password", post(change_password))
        .route("/users", get(list_users))
}

async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    session: Session,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<UserDto>> {
    let user = state
        .auth
        .authenticate(&req.username, &req.password, &ip)
        .await?;
    session::open(&session, &user).await?;
    tracing::info!(user_id = user.id.0, %ip, "login succeeded");
    Ok(Json(user_dto(&user)))
}

async fn logout(session: Session) -> AppResult<StatusCode> {
    session::close(&session).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(CurrentUser(user): CurrentUser) -> Json<UserDto> {
    Json(user_dto(&user))
}

/// Change son propre mot de passe (meme blocage anti force brute que la connexion). Toutes les sessions du compte sont revoquees, y compris celle-ci :
/// il faut se reconnecter.
async fn change_password(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    CurrentUser(user): CurrentUser,
    session: Session,
    Json(req): Json<ChangePasswordRequest>,
) -> AppResult<StatusCode> {
    state
        .auth
        .change_own_password(&user, &req.current_password, &req.new_password, &ip)
        .await?;
    session::close(&session).await?;
    tracing::info!(user_id = user.id.0, "password changed, sessions revoked");
    Ok(StatusCode::NO_CONTENT)
}

async fn list_users(
    State(state): State<AppState>,
    RequireAdmin(_): RequireAdmin,
) -> AppResult<Json<Vec<UserDto>>> {
    let users = state.store.users().list().await?;
    Ok(Json(users.iter().map(user_dto).collect()))
}
