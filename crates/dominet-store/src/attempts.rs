// SPDX-License-Identifier: AGPL-3.0-or-later
//! Log of login attempts (`login_attempt`).

use chrono::{DateTime, Utc};
use dominet_domain::auth::AttemptStats;
use dominet_domain::repo::{LoginAttemptRepo, RepoResult};

use crate::pg::repo_err;

/// PostgreSQL access to the attempts log (implements `LoginAttemptRepo`).
#[derive(Debug, Clone)]
pub struct PgAttempts {
    pool: sqlx::PgPool,
}

impl PgAttempts {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

fn clamp(n: Option<i64>) -> u32 {
    u32::try_from(n.unwrap_or(0)).unwrap_or(u32::MAX)
}

impl LoginAttemptRepo for PgAttempts {
    async fn record(&self, username: &str, ip: &str, succeeded: bool) -> RepoResult<()> {
        sqlx::query!(
            "INSERT INTO login_attempt (username, ip, succeeded) VALUES (lower($1), $2, $3)",
            username,
            ip,
            succeeded
        )
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn stats(
        &self,
        username: &str,
        ip: &str,
        since: DateTime<Utc>,
    ) -> RepoResult<AttemptStats> {
        // The "account" and "pair" counters restart from zero after a successful login of the account.
        let row = sqlx::query!(
            r#"WITH last_ok AS (
                   SELECT coalesce(max(attempted_at), '-infinity'::timestamptz) AS t
                   FROM login_attempt WHERE username = lower($1) AND succeeded
               )
               SELECT
                   count(*) FILTER (WHERE NOT succeeded AND username = lower($1) AND ip = $2
                                    AND attempted_at > (SELECT t FROM last_ok)) AS pair,
                   count(*) FILTER (WHERE NOT succeeded AND username = lower($1)
                                    AND attempted_at > (SELECT t FROM last_ok)) AS by_user,
                   count(*) FILTER (WHERE NOT succeeded AND ip = $2) AS by_ip,
                   max(attempted_at) FILTER (WHERE NOT succeeded) AS last_failure
               FROM login_attempt
               WHERE (username = lower($1) OR ip = $2) AND attempted_at >= $3"#,
            username,
            ip,
            since
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(AttemptStats {
            failures_pair: clamp(row.pair),
            failures_user: clamp(row.by_user),
            failures_ip: clamp(row.by_ip),
            last_failure_at: row.last_failure,
        })
    }

    async fn purge_before(&self, before: DateTime<Utc>) -> RepoResult<u64> {
        let done = sqlx::query!("DELETE FROM login_attempt WHERE attempted_at < $1", before)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(done.rows_affected())
    }
}
