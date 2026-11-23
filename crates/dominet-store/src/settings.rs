// SPDX-License-Identifier: AGPL-3.0-or-later
//! Repository of the application settings (`app_setting`).

use chrono::{DateTime, Utc};
use dominet_domain::RepoError;
use dominet_domain::repo::{RepoResult, SettingRepo};
use dominet_domain::setting::{AppSetting, SettingDraft, SettingType};

use crate::pg::{corrupt, repo_err};

struct SettingRow {
    key: String,
    value: String,
    value_type: String,
    category: String,
    description: String,
    is_secret: bool,
    updated_at: DateTime<Utc>,
}

impl SettingRow {
    fn into_setting(self) -> RepoResult<AppSetting> {
        let value_type = self
            .value_type
            .parse::<SettingType>()
            .map_err(|_| corrupt(format!("type \"{}\"", self.value_type)))?;
        Ok(AppSetting {
            key: self.key,
            value: self.value,
            value_type,
            category: self.category,
            description: self.description,
            is_secret: self.is_secret,
            updated_at: self.updated_at,
        })
    }
}

/// Acces PostgreSQL aux settings (implemente `SettingRepo`).
#[derive(Debug, Clone)]
pub struct PgSettings {
    pool: sqlx::PgPool,
}

impl PgSettings {
    pub(crate) fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

impl SettingRepo for PgSettings {
    async fn list(&self) -> RepoResult<Vec<AppSetting>> {
        sqlx::query_as!(
            SettingRow,
            "SELECT key, value, value_type, category, description, is_secret, updated_at
             FROM app_setting ORDER BY category, key"
        )
        .fetch_all(self.pool())
        .await
        .map_err(repo_err)?
        .into_iter()
        .map(SettingRow::into_setting)
        .collect()
    }

    async fn get(&self, key: &str) -> RepoResult<Option<AppSetting>> {
        sqlx::query_as!(
            SettingRow,
            "SELECT key, value, value_type, category, description, is_secret, updated_at
             FROM app_setting WHERE key = $1",
            key
        )
        .fetch_optional(self.pool())
        .await
        .map_err(repo_err)?
        .map(SettingRow::into_setting)
        .transpose()
    }

    async fn upsert(&self, draft: &SettingDraft, stored_value: &str) -> RepoResult<AppSetting> {
        sqlx::query_as!(
            SettingRow,
            "INSERT INTO app_setting (key, value, value_type, category, description, is_secret)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, value_type = EXCLUDED.value_type,
                 category = EXCLUDED.category, description = EXCLUDED.description, is_secret = EXCLUDED.is_secret
             RETURNING key, value, value_type, category, description, is_secret, updated_at",
            draft.key.as_str(),
            stored_value,
            draft.value_type.as_str(),
            draft.category,
            draft.description,
            draft.is_secret,
        )
        .fetch_one(self.pool())
        .await
        .map_err(repo_err)?
        .into_setting()
    }

    async fn insert_if_absent(&self, draft: &SettingDraft, stored_value: &str) -> RepoResult<bool> {
        let done = sqlx::query!(
            "INSERT INTO app_setting (key, value, value_type, category, description, is_secret)
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (key) DO NOTHING",
            draft.key.as_str(),
            stored_value,
            draft.value_type.as_str(),
            draft.category,
            draft.description,
            draft.is_secret,
        )
        .execute(self.pool())
        .await
        .map_err(repo_err)?;
        Ok(done.rows_affected() == 1)
    }

    async fn delete(&self, key: &str) -> RepoResult<()> {
        let done = sqlx::query!("DELETE FROM app_setting WHERE key = $1", key)
            .execute(self.pool())
            .await
            .map_err(repo_err)?;
        if done.rows_affected() == 0 {
            Err(RepoError::NotFound)
        } else {
            Ok(())
        }
    }
}
