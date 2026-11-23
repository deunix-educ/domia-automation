// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository of accounts (`users`).

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::repo::{RepoResult, UserRepo};
use dominet_domain::user::{Credentials, NewUser, Role, User, UserId, UserProfile};

use crate::pg::{corrupt, repo_err};

struct UserRow {
    id: i64,
    username: String,
    email: String,
    first_name: String,
    last_name: String,
    mobile: String,
    role: String,
    is_active: bool,
    last_login_at: Option<DateTime<Utc>>,
    password_changed_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl UserRow {
    fn into_user(self) -> RepoResult<User> {
        let role = self
            .role
            .parse::<Role>()
            .map_err(|_| corrupt(format!("role \"{}\"", self.role)))?;
        Ok(User {
            id: UserId(self.id),
            username: self.username,
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            mobile: self.mobile,
            role,
            is_active: self.is_active,
            last_login_at: self.last_login_at,
            password_changed_at: self.password_changed_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// Same as `UserRow` with the password hash (login only).
struct CredentialsRow {
    id: i64,
    username: String,
    email: String,
    first_name: String,
    last_name: String,
    mobile: String,
    role: String,
    is_active: bool,
    last_login_at: Option<DateTime<Utc>>,
    password_changed_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    password_hash: String,
}

impl CredentialsRow {
    fn into_credentials(self) -> RepoResult<Credentials> {
        let password_hash = self.password_hash;
        let user = UserRow {
            id: self.id,
            username: self.username,
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            mobile: self.mobile,
            role: self.role,
            is_active: self.is_active,
            last_login_at: self.last_login_at,
            password_changed_at: self.password_changed_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
        .into_user()?;
        Ok(Credentials {
            user,
            password_hash,
        })
    }
}

/// Acces PostgreSQL aux users (implemente `UserRepo`).
#[derive(Debug, Clone)]
pub struct PgUsers {
    pool: sqlx::PgPool,
}

impl PgUsers {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

impl UserRepo for PgUsers {
    async fn create(&self, new: &NewUser) -> RepoResult<User> {
        sqlx::query_as!(
            UserRow,
            "INSERT INTO users (username, email, first_name, last_name, mobile, password_hash, role, is_active)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             RETURNING id, username, email, first_name, last_name, mobile, role, is_active,
                       last_login_at, password_changed_at, created_at, updated_at",
            new.username.as_str(),
            new.email,
            new.first_name,
            new.last_name,
            new.mobile,
            new.password_hash,
            new.role.as_str(),
            new.is_active,
        )
        .fetch_one(self.pool())
        .await
        .map_err(repo_err)?
        .into_user()
    }

    async fn find_by_id(&self, id: UserId) -> RepoResult<Option<User>> {
        sqlx::query_as!(
            UserRow,
            "SELECT id, username, email, first_name, last_name, mobile, role, is_active,
                    last_login_at, password_changed_at, created_at, updated_at
             FROM users WHERE id = $1",
            id.0
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?
        .map(UserRow::into_user)
        .transpose()
    }

    async fn find_by_username(&self, username: &str) -> RepoResult<Option<User>> {
        sqlx::query_as!(
            UserRow,
            "SELECT id, username, email, first_name, last_name, mobile, role, is_active,
                    last_login_at, password_changed_at, created_at, updated_at
             FROM users WHERE lower(username) = lower($1)",
            username
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?
        .map(UserRow::into_user)
        .transpose()
    }

    async fn find_credentials(&self, username: &str) -> RepoResult<Option<Credentials>> {
        sqlx::query_as!(
            CredentialsRow,
            "SELECT id, username, email, first_name, last_name, mobile, role, is_active,
                    last_login_at, password_changed_at, created_at, updated_at, password_hash
             FROM users WHERE lower(username) = lower($1)",
            username
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?
        .map(CredentialsRow::into_credentials)
        .transpose()
    }

    async fn list(&self) -> RepoResult<Vec<User>> {
        sqlx::query_as!(
            UserRow,
            "SELECT id, username, email, first_name, last_name, mobile, role, is_active,
                    last_login_at, password_changed_at, created_at, updated_at
             FROM users ORDER BY lower(username)"
        )
        .fetch_all(self.pool())
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(UserRow::into_user)
        .collect()
    }

    async fn update_profile(&self, id: UserId, profile: &UserProfile) -> RepoResult<User> {
        let mut tx = self.pool().begin().await.map_err(repo_err)?;
        let leaves_admins = lock_admin_set(&mut tx, id).await?;
        // The account was an active administrator: it must not stop being one if it is the last.
        let stays_active_admin = profile.role == Role::Admin && profile.is_active;
        if leaves_admins == AdminState::ActiveAdmin
            && !stays_active_admin
            && other_active_admins(&mut tx, id).await? == 0
        {
            return Err(RepoError::LastAdmin);
        }
        let row = sqlx::query_as!(
            UserRow,
            "UPDATE users SET email = $2, first_name = $3, last_name = $4, mobile = $5, role = $6, is_active = $7
             WHERE id = $1
             RETURNING id, username, email, first_name, last_name, mobile, role, is_active,
                       last_login_at, password_changed_at, created_at, updated_at",
            id.0,
            profile.email,
            profile.first_name,
            profile.last_name,
            profile.mobile,
            profile.role.as_str(),
            profile.is_active,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        row.into_user()
    }

    async fn set_password_hash(&self, id: UserId, hash: &str) -> RepoResult<()> {
        let done = sqlx::query!(
            "UPDATE users SET password_hash = $2, password_changed_at = now() WHERE id = $1",
            id.0,
            hash
        )
        .execute(self.pool())
        .await
        .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn touch_last_login(&self, id: UserId) -> RepoResult<()> {
        let done = sqlx::query!("UPDATE users SET last_login_at = now() WHERE id = $1", id.0)
            .execute(self.pool())
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn delete(&self, id: UserId) -> RepoResult<()> {
        let mut tx = self.pool().begin().await.map_err(repo_err)?;
        let state = lock_admin_set(&mut tx, id).await?;
        if state == AdminState::ActiveAdmin && other_active_admins(&mut tx, id).await? == 0 {
            return Err(RepoError::LastAdmin);
        }
        sqlx::query!("DELETE FROM users WHERE id = $1", id.0)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn count_active_admins(&self) -> RepoResult<i64> {
        let n =
            sqlx::query_scalar!("SELECT count(*) FROM users WHERE role = 'admin' AND is_active")
                .fetch_one(self.pool())
                .await
                .map_err(repo_err)?;
        Ok(n.unwrap_or(0))
    }
}

/// Key of the advisory lock that serialises every change to the set of administrators.
const ADMIN_SET_LOCK: i64 = 0x0044_4F4D_4941_444D;

#[derive(Debug, PartialEq, Eq)]
enum AdminState {
    ActiveAdmin,
    Other,
}

/// Takes the administrators lock (until the end of the transaction) then reads the state of the targeted account.
///
/// Without this lock, two administrators demoting each other at the same time would each see "there is another one left"
/// and the database would end up without an active administrator: checking then writing is not atomic.
async fn lock_admin_set(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: UserId,
) -> RepoResult<AdminState> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ADMIN_SET_LOCK)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    let row = sqlx::query!("SELECT role, is_active FROM users WHERE id = $1", id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(repo_err)?
        .ok_or(RepoError::NotFound)?;
    Ok(if row.role == "admin" && row.is_active {
        AdminState::ActiveAdmin
    } else {
        AdminState::Other
    })
}

async fn other_active_admins(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: UserId,
) -> RepoResult<i64> {
    let n = sqlx::query_scalar!(
        "SELECT count(*) FROM users WHERE role = 'admin' AND is_active AND id <> $1",
        id.0
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(n.unwrap_or(0))
}
