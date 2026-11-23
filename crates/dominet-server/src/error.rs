// SPDX-License-Identifier: AGPL-3.0-or-later
//! Application error and its translation into an HTTP response.
//!
//! The body is always `{"error":{"code":"...","message":"..."}}`. Internal errors are logged
//! with their cause but the client only receives a generic message; the request identifier travels in
//! the `x-request-id` header.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use dominet_domain::{RepoError, ValidationError};
use serde_json::json;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("ressource introuvable")]
    NotFound,
    #[error("authentication required")]
    Unauthorized,
    #[error("identifiants invalides")]
    InvalidCredentials,
    #[error("access denied")]
    Forbidden,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("trop de tentatives, reessayer dans {retry_after_s} s")]
    TooManyRequests { retry_after_s: u64 },
    #[error("service indisponible : {0}")]
    Unavailable(String),
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}

impl AppError {
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unauthorized | Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable code, usable by the front end and the tests.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Unauthorized => "unauthorized",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Forbidden => "forbidden",
            Self::BadRequest(_) => "bad_request",
            Self::Conflict(_) => "conflict",
            Self::TooManyRequests { .. } => "too_many_requests",
            Self::Unavailable(_) => "unavailable",
            Self::Internal(_) => "internal",
        }
    }

    pub fn internal(err: impl Into<anyhow::Error>) -> Self {
        Self::Internal(err.into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => Self::NotFound,
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Conflict("this value already exists".into())
            }
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
                Self::Conflict("this resource is still used elsewhere".into())
            }
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
                Self::Unavailable("database unreachable".into())
            }
            _ => Self::Internal(err.into()),
        }
    }
}

impl From<crate::auth::service::AuthError> for AppError {
    fn from(err: crate::auth::service::AuthError) -> Self {
        use crate::auth::service::AuthError;
        match err {
            AuthError::InvalidCredentials => Self::InvalidCredentials,
            AuthError::Locked { retry_after_s } => Self::TooManyRequests { retry_after_s },
            AuthError::Invalid(v) => v.into(),
            AuthError::Repo(r) => r.into(),
            AuthError::Internal(e) => Self::Internal(e),
        }
    }
}

impl From<crate::settings::SettingsError> for AppError {
    fn from(err: crate::settings::SettingsError) -> Self {
        use crate::settings::SettingsError;
        match err {
            SettingsError::UnknownKey(_) => Self::NotFound,
            SettingsError::Invalid(v) => v.into(),
            e @ (SettingsError::MasterKeyMissing | SettingsError::Unreadable(_)) => {
                Self::Unavailable(e.to_string())
            }
            SettingsError::Repo(r) => r.into(),
        }
    }
}

impl From<ValidationError> for AppError {
    fn from(err: ValidationError) -> Self {
        Self::BadRequest(err.0)
    }
}

impl From<RepoError> for AppError {
    fn from(err: RepoError) -> Self {
        match err {
            RepoError::NotFound => Self::NotFound,
            RepoError::Conflict { constraint } => {
                Self::Conflict(conflict_message(&constraint).into())
            }
            RepoError::LastAdmin => {
                Self::Conflict("at least one active administrator must remain".into())
            }
            RepoError::Unavailable => Self::Unavailable("database unreachable".into()),
            RepoError::Backend(cause) => Self::Internal(anyhow::anyhow!(cause)),
        }
    }
}

/// Useful message for the constraints the user can trigger themselves.
#[must_use]
pub fn conflict_message(constraint: &str) -> &'static str {
    match constraint {
        "users_username_lower_key" => "this user name already exists",
        "app_setting_key_key" => "this setting already exists",
        _ => "this value already exists or this resource is still in use",
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match &self {
            Self::Internal(cause) => tracing::error!(error = ?cause, "internal error"),
            Self::Unavailable(why) => tracing::warn!(%why, "service unavailable"),
            _ => tracing::debug!(error = %self, "request refused"),
        }
        // Public message: never the cause of an internal error.
        let message = match &self {
            Self::Internal(_) => "internal error".to_string(),
            other => other.to_string(),
        };
        let mut response = (
            self.status(),
            Json(json!({ "error": { "code": self.code(), "message": message } })),
        )
            .into_response();
        if let Self::TooManyRequests { retry_after_s } = self
            && let Ok(v) = HeaderValue::from_str(&retry_after_s.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, v);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn body_json(r: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(r.into_body(), 1 << 16).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn status_and_code_mapping() {
        let cases = [
            (AppError::NotFound, StatusCode::NOT_FOUND, "not_found"),
            (
                AppError::Unauthorized,
                StatusCode::UNAUTHORIZED,
                "unauthorized",
            ),
            (AppError::Forbidden, StatusCode::FORBIDDEN, "forbidden"),
            (
                AppError::BadRequest("x".into()),
                StatusCode::BAD_REQUEST,
                "bad_request",
            ),
            (
                AppError::Conflict("x".into()),
                StatusCode::CONFLICT,
                "conflict",
            ),
            (
                AppError::Unavailable("x".into()),
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
            ),
        ];
        for (err, status, code) in cases {
            let r = err.into_response();
            assert_eq!(r.status(), status);
            assert_eq!(body_json(r).await["error"]["code"], code);
        }
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_their_cause() {
        let r = AppError::internal(anyhow::anyhow!("mot de passe=hunter2 dans la requete"))
            .into_response();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(r).await;
        assert_eq!(body["error"]["message"], "internal error");
        assert!(!body.to_string().contains("hunter2"));
    }

    #[tokio::test]
    async fn too_many_requests_sets_retry_after() {
        let r = AppError::TooManyRequests { retry_after_s: 42 }.into_response();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(r.headers()[header::RETRY_AFTER], "42");
    }

    #[test]
    fn repo_errors_are_translated() {
        assert!(matches!(
            AppError::from(RepoError::NotFound),
            AppError::NotFound
        ));
        assert!(matches!(
            AppError::from(RepoError::Unavailable),
            AppError::Unavailable(_)
        ));
        let backend = RepoError::Backend("detail technique".into());
        assert!(matches!(AppError::from(backend), AppError::Internal(_)));
        let AppError::Conflict(msg) = AppError::from(RepoError::Conflict {
            constraint: "users_username_lower_key".into(),
        }) else {
            panic!("conflit attendu")
        };
        assert_eq!(msg, "this user name already exists");
        let AppError::Conflict(generic) = AppError::from(RepoError::Conflict {
            constraint: "autre".into(),
        }) else {
            panic!("conflit attendu")
        };
        assert!(generic.contains("already exists"));
    }

    #[test]
    fn validation_errors_become_bad_requests_with_their_message() {
        let e = dominet_domain::user::Username::parse("a b").unwrap_err();
        let msg = e.0.clone();
        let AppError::BadRequest(got) = AppError::from(e) else {
            panic!("400 attendu")
        };
        assert_eq!(got, msg);
    }

    #[test]
    fn sqlx_errors_are_classified() {
        assert!(matches!(
            AppError::from(sqlx::Error::RowNotFound),
            AppError::NotFound
        ));
        assert!(matches!(
            AppError::from(sqlx::Error::PoolTimedOut),
            AppError::Unavailable(_)
        ));
        assert!(matches!(
            AppError::from(sqlx::Error::PoolClosed),
            AppError::Unavailable(_)
        ));
        assert!(matches!(
            AppError::from(sqlx::Error::WorkerCrashed),
            AppError::Internal(_)
        ));
    }
}
