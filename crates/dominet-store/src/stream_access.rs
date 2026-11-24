// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera viewing secrets (`stream_access`), ADR 0064. Only the SHA-256 of a secret is stored.

use chrono::{DateTime, Utc};
use dominet_domain::repo::{RepoResult, StreamAccessRepo};
use dominet_domain::stream_access::{
    ActiveStreamAccess, NewStreamCredential, StreamAccess, StreamAccessId, StreamAccessKind,
    viewing_expires_at,
};
use dominet_domain::user::UserId;

use crate::pg::{corrupt, repo_err};

struct Row {
    id: i64,
    user_id: i64,
    kind: String,
    name: String,
    prefix: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

impl Row {
    fn into_access(self) -> RepoResult<StreamAccess> {
        Ok(StreamAccess {
            id: StreamAccessId(self.id),
            user: UserId(self.user_id),
            kind: StreamAccessKind::parse(&self.kind)
                .ok_or_else(|| corrupt(format!("stream access kind \"{}\"", self.kind)))?,
            name: self.name,
            prefix: self.prefix,
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PgStreamAccess {
    pool: sqlx::PgPool,
}

impl PgStreamAccess {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl StreamAccessRepo for PgStreamAccess {
    async fn create_viewing(
        &self,
        user: UserId,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
    ) -> RepoResult<StreamAccess> {
        sqlx::query_as!(
            Row,
            "INSERT INTO stream_access (user_id, kind, secret_hash, prefix, created_at, expires_at)
             VALUES ($1, 'viewing', $2, $3, $4, $5)
             RETURNING id, user_id, kind, name, prefix, created_at, expires_at, last_used_at, revoked_at",
            user.0,
            hash,
            prefix,
            now,
            viewing_expires_at(now),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?
        .into_access()
    }

    async fn create_credential(
        &self,
        user: UserId,
        new: &NewStreamCredential,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
    ) -> RepoResult<StreamAccess> {
        sqlx::query_as!(
            Row,
            "INSERT INTO stream_access (user_id, kind, name, secret_hash, prefix, created_at, expires_at)
             VALUES ($1, 'credential', $2, $3, $4, $5, $6)
             RETURNING id, user_id, kind, name, prefix, created_at, expires_at, last_used_at, revoked_at",
            user.0,
            new.name,
            hash,
            prefix,
            now,
            new.expires_at(now),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?
        .into_access()
    }

    async fn find_active(
        &self,
        kind: StreamAccessKind,
        hash: &[u8],
        now: DateTime<Utc>,
    ) -> RepoResult<Option<ActiveStreamAccess>> {
        let row = sqlx::query!(
            "SELECT s.id, s.user_id, s.kind, s.name, s.prefix, s.created_at, s.expires_at, s.last_used_at, s.revoked_at,
                    u.username
             FROM stream_access s JOIN users u ON u.id = s.user_id
             WHERE s.secret_hash = $1 AND s.kind = $2 AND s.revoked_at IS NULL AND s.expires_at > $3
               AND u.is_active",
            hash,
            kind.as_str(),
            now
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(|r| {
            Ok(ActiveStreamAccess {
                username: r.username,
                access: Row {
                    id: r.id,
                    user_id: r.user_id,
                    kind: r.kind,
                    name: r.name,
                    prefix: r.prefix,
                    created_at: r.created_at,
                    expires_at: r.expires_at,
                    last_used_at: r.last_used_at,
                    revoked_at: r.revoked_at,
                }
                .into_access()?,
            })
        })
        .transpose()
    }

    async fn touch(&self, id: StreamAccessId, now: DateTime<Utc>) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE stream_access SET last_used_at = $2
             WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2::timestamptz - interval '1 minute')",
            id.0,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn list_credentials(&self, user: UserId) -> RepoResult<Vec<StreamAccess>> {
        sqlx::query_as!(
            Row,
            "SELECT id, user_id, kind, name, prefix, created_at, expires_at, last_used_at, revoked_at
             FROM stream_access WHERE user_id = $1 AND kind = 'credential'
             ORDER BY created_at DESC, id DESC",
            user.0
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(Row::into_access)
        .collect()
    }

    async fn revoke(
        &self,
        id: StreamAccessId,
        user: UserId,
        now: DateTime<Utc>,
    ) -> RepoResult<bool> {
        let done = sqlx::query!(
            "UPDATE stream_access SET revoked_at = $3
             WHERE id = $1 AND user_id = $2 AND kind = 'credential' AND revoked_at IS NULL AND expires_at > $3",
            id.0,
            user.0,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected() > 0)
    }

    async fn purge_expired_viewing(&self, before: DateTime<Utc>) -> RepoResult<u64> {
        let done = sqlx::query!(
            "DELETE FROM stream_access WHERE kind = 'viewing' AND expires_at < $1",
            before
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected())
    }
}
