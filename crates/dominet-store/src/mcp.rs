// SPDX-License-Identifier: AGPL-3.0-or-later
//! MCP access tokens (`mcp_token`) and audit trail (`mcp_audit`), ADR 0059. Only the SHA-256 of a token is stored.

use chrono::{DateTime, Utc};
use dominet_domain::mcp::{
    AuditKind, McpAuditEntry, McpToken, McpTokenId, NewAuditEntry, NewMcpToken, Scope, Scopes,
};
use dominet_domain::repo::{McpAuditRepo, McpTokenRepo, RepoResult};
use dominet_domain::user::UserId;

use crate::pg::{corrupt, repo_err};

struct TokenRow {
    id: i64,
    name: String,
    prefix: String,
    scopes: Vec<String>,
    created_at: DateTime<Utc>,
    created_by: Option<i64>,
    expires_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

impl TokenRow {
    fn into_token(self) -> RepoResult<McpToken> {
        let mut scopes = Scopes::new();
        for s in &self.scopes {
            scopes.insert(Scope::parse(s).ok_or_else(|| corrupt(format!("scope \"{s}\"")))?);
        }
        Ok(McpToken {
            id: McpTokenId(self.id),
            name: self.name,
            prefix: self.prefix,
            scopes,
            created_at: self.created_at,
            created_by: self.created_by.map(UserId),
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PgMcpTokens {
    pool: sqlx::PgPool,
}

impl PgMcpTokens {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl McpTokenRepo for PgMcpTokens {
    async fn create(
        &self,
        new: &NewMcpToken,
        hash: &[u8],
        prefix: &str,
        now: DateTime<Utc>,
        created_by: Option<UserId>,
    ) -> RepoResult<McpToken> {
        let scopes: Vec<String> = new.scopes.iter().map(|s| s.as_str().to_string()).collect();
        sqlx::query_as!(
            TokenRow,
            "INSERT INTO mcp_token (name, token_hash, prefix, scopes, created_at, created_by, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id, name, prefix, scopes, created_at, created_by, expires_at, last_used_at, revoked_at",
            new.name,
            hash,
            prefix,
            &scopes,
            now,
            created_by.map(|u| u.0),
            new.expires_at(now),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?
        .into_token()
    }

    async fn list(&self) -> RepoResult<Vec<McpToken>> {
        sqlx::query_as!(
            TokenRow,
            "SELECT id, name, prefix, scopes, created_at, created_by, expires_at, last_used_at, revoked_at
             FROM mcp_token ORDER BY created_at DESC, id DESC"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(TokenRow::into_token)
        .collect()
    }

    async fn find_active(&self, hash: &[u8], now: DateTime<Utc>) -> RepoResult<Option<McpToken>> {
        sqlx::query_as!(
            TokenRow,
            "SELECT id, name, prefix, scopes, created_at, created_by, expires_at, last_used_at, revoked_at
             FROM mcp_token
             WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > $2",
            hash,
            now
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?
        .map(TokenRow::into_token)
        .transpose()
    }

    async fn touch(&self, id: McpTokenId, now: DateTime<Utc>) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE mcp_token SET last_used_at = $2
             WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2::timestamptz - interval '1 minute')",
            id.0,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn revoke(&self, id: McpTokenId, now: DateTime<Utc>) -> RepoResult<bool> {
        let done = sqlx::query!(
            "UPDATE mcp_token SET revoked_at = $2
             WHERE id = $1 AND revoked_at IS NULL AND expires_at > $2",
            id.0,
            now
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected() > 0)
    }
}

#[derive(Debug, Clone)]
pub struct PgMcpAudit {
    pool: sqlx::PgPool,
}

impl PgMcpAudit {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl McpAuditRepo for PgMcpAudit {
    async fn record(&self, e: &NewAuditEntry, now: DateTime<Utc>) -> RepoResult<i64> {
        sqlx::query_scalar!(
            "INSERT INTO mcp_audit (at, kind, actor, actor_kind, token_id, tool, params, outcome, detail)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            now,
            e.kind.as_str(),
            e.actor,
            e.actor_kind,
            e.token_id.map(|t| t.0),
            e.tool,
            e.params,
            e.outcome,
            e.detail,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)
    }

    async fn complete(&self, id: i64, outcome: &str, detail: &str) -> RepoResult<()> {
        sqlx::query!(
            "UPDATE mcp_audit SET outcome = $2, detail = $3 WHERE id = $1",
            id,
            outcome,
            detail
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn recent(&self, kind: Option<AuditKind>, limit: i64) -> RepoResult<Vec<McpAuditEntry>> {
        let rows = sqlx::query!(
            "SELECT id, at, kind, actor, actor_kind, token_id, tool, params, outcome, detail
             FROM mcp_audit
             WHERE ($1::text IS NULL OR kind = $1)
             ORDER BY at DESC, id DESC LIMIT $2",
            kind.map(AuditKind::as_str),
            limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(rows
            .into_iter()
            .map(|r| McpAuditEntry {
                id: r.id,
                at: r.at,
                kind: r.kind,
                actor: r.actor,
                actor_kind: r.actor_kind,
                token_id: r.token_id.map(McpTokenId),
                tool: r.tool,
                params: r.params,
                outcome: r.outcome,
                detail: r.detail,
            })
            .collect())
    }

    async fn purge_before(&self, before: DateTime<Utc>) -> RepoResult<u64> {
        let done = sqlx::query!("DELETE FROM mcp_audit WHERE at < $1", before)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(done.rows_affected())
    }
}
