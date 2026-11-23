// SPDX-License-Identifier: AGPL-3.0-or-later
//! PostgreSQL session store for `tower-sessions` 0.15 (ADR 0004).
//!
//! The official store stayed on `sqlx` 0.8 / `tower-sessions` 0.14; this one uses `sqlx` 0.9. The `user_id`
//! column is filled from the `"user_id"` key of the session data: it allows revoking
//! all the sessions of an account at once.

use chrono::{DateTime, Utc};
use dominet_domain::user::UserId;
use sqlx::PgPool;
use time::OffsetDateTime;
use tower_sessions_core::session::{Id, Record};
use tower_sessions_core::session_store::{self, ExpiredDeletion, SessionStore};

/// Session key carrying the identifier of the logged-in account.
pub const USER_ID_KEY: &str = "user_id";

#[derive(Debug, Clone)]
pub struct PgSessionStore {
    pool: PgPool,
}

impl PgSessionStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Deletes all the sessions of an account (password change, deactivation, deletion).
    ///
    /// # Errors
    /// Returns the database error.
    pub async fn delete_for_user(&self, user: UserId) -> Result<u64, sqlx::Error> {
        let done = sqlx::query!("DELETE FROM session WHERE user_id = $1", user.0)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }

    /// Number of stored sessions (expired ones included): diagnostics and tests.
    ///
    /// # Errors
    /// Returns the database error.
    pub async fn count(&self) -> Result<i64, sqlx::Error> {
        Ok(sqlx::query_scalar!("SELECT count(*) FROM session")
            .fetch_one(&self.pool)
            .await?
            .unwrap_or(0))
    }
}

fn backend(err: &sqlx::Error) -> session_store::Error {
    session_store::Error::Backend(err.to_string())
}

fn to_chrono(t: OffsetDateTime) -> DateTime<Utc> {
    DateTime::from_timestamp(t.unix_timestamp(), t.nanosecond()).unwrap_or(DateTime::<Utc>::MAX_UTC)
}

fn to_time(t: DateTime<Utc>) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(t.timestamp()).map_or(OffsetDateTime::UNIX_EPOCH, |d| {
        d.replace_nanosecond(t.timestamp_subsec_nanos())
            .unwrap_or(d)
    })
}

fn user_id_of(record: &Record) -> Option<i64> {
    record
        .data
        .get(USER_ID_KEY)
        .and_then(serde_json::Value::as_i64)
}

fn data_json(record: &Record) -> Result<serde_json::Value, session_store::Error> {
    serde_json::to_value(&record.data).map_err(|e| session_store::Error::Encode(e.to_string()))
}

#[async_trait::async_trait]
impl SessionStore for PgSessionStore {
    async fn create(&self, record: &mut Record) -> session_store::Result<()> {
        // Identifier collision (almost impossible over 128 bits): we draw another one.
        loop {
            let done = sqlx::query!(
                "INSERT INTO session (id, data, expiry_date, user_id) VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING",
                record.id.to_string(),
                data_json(record)?,
                to_chrono(record.expiry_date),
                user_id_of(record),
            )
            .execute(&self.pool)
            .await
            .map_err(|e| backend(&e))?;
            if done.rows_affected() == 1 {
                return Ok(());
            }
            record.id = Id::default();
        }
    }

    async fn save(&self, record: &Record) -> session_store::Result<()> {
        sqlx::query!(
            "INSERT INTO session (id, data, expiry_date, user_id) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET data = EXCLUDED.data, expiry_date = EXCLUDED.expiry_date, user_id = EXCLUDED.user_id",
            record.id.to_string(),
            data_json(record)?,
            to_chrono(record.expiry_date),
            user_id_of(record),
        )
        .execute(&self.pool)
        .await
        .map_err(|e| backend(&e))?;
        Ok(())
    }

    async fn load(&self, id: &Id) -> session_store::Result<Option<Record>> {
        let row = sqlx::query!(
            "SELECT data, expiry_date FROM session WHERE id = $1 AND expiry_date > now()",
            id.to_string()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| backend(&e))?;
        row.map(|r| {
            let data = serde_json::from_value(r.data)
                .map_err(|e| session_store::Error::Decode(e.to_string()))?;
            Ok(Record {
                id: *id,
                data,
                expiry_date: to_time(r.expiry_date),
            })
        })
        .transpose()
    }

    async fn delete(&self, id: &Id) -> session_store::Result<()> {
        sqlx::query!("DELETE FROM session WHERE id = $1", id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| backend(&e))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ExpiredDeletion for PgSessionStore {
    async fn delete_expired(&self) -> session_store::Result<()> {
        sqlx::query!("DELETE FROM session WHERE expiry_date <= now()")
            .execute(&self.pool)
            .await
            .map_err(|e| backend(&e))?;
        Ok(())
    }
}
