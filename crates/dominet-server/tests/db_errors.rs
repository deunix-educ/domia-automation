// SPDX-License-Identifier: AGPL-3.0-or-later
//! Checks on a real database that SQL errors are translated into useful `AppError`s.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use dominet_server::error::AppError;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn unique_violation_becomes_a_conflict(pool: PgPool) {
    sqlx::query("INSERT INTO users (username, password_hash) VALUES ('bob', 'h')")
        .execute(&pool)
        .await
        .unwrap();
    let err = sqlx::query("INSERT INTO users (username, password_hash) VALUES ('BOB', 'h')")
        .execute(&pool)
        .await
        .unwrap_err();
    let app: AppError = err.into();
    assert!(matches!(app, AppError::Conflict(_)), "{app:?}");
    assert_eq!(app.into_response().status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn foreign_key_violation_becomes_a_conflict(pool: PgPool) {
    let err =
        sqlx::query("INSERT INTO message (sender_id, subject, body) VALUES (424242, 's', 'b')")
            .execute(&pool)
            .await
            .unwrap_err();
    assert!(matches!(AppError::from(err), AppError::Conflict(_)));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn missing_row_becomes_not_found(pool: PgPool) {
    let err = sqlx::query("SELECT id FROM users WHERE id = 1")
        .fetch_one(&pool)
        .await
        .unwrap_err();
    assert!(matches!(AppError::from(err), AppError::NotFound));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn check_violation_is_an_internal_error_that_hides_the_sql(pool: PgPool) {
    // A CHECK constraint violated by application code is a bug: 500 without detail for the client.
    let err =
        sqlx::query("INSERT INTO users (username, password_hash, role) VALUES ('x', 'h', 'root')")
            .execute(&pool)
            .await
            .unwrap_err();
    let app = AppError::from(err);
    assert!(matches!(app, AppError::Internal(_)));
    assert_eq!(
        app.into_response().status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
